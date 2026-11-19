import { useEffect, useMemo, useState } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { TaskForm, type TaskFormData } from "@/components/TaskForm";
import { useConnection } from "@/hooks/useConnection";
import { formatError } from "@/domain/errors";
import { fetchTask, spawnTask, type TaskDetails } from "@/domain/remoteApi";
import type { TaskStatus } from "@/components/TaskForm";

function splitTaskBody(body: string): { title: string; description: string } {
  const lines = body.split("\n");
  const title = (lines[0] ?? "").trim();
  const description = lines.slice(1).join("\n").trim();

  return {
    title: title || "(untitled)",
    description,
  };
}

const TaskEditPage = () => {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const { isConnected, rpc } = useConnection();

  const [task, setTask] = useState<TaskDetails | null>(null);
  const [status, setStatus] = useState<TaskStatus>("draft");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!id) {
      setTask(null);
      setError("Task id is missing.");
      return;
    }

    if (!isConnected) {
      setTask(null);
      setError("Connect to CCBox to load this task.");
      return;
    }

    let cancelled = false;
    setLoading(true);
    setError(null);

    void fetchTask(rpc, id)
      .then((loaded) => {
        if (cancelled) return;
        if (!loaded) {
          setTask(null);
          setError("Task not found.");
          return;
        }

        setTask(loaded);
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
  }, [id, isConnected, rpc]);

  const initialData = useMemo(() => {
    if (!task) return undefined;
    const parts = splitTaskBody(task.body);
    return {
      title: parts.title,
      body: parts.description,
      engine: "codex" as const,
      project: task.projectPath,
      images: [],
    };
  }, [task]);

  const handleRun = (data: TaskFormData) => {
    if (!id) return;
    setStatus("running");
    setError(null);

    void spawnTask(rpc, id, data.engine)
      .then((processId) => {
        if (!processId) {
          setStatus("failed");
          setError("Task spawn failed.");
          return;
        }
        navigate("/processes");
      })
      .catch((reason: unknown) => {
        setStatus("failed");
        setError(formatError(reason));
      });
  };

  if (loading) {
    return (
      <div className="p-4 lg:p-6 max-w-[800px] mx-auto">
        <p className="text-text-secondary">Loading task...</p>
      </div>
    );
  }

  if (!task || !initialData) {
    return (
      <div className="p-4 lg:p-6 max-w-[800px] mx-auto space-y-2">
        <p className="text-text-secondary">{error ?? "Task not found."}</p>
      </div>
    );
  }

  return (
    <div className="space-y-3">
      {error && <div className="max-w-[800px] mx-auto px-4 text-sm text-status-error">{error}</div>}
      <TaskForm
        heading={task.title}
        submitLabel="Save Changes"
        shortcutLabel="to save"
        initial={initialData}
        status={status}
        logs={[]}
        onSubmit={() => setError("Task updates are not supported by the current API.")}
        onCancel={() => navigate("/tasks")}
        onRun={handleRun}
      />
    </div>
  );
};

export default TaskEditPage;
