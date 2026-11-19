import { useEffect, useMemo, useState } from "react";
import { useParams, useNavigate, Link } from "react-router-dom";
import { EngineBadge } from "@/components/StatusIndicators";
import type { Engine } from "@/components/StatusIndicators";
import { ChevronLeft } from "lucide-react";
import { useConnection } from "@/hooks/useConnection";
import { formatError } from "@/domain/errors";
import { fetchSessionsForProject, formatTimestamp, type SessionListItem } from "@/domain/remoteApi";

const engines: readonly (Engine | "all")[] = ["all", "codex", "claude", "gemini", "opencode"];

const ProjectDetailPage = () => {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const { isConnected, rpc } = useConnection();

  const [engineFilter, setEngineFilter] = useState<Engine | "all">("all");
  const [sessions, setSessions] = useState<readonly SessionListItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const projectPath = useMemo(() => {
    if (!id) return "";
    try {
      return decodeURIComponent(id);
    } catch {
      return id;
    }
  }, [id]);

  useEffect(() => {
    if (!isConnected || !projectPath) {
      setSessions([]);
      setError(null);
      setLoading(false);
      return;
    }

    let cancelled = false;
    setLoading(true);
    setError(null);

    void fetchSessionsForProject(rpc, projectPath)
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
  }, [isConnected, rpc, projectPath]);

  const filtered = engineFilter === "all" ? sessions : sessions.filter((session) => session.engine === engineFilter);

  return (
    <div className="p-4 lg:p-6 max-w-[1400px] mx-auto space-y-4">
      <div className="flex items-center gap-2 text-sm">
        <Link to="/projects" className="text-text-secondary hover:text-foreground transition-colors flex items-center gap-1">
          <ChevronLeft className="w-3.5 h-3.5" />
          Projects
        </Link>
        <span className="text-text-muted">/</span>
        <span className="text-foreground font-medium truncate">{projectPath || "Unknown project"}</span>
      </div>

      <div className="flex items-center justify-between">
        <h1 className="text-xl font-sans font-semibold text-foreground">
          Sessions
          <span className="text-text-secondary font-normal ml-2 text-base">({sessions.length})</span>
        </h1>
      </div>

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
        <div className="grid grid-cols-[80px_180px_1fr_140px] px-3 py-2 border-b border-border text-xs text-text-secondary uppercase tracking-wider font-sans">
          <span>Engine</span>
          <span>Started</span>
          <span>Title</span>
          <span>Session ID</span>
        </div>
        {filtered.map((session) => (
          <div
            key={session.sessionId}
            onClick={() => navigate(`/sessions/${session.sessionId}`)}
            className="grid grid-cols-[80px_180px_1fr_140px] px-3 py-2.5 border-b border-border last:border-b-0 hover:bg-bg-hover cursor-pointer transition-colors"
          >
            <EngineBadge engine={session.engine} short />
            <span className="text-text-secondary">{formatTimestamp(session.startedTs)}</span>
            <span className="text-foreground truncate">{session.title || "(untitled)"}</span>
            <span className="text-text-muted font-mono text-xs">{session.sessionId}</span>
          </div>
        ))}
        {!loading && filtered.length === 0 && (
          <div className="px-3 py-6 text-center text-sm text-text-muted">No sessions match the filter.</div>
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
              <span className="text-text-muted">{session.sessionId}</span>
            </div>
          </div>
        ))}
      </div>

      <div className="text-xs text-text-secondary">Showing {filtered.length} of {sessions.length} sessions</div>
    </div>
  );
};

export default ProjectDetailPage;
