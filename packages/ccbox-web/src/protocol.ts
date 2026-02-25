import { base64ToBytes, bytesToBase64 } from "./base64";
import { buildAuthMessageV1 } from "./authMessage";
import type { DeviceIdentity } from "./identity";
import { sign } from "./identity";

type Envelope = {
  readonly v: 1;
  readonly type: string;
  readonly ts: string;
  readonly payload: unknown;
};

type AuthHello = {
  readonly v: 1;
  readonly type: "auth/hello";
  readonly ts: string;
  readonly payload: { readonly device_id: string; readonly device_kind: "client" };
};

type AuthChallenge = {
  readonly v: 1;
  readonly type: "auth/challenge";
  readonly ts: string;
  readonly payload: { readonly nonce_b64: string; readonly expires_in_ms: number };
};

type AuthOk = {
  readonly v: 1;
  readonly type: "auth/ok";
  readonly ts: string;
  readonly payload: { readonly device_id: string };
};

type AuthErr = {
  readonly v: 1;
  readonly type: "auth/err";
  readonly ts: string;
  readonly payload: { readonly code: string };
};

type RpcRequest = {
  readonly v: 1;
  readonly type: "rpc/request";
  readonly ts: string;
  readonly payload: { readonly id: string; readonly method: string; readonly params: unknown };
};

type RpcResponsePayloadOk = { readonly id: string; readonly ok: true; readonly result: unknown };
type RpcResponsePayloadErr = {
  readonly id: string;
  readonly ok: false;
  readonly error: { readonly code: string; readonly message: string };
};
type RpcResponsePayload = RpcResponsePayloadOk | RpcResponsePayloadErr;

type RpcResponse = {
  readonly v: 1;
  readonly type: "rpc/response";
  readonly ts: string;
  readonly payload: RpcResponsePayload;
};

type Event = {
  readonly v: 1;
  readonly type: "event";
  readonly ts: string;
  readonly payload: { readonly topic: string; readonly data: unknown };
};

export type RemoteClient = {
  readonly wsUrl: string;
  close: () => void;
  rpc: (method: string, params: unknown) => Promise<unknown>;
};

function nowTs(): string {
  return new Date().toISOString();
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isEnvelope(value: unknown): value is Envelope {
  if (!isObject(value)) return false;
  return value.v === 1 && typeof value.type === "string" && typeof value.ts === "string";
}

function isAuthChallenge(value: Envelope): value is AuthChallenge {
  if (value.type !== "auth/challenge") return false;
  if (!isObject(value.payload)) return false;
  return (
    typeof value.payload.nonce_b64 === "string" && typeof value.payload.expires_in_ms === "number"
  );
}

function isAuthOk(value: Envelope): value is AuthOk {
  if (value.type !== "auth/ok") return false;
  if (!isObject(value.payload)) return false;
  return typeof value.payload.device_id === "string";
}

function isAuthErr(value: Envelope): value is AuthErr {
  if (value.type !== "auth/err") return false;
  if (!isObject(value.payload)) return false;
  return typeof value.payload.code === "string";
}

function isRpcResponse(value: Envelope): value is RpcResponse {
  if (value.type !== "rpc/response") return false;
  if (!isObject(value.payload)) return false;
  if (typeof value.payload.id !== "string") return false;
  if (typeof value.payload.ok !== "boolean") return false;
  if (value.payload.ok === true) return true;
  if (!isObject(value.payload.error)) return false;
  return typeof value.payload.error.code === "string" && typeof value.payload.error.message === "string";
}

function isEvent(value: Envelope): value is Event {
  if (value.type !== "event") return false;
  if (!isObject(value.payload)) return false;
  return typeof value.payload.topic === "string" && "data" in value.payload;
}

type Hooks = {
  readonly onStatus: (s: string) => void;
  readonly onError: (e: string) => void;
  readonly onEvent?: (topic: string, data: unknown) => void;
};

export async function connectRemoteClient(
  wsUrl: string,
  identity: DeviceIdentity,
  hooks: Hooks,
): Promise<RemoteClient> {
  hooks.onStatus("connecting");

  const ws = new WebSocket(wsUrl);
  const pending = new Map<string, (payload: RpcResponsePayload) => void>();

  let authenticated = false;

  let resolveAuth: () => void = () => {};
  let rejectAuth: (err: Error) => void = () => {};
  const authPromise = new Promise<void>((resolve, reject) => {
    resolveAuth = resolve;
    rejectAuth = reject;
  });

  function send(obj: unknown) {
    ws.send(JSON.stringify(obj));
  }

  await new Promise<void>((resolve, reject) => {
    ws.onopen = () => resolve();
    ws.onerror = () => reject(new Error("WebSocket error"));
  });

  hooks.onStatus("socket open; authenticating");

  ws.onmessage = async (event) => {
    let parsed: unknown;
    try {
      parsed = JSON.parse(String(event.data));
    } catch (err) {
      hooks.onError(`Bad JSON message: ${String(err)}`);
      return;
    }

    if (!isEnvelope(parsed)) return;

    if (isAuthChallenge(parsed)) {
      const nonceBytes = base64ToBytes(parsed.payload.nonce_b64);
      const message = buildAuthMessageV1("client", identity.deviceId, nonceBytes);
      const signature = await sign(identity, message);
      send({
        v: 1,
        type: "auth/response",
        ts: nowTs(),
        payload: { signature_b64: bytesToBase64(signature) },
      });
      return;
    }

    if (isAuthOk(parsed)) {
      authenticated = true;
      hooks.onStatus("authenticated");
      resolveAuth();
      return;
    }

    if (isAuthErr(parsed)) {
      const msg = `Auth failed: ${parsed.payload.code}`;
      hooks.onError(msg);
      rejectAuth(new Error(msg));
      ws.close();
      return;
    }

    if (isRpcResponse(parsed)) {
      const handler = pending.get(parsed.payload.id);
      if (handler) {
        pending.delete(parsed.payload.id);
        handler(parsed.payload);
      }
      return;
    }

    if (isEvent(parsed)) {
      hooks.onEvent?.(parsed.payload.topic, parsed.payload.data);
    }
  };

  ws.onclose = () => {
    hooks.onStatus(authenticated ? "disconnected" : "disconnected (unauthenticated)");
    if (!authenticated) rejectAuth(new Error("Disconnected before authentication completed"));
    for (const [id, handler] of pending) {
      pending.delete(id);
      handler({ id, ok: false, error: { code: "Disconnected", message: "socket closed" } });
    }
  };

  const hello: AuthHello = {
    v: 1,
    type: "auth/hello",
    ts: nowTs(),
    payload: { device_id: identity.deviceId, device_kind: "client" },
  };
  send(hello);

  await authPromise;

  const client: RemoteClient = {
    wsUrl,
    close: () => ws.close(),
    rpc: (method, params) => {
      if (!authenticated) return Promise.reject(new Error("NotAuthenticated"));
      const id = crypto.randomUUID();
      const req: RpcRequest = {
        v: 1,
        type: "rpc/request",
        ts: nowTs(),
        payload: { id, method, params },
      };

      return new Promise<unknown>((resolve, reject) => {
        pending.set(id, (payload) => {
          if (payload.ok) resolve(payload.result);
          else reject(new Error(`${payload.error.code}: ${payload.error.message}`));
        });
        send(req);
      });
    },
  };

  return client;
}
