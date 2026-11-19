import { useState, useMemo, useEffect, useRef } from "react";
import { useNavigate } from "react-router-dom";
import { motion, AnimatePresence } from "framer-motion";
import {
  LayoutDashboard,
  FolderOpen,
  MessageSquare,
  ListTodo,
  Activity,
  Rocket,
  Settings,
  Search,
} from "lucide-react";

interface CommandItem {
  id: string;
  label: string;
  description?: string;
  icon: React.ReactNode;
  action: () => void;
}

interface CommandPaletteProps {
  open: boolean;
  onClose: () => void;
}

export function CommandPalette({ open, onClose }: CommandPaletteProps) {
  const navigate = useNavigate();
  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  const items: CommandItem[] = useMemo(
    () => [
      { id: "dashboard", label: "Dashboard", description: "Overview & status", icon: <LayoutDashboard className="w-4 h-4" />, action: () => navigate("/") },
      { id: "projects", label: "Projects", description: "Browse projects", icon: <FolderOpen className="w-4 h-4" />, action: () => navigate("/projects") },
      { id: "sessions", label: "Sessions", description: "View agent sessions", icon: <MessageSquare className="w-4 h-4" />, action: () => navigate("/sessions") },
      { id: "tasks", label: "Tasks", description: "Manage tasks", icon: <ListTodo className="w-4 h-4" />, action: () => navigate("/tasks") },
      { id: "processes", label: "Processes", description: "Monitor processes", icon: <Activity className="w-4 h-4" />, action: () => navigate("/processes") },
      { id: "spawn", label: "Spawn", description: "Start new agent run", icon: <Rocket className="w-4 h-4" />, action: () => navigate("/spawn") },
      { id: "settings", label: "Settings", description: "Device & connection", icon: <Settings className="w-4 h-4" />, action: () => navigate("/settings") },
    ],
    [navigate]
  );

  const filtered = useMemo(() => {
    if (!query) return items;
    const q = query.toLowerCase();
    return items.filter(
      (item) =>
        item.label.toLowerCase().includes(q) ||
        item.description?.toLowerCase().includes(q)
    );
  }, [items, query]);

  useEffect(() => {
    if (open) {
      setQuery("");
      setSelectedIndex(0);
      setTimeout(() => inputRef.current?.focus(), 50);
    }
  }, [open]);

  useEffect(() => {
    setSelectedIndex(0);
  }, [query]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelectedIndex((i) => Math.min(i + 1, filtered.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelectedIndex((i) => Math.max(i - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (filtered[selectedIndex]) {
        filtered[selectedIndex].action();
        onClose();
      }
    } else if (e.key === "Escape") {
      onClose();
    }
  };

  return (
    <AnimatePresence>
      {open && (
        <>
          <motion.div
            className="fixed inset-0 bg-background/60 backdrop-blur-sm z-50"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            onClick={onClose}
          />
          <motion.div
            className="fixed top-[20%] left-1/2 w-[calc(100%-2rem)] max-w-md -translate-x-1/2 z-50 border border-border bg-surface rounded-xl shadow-dropdown overflow-hidden"
            initial={{ opacity: 0, y: -10, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -10, scale: 0.98 }}
            transition={{ duration: 0.15 }}
          >
            <div className="flex items-center gap-2 px-4 py-3 border-b border-border">
              <Search className="w-4 h-4 text-text-muted shrink-0" />
              <input
                ref={inputRef}
                type="text"
                placeholder="Type a command..."
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                onKeyDown={handleKeyDown}
                className="flex-1 bg-transparent text-foreground text-sm font-mono placeholder:text-text-muted outline-none"
              />
              <kbd className="text-xs text-text-muted bg-secondary px-1.5 py-0.5 font-mono rounded-md">esc</kbd>
            </div>
            <div className="max-h-64 overflow-y-auto py-1">
              {filtered.length === 0 ? (
                <div className="px-4 py-6 text-center text-sm text-text-muted">No results</div>
              ) : (
                filtered.map((item, i) => (
                  <button
                    key={item.id}
                    className={`w-full flex items-center gap-3 px-4 py-2.5 text-left text-sm transition-colors ${
                      i === selectedIndex ? "bg-bg-active text-foreground" : "text-text-secondary hover:bg-bg-hover"
                    }`}
                    onClick={() => {
                      item.action();
                      onClose();
                    }}
                    onMouseEnter={() => setSelectedIndex(i)}
                  >
                    <span className="text-text-muted">{item.icon}</span>
                    <span className="font-sans font-medium">{item.label}</span>
                    {item.description && (
                      <span className="text-text-muted text-xs ml-auto">{item.description}</span>
                    )}
                  </button>
                ))
              )}
            </div>
          </motion.div>
        </>
      )}
    </AnimatePresence>
  );
}
