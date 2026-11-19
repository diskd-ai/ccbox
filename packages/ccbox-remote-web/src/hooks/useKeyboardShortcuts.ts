import { useEffect, useCallback } from "react";

interface ShortcutDef {
  key: string;                           // e.g. "k", "Enter", "Escape"
  ctrl?: boolean;
  meta?: boolean;
  shift?: boolean;
  action: () => void;
  allowInInput?: boolean;               // default false
  description?: string;
}

/**
 * Register keyboard shortcuts. Automatically handles cleanup on unmount.
 * Shortcuts are disabled when a text input is focused unless allowInInput is true.
 */
export function useKeyboardShortcuts(shortcuts: ShortcutDef[]) {
  const handler = useCallback(
    (event: KeyboardEvent) => {
      const target = event.target as HTMLElement;
      const isInput =
        target.tagName === "INPUT" ||
        target.tagName === "TEXTAREA" ||
        target.tagName === "SELECT" ||
        target.isContentEditable;

      for (const shortcut of shortcuts) {
        if (shortcut.ctrl && !event.ctrlKey && !event.metaKey) continue;
        if (shortcut.meta && !event.metaKey) continue;
        if (shortcut.shift && !event.shiftKey) continue;
        if (!shortcut.ctrl && !shortcut.meta && (event.ctrlKey || event.metaKey)) continue;
        if (!shortcut.shift && event.shiftKey && shortcut.key.length === 1) continue;

        const keyMatch =
          event.key === shortcut.key ||
          event.key.toLowerCase() === shortcut.key.toLowerCase();

        if (!keyMatch) continue;

        // Block if in input and not explicitly allowed
        if (isInput && !shortcut.allowInInput) continue;

        event.preventDefault();
        event.stopPropagation();
        shortcut.action();
        return;
      }
    },
    [shortcuts]
  );

  useEffect(() => {
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [handler]);
}

// ── Shortcut definitions for help overlay ─────────────────────

export interface ShortcutGroup {
  title: string;
  shortcuts: { keys: string; description: string }[];
}

export const GLOBAL_SHORTCUTS: ShortcutGroup = {
  title: "Global",
  shortcuts: [
    { keys: "Ctrl+K", description: "Command palette" },
    { keys: "Ctrl+R", description: "Refresh current data" },
    { keys: "Ctrl+Q", description: "Disconnect" },
    { keys: "1-6", description: "Navigate to page" },
    { keys: "?", description: "Keyboard shortcuts" },
  ],
};

export const LIST_SHORTCUTS: ShortcutGroup = {
  title: "Lists & Tables",
  shortcuts: [
    { keys: "j / ↓", description: "Move selection down" },
    { keys: "k / ↑", description: "Move selection up" },
    { keys: "Enter", description: "Open selected item" },
    { keys: "/", description: "Focus filter" },
    { keys: "Esc", description: "Clear / deselect" },
    { keys: "Backspace", description: "Go back" },
  ],
};

export const SESSION_SHORTCUTS: ShortcutGroup = {
  title: "Session Detail",
  shortcuts: [
    { keys: "Tab", description: "Toggle panes" },
    { keys: "Enter", description: "Jump Tool ↔ Output" },
    { keys: "s", description: "Skills tab" },
    { keys: "t", description: "Stats tab" },
    { keys: "f", description: "Fork (Codex)" },
    { keys: "Ctrl+X", description: "Errors only" },
    { keys: "Ctrl+O", description: "Errors first" },
  ],
};

export const PROCESS_SHORTCUTS: ShortcutGroup = {
  title: "Process Logs",
  shortcuts: [
    { keys: "1 / 2 / 3", description: "stdout / stderr / combined" },
    { keys: "c", description: "Clear logs" },
    { keys: "g", description: "Scroll to top" },
    { keys: "G", description: "Scroll to bottom" },
  ],
};

export const SPAWN_SHORTCUTS: ShortcutGroup = {
  title: "Spawn Form",
  shortcuts: [
    { keys: "Ctrl+Enter", description: "Submit" },
    { keys: "Esc", description: "Cancel" },
  ],
};

export const ALL_SHORTCUT_GROUPS: ShortcutGroup[] = [
  GLOBAL_SHORTCUTS,
  LIST_SHORTCUTS,
  SESSION_SHORTCUTS,
  PROCESS_SHORTCUTS,
  SPAWN_SHORTCUTS,
];
