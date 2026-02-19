# ccbox Roadmap

This roadmap lists planned user-facing improvements. Items are grouped by theme and tracked as
checkboxes.

## Multi-agent

- [ ] Claude sessions browser (projects/sessions/timeline) from `~/.claude` (separate view)
- [ ] Gemini sessions browser + spawning
- [ ] Normalize session model across engines (Codex/Claude/Gemini)

## UX / Navigation

- [x] Focused pane indicator + `Tab` focus switch + scrollbars for multi-pane views (Session Detail)
- [ ] Infer better session names/titles
- [ ] Running sessions switcher (jump to live processes from Projects/Sessions + quick return hotkey)

## Search

- [ ] Unified Search: one search box across all agents
- [ ] Full-text search: search the whole conversation content
- [ ] Very fast: USearch-powered indexing + queries
- [ ] Fuzzy matching: typo-tolerant search with smart ranking
- [ ] Beautiful TUI: fzf-style results with agent icons + live preview
- [ ] Direct resume: select + `Enter` to open a session

## Tasks

- [ ] Tasks window (System menu)
  - [ ] Create task (fullscreen editor; supports inserting images as `[Image N]` references)
  - [ ] View task
  - [ ] Delete task
  - [ ] Spawn task execution (Codex/Claude)
  - [ ] Persist tasks in `~/.ccbox/tasks.db` (SQLite) with migrations (rusqlite + sqlx)

## Spawning / Processes

- [x] Spawn Codex/Claude processes + Processes screen (stdout/stderr/log, kill, open session log)
- [ ] Spawn agents in detached TTY sessions (screen/tmux-like): interactive stdin/stdout
- [ ] Switch into a running session from Projects/Sessions and back (tmux-like hotkey)
- [ ] Fork/resume Codex from a timeline record (mid-session resume)
- [ ] Show session diff (files changed + summary)

## Automation / Loops

- [ ] Multi-session workflows (handoffs, context carry, session grouping)
- [ ] Manage Ralph loops
- [ ] Loops planning + task decomposition routines

## Remote control

- [ ] JSON-RPC
- [ ] Telegram
- [ ] Slack
- [ ] WhatsApp
- [ ] Email

## Updates

- [x] Update notifications on start + `ccbox update` self-updater
