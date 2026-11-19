import { create } from "zustand";
import type { Engine } from "@/domain/types";

type Theme = "dark" | "light" | "system";

interface PreferencesStore {
  theme: Theme;
  defaultEngine: Engine;
  logWordWrap: boolean;
  timelineSplitRatio: number;

  setTheme: (theme: Theme) => void;
  setDefaultEngine: (engine: Engine) => void;
  setLogWordWrap: (wrap: boolean) => void;
  setTimelineSplitRatio: (ratio: number) => void;
  initTheme: () => void;
}

function load<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(`ccbox-pref-${key}`);
    return raw !== null ? JSON.parse(raw) : fallback;
  } catch {
    return fallback;
  }
}

function save(key: string, value: unknown) {
  localStorage.setItem(`ccbox-pref-${key}`, JSON.stringify(value));
}

function applyThemeToDOM(theme: Theme) {
  const root = document.documentElement;
  if (theme === "dark") {
    root.classList.add("dark");
  } else if (theme === "light") {
    root.classList.remove("dark");
  } else {
    // system
    const prefersDark = window.matchMedia("(prefers-color-scheme: dark)").matches;
    if (prefersDark) {
      root.classList.add("dark");
    } else {
      root.classList.remove("dark");
    }
  }
}

export const usePreferencesStore = create<PreferencesStore>((set, get) => ({
  theme: load("theme", "system"),
  defaultEngine: load("defaultEngine", "codex"),
  logWordWrap: load("logWordWrap", true),
  timelineSplitRatio: load("timelineSplitRatio", 0.4),

  setTheme: (theme) => {
    save("theme", theme);
    set({ theme });
    applyThemeToDOM(theme);
  },

  initTheme: () => {
    const theme = get().theme;
    applyThemeToDOM(theme);

    // Listen for system changes when in "system" mode
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    mq.addEventListener("change", () => {
      if (get().theme === "system") {
        applyThemeToDOM("system");
      }
    });
  },

  setDefaultEngine: (engine) => {
    save("defaultEngine", engine);
    set({ defaultEngine: engine });
  },

  setLogWordWrap: (wrap) => {
    save("logWordWrap", wrap);
    set({ logWordWrap: wrap });
  },

  setTimelineSplitRatio: (ratio) => {
    save("timelineSplitRatio", ratio);
    set({ timelineSplitRatio: ratio });
  },
}));
