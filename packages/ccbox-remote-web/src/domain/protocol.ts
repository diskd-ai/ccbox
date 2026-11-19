export const REMOTE_PROTOCOL_VERSION = 1 as const;

export interface EnvelopeIn {
  readonly v: number;
  readonly type: string;
  readonly ts: string;
  readonly payload: unknown;
}

export interface EnvelopeOut<TPayload> {
  readonly v: typeof REMOTE_PROTOCOL_VERSION;
  readonly type: string;
  readonly ts: string;
  readonly payload: TPayload;
}

export interface AuthHelloPayload {
  readonly device_id: string;
  readonly device_kind: "client";
}

export interface AuthChallengePayload {
  readonly nonce_b64: string;
  readonly expires_in_ms: number;
}

export interface AuthResponsePayload {
  readonly signature_b64: string;
  readonly public_key_b64?: string;
}

export interface AuthOkPayload {
  readonly device_id: string;
}

export interface AuthErrPayload {
  readonly code: string;
}

export interface RpcRequestPayload {
  readonly id: string;
  readonly method: string;
  readonly params: unknown;
}

export interface RpcErrorPayload {
  readonly code: string;
  readonly message: string;
}

export interface RpcResponsePayloadOk {
  readonly id: string;
  readonly ok: true;
  readonly result: unknown;
}

export interface RpcResponsePayloadErr {
  readonly id: string;
  readonly ok: false;
  readonly error: RpcErrorPayload;
}

export type RpcResponsePayload = RpcResponsePayloadOk | RpcResponsePayloadErr;

export interface EventPayload {
  readonly topic: string;
  readonly data: unknown;
}

function isObject(value: unknown): value is { [key: string]: unknown } {
  return typeof value === "object" && value !== null;
}

function asString(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

function asNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

export function nowIso(): string {
  return new Date().toISOString();
}

export function makeEnvelope<TPayload>(type: string, payload: TPayload): EnvelopeOut<TPayload> {
  return {
    v: REMOTE_PROTOCOL_VERSION,
    type,
    ts: nowIso(),
    payload,
  };
}

export function isEnvelope(value: unknown): value is EnvelopeIn {
  if (!isObject(value)) return false;
  return typeof value.v === "number" && typeof value.type === "string" && typeof value.ts === "string" && "payload" in value;
}

export function parseAuthChallengePayload(env: EnvelopeIn): AuthChallengePayload | null {
  if (env.type !== "auth/challenge") return null;
  if (!isObject(env.payload)) return null;

  const nonce_b64 = asString(env.payload.nonce_b64);
  const expires_in_ms = asNumber(env.payload.expires_in_ms);
  if (!nonce_b64 || expires_in_ms === null) return null;
  return { nonce_b64, expires_in_ms };
}

export function parseAuthOkPayload(env: EnvelopeIn): AuthOkPayload | null {
  if (env.type !== "auth/ok") return null;
  if (!isObject(env.payload)) return null;

  const device_id = asString(env.payload.device_id);
  if (!device_id) return null;
  return { device_id };
}

export function parseAuthErrPayload(env: EnvelopeIn): AuthErrPayload | null {
  if (env.type !== "auth/err") return null;
  if (!isObject(env.payload)) return null;

  const code = asString(env.payload.code);
  if (!code) return null;
  return { code };
}

export function parseRpcResponsePayload(env: EnvelopeIn): RpcResponsePayload | null {
  if (env.type !== "rpc/response") return null;
  if (!isObject(env.payload)) return null;

  const id = asString(env.payload.id);
  if (!id) return null;

  if (env.payload.ok === true) {
    return {
      id,
      ok: true,
      result: env.payload.result,
    };
  }

  if (env.payload.ok === false && isObject(env.payload.error)) {
    const code = asString(env.payload.error.code);
    const message = asString(env.payload.error.message);
    if (!code || !message) return null;

    return {
      id,
      ok: false,
      error: { code, message },
    };
  }

  return null;
}

export function parseEventPayload(env: EnvelopeIn): EventPayload | null {
  if (env.type !== "event") return null;
  if (!isObject(env.payload)) return null;

  const topic = asString(env.payload.topic);
  if (!topic) return null;

  return {
    topic,
    data: env.payload.data,
  };
}
