import { cn } from "@/lib/utils";

type Engine = "codex" | "claude" | "gemini" | "opencode";

const engineConfig: Record<Engine, { label: string; short: string; colorClass: string }> = {
  codex: { label: "Codex", short: "Cdx", colorClass: "bg-engine-codex" },
  claude: { label: "Claude", short: "Cld", colorClass: "bg-engine-claude" },
  gemini: { label: "Gemini", short: "Gem", colorClass: "bg-engine-gemini" },
  opencode: { label: "OpenCode", short: "OC", colorClass: "bg-engine-opencode" },
};

interface EngineBadgeProps {
  engine: Engine;
  short?: boolean;
  className?: string;
}

export function EngineBadge({ engine, short = false, className }: EngineBadgeProps) {
  const config = engineConfig[engine];
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 px-1.5 py-0.5 text-xs font-medium rounded-md",
        className
      )}
    >
      <span className={cn("w-1.5 h-1.5 rounded-full shrink-0", config.colorClass)} />
      <span className="text-foreground">{short ? config.short : config.label}</span>
    </span>
  );
}

type Status = "running" | "online" | "exited" | "error";

const statusConfig: Record<Status, { colorClass: string; glowClass: string; pulse: boolean }> = {
  running: { colorClass: "bg-status-running", glowClass: "glow-running", pulse: true },
  online: { colorClass: "bg-status-online", glowClass: "glow-online", pulse: false },
  exited: { colorClass: "bg-status-exited", glowClass: "", pulse: false },
  error: { colorClass: "bg-status-error", glowClass: "glow-error", pulse: false },
};

interface StatusDotProps {
  status: Status;
  className?: string;
}

export function StatusDot({ status, className }: StatusDotProps) {
  const config = statusConfig[status];
  return (
    <span
      className={cn(
        "w-2 h-2 rounded-full inline-block shrink-0",
        config.colorClass,
        config.glowClass,
        config.pulse && "animate-pulse-dot",
        className
      )}
    />
  );
}

export type { Engine, Status };
