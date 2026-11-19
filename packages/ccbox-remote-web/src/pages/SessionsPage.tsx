import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { EngineBadge } from "@/components/StatusIndicators";
import type { Engine } from "@/components/StatusIndicators";
import { useConnection } from "@/hooks/useConnection";
import { formatError } from "@/domain/errors";
import { fetchAllSessions, formatTimestamp, type SessionListItem } from "@/domain/remoteApi";

const engines: readonly (Engine | "all")[] = ["all", "codex", "claude", "gemini", "opencode"];

const SessionsPage = () => {
  const navigate = useNavigate();
  const { isConnected, rpc } = useConnection();

  const [engineFilter, setEngineFilter] = useState<Engine | "all">("all");
  const [sessions, setSessions] = useState<readonly SessionListItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!isConnected) {
      setSessions([]);
      setLoading(false);
      setError(null);
      return;
    }

    let cancelled = false;
    setLoading(true);
    setError(null);

    void fetchAllSessions(rpc)
      .then((rows) => {
        if (cancelled) return;
        setSessions(rows);
      })
      .catch((reason: unknown) => {
        if (cancelled) return;
        setError(formatError(reason));
      })
      .finally(() => {
        if (cancelled) return;
        setLoading(false);
      });

    return () => {
      cancelled = true;
    };
  }, [isConnected, rpc]);

  const filtered = useMemo(() => {
    if (engineFilter === "all") return sessions;
    return sessions.filter((session) => session.engine === engineFilter);
  }, [sessions, engineFilter]);

  return (
    <div className="p-4 lg:p-6 max-w-[1400px] mx-auto space-y-4">
      <h1 className="text-xl font-sans font-semibold text-foreground">Sessions</h1>

      {!isConnected && (
        <div className="text-sm text-text-secondary border border-border bg-surface rounded-xl px-3 py-2">
          Connect to CCBox to load sessions.
        </div>
      )}
      {loading && <div className="text-sm text-text-secondary">Loading sessions...</div>}
      {error && <div className="text-sm text-status-error">{error}</div>}

      <div className="flex items-center gap-1.5 overflow-x-auto pb-1">
        {engines.map((engine) => (
          <button
            key={engine}
            onClick={() => setEngineFilter(engine)}
            className={`px-3 py-1 text-xs font-sans font-medium transition-all shrink-0 border rounded-lg ${
              engineFilter === engine
                ? "border-primary bg-primary text-primary-foreground shadow-card"
                : "border-border bg-surface text-text-secondary hover:text-foreground hover:bg-bg-hover"
            }`}
          >
            {engine === "all" ? "All" : engine.charAt(0).toUpperCase() + engine.slice(1)}
          </button>
        ))}
      </div>

      <div className="border border-border bg-surface rounded-xl shadow-card hidden md:block overflow-hidden">
        <div className="grid grid-cols-[80px_180px_1fr_200px_140px] px-3 py-2 border-b border-border text-xs text-text-secondary uppercase tracking-wider font-sans">
          <span>Engine</span>
          <span>Started</span>
          <span>Title</span>
          <span>Project</span>
          <span>Session ID</span>
        </div>
        {filtered.map((session) => (
          <div
            key={session.sessionId}
            onClick={() => navigate(`/sessions/${session.sessionId}`)}
            className="grid grid-cols-[80px_180px_1fr_200px_140px] px-3 py-2.5 border-b border-border last:border-b-0 hover:bg-bg-hover cursor-pointer transition-colors"
          >
            <EngineBadge engine={session.engine} short />
            <span className="text-text-secondary">{formatTimestamp(session.startedTs)}</span>
            <span className="text-foreground truncate">{session.title || "(untitled)"}</span>
            <span
              onClick={(event) => {
                event.stopPropagation();
                navigate(`/projects/${encodeURIComponent(session.projectPath)}`);
              }}
              className="text-text-secondary truncate hover:text-foreground"
            >
              {session.projectPath}
            </span>
            <span className="text-text-muted font-mono text-xs">{session.sessionId}</span>
          </div>
        ))}
        {!loading && filtered.length === 0 && (
          <div className="px-3 py-6 text-center text-sm text-text-muted">No sessions returned.</div>
        )}
      </div>

      <div className="md:hidden space-y-2">
        {filtered.map((session) => (
          <div
            key={session.sessionId}
            onClick={() => navigate(`/sessions/${session.sessionId}`)}
            className="border border-border bg-surface px-3 py-2.5 space-y-1 rounded-xl shadow-card cursor-pointer hover:bg-bg-hover hover:shadow-elevated transition-all"
          >
            <div className="flex items-center gap-2">
              <EngineBadge engine={session.engine} short />
              <span className="text-foreground text-sm truncate">{session.title || "(untitled)"}</span>
            </div>
            <div className="flex items-center gap-3 text-xs text-text-secondary">
              <span>{formatTimestamp(session.startedTs)}</span>
              <span className="text-text-muted truncate">{session.projectPath}</span>
            </div>
          </div>
        ))}
      </div>

      <div className="text-xs text-text-secondary">Showing {filtered.length} of {sessions.length} sessions</div>
    </div>
  );
};

export default SessionsPage;
