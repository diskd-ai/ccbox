import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { TaskForm, type TaskFormData } from "@/components/TaskForm";
import { useConnection } from "@/hooks/useConnection";
import { formatError } from "@/domain/errors";
import { createTask } from "@/domain/remoteApi";

function buildTaskBody(data: TaskFormData): string {
  const title = data.title.trim();
  const body = data.body.trim();

  if (title && body) return `${title}\n\n${body}`;
  if (title) return title;
  return body;
}

const TaskCreatePage = () => {
  const navigate = useNavigate();
  const { isConnected, rpc } = useConnection();

  const [error, setError] = useState<string | null>(null);

  const handleSubmit = (data: TaskFormData) => {
    if (!isConnected) {
      setError("Connect to CCBox before creating tasks.");
      return;
    }

    const projectPath = data.project.trim();
    if (!projectPath) {
      setError("Project path is required.");
      return;
    }

    const body = buildTaskBody(data);
    if (!body.trim()) {
      setError("Task body is required.");
      return;
    }

    setError(null);

    void createTask(rpc, projectPath, body)
      .then((taskId) => {
        if (!taskId) {
          setError("Task creation failed.");
          return;
        }
        navigate("/tasks");
      })
      .catch((reason: unknown) => {
        setError(formatError(reason));
      });
  };

  return (
    <div className="space-y-3">
      {error && <div className="max-w-[800px] mx-auto px-4 text-sm text-status-error">{error}</div>}
      <TaskForm
        heading="New Task"
        submitLabel="Create Task"
        shortcutLabel="to create"
        onSubmit={handleSubmit}
        onCancel={() => navigate("/tasks")}
      />
    </div>
  );
};

export default TaskCreatePage;
