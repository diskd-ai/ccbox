import { useConnectionStore } from "@/stores/connectionStore";
import { useSubscriptionStore } from "@/stores/subscriptionStore";

/**
 * Primary connection hook for components.
 * Provides reactive state + actions.
 */
export function useConnection() {
  const state = useConnectionStore((s) => s.state);
  const url = useConnectionStore((s) => s.url);
  const ccboxInfo = useConnectionStore((s) => s.ccboxInfo);
  const latency = useConnectionStore((s) => s.latency);
  const reconnectAttempt = useConnectionStore((s) => s.reconnectAttempt);
  const error = useConnectionStore((s) => s.error);
  const connect = useConnectionStore((s) => s.connect);
  const disconnect = useConnectionStore((s) => s.disconnect);
  const rpc = useConnectionStore((s) => s.rpc);

  const isConnected = state === "online";

  return {
    state,
    url,
    ccboxInfo,
    latency,
    reconnectAttempt,
    error,
    isConnected,
    connect,
    disconnect,
    rpc,
  };
}

/**
 * Hook for subscription count (status bar).
 */
export function useSubscriptionCount() {
  return useSubscriptionStore((s) => s.getActiveCount());
}
