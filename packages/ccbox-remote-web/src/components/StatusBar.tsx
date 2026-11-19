import { cn } from "@/lib/utils";
import { useConnection, useSubscriptionCount } from "@/hooks/useConnection";
import type { ConnectionState } from "@/domain/types";

interface StatusBarProps {
  className?: string;
}

const STATE_DOT_CLASS: Record<ConnectionState, string> = {
  disconnected: "bg-text-muted",
  connecting: "bg-status-warning animate-pulse-dot glow-warning",
  awaiting_challenge: "bg-status-warning animate-pulse-dot glow-warning",
  authenticating: "bg-status-warning animate-pulse-dot glow-warning",
  online: "bg-status-online glow-online",
  reconnecting: "bg-status-warning animate-pulse-dot glow-warning",
  auth_failed: "bg-status-error glow-error",
};

const STATE_LABEL: Record<ConnectionState, string> = {
  disconnected: "Disconnected",
  connecting: "Connecting...",
  awaiting_challenge: "Awaiting Challenge...",
  authenticating: "Auth...",
  online: "Online",
  reconnecting: "Reconnecting...",
  auth_failed: "Auth Failed",
};

export function StatusBar({ className }: StatusBarProps) {
  const { state, ccboxInfo, latency } = useConnection();
  const subCount = useSubscriptionCount();

  return (
    <div
      className={cn(
        "items-center justify-between border-t border-border px-4 h-7 shrink-0 bg-surface text-xs text-text-secondary font-mono shadow-card",
        className
      )}
    >
      <div className="flex items-center gap-4">
        <span className="flex items-center gap-1.5">
          <span className={cn("w-1.5 h-1.5 rounded-full inline-block", STATE_DOT_CLASS[state])} />
          {STATE_LABEL[state]}
        </span>
        {ccboxInfo && (
          <>
            <span className="text-text-muted">|</span>
            <span>
              ccbox: <span className="text-foreground">{ccboxInfo.label}</span> v{ccboxInfo.version}
            </span>
          </>
        )}
        {state === "online" && (
          <>
            <span className="text-text-muted">|</span>
            <span>
              latency: <span className="text-foreground">{latency}ms</span>
            </span>
          </>
        )}
      </div>
      <div className="flex items-center gap-4">
        <span>
          subs: <span className="text-foreground">{subCount}</span>
        </span>
        {ccboxInfo && ccboxInfo.capabilities.length > 0 && (
          <span>
            caps: <span className="text-foreground">{ccboxInfo.capabilities.join(", ")}</span>
          </span>
        )}
      </div>
    </div>
  );
}
