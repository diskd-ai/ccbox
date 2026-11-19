import { useState, useRef } from "react";
import { ChevronLeft, ImagePlus, X, Play, Lock, Terminal } from "lucide-react";
import type { Engine } from "@/components/StatusIndicators";

const engines: Engine[] = ["codex", "claude", "gemini", "opencode"];

const engineColors: Record<Engine, string> = {
  codex: "border-engine-codex text-engine-codex bg-engine-codex/10",
  claude: "border-engine-claude text-engine-claude bg-engine-claude/10",
  gemini: "border-engine-gemini text-engine-gemini bg-engine-gemini/10",
  opencode: "border-engine-opencode text-engine-opencode bg-engine-opencode/10",
};

const mockProjects = [
  "/Users/alex/proj/alpha",
  "/Users/alex/proj/beta",
  "/Users/alex/proj/gamma",
  "/Users/alex/proj/delta",
  "/Users/alex/proj/epsilon",
];

export type TaskStatus = "draft" | "pending" | "running" | "done" | "failed";

export interface TaskFormData {
  title: string;
  body: string;
  engine: Engine;
  project: string;
  images: { id: string; name: string; preview: string }[];
}

export interface TaskLogLine {
  ts: string;
  level: "info" | "warn" | "error" | "cmd" | "success";
  text: string;
}

interface TaskFormProps {
  heading: string;
  submitLabel: string;
  shortcutLabel: string;
  initial?: Partial<TaskFormData>;
  status?: TaskStatus;
  logs?: TaskLogLine[];
  onSubmit: (data: TaskFormData) => void;
  onCancel: () => void;
  onRun?: (data: TaskFormData) => void;
}

const STATUS_BADGE: Record<TaskStatus, { label: string; className: string }> = {
  draft: { label: "Draft", className: "bg-secondary text-text-secondary" },
  pending: { label: "Pending", className: "bg-status-warning/15 text-status-warning" },
  running: { label: "Running", className: "bg-status-running/15 text-status-running" },
  done: { label: "Done", className: "bg-status-online/15 text-status-online" },
  failed: { label: "Failed", className: "bg-status-error/15 text-status-error" },
};

const LOG_COLORS: Record<TaskLogLine["level"], string> = {
  info: "text-text-secondary",
  warn: "text-status-warning",
  error: "text-status-error",
  cmd: "text-engine-codex",
  success: "text-status-online",
};

export function TaskForm({ heading, submitLabel, shortcutLabel, initial, status = "draft", logs = [], onSubmit, onCancel, onRun }: TaskFormProps) {
  const fileInputRef = useRef<HTMLInputElement>(null);
  const locked = status !== "draft";

  const [title, setTitle] = useState(initial?.title ?? "");
  const [body, setBody] = useState(initial?.body ?? "");
  const [engine, setEngine] = useState<Engine>(initial?.engine ?? "codex");
  const [project, setProject] = useState(initial?.project ?? "");
  const [images, setImages] = useState<TaskFormData["images"]>(initial?.images ?? []);
  const [projectSuggestions, setProjectSuggestions] = useState<string[]>([]);
  const [showSuggestions, setShowSuggestions] = useState(false);

  const handleProjectChange = (value: string) => {
    setProject(value);
    if (value.length > 0) {
      const filtered = mockProjects.filter((p) => p.toLowerCase().includes(value.toLowerCase()));
      setProjectSuggestions(filtered);
      setShowSuggestions(filtered.length > 0);
    } else {
      setShowSuggestions(false);
    }
  };

  const selectProject = (p: string) => {
    setProject(p);
    setShowSuggestions(false);
  };

  const handleImageUpload = (e: React.ChangeEvent<HTMLInputElement>) => {
    if (locked) return;
    const files = e.target.files;
    if (!files) return;
    Array.from(files).forEach((file) => {
      const reader = new FileReader();
      reader.onload = () => {
        setImages((prev) => [
          ...prev,
          { id: crypto.randomUUID(), name: file.name, preview: reader.result as string },
        ]);
      };
      reader.readAsDataURL(file);
    });
    e.target.value = "";
  };

  const handlePaste = (e: React.ClipboardEvent) => {
    if (locked) return;
    const items = e.clipboardData.items;
    for (const item of items) {
      if (item.type.startsWith("image/")) {
        e.preventDefault();
        const file = item.getAsFile();
        if (!file) continue;
        const reader = new FileReader();
        reader.onload = () => {
          setImages((prev) => [
            ...prev,
            { id: crypto.randomUUID(), name: `pasted-${Date.now()}.png`, preview: reader.result as string },
          ]);
        };
        reader.readAsDataURL(file);
      }
    }
  };

  const removeImage = (id: string) => {
    if (locked) return;
    setImages((prev) => prev.filter((img) => img.id !== id));
  };

  const handleSubmit = () => {
    if (!title.trim() || locked) return;
    onSubmit({ title, body, engine, project, images });
  };

  const canSubmit = title.trim().length > 0 && !locked;
  const badge = STATUS_BADGE[status];

  return (
    <div className="p-4 lg:p-6 max-w-[800px] mx-auto space-y-5">
      {/* Header */}
      <div className="flex items-center gap-2">
        <button onClick={onCancel} className="text-text-secondary hover:text-foreground transition-colors">
          <ChevronLeft className="w-4 h-4" />
        </button>
        <h1 className="text-xl font-sans font-semibold text-foreground">{heading}</h1>
        {status !== "draft" && (
          <span className={`ml-2 px-2 py-0.5 text-xs font-sans font-medium rounded-md ${badge.className}`}>
            {badge.label}
          </span>
        )}
        {locked && <Lock className="w-3.5 h-3.5 text-text-muted ml-1" />}
      </div>

      <div className="border border-border bg-surface rounded-xl shadow-card overflow-hidden">
        <div className="p-4 space-y-4">
          {/* Title */}
          <div className="space-y-1.5">
            <label className="text-xs text-text-secondary font-sans font-medium uppercase tracking-wider">Title</label>
            <input
              type="text"
              placeholder="What needs to be done?"
              value={title}
              onChange={(e) => !locked && setTitle(e.target.value)}
              readOnly={locked}
              autoFocus={!locked}
              className={`w-full bg-background border border-border text-foreground px-3 py-2 text-sm rounded-lg transition-shadow ${
                locked
                  ? "opacity-70 cursor-not-allowed"
                  : "focus:outline-none focus:border-ring focus:shadow-elevated placeholder:text-text-muted"
              }`}
            />
          </div>

          {/* Description */}
          <div className="space-y-1.5">
            <label className="text-xs text-text-secondary font-sans font-medium uppercase tracking-wider">Description</label>
            <textarea
              placeholder="Describe the task in detail. Supports markdown..."
              value={body}
              onChange={(e) => !locked && setBody(e.target.value)}
              onPaste={locked ? undefined : handlePaste}
              readOnly={locked}
              rows={6}
              className={`w-full bg-background border border-border text-foreground px-3 py-2 text-sm rounded-lg resize-y font-mono transition-shadow ${
                locked
                  ? "opacity-70 cursor-not-allowed"
                  : "focus:outline-none focus:border-ring focus:shadow-elevated placeholder:text-text-muted"
              }`}
            />
            <div className="text-xs text-text-muted text-right">{body.length} chars</div>
          </div>

          {/* Engine */}
          <div className="space-y-1.5">
            <label className="text-xs text-text-secondary font-sans font-medium uppercase tracking-wider">Engine</label>
            <div className="flex items-center gap-2 flex-wrap">
              {engines.map((eng) => (
                <button
                  key={eng}
                  onClick={() => !locked && setEngine(eng)}
                  disabled={locked}
                  className={`px-3 py-1.5 text-sm font-sans font-medium border transition-all rounded-lg ${
                    engine === eng
                      ? `${engineColors[eng]} border-current`
                      : "border-border bg-background text-text-secondary hover:text-foreground hover:bg-bg-hover"
                  } ${locked ? "opacity-70 cursor-not-allowed" : ""}`}
                >
                  {eng.charAt(0).toUpperCase() + eng.slice(1)}
                </button>
              ))}
            </div>
          </div>

          {/* Project */}
          <div className="space-y-1.5 relative">
            <label className="text-xs text-text-secondary font-sans font-medium uppercase tracking-wider">Project</label>
            <input
              type="text"
              placeholder="/Users/alex/proj/..."
              value={project}
              onChange={(e) => !locked && handleProjectChange(e.target.value)}
              onFocus={() => { if (!locked && project.length > 0 && projectSuggestions.length > 0) setShowSuggestions(true); }}
              onBlur={() => setTimeout(() => setShowSuggestions(false), 150)}
              readOnly={locked}
              className={`w-full bg-background border border-border text-foreground px-3 py-2 text-sm rounded-lg transition-shadow ${
                locked
                  ? "opacity-70 cursor-not-allowed"
                  : "focus:outline-none focus:border-ring focus:shadow-elevated placeholder:text-text-muted"
              }`}
            />
            {!locked && showSuggestions && (
              <div className="absolute z-10 top-full mt-1 left-0 right-0 bg-surface border border-border rounded-lg shadow-dropdown overflow-hidden">
                {projectSuggestions.map((p) => (
                  <button key={p} onMouseDown={() => selectProject(p)} className="w-full text-left px-3 py-2 text-sm text-foreground hover:bg-bg-hover transition-colors">
                    {p}
                  </button>
                ))}
              </div>
            )}
          </div>

          {/* Images */}
          <div className="space-y-1.5">
            <label className="text-xs text-text-secondary font-sans font-medium uppercase tracking-wider">Images</label>
            {images.length > 0 && (
              <div className="flex flex-wrap gap-2 mb-2">
                {images.map((img) => (
                  <div key={img.id} className="relative group">
                    <img src={img.preview} alt={img.name} className="w-20 h-20 object-cover rounded-lg border border-border" />
                    {!locked && (
                      <button
                        onClick={() => removeImage(img.id)}
                        className="absolute -top-1.5 -right-1.5 w-5 h-5 bg-destructive text-destructive-foreground rounded-full flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity shadow-card"
                      >
                        <X className="w-3 h-3" />
                      </button>
                    )}
                  </div>
                ))}
              </div>
            )}
            {!locked && (
              <>
                <button
                  onClick={() => fileInputRef.current?.click()}
                  className="flex items-center gap-2 px-3 py-2 text-sm text-text-secondary border border-dashed border-border rounded-lg hover:text-foreground hover:border-ring hover:bg-bg-hover transition-all"
                >
                  <ImagePlus className="w-4 h-4" />
                  <span>Add images</span>
                  <span className="text-text-muted text-xs">(or paste from clipboard)</span>
                </button>
                <input ref={fileInputRef} type="file" accept="image/*" multiple onChange={handleImageUpload} className="hidden" />
              </>
            )}
          </div>
        </div>

        {/* Logs panel */}
        {locked && logs.length > 0 && (
          <div className="border-t border-border">
            <div className="flex items-center gap-2 px-4 py-2 border-b border-border bg-bg-hover/30">
              <Terminal className="w-3.5 h-3.5 text-text-muted" />
              <span className="text-xs font-sans font-medium text-text-secondary uppercase tracking-wider">Logs</span>
              <span className="text-xs text-text-muted ml-auto">{logs.length} lines</span>
            </div>
            <div className="max-h-64 overflow-y-auto bg-background font-mono text-xs">
              {logs.map((line, i) => (
                <div key={i} className="flex gap-0 hover:bg-bg-hover/50 transition-colors">
                  <span className="w-10 shrink-0 text-right pr-2 py-0.5 text-text-muted select-none border-r border-border">{i + 1}</span>
                  <span className="text-text-muted px-2 py-0.5 shrink-0 w-20">{line.ts}</span>
                  <span className={`px-1 py-0.5 whitespace-pre-wrap break-all ${LOG_COLORS[line.level]}`}>{line.text}</span>
                </div>
              ))}
            </div>
          </div>
        )}

        {/* Footer */}
        <div className="flex items-center justify-between px-4 py-3 border-t border-border bg-bg-hover/30">
          <span className="text-xs text-text-muted">
            {locked ? (
              <span className="flex items-center gap-1.5">
                <Lock className="w-3 h-3" />
                Task is locked
              </span>
            ) : (
              <>
                <kbd className="bg-secondary px-1.5 py-0.5 font-mono rounded-md">⌘↵</kbd> {shortcutLabel}
              </>
            )}
          </span>
          <div className="flex items-center gap-2">
            <button onClick={onCancel} className="px-3 py-1.5 text-sm font-sans border border-border text-text-secondary rounded-lg hover:text-foreground hover:bg-bg-hover transition-colors">
              {locked ? "Back" : "Cancel"}
            </button>
            {!locked && (
              <button
                onClick={handleSubmit}
                disabled={!canSubmit}
                className="px-4 py-1.5 text-sm font-sans font-medium bg-primary text-primary-foreground rounded-lg hover:opacity-90 transition-opacity disabled:opacity-40 shadow-card"
              >
                {submitLabel}
              </button>
            )}
            {status === "draft" && onRun && (
              <button
                onClick={() => onRun({ title, body, engine, project, images })}
                disabled={!title.trim()}
                className="flex items-center gap-1.5 px-4 py-1.5 text-sm font-sans font-medium bg-status-online text-primary-foreground rounded-lg hover:opacity-90 transition-opacity disabled:opacity-40 shadow-card"
              >
                <Play className="w-3 h-3" />
                Run
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
