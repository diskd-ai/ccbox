# Changelog

All notable user-facing changes to `ccbox` are documented in this file.

## [Unreleased]

### Changes

### Fixes

## [0.1.7] - 2026-02-19

### Changes

- TUI: add Tasks window backed by SQLite (`~/.ccbox/tasks.db`) to create/view/delete/spawn reusable tasks (supports `[Image N]` references).

### Fixes

## [0.1.6] - 2026-02-19

### Changes

- TUI: show `F1`/`F2`/`F3` shortcuts in the menu bar.
- TUI: Session Stats window uses colored metrics and comma-formatted numbers.
- TUI: `F3` shows project-level statistics (uses a cached session index for fast startup).
- TUI: add "New Task" to the System menu.
- TUI: add a Window menu to navigate between screens/windows.
- TUI: focused pane borders are double-line white; unfocused borders are dark gray.
- TUI: add top/bottom padding lines inside menu dropdowns.
- Docs: explain installing the `ccbox` skill for Codex/Claude/Gemini.

### Fixes

- TUI: Details pane scrolling accounts for wrapped content so it can reach the end.

## [0.1.5] - 2026-02-19

### Changes

- TUI: show update-available hint in green.
- TUI: show colored `F1`/`F2`/`F3` shortcuts in the menu bar.
- TUI: remove the emoji from the System menu label.
- Docs: move roadmap to `ROADMAP.md`.

## [0.1.4] - 2026-02-19

### Changes

- TUI: main menu (`F2`) with global + view-specific actions.
- TUI: Session Detail supports pane focus (`Tab`) and scrollbars for long Timeline/Details content.
- TUI: session timeline details are always expanded; `Enter` now only jumps Tool -> ToolOut.
- TUI: highlight update-available hint in light green.
- TUI: pretty-print JSON in the timeline details view.
- TUI: add a Session Stats window (`F3`) for duration/tokens/tool outcomes and `apply_patch` changes.
- TUI: Sessions view supports type-to-filter (search).
- TUI: highlight matched text in Projects/Sessions filters.
- TUI: show current version in the footer.
- TUI: help window shows app name/version and a short intro header.
- TUI: align the menu bar/menu overlay with content padding.

## [0.1.3] - 2026-02-19

### Changes

- Documentation improvements: roadmap now lists Claude sessions/spawning first; run and CLI examples use `ccbox`.
- Contributor guidelines: add a release checklist to `AGENTS.md`.

## [0.1.2] - 2026-02-19

### Changes

- CLI: add pagination to `sessions` and `history` via `--limit` (default: 10) and `--offset`.
- CLI: add `--size` for session file sizes and history stats.
- Update: add background update checks on start and `ccbox update` self-updater (macOS/Linux).

## [0.1.1] - 2026-02-19

### Changes

- Release pipeline: add GitHub Actions workflow that builds archives for macOS, Linux, and Windows.
- Install: add `scripts/install.sh` for quick install on macOS/Linux.
- CLI: add non-TUI mode (`projects`, `sessions`, `history`) and bundled `ccbox` skill.
- Documentation: add README screenshots.

### Fixes

- CLI: `history` accepts a project directory and uses the latest session (avoids hanging when a directory path is provided).
- Windows: improve default sessions directory resolution.

## [0.1.0] - 2026-02-19

### Changes

- Initial TUI prototype: browse projects/sessions/timeline, spawn sessions, view processes, and auto-rescan with a file watcher.
