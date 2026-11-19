import { useEffect, useMemo, useRef, useState } from "react";
import { EngineBadge, StatusDot } from "@/components/StatusIndicators";
import type { Status } from "@/components/StatusIndicators";
import { X, Skull } from "lucide-react";
import { useConnection } from "@/hooks/useConnection";
import { formatError } from "@/domain/errors";
import { useSubscriptionStore } from "@/stores/subscriptionStore";
import {
  fetchProcesses,
  formatTimestamp,
  killProcess,
  parseProcessesLogEvent,
  type ProcessListItem,
} from "@/domain/remoteApi";

type LogStream = "stdout" | "stderr" | "combined";

interface LogRow {
  readonly text: string;
  readonly source: LogStream;
}

function normalizeStatus(status: ProcessListItem["status"]): Status {
  if (status === "running" || status === "exited" || status === "error") return status;
  return "error";
}

const ProcessesPage = () => {
  const { isConnected, rpc } = useConnection();
  const subscribe = useSubscriptionStore((state) => state.subscribe);
  const unsubscribe = useSubscriptionStore((state) => state.unsubscribe);
  const onEvent = useSubscriptionStore((state) => state.onEvent);

  const [processes, setProcesses] = useState<readonly ProcessListItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [stream, setStream] = useState<LogStream>("stdout");
  const [killConfirm, setKillConfirm] = useState<string | null>(null);

  const [logsByProcess, setLogsByProcess] = useState<{ [processId: string]: readonly LogRow[] }>({});
  const [offsets, setOffsets] = useState<{ [offsetKey: string]: number }>({});

  const logEndRef = useRef<HTMLDivElement>(null);

  const selected = selectedId ? processes.find((process) => process.processId === selectedId) ?? null : null;

  const logRows = useMemo(() => {
    if (!selectedId) return [];
    const rows = logsByProcess[selectedId] ?? [];

    if (stream === "combined") return rows;
    return rows.filter((row) => row.source === stream || row.source === "combined");
  }, [selectedId, logsByProcess, stream]);

  const loadProcesses = useMemo(() => {
    return async () => {
      if (!isConnected) {
        setProcesses([]);
        setLoading(false);
        return;
      }

      setLoading(true);
      setError(null);

      try {
        const rows = await fetchProcesses(rpc);
        setProcesses(rows);
      } catch (reason: unknown) {
        setError(formatError(reason));
      } finally {
        setLoading(false);
      }
    };
  }, [isConnected, rpc]);

  useEffect(() => {
    void loadProcesses();
  }, [loadProcesses]);

  useEffect(() => {
    if (!isConnected) return;

    const timer = setInterval(() => {
      void loadProcesses();
    }, 5000);

    return () => {
      clearInterval(timer);
    };
  }, [isConnected, loadProcesses]);

  useEffect(() => {
    const stopListening = onEvent("processes.log", (data) => {
      const event = parseProcessesLogEvent(data);
      if (!event) return;

      const chunks = event.text
        .replace(/\r/g, "")
        .split("\n")
        .filter((line) => line.length > 0)
        .map<LogRow>((line) => ({ text: line, source: event.stream }));

      if (chunks.length === 0) return;

      setLogsByProcess((current) => {
        const previous = current[event.processId] ?? [];
        return {
          ...current,
          [event.processId]: [...previous, ...chunks],
        };
      });

      setOffsets((current) => ({
        ...current,
        [`${event.processId}:${event.stream}`]: event.offset,
      }));
    });

    return () => {
      stopListening();
    };
  }, [onEvent]);

  useEffect(() => {
    if (!isConnected || !selectedId) return;

    let subscriptionId: string | null = null;

    const offsetKey = `${selectedId}:${stream}`;
    const fromOffset = offsets[offsetKey] ?? 0;

    void subscribe({
      rpcMethod: "processes.subscribeLogs",
      eventTopic: "processes.log",
      params: {
        process_id: selectedId,
        stream,
        from_offset: fromOffset,
      },
      cursorParam: "from_offset",
      eventCursorField: "offset",
    })
      .then((id) => {
        subscriptionId = id;
      })
      .catch((reason: unknown) => {
        setError(formatError(reason));
      });

    return () => {
      if (subscriptionId) unsubscribe(subscriptionId);
    };
  }, [isConnected, selectedId, stream, offsets, subscribe, unsubscribe]);

  useEffect(() => {
    logEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [selectedId, stream, logRows.length]);

  const handleKill = async (processId: string) => {
    try {
      await killProcess(rpc, processId);
      setKillConfirm(null);
      await loadProcesses();
    } catch (reason: unknown) {
      setError(formatError(reason));
      setKillConfirm(null);
    }
  };

  const getLineColor = (line: LogRow) => {
    if (line.source === "stderr") return "text-status-error";
    if (line.text.startsWith("$")) return "text-status-online";
    if (line.text.includes("PASS")) return "text-engine-gemini";
    if (line.text.includes("Error") || line.text.includes("error")) return "text-status-error";
    return "text-foreground";
  };

  return (
    <div className="flex flex-col h-full">
      <div className="flex-1 overflow-y-auto p-4 lg:p-6 max-w-[1400px] mx-auto w-full space-y-4">
        <h1 className="text-xl font-sans font-semibold text-foreground">Processes</h1>

        {!isConnected && (
          <div className="text-sm text-text-secondary border border-border bg-surface rounded-xl px-3 py-2">
            Connect to CCBox to load processes.
          </div>
        )}
        {loading && <div className="text-sm text-text-secondary">Loading processes...</div>}
        {error && <div className="text-sm text-status-error">{error}</div>}

        <div className="border border-border bg-surface rounded-xl shadow-card hidden md:block overflow-hidden">
          <div className="grid grid-cols-[40px_80px_1fr_180px_120px_80px] px-3 py-2 border-b border-border text-xs text-text-secondary uppercase tracking-wider font-sans">
            <span></span>
            <span>Engine</span>
            <span>Project</span>
            <span>Started</span>
            <span>ID</span>
            <span></span>
          </div>
          {processes.map((process) => (
            <div
              key={process.processId}
              onClick={() => setSelectedId(selectedId === process.processId ? null : process.processId)}
              className={`grid grid-cols-[40px_80px_1fr_180px_120px_80px] px-3 py-2.5 border-b border-border last:border-b-0 cursor-pointer transition-all ${
                selectedId === process.processId ? "bg-bg-active" : "hover:bg-bg-hover"
              }`}
            >
              <StatusDot status={normalizeStatus(process.status)} />
              <EngineBadge engine={process.engine} short />
              <span className="text-foreground truncate">{process.projectPath}</span>
              <span className="text-text-secondary">{formatTimestamp(process.startedTs)}</span>
              <span className="text-text-muted font-mono text-xs">{process.processId}</span>
              <div className="flex justify-end">
                {process.status === "running" && (
                  <button
                    onClick={(event) => {
                      event.stopPropagation();
                      setKillConfirm(process.processId);
                    }}
                    className="px-2 py-0.5 text-xs border border-status-error text-status-error rounded-md hover:bg-status-error hover:text-primary-foreground transition-colors"
                  >
                    Kill
                  </button>
                )}
              </div>
            </div>
          ))}
          {!loading && processes.length === 0 && (
            <div className="px-3 py-4 text-sm text-text-muted">No active processes returned.</div>
          )}
        </div>

        <div className="md:hidden space-y-2">
          {processes.map((process) => (
            <div
              key={process.processId}
              onClick={() => setSelectedId(selectedId === process.processId ? null : process.processId)}
              className={`border border-border bg-surface px-3 py-2.5 space-y-1 rounded-xl shadow-card cursor-pointer transition-all ${
                selectedId === process.processId ? "border-primary shadow-elevated" : "hover:bg-bg-hover"
              }`}
            >
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2 min-w-0">
                  <StatusDot status={normalizeStatus(process.status)} />
                  <EngineBadge engine={process.engine} short />
                  <span className="text-foreground text-sm truncate">{process.projectPath}</span>
                </div>
                {process.status === "running" && (
                  <button
                    onClick={(event) => {
                      event.stopPropagation();
                      setKillConfirm(process.processId);
                    }}
                    className="px-2 py-0.5 text-xs border border-status-error text-status-error rounded-md hover:bg-status-error hover:text-primary-foreground transition-colors"
                  >
                    Kill
                  </button>
                )}
              </div>
              <div className="text-xs text-text-secondary">{process.processId} · {formatTimestamp(process.startedTs)}</div>
            </div>
          ))}
        </div>
      </div>

      {selected && (
        <div className="border-t border-border bg-surface shadow-elevated shrink-0 animate-in slide-in-from-bottom duration-200">
          <div className="flex items-center justify-between px-4 py-2 border-b border-border">
            <div className="flex items-center gap-3 min-w-0">
              <div className="flex items-center gap-2 min-w-0">
                <StatusDot status={normalizeStatus(selected.status)} />
                <EngineBadge engine={selected.engine} short />
                <span className="text-sm font-medium text-foreground truncate">{selected.projectPath}</span>
              </div>
              <span className="text-xs text-text-muted font-mono">{selected.processId}</span>
            </div>

            <div className="flex items-center gap-2">
              <div className="flex items-center gap-0.5 bg-background rounded-lg p-0.5 border border-border">
                {(["stdout", "stderr", "combined"] as const).map((value) => (
                  <button
                    key={value}
                    onClick={() => setStream(value)}
                    className={`px-2.5 py-1 text-xs font-sans font-medium transition-all rounded-md ${
                      stream === value
                        ? "bg-primary text-primary-foreground shadow-card"
                        : "text-text-secondary hover:text-foreground"
                    }`}
                  >
                    {value}
                  </button>
                ))}
              </div>
              <button
                onClick={() => setSelectedId(null)}
                className="text-text-secondary hover:text-foreground transition-colors p-1"
              >
                <X className="w-4 h-4" />
              </button>
            </div>
          </div>

          <div className="px-4 py-2 font-mono text-sm space-y-px max-h-64 overflow-y-auto bg-background">
            {logRows.length === 0 ? (
              <div className="py-4 text-center text-text-muted text-xs">No log output for this stream.</div>
            ) : (
              logRows.map((line, index) => (
                <div key={`${index}-${line.text.slice(0, 12)}`} className="flex hover:bg-bg-hover/50 rounded px-1 -mx-1">
                  <span className="text-text-muted w-7 shrink-0 select-none text-right mr-3 tabular-nums">{index + 1}</span>
                  <span className={getLineColor(line)}>{line.text}</span>
                </div>
              ))
            )}
            <div ref={logEndRef} />
          </div>
        </div>
      )}

      {killConfirm && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-background/60 backdrop-blur-sm">
          <div className="bg-surface border border-border rounded-xl shadow-dropdown p-5 max-w-sm w-full mx-4 space-y-4">
            <div className="flex items-center gap-3">
              <div className="w-10 h-10 rounded-full bg-status-error/10 flex items-center justify-center">
                <Skull className="w-5 h-5 text-status-error" />
              </div>
              <div>
                <h3 className="text-sm font-sans font-semibold text-foreground">Kill Process</h3>
                <p className="text-xs text-text-secondary">This will terminate process {killConfirm}.</p>
              </div>
            </div>
            <div className="flex items-center justify-end gap-2">
              <button
                onClick={() => setKillConfirm(null)}
                className="px-3 py-1.5 text-sm font-sans border border-border text-text-secondary rounded-lg hover:text-foreground hover:bg-bg-hover transition-colors"
              >
                Cancel
              </button>
              <button
                onClick={() => void handleKill(killConfirm)}
                className="px-4 py-1.5 text-sm font-sans font-medium bg-status-error text-primary-foreground rounded-lg hover:opacity-90 transition-opacity shadow-card"
              >
                Kill Process
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};

export default ProcessesPage;
