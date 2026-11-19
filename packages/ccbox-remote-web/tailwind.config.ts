import type { Config } from "tailwindcss";

export default {
  darkMode: ["class"],
  content: ["./pages/**/*.{ts,tsx}", "./components/**/*.{ts,tsx}", "./app/**/*.{ts,tsx}", "./src/**/*.{ts,tsx}"],
  prefix: "",
  theme: {
    container: {
      center: true,
      padding: "2rem",
      screens: {
        "2xl": "1400px",
      },
    },
    fontFamily: {
      mono: ["var(--font-mono)"],
      sans: ["var(--font-sans)"],
    },
    fontSize: {
      xs: ["var(--text-xs)", { lineHeight: "1.4" }],
      sm: ["var(--text-sm)", { lineHeight: "1.5" }],
      base: ["var(--text-base)", { lineHeight: "1.5" }],
      md: ["var(--text-md)", { lineHeight: "1.5" }],
      lg: ["var(--text-lg)", { lineHeight: "1.4" }],
      xl: ["var(--text-xl)", { lineHeight: "1.3" }],
      "2xl": ["var(--text-2xl)", { lineHeight: "1.2" }],
    },
    extend: {
      colors: {
        border: "hsl(var(--border))",
        input: "hsl(var(--input))",
        ring: "hsl(var(--ring))",
        background: "hsl(var(--background))",
        foreground: "hsl(var(--foreground))",
        primary: {
          DEFAULT: "hsl(var(--primary))",
          foreground: "hsl(var(--primary-foreground))",
        },
        secondary: {
          DEFAULT: "hsl(var(--secondary))",
          foreground: "hsl(var(--secondary-foreground))",
        },
        destructive: {
          DEFAULT: "hsl(var(--destructive))",
          foreground: "hsl(var(--destructive-foreground))",
        },
        muted: {
          DEFAULT: "hsl(var(--muted))",
          foreground: "hsl(var(--muted-foreground))",
        },
        accent: {
          DEFAULT: "hsl(var(--accent))",
          foreground: "hsl(var(--accent-foreground))",
        },
        popover: {
          DEFAULT: "hsl(var(--popover))",
          foreground: "hsl(var(--popover-foreground))",
        },
        card: {
          DEFAULT: "hsl(var(--card))",
          foreground: "hsl(var(--card-foreground))",
        },
        sidebar: {
          DEFAULT: "hsl(var(--sidebar-background))",
          foreground: "hsl(var(--sidebar-foreground))",
          primary: "hsl(var(--sidebar-primary))",
          "primary-foreground": "hsl(var(--sidebar-primary-foreground))",
          accent: "hsl(var(--sidebar-accent))",
          "accent-foreground": "hsl(var(--sidebar-accent-foreground))",
          border: "hsl(var(--sidebar-border))",
          ring: "hsl(var(--sidebar-ring))",
        },
        // ccbox semantic
        surface: "hsl(var(--bg-surface))",
        elevated: "hsl(var(--bg-elevated))",
        "bg-hover": "hsl(var(--bg-hover))",
        "bg-active": "hsl(var(--bg-active))",
        "text-primary": "hsl(var(--text-primary))",
        "text-secondary": "hsl(var(--text-secondary))",
        "text-muted": "hsl(var(--text-muted))",
        // Engine colors
        "engine-codex": "hsl(var(--engine-codex))",
        "engine-claude": "hsl(var(--engine-claude))",
        "engine-gemini": "hsl(var(--engine-gemini))",
        "engine-opencode": "hsl(var(--engine-opencode))",
        // Status colors
        "status-online": "hsl(var(--status-online))",
        "status-running": "hsl(var(--status-running))",
        "status-exited": "hsl(var(--status-exited))",
        "status-error": "hsl(var(--status-error))",
        "status-warning": "hsl(var(--status-warning))",
      },
      borderRadius: {
        lg: "var(--radius)",
        md: "calc(var(--radius) - 2px)",
        sm: "calc(var(--radius) - 4px)",
      },
      keyframes: {
        "accordion-down": {
          from: { height: "0" },
          to: { height: "var(--radix-accordion-content-height)" },
        },
        "accordion-up": {
          from: { height: "var(--radix-accordion-content-height)" },
          to: { height: "0" },
        },
      },
      animation: {
        "accordion-down": "accordion-down 0.2s ease-out",
        "accordion-up": "accordion-up 0.2s ease-out",
      },
    },
  },
  plugins: [require("tailwindcss-animate")],
} satisfies Config;
