import { create } from "zustand";
import type { CcboxInfo, ConnectionState, DeviceIdentity } from "@/domain/types";
import {
  isEnvelope,
  makeEnvelope,
  parseAuthChallengePayload,
  parseAuthErrPayload,
  parseAuthOkPayload,
  parseEventPayload,
  parseRpcResponsePayload,
} from "@/domain/protocol";
import { toWsUrl } from "@/domain/url";
import { getOrCreateIdentity, signChallenge } from "@/domain/identity";
import { parseCcboxInfo } from "@/domain/remoteApi";
import { useSubscriptionStore } from "@/stores/subscriptionStore";

interface PendingRpc {
  readonly resolve: (result: unknown) => void;
  readonly reject: (error: { code: string; message: string }) => void;
  readonly timer: ReturnType<typeof setTimeout>;
}

interface ConnectionStore {
  state: ConnectionState;
  url: string;
  ccboxInfo: CcboxInfo | null;
  latency: number;
  reconnectAttempt: number;
  error: string | null;

  _ws: WebSocket | null;
  _identity: DeviceIdentity | null;
  _pending: Map<string, PendingRpc>;
  _reconnectTimer: ReturnType<typeof setTimeout> | null;
  _latencyTimer: ReturnType<typeof setInterval> | null;
  _manualDisconnect: boolean;

  connect: (urlInput: string) => Promise<void>;
  disconnect: () => void;
  rpc: <T = unknown>(method: string, params?: unknown) => Promise<T>;

  _setState: (state: ConnectionState) => void;
  _handleMessage: (data: string) => Promise<void>;
  _scheduleReconnect: () => void;
  _measureLatency: () => void;
  _refreshCcboxInfo: () => Promise<void>;
}

const RECONNECT_BASE_MS = 250;
const RECONNECT_MAX_MS = 30_000;
const RPC_TIMEOUT_MS = 15_000;

function backoffWithJitter(attempt: number): number {
  const delay = Math.min(RECONNECT_BASE_MS * Math.pow(2, attempt), RECONNECT_MAX_MS);
  return delay + Math.random() * delay * 0.25;
}

function rejectAllPending(
  pending: Map<string, PendingRpc>,
  error: { code: string; message: string },
): Map<string, PendingRpc> {
  pending.forEach((entry) => {
    clearTimeout(entry.timer);
    entry.reject(error);
  });
  return new Map();
}

export const useConnectionStore = create<ConnectionStore>((set, get) => ({
  state: "disconnected",
  url: sessionStorage.getItem("ccbox-url") || "",
  ccboxInfo: null,
  latency: 0,
  reconnectAttempt: 0,
  error: null,

  _ws: null,
  _identity: null,
  _pending: new Map(),
  _reconnectTimer: null,
  _latencyTimer: null,
  _manualDisconnect: false,

  _setState: (state) => set({ state }),

  connect: async (urlInput: string) => {
    const store = get();

    if (store._reconnectTimer) clearTimeout(store._reconnectTimer);
    if (store._latencyTimer) clearInterval(store._latencyTimer);
    if (store._ws) {
      store._ws.onclose = null;
      store._ws.close();
    }

    const pending = rejectAllPending(store._pending, { code: "Disconnected", message: "Disconnected" });

    const trimmed = urlInput.trim();
    if (!trimmed) {
      set({
        state: "disconnected",
        error: "Missing connect value",
        _ws: null,
        _pending: pending,
      });
      return;
    }

    let wsUrl: string;
    try {
      wsUrl = toWsUrl(trimmed);
    } catch (error) {
      set({
        state: "disconnected",
        error: String(error),
        _ws: null,
        _pending: pending,
      });
      return;
    }

    sessionStorage.setItem("ccbox-url", trimmed);
    const reconnectAttempt = store.state === "reconnecting" ? store.reconnectAttempt : 0;

    set({
      state: "connecting",
      url: trimmed,
      ccboxInfo: null,
      latency: 0,
      reconnectAttempt,
      error: null,
      _pending: pending,
      _manualDisconnect: false,
      _reconnectTimer: null,
      _latencyTimer: null,
    });

    try {
      const ws = new WebSocket(wsUrl);
      set({ _ws: ws });

      ws.onopen = async () => {
        try {
          const identity = await getOrCreateIdentity();
          set({ _identity: identity, state: "awaiting_challenge" });

          if (ws.readyState === WebSocket.OPEN) {
            ws.send(
              JSON.stringify(
                makeEnvelope("auth/hello", {
                  device_id: identity.device_id,
                  device_kind: "client",
                }),
              ),
            );
          }
        } catch (error) {
          set({ state: "auth_failed", error: `Failed to load identity: ${String(error)}` });
          ws.close();
        }
      };

      ws.onmessage = (event) => {
        void get()._handleMessage(String(event.data));
      };

      ws.onclose = () => {
        const current = get();
        const pendingAfterClose = rejectAllPending(current._pending, {
          code: "Disconnected",
          message: "Socket closed",
        });

        if (current._latencyTimer) clearInterval(current._latencyTimer);

        set({ _ws: null, _pending: pendingAfterClose, _latencyTimer: null });

        if (current._manualDisconnect) {
          set({ state: "disconnected", _manualDisconnect: false });
          return;
        }

        if (current.state === "auth_failed") {
          return;
        }

        if (
          current.state === "online" ||
          current.state === "reconnecting" ||
          current.state === "connecting" ||
          current.state === "awaiting_challenge" ||
          current.state === "authenticating"
        ) {
          get()._scheduleReconnect();
          return;
        }

        if (current.state !== "disconnected") {
          set({ state: "disconnected" });
        }
      };

      ws.onerror = () => {
        // onclose handles state transitions.
      };
    } catch (error) {
      set({ state: "disconnected", error: `Failed to create WebSocket: ${String(error)}` });
    }
  },

  disconnect: () => {
    const store = get();

    if (store._reconnectTimer) clearTimeout(store._reconnectTimer);
    if (store._latencyTimer) clearInterval(store._latencyTimer);

    const pending = rejectAllPending(store._pending, { code: "Disconnected", message: "Disconnected" });

    if (store._ws) {
      store._ws.onclose = null;
      store._ws.close();
    }

    set({
      state: "disconnected",
      _ws: null,
      _pending: pending,
      _reconnectTimer: null,
      _latencyTimer: null,
      _manualDisconnect: false,
      ccboxInfo: null,
      latency: 0,
      reconnectAttempt: 0,
      error: null,
    });
  },

  _handleMessage: async (data: string) => {
    let parsed: unknown;
    try {
      parsed = JSON.parse(data);
    } catch {
      console.warn("[ccbox-remote-web] Failed to parse websocket message");
      return;
    }

    if (!isEnvelope(parsed)) return;

    const challenge = parseAuthChallengePayload(parsed);
    if (challenge) {
      set({ state: "authenticating" });
      const ws = get()._ws;
      if (!ws || ws.readyState !== WebSocket.OPEN) return;

      try {
        const identity = get()._identity ?? (await getOrCreateIdentity());
        set({ _identity: identity });
        const signature_b64 = await signChallenge(identity, challenge.nonce_b64);

        ws.send(
          JSON.stringify(
            makeEnvelope("auth/response", {
              signature_b64,
              public_key_b64: identity.public_key_b64,
            }),
          ),
        );
      } catch (error) {
        set({ state: "auth_failed", error: `Failed to sign challenge: ${String(error)}` });
        ws.close();
      }
      return;
    }

    const authOk = parseAuthOkPayload(parsed);
    if (authOk) {
      if (get()._latencyTimer) clearInterval(get()._latencyTimer);

      const timer = setInterval(() => {
        get()._measureLatency();
      }, 15_000);

      set({
        state: "online",
        reconnectAttempt: 0,
        error: null,
        _latencyTimer: timer,
      });

      await get()._refreshCcboxInfo();
      useSubscriptionStore.getState().resubscribeAll();
      return;
    }

    const authErr = parseAuthErrPayload(parsed);
    if (authErr) {
      set({
        state: "auth_failed",
        error: authErr.code,
      });
      return;
    }

    const rpcResponse = parseRpcResponsePayload(parsed);
    if (rpcResponse) {
      const pending = get()._pending.get(rpcResponse.id);
      if (!pending) return;

      clearTimeout(pending.timer);
      const next = new Map(get()._pending);
      next.delete(rpcResponse.id);
      set({ _pending: next });

      if (rpcResponse.ok) {
        pending.resolve(rpcResponse.result);
      } else {
        pending.reject(rpcResponse.error);
      }
      return;
    }

    const eventPayload = parseEventPayload(parsed);
    if (eventPayload) {
      useSubscriptionStore.getState().handleEvent(eventPayload.topic, eventPayload.data);
    }
  },

  rpc: <T = unknown>(method: string, params?: unknown): Promise<T> => {
    return new Promise<T>((resolve, reject) => {
      const store = get();
      const ws = store._ws;

      if (store.state !== "online" || !ws || ws.readyState !== WebSocket.OPEN) {
        reject({ code: "NotConnected", message: "Not connected" });
        return;
      }

      const id = crypto.randomUUID();
      const timer = setTimeout(() => {
        const next = new Map(get()._pending);
        next.delete(id);
        set({ _pending: next });
        reject({ code: "Timeout", message: "RPC timeout" });
      }, RPC_TIMEOUT_MS);

      const nextPending = new Map(store._pending);
      nextPending.set(id, {
        resolve: resolve as (result: unknown) => void,
        reject,
        timer,
      });
      set({ _pending: nextPending });

      ws.send(
        JSON.stringify(
          makeEnvelope("rpc/request", {
            id,
            method,
            params: params ?? {},
          }),
        ),
      );
    });
  },

  _scheduleReconnect: () => {
    const store = get();
    if (!store.url) {
      set({ state: "disconnected" });
      return;
    }

    const attempt = store.reconnectAttempt + 1;
    const delay = backoffWithJitter(attempt);

    set({ state: "reconnecting", reconnectAttempt: attempt });

    const timer = setTimeout(() => {
      const current = get();
      if (current.state === "reconnecting") {
        void current.connect(current.url);
      }
    }, delay);

    set({ _reconnectTimer: timer });
  },

  _measureLatency: () => {
    const start = performance.now();

    get()
      .rpc("ccbox.getInfo", {})
      .then(() => {
        set({ latency: Math.round(performance.now() - start) });
      })
      .catch(() => {
        // Ignore transient latency failures.
      });
  },

  _refreshCcboxInfo: async () => {
    try {
      const result = await get().rpc("ccbox.getInfo", {});
      const parsed = parseCcboxInfo(result);
      if (parsed) {
        set({ ccboxInfo: parsed });
      }
    } catch {
      // keep ccboxInfo null on failure
    }
  },
}));
