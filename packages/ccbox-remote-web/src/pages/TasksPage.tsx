import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { Play } from "lucide-react";
import type { Engine } from "@/components/StatusIndicators";
import type { TaskStatus } from "@/components/TaskForm";
import { useConnection } from "@/hooks/useConnection";
import { formatError } from "@/domain/errors";
import { fetchTasks, formatTimestamp, spawnTask, type TaskListItem } from "@/domain/remoteApi";

interface TaskRow extends TaskListItem {
  readonly engine: Engine;
  readonly status: TaskStatus;
}

const STATUS_BADGE: Record<TaskStatus, { label: string; className: string; dot: string }> = {
  draft: { label: "Draft", className: "text-text-secondary", dot: "bg-text-muted" },
  pending: { label: "Pending", className: "text-status-warning", dot: "bg-status-warning" },
  running: { label: "Running", className: "text-status-running", dot: "bg-status-running animate-pulse-dot" },
  done: { label: "Done", className: "text-status-online", dot: "bg-status-online" },
  failed: { label: "Failed", className: "text-status-error", dot: "bg-status-error" },
};

const TasksPage = () => {
  const navigate = useNavigate();
  const { isConnected, rpc } = useConnection();

  const [tasks, setTasks] = useState<readonly TaskRow[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!isConnected) {
      setTasks([]);
      setLoading(false);
      setError(null);
      return;
    }

    let cancelled = false;
    setLoading(true);
    setError(null);

    void fetchTasks(rpc)
      .then((rows) => {
        if (cancelled) return;
        setTasks(
          rows.map((task) => ({
            ...task,
            engine: "codex",
            status: "draft",
          })),
        );
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

  const rows = useMemo(() => tasks, [tasks]);

  const handleRun = async (event: React.MouseEvent, task: TaskRow) => {
    event.stopPropagation();
    setTasks((current) =>
      current.map((row) => (row.taskId === task.taskId ? { ...row, status: "running" } : row)),
    );

    try {
      const processId = await spawnTask(rpc, task.taskId, task.engine);
      if (!processId) {
        setTasks((current) =>
          current.map((row) => (row.taskId === task.taskId ? { ...row, status: "failed" } : row)),
        );
        return;
      }

      navigate("/processes");
    } catch {
      setTasks((current) =>
        current.map((row) => (row.taskId === task.taskId ? { ...row, status: "failed" } : row)),
      );
    }
  };

  return (
    <div className="p-4 lg:p-6 max-w-[1400px] mx-auto space-y-4">
      <div className="flex items-center justify-between">
        <h1 className="text-xl font-sans font-semibold text-foreground">Tasks</h1>
        <button
          onClick={() => navigate("/tasks/new")}
          className="flex items-center gap-1.5 px-3 py-1.5 text-sm font-sans font-medium bg-primary text-primary-foreground rounded-lg hover:opacity-90 transition-opacity shadow-card"
        >
          + New Task
        </button>
      </div>

      {!isConnected && (
        <div className="text-sm text-text-secondary border border-border bg-surface rounded-xl px-3 py-2">
          Connect to CCBox to load tasks.
        </div>
      )}
      {loading && <div className="text-sm text-text-secondary">Loading tasks...</div>}
      {error && <div className="text-sm text-status-error">{error}</div>}

      <div className="border border-border bg-surface rounded-xl shadow-card hidden md:block overflow-hidden">
        <div className="grid grid-cols-[80px_1fr_200px_180px_60px] px-3 py-2 border-b border-border text-xs text-text-secondary uppercase tracking-wider font-sans">
          <span>Status</span>
          <span>Title</span>
          <span>Project</span>
          <span>Updated</span>
          <span className="text-right">Action</span>
        </div>

        {rows.map((task) => {
          const badge = STATUS_BADGE[task.status];
          return (
            <div
              key={task.taskId}
              onClick={() => navigate(`/tasks/${task.taskId}/edit`)}
              className="grid grid-cols-[80px_1fr_200px_180px_60px] px-3 py-2 border-b border-border last:border-b-0 hover:bg-bg-hover cursor-pointer transition-colors items-center"
            >
              <span className={`flex items-center gap-1.5 text-xs font-medium ${badge.className}`}>
                <span className={`w-1.5 h-1.5 rounded-full ${badge.dot}`} />
                {badge.label}
              </span>
              <span className="text-foreground truncate">{task.title || "(untitled)"}</span>
              <span className="text-text-secondary truncate">{task.projectPath}</span>
              <span className="text-text-secondary text-xs">{formatTimestamp(task.updatedTs)}</span>
              <span className="text-right">
                {task.status === "draft" && (
                  <button
                    onClick={(event) => void handleRun(event, task)}
                    className="inline-flex items-center gap-1 px-2 py-1 text-xs font-sans font-medium bg-status-online/15 text-status-online rounded-md hover:bg-status-online/25 transition-colors"
                  >
                    <Play className="w-3 h-3" />
                    Run
                  </button>
                )}
              </span>
            </div>
          );
        })}

        {!loading && rows.length === 0 && (
          <div className="px-3 py-4 text-sm text-text-muted">No tasks returned.</div>
        )}
      </div>

      <div className="md:hidden space-y-2">
        {rows.map((task) => {
          const badge = STATUS_BADGE[task.status];
          return (
            <div
              key={task.taskId}
              onClick={() => navigate(`/tasks/${task.taskId}/edit`)}
              className="border border-border bg-surface px-3 py-2.5 space-y-1.5 rounded-xl shadow-card cursor-pointer hover:bg-bg-hover hover:shadow-elevated transition-all"
            >
              <div className="flex items-center justify-between">
                <span className="text-foreground text-sm font-medium">{task.title || "(untitled)"}</span>
                <span className={`flex items-center gap-1 text-xs font-medium ${badge.className}`}>
                  <span className={`w-1.5 h-1.5 rounded-full ${badge.dot}`} />
                  {badge.label}
                </span>
              </div>
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-3 text-xs text-text-secondary">
                  <span className="truncate">{task.projectPath}</span>
                  <span>{formatTimestamp(task.updatedTs)}</span>
                </div>
                {task.status === "draft" && (
                  <button
                    onClick={(event) => void handleRun(event, task)}
                    className="flex items-center gap-1 px-2 py-1 text-xs font-sans font-medium bg-status-online/15 text-status-online rounded-md hover:bg-status-online/25 transition-colors"
                  >
                    <Play className="w-3 h-3" />
                    Run
                  </button>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
};

export default TasksPage;
