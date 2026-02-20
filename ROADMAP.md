# ccbox Roadmap

This roadmap lists planned user-facing improvements. Items are grouped by theme and tracked as
checkboxes.

## Multi-agent

- [x] Claude sessions browser (separate module; projects/sessions/timeline) from `~/.claude/projects`
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

- [x] Tasks window (System menu)
  - [x] Create task (fullscreen editor; supports inserting images as `[Image N]` references)
  - [x] View task
  - [x] Delete task
  - [x] Spawn task execution (Codex/Claude)
  - [x] Persist tasks in `~/.ccbox/tasks.db` (SQLite) with migrations (rusqlite + sqlx)

## Spawning / Processes

- [x] Spawn Codex/Claude processes + Processes screen (stdout/stderr/log, kill, open session log)
- [x] TTY spawn mode with attach/detach (spawn from New Session; attach from Processes)
- [ ] Switch into a running session from Projects/Sessions and back (tmux-like hotkey)
- [x] Fork/resume Codex from a timeline record (mid-session resume)
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
