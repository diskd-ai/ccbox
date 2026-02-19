# Changelog

All notable user-facing changes to `ccbox` are documented in this file.

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
