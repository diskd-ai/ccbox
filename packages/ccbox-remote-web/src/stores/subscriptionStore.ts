import { create } from "zustand";
import { useConnectionStore } from "@/stores/connectionStore";

export interface Subscription {
  readonly id: string;
  readonly rpcMethod: string;
  readonly eventTopic: string;
  readonly params: { [key: string]: unknown };
  readonly cursorParam: string | null;
  readonly eventCursorField: string | null;
  readonly cursor: number | null;
  readonly status: "active" | "error";
}

type EventHandler = (data: unknown) => void;

interface SubscribeArgs {
  readonly rpcMethod: string;
  readonly eventTopic: string;
  readonly params: { [key: string]: unknown };
  readonly cursorParam?: string;
  readonly eventCursorField?: string;
}

interface SubscriptionStore {
  subscriptions: Map<string, Subscription>;
  _handlers: Map<string, Set<EventHandler>>;

  subscribe: (args: SubscribeArgs) => Promise<string>;
  unsubscribe: (id: string) => void;
  clearSubscriptions: () => void;
  handleEvent: (topic: string, data: unknown) => void;
  onEvent: (topic: string, handler: EventHandler) => () => void;
  resubscribeAll: () => void;
  getActiveCount: () => number;
}

let subCounter = 0;

function asNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function extractCursor(value: unknown, field: string | null): number | null {
  if (!field) return null;
  if (typeof value !== "object" || value === null) return null;
  return asNumber((value as { [key: string]: unknown })[field]);
}

function mergeParamsWithCursor(
  params: { [key: string]: unknown },
  cursorParam: string | null,
  cursor: number | null,
): { [key: string]: unknown } {
  if (!cursorParam || cursor === null) return { ...params };
  return { ...params, [cursorParam]: cursor };
}

export const useSubscriptionStore = create<SubscriptionStore>((set, get) => ({
  subscriptions: new Map(),
  _handlers: new Map(),

  subscribe: async ({ rpcMethod, eventTopic, params, cursorParam, eventCursorField }) => {
    const { rpc } = useConnectionStore.getState();
    const id = `sub-${++subCounter}`;

    const result = await rpc(rpcMethod, params);
    const cursor = extractCursor(result, eventCursorField ?? null);

    const next = new Map(get().subscriptions);
    next.set(id, {
      id,
      rpcMethod,
      eventTopic,
      params: { ...params },
      cursorParam: cursorParam ?? null,
      eventCursorField: eventCursorField ?? null,
      cursor,
      status: "active",
    });

    set({ subscriptions: next });
    return id;
  },

  unsubscribe: (id: string) => {
    const next = new Map(get().subscriptions);
    next.delete(id);
    set({ subscriptions: next });
  },

  clearSubscriptions: () => {
    set({ subscriptions: new Map() });
  },

  handleEvent: (topic: string, data: unknown) => {
    const next = new Map(get().subscriptions);

    for (const [id, subscription] of next.entries()) {
      if (subscription.eventTopic !== topic) continue;

      const cursor = extractCursor(data, subscription.eventCursorField);
      if (cursor === null) continue;

      next.set(id, {
        ...subscription,
        cursor,
      });
    }

    set({ subscriptions: next });

    const handlers = get()._handlers.get(topic);
    if (handlers) {
      handlers.forEach((handler) => handler(data));
    }
  },

  onEvent: (topic: string, handler: EventHandler) => {
    const handlers = get()._handlers;
    const current = handlers.get(topic) ?? new Set<EventHandler>();
    current.add(handler);
    handlers.set(topic, current);

    return () => {
      const value = handlers.get(topic);
      if (!value) return;
      value.delete(handler);
      if (value.size === 0) {
        handlers.delete(topic);
      }
    };
  },

  resubscribeAll: () => {
    const { rpc } = useConnectionStore.getState();
    const next = new Map(get().subscriptions);

    for (const [id, subscription] of next.entries()) {
      const params = mergeParamsWithCursor(
        subscription.params,
        subscription.cursorParam,
        subscription.cursor,
      );

      rpc(subscription.rpcMethod, params)
        .then((result) => {
          const updatedCursor = extractCursor(result, subscription.eventCursorField);
          const latest = get().subscriptions.get(id);
          if (!latest) return;

          const updated = new Map(get().subscriptions);
          updated.set(id, {
            ...latest,
            cursor: updatedCursor ?? latest.cursor,
            status: "active",
          });
          set({ subscriptions: updated });
        })
        .catch(() => {
          const latest = get().subscriptions.get(id);
          if (!latest) return;

          const updated = new Map(get().subscriptions);
          updated.set(id, {
            ...latest,
            status: "error",
          });
          set({ subscriptions: updated });
        });
    }
  },

  getActiveCount: () => {
    let count = 0;
    get().subscriptions.forEach((subscription) => {
      if (subscription.status === "active") {
        count += 1;
      }
    });
    return count;
  },
}));
