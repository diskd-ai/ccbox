import { ReactNode, useState, useCallback } from "react";
import { Link, useLocation, useNavigate } from "react-router-dom";
import { StatusBar } from "./StatusBar";
import { CommandPalette } from "./CommandPalette";
import { KeyboardShortcutsHelp } from "./KeyboardShortcutsHelp";
import { ConnectionOverlay } from "./ConnectionOverlay";
import { IntroPage } from "./IntroPage";
import { useConnection } from "@/hooks/useConnection";
import { useKeyboardShortcuts } from "@/hooks/useKeyboardShortcuts";
import type { ConnectionState } from "@/domain/types";
import {
  FolderOpen,
  MessageSquare,
  ListTodo,
  Activity,
  Settings,
} from "lucide-react";

const navItems = [
  { path: "/", label: "Sessions", icon: MessageSquare },
  { path: "/projects", label: "Projects", icon: FolderOpen },
  { path: "/tasks", label: "Tasks", icon: ListTodo },
  { path: "/processes", label: "Processes", icon: Activity },
];

const NAV_PATHS = ["/", "/projects", "/tasks", "/processes"];

function MobileTabItem({ path, label, icon: Icon }: { path: string; label: string; icon: React.ComponentType<{ className?: string }> }) {
  const location = useLocation();
  const isActive = path === "/" ? location.pathname === "/" : location.pathname.startsWith(path);
  return (
    <Link
      to={path}
      className={`flex flex-col items-center gap-0.5 px-2 py-1 text-xs transition-colors ${
        isActive ? "text-foreground" : "text-text-secondary"
      }`}
    >
      <Icon className="w-4 h-4" />
      <span className="text-xs hidden min-[375px]:inline">{label.slice(0, 4)}</span>
    </Link>
  );
}

function NavItem({ path, label, icon: Icon }: { path: string; label: string; icon: React.ComponentType<{ className?: string }> }) {
  const location = useLocation();
  const isActive = path === "/" ? location.pathname === "/" : location.pathname.startsWith(path);
  return (
    <Link
      to={path}
      className={`flex items-center gap-2 px-3 py-1.5 text-sm transition-colors font-sans rounded-lg ${
        isActive
          ? "text-foreground bg-bg-active font-medium"
          : "text-text-secondary hover:text-foreground hover:bg-bg-hover"
      }`}
    >
      <Icon className="w-3.5 h-3.5" />
      <span className="hidden lg:inline">{label}</span>
    </Link>
  );
}

const STATE_DOT: Record<ConnectionState, { color: string; glow: string; pulse: boolean }> = {
  disconnected: { color: "bg-text-muted", glow: "", pulse: false },
  connecting: { color: "bg-status-warning", glow: "glow-warning", pulse: true },
  awaiting_challenge: { color: "bg-status-warning", glow: "glow-warning", pulse: true },
  authenticating: { color: "bg-status-warning", glow: "glow-warning", pulse: true },
  online: { color: "bg-status-online", glow: "glow-online", pulse: false },
  reconnecting: { color: "bg-status-warning", glow: "glow-warning", pulse: true },
  auth_failed: { color: "bg-status-error", glow: "glow-error", pulse: false },
};

const STATE_LABEL: Record<ConnectionState, string> = {
  disconnected: "Disconnected",
  connecting: "Connecting...",
  awaiting_challenge: "Awaiting Challenge...",
  authenticating: "Authenticating...",
  online: "Online",
  reconnecting: "Reconnecting...",
  auth_failed: "Auth Failed",
};

function ConnectionIndicator() {
  const { state } = useConnection();
  const dot = STATE_DOT[state];
  const isDisconnected = state === "disconnected" || state === "auth_failed";
  return (
    <div className="flex items-center gap-1.5 text-xs text-text-secondary">
      <div className={`w-1.5 h-1.5 rounded-full ${dot.color} ${dot.glow} ${dot.pulse ? "animate-pulse-dot" : ""}`} />
      <span className="hidden sm:inline">{STATE_LABEL[state]}</span>
      {isDisconnected && (
        <Link
          to="/settings"
          className="ml-1 px-2.5 py-0.5 text-xs font-sans font-medium bg-primary text-primary-foreground rounded-md hover:opacity-90 transition-opacity"
        >
          Connect
        </Link>
      )}
    </div>
  );
}

export function AppLayout({ children }: { children: ReactNode }) {
  const { state } = useConnection();
  const navigate = useNavigate();
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);

  const openPalette = useCallback(() => setPaletteOpen(true), []);
  const closePalette = useCallback(() => setPaletteOpen(false), []);

  useKeyboardShortcuts([
    { key: "k", ctrl: true, action: openPalette, allowInInput: true },
    { key: "?", action: () => setShortcutsOpen(true) },
    { key: "1", action: () => navigate(NAV_PATHS[0]) },
    { key: "2", action: () => navigate(NAV_PATHS[1]) },
    { key: "3", action: () => navigate(NAV_PATHS[2]) },
    { key: "4", action: () => navigate(NAV_PATHS[3]) },
  ]);

  // Show intro page when not connected.
  // "reconnecting" keeps the full layout visible (ConnectionOverlay handles it).
  const showIntro = state !== "online" && state !== "reconnecting";

  if (showIntro) {
    return <IntroPage />;
  }

  return (
    <div className="flex flex-col h-dvh overflow-hidden">
      {/* Top header bar */}
      <header className="fixed top-0 left-0 right-0 z-30 flex items-center justify-between border-b border-border px-4 h-11 bg-surface shadow-card">
        <div className="flex items-center gap-1">
          <Link to="/" className="font-mono text-md font-bold text-foreground mr-4 tracking-tight">
            ccbox
          </Link>
          <nav className="hidden md:flex items-center gap-0.5">
            {navItems.map((item) => (
              <NavItem key={item.path} {...item} />
            ))}
          </nav>
        </div>
        <div className="flex items-center gap-3">
          <button
            onClick={openPalette}
            className="hidden md:flex items-center gap-1.5 text-xs text-text-muted hover:text-text-secondary transition-colors"
          >
            <kbd className="bg-secondary px-1.5 py-0.5 font-mono text-xs rounded-md">⌘K</kbd>
          </button>
          <ConnectionIndicator />
          <Link to="/settings" className="text-text-secondary hover:text-foreground transition-colors">
            <Settings className="w-3.5 h-3.5" />
          </Link>
        </div>
      </header>

      {/* Main content */}
      <main className="flex-1 overflow-auto pt-11 pb-12 md:pb-7">
        {children}
      </main>

      {/* Status bar - desktop only */}
      <StatusBar className="fixed bottom-0 left-0 right-0 z-30 hidden md:flex" />

      {/* Mobile bottom tab bar */}
      <nav className="fixed bottom-0 left-0 right-0 z-30 md:hidden flex items-center justify-around border-t border-border bg-surface h-12 shadow-elevated">
        {navItems.map((item) => (
          <MobileTabItem key={item.path} {...item} />
        ))}
      </nav>

      {/* Overlays */}
      <CommandPalette open={paletteOpen} onClose={closePalette} />
      <KeyboardShortcutsHelp open={shortcutsOpen} onClose={() => setShortcutsOpen(false)} />
      <ConnectionOverlay />
    </div>
  );
}
