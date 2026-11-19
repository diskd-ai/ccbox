import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { Search, X } from "lucide-react";
import { useConnection } from "@/hooks/useConnection";
import { formatError } from "@/domain/errors";
import { fetchProjects, formatTimestamp, type ProjectListItem } from "@/domain/remoteApi";

const ProjectsPage = () => {
  const navigate = useNavigate();
  const { isConnected, rpc } = useConnection();

  const [filter, setFilter] = useState("");
  const [projects, setProjects] = useState<readonly ProjectListItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!isConnected) {
      setProjects([]);
      setError(null);
      setLoading(false);
      return;
    }

    let cancelled = false;
    setLoading(true);
    setError(null);

    void fetchProjects(rpc)
      .then((rows) => {
        if (cancelled) return;
        setProjects(rows);
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
    if (!filter.trim()) return projects;
    const needle = filter.toLowerCase();
    return projects.filter((project) => project.path.toLowerCase().includes(needle));
  }, [projects, filter]);

  const handleProjectClick = (projectPath: string) => {
    navigate(`/projects/${encodeURIComponent(projectPath)}`);
  };

  return (
    <div className="p-4 lg:p-6 max-w-[1400px] mx-auto space-y-4">
      <div className="flex items-center justify-between">
        <h1 className="text-xl font-sans font-semibold text-foreground">Projects</h1>
        <div className="relative">
          <Search className="absolute left-2 top-1/2 -translate-y-1/2 w-3.5 h-3.5 text-text-muted" />
          <input
            type="text"
            placeholder="Filter..."
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            className="bg-surface border border-border text-foreground pl-7 pr-7 py-1.5 text-sm w-48 rounded-lg shadow-card focus:outline-none focus:border-ring focus:shadow-elevated placeholder:text-text-muted transition-shadow"
          />
          {filter && (
            <button
              onClick={() => setFilter("")}
              className="absolute right-2 top-1/2 -translate-y-1/2 text-text-muted hover:text-foreground"
            >
              <X className="w-3 h-3" />
            </button>
          )}
        </div>
      </div>

      {!isConnected && (
        <div className="text-sm text-text-secondary border border-border bg-surface rounded-xl px-3 py-2">
          Connect to CCBox to load projects.
        </div>
      )}

      {loading && <div className="text-sm text-text-secondary">Loading projects...</div>}
      {error && <div className="text-sm text-status-error">{error}</div>}

      <div className="border border-border bg-surface rounded-xl shadow-card hidden md:block overflow-hidden">
        <div className="grid grid-cols-[1fr_100px_180px] px-3 py-2 border-b border-border text-xs text-text-secondary uppercase tracking-wider font-sans">
          <span>Path</span>
          <span className="text-right">Sessions</span>
          <span className="text-right">Last Modified</span>
        </div>
        {filtered.map((project) => (
          <div
            key={project.path}
            onClick={() => handleProjectClick(project.path)}
            className="grid grid-cols-[1fr_100px_180px] px-3 py-2 border-b border-border last:border-b-0 hover:bg-bg-hover cursor-pointer transition-colors"
          >
            <span className="flex items-center gap-2">
              {(project.sessionCount ?? 0) > 0 && (
                <span className="w-1.5 h-1.5 rounded-full bg-status-online glow-online" />
              )}
              <span className="text-foreground">{project.path}</span>
            </span>
            <span className="text-right text-text-secondary">{project.sessionCount ?? "-"}</span>
            <span className="text-right text-text-secondary">{formatTimestamp(project.lastSeenTs)}</span>
          </div>
        ))}
        {!loading && filtered.length === 0 && (
          <div className="px-3 py-4 text-sm text-text-muted">No projects returned.</div>
        )}
      </div>

      <div className="md:hidden space-y-2">
        {filtered.map((project) => (
          <div
            key={project.path}
            onClick={() => handleProjectClick(project.path)}
            className="border border-border bg-surface px-3 py-2.5 space-y-1 rounded-xl shadow-card cursor-pointer hover:bg-bg-hover hover:shadow-elevated transition-all"
          >
            <div className="flex items-center gap-2">
              {(project.sessionCount ?? 0) > 0 && (
                <span className="w-1.5 h-1.5 rounded-full bg-status-online glow-online" />
              )}
              <span className="text-foreground text-sm truncate">{project.path}</span>
            </div>
            <div className="flex items-center gap-3 text-xs text-text-secondary">
              <span>{project.sessionCount ?? "-"} sessions</span>
              <span>{formatTimestamp(project.lastSeenTs)}</span>
            </div>
          </div>
        ))}
      </div>

      <div className="text-xs text-text-secondary">Showing {filtered.length} of {projects.length} projects</div>
    </div>
  );
};

export default ProjectsPage;
