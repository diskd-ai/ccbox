# ccbox

TUI “box” for managing coding-agent sessions (Codex, Claude, Gemini, OpenCode): browse local session logs now, and later reconstruct exactly what the agent did (files, tools, tokens).

## Table of contents

- [Status](#status)
- [Key features (what it’s for)](#key-features-what-its-for)
  - [Skill spans (and loop detection)](#skill-spans-and-loop-detection)
  - [Timeline analysis (TUI + CLI)](#timeline-analysis-tui--cli)
  - [Fork/resume sessions (Codex)](#forkresume-sessions-codex)
  - [ccbox-insights skill (code-insights)](#ccbox-insights-skill-code-insights)
- [Install](#install)
- [Screenshots](#screenshots)
  - [Projects](#projects)
  - [Menu bar](#menu-bar)
  - [Session Detail (timeline)](#session-detail-timeline)
  - [Session actions (fork/resume)](#session-actions-forkresume)
  - [Session stats](#session-stats)
  - [Tasks](#tasks)
  - [Processes](#processes)
- [Roadmap](#roadmap)
- [Run](#run)
- [Skill (skills.sh)](#skill-skillssh)
- [Keybindings (prototype)](#keybindings-prototype)
- [License](#license)

## Status

Prototype features:
- Full-screen **Projects** → **Sessions** → **Session Detail** timeline
- “Online” dot (`●`) for recently modified projects/sessions
- Delete project/session logs with confirmation
- New Session prompt editor (`n`) that spawns Codex/Claude in `Pipes` or `TTY` mode
- Processes screen (`P`) for stdout/stderr/log viewing + killing spawned agents
- Attach/detach to spawned `TTY` sessions (`a` to attach, `Ctrl-]` to detach)
- Auto-rescans when session sources change (file watcher for Codex/Claude/Gemini/OpenCode)

## Key features (what it’s for)

### Skill spans (and loop detection)

When an agent session activates a skill (for example a commit helper, design-doc generator, or an install workflow), `ccbox` detects the skill boundaries and overlays that context on the timeline:
- **Visual overlay (TUI):** colored gutter markers show which timeline items happened “inside” which skill span; nested skills get their own span depth.
- **Loop detection:** repeated consecutive invocations of the same top-level skill are flagged so you can spot “skill recursion” quickly.
- **CLI export:** `ccbox skills` prints a per-skill summary (or `--json`) so other automation can reason over skill usage.

Use cases:
- “Why did this session burn tokens?”: see which skill dominated time/tool calls.
- “Which skill caused the failures?”: attribute tool failures to the active skill context.
- “Is a skill looping?”: catch repeated skill invocations early and adjust instructions/skills.

How to use:
- TUI: open a session → press `S` for the Skills overlay.
- CLI: `ccbox skills [log|project] [session-id] --json` (use `--id` if you prefer flags).

### Timeline analysis (TUI + CLI)

The Session Detail timeline is the evidence trail of what happened: user requests, assistant output, tool calls, tool outputs, and token/stat markers (when available).

Use cases:
- Root-cause a failed run by replaying tool calls and their outputs.
- Write a precise “what happened” report for teammates or incident notes.
- Confirm what was actually executed (commands, files touched) without guessing.

How to use:
- TUI: open a session → `Tab` changes focus between timeline/details → `Enter` jumps Tool → ToolOut.
- CLI: `ccbox history [log|project] [session-id] --full` for a complete, copy/paste-friendly timeline.

### Fork/resume sessions (Codex)

For Codex sessions, `ccbox` can fork/resume from a selected point in the timeline to create a new run with the same context up to that moment.

Use cases:
- “Try a different fix” from the same starting point without re-reading the whole session.
- Continue after a bad turn or failed tool call with a clean branch.
- Break a long session into smaller, more focused follow-up runs.

How to use:
- TUI: open a Codex session → select a Turn/User/Out/ToolOut item → press `f` (or use the Session menu).

### ccbox-insights skill (code-insights)

`ccbox-insights` is an installable agent skill that reads session history via `ccbox` (including optional skill-span context) and produces:
- A “lessons learned” memo backed by evidence
- A list of recurring failure patterns (invalid tool use vs runtime failures)
- Copy-ready, additive instruction snippets for project `AGENTS.md` (and optional global rules)

Use cases:
- After a week of work, turn noisy session logs into better standing instructions.
- Reduce repeated tool-call errors and avoid “clarify/correct” churn.
- Save time and tokens by standardizing the workflows that actually worked.

How to use:
- Install: `npx skills add diskd-ai/ccbox --skill ccbox-insights --global --yes`
- Prompt: “Use the ccbox-insights skill to analyze the latest N sessions for this project and propose AGENTS.md additions.”

## Install

Quick install from GitHub Releases (macOS/Linux):
```sh
curl -fsSL -H 'Cache-Control: no-cache' -o - https://raw.githubusercontent.com/diskd-ai/ccbox/main/scripts/install.sh | /bin/bash
```

Homebrew (recommended):
```sh
brew tap diskd-ai/ccbox
brew install ccbox
```

Developer build (from source): see `AGENTS.md`.

## Screenshots

### Projects

![Projects screen showing the menu bar, searchable projects list, and session-count/last-modified columns.](assets/projects.png)

What’s happening / features:
- Browse projects discovered from your local Codex/Claude/Gemini session logs and OpenCode sessions.
- Type to filter (matching text is highlighted); `Esc` clears.
- Shift+Arrows multi-select; `Del` deletes selected (with confirmation).
- Project table includes path, session count, and last modified time; `●` indicates a recently modified (“online”) project.

### Menu bar

![Menu bar with the Window menu open, showing available screens and shortcuts.](assets/menu-window.png)

What’s happening / features:
- `F2` opens the menu; arrows/Enter (and mouse) navigate.
- The Engine menu (Projects/Sessions) filters by agent engine: All/Codex/Claude/Gemini/OpenCode.
- The Window menu provides shortcuts to every screen.

### Session Detail (timeline)

![Session Detail screen showing the timeline (left) and details (right) with focus styling and scrollbars.](assets/timeline.png)

What’s happening / features:
- Timeline shows session events in order; details are always expanded.
- `Tab` switches focus (focused pane uses a double border); scrollbars indicate overflow.
- `Enter` jumps Tool → ToolOut; `o` previews the last Out; `F3` opens statistics.

### Session actions (fork/resume)

![Session menu showing actions like fork/resume, focus switching, result preview, and visible context.](assets/session-menu.png)

What’s happening / features:
- Fork/resume Codex from a selected Turn/User/Out/ToolOut record.
- Toggle Visible Context for the current turn.

### Session stats

![Session statistics window with duration, tokens, tool usage, and change summary.](assets/session-stats.png)

What’s happening / features:
- Time spent, token usage, tool-call breakdown (success/error/unknown), and `apply_patch` changes.

### Tasks

![Tasks screen showing tasks list with engine, project path, and image counts.](assets/tasks.png)

What’s happening / features:
- Type to filter; `n` creates; `Ctrl+Enter` spawns; Shift+Tab switches engine.

### Processes

![Processes screen showing spawned background agents with status and quick access to outputs.](assets/processes.png)

What’s happening / features:
- View output (`s`/`e`/`l`), kill (`k`), attach (`a`), and open the related session.

## Roadmap

See `ROADMAP.md`.

## Run

```bash
ccbox
```

CLI mode (no TUI):

```bash
ccbox projects
ccbox sessions                     # defaults to current folder (or a parent folder) project
ccbox sessions "/path/to/project"
ccbox history                      # defaults to latest session in current folder project
ccbox history "/path/to/session.jsonl"
ccbox history "/path/to/session.jsonl" --full
ccbox skills                       # defaults to latest session in current folder project
ccbox skills "/path/to/project"    # latest session in that project
ccbox skills "/path/to/project" "SESSION_ID"
ccbox skills --id "SESSION_ID" --json
ccbox sessions --limit 50 --offset 0 --size
ccbox history --limit 200 --offset 0 --full --size
ccbox update
```

CLI details:
- Auto-selects the project for the current folder (or nearest parent) when `project-path` is omitted.
- Pagination: `sessions` and `history` default to `--limit 10`; use `--limit N` and `--offset N`.
- `projects` output: `project_name<TAB>project_path<TAB>session_count`
- `sessions` output: `started_at_rfc3339<TAB>session_id<TAB>title<TAB>log_path` (newest-first; `--size` adds `file_size_bytes` before `log_path`)
- `history` accepts a session `.jsonl` path or a **project directory**; if a directory is provided it selects that project’s latest session.
- `history` prints a readable timeline; `--full` includes long details (tool calls/outputs, full messages); `--size` prints stats to stderr.
- `skills` accepts a session `.jsonl` path or a **project directory**, plus an optional `session-id` (positional or `--id`); `--json` prints structured spans/loops.
- Pipe-friendly output (handles broken pipes like `ccbox history | head`).
- Parse warnings and “truncated” notices are printed to stderr.
- On TUI start, `ccbox` checks for a newer GitHub Release in the background and shows a hint if one is available.

Optional overrides:
- `CODEX_SESSIONS_DIR` (defaults to `~/.codex/sessions`; Windows: `%USERPROFILE%\\.codex\\sessions`)
- `CLAUDE_PROJECTS_DIR` (defaults to `~/.claude/projects`)
- `CCBOX_GEMINI_DIR` (defaults to `~/.gemini`; sessions are discovered from `tmp/<project-hash>/chats/session-*.json`)
- `CCBOX_OPENCODE_DB_PATH` (defaults to `XDG_DATA_HOME/opencode/opencode.db`, else `~/.local/share/opencode/opencode.db`)

Notes:
- Spawning sessions requires `codex` on your `$PATH` (and `claude` if you switch engines).

## Skill (skills.sh)

This repo ships agent skills:

- `ccbox`: inspect local session logs using the `ccbox` CLI (`projects`, `sessions`, `history`)
- `ccbox-insights`: analyze tool-call failures in session logs and propose additive instructions (project `AGENTS.md` + global)

Install the code-insights skill (`ccbox-insights`):

```bash
npx skills add diskd-ai/ccbox --skill ccbox-insights --global --yes
```

Analyzes unsuccessful tool calls and suggests additive fixes in `AGENTS.md` (project-level).

Helps save time and tokens.

Install one or both:

```bash
npx skills add diskd-ai/ccbox --skill ccbox --global --yes
npx skills add diskd-ai/ccbox --skill ccbox-insights --global --yes
```

No additional setup is required after `npx skills add ... --global`.

Requirements: `ccbox` on your `$PATH` and access to your sessions directory (`CODEX_SESSIONS_DIR` if needed).

Example prompts:

- Codex: `codex "Use the ccbox skill to summarize the latest session for this repo."`
- Claude: `claude "Use the ccbox skill to summarize the latest session for this repo."`
- Gemini: `gemini "Use the ccbox skill to summarize the latest session for this repo."`
- Insights (project): `Use the ccbox-insights skill to analyze tool-call failures in the latest 20 sessions for "/path/to/project".`
- Insights (global for codex): `Use the ccbox-insights skill to analyze **codex** tool-call failures across my top 5 projects (by session count) and propose global instruction updates.`

## Keybindings (prototype)

- Global: `Ctrl+R` rescan · `F2` system menu · `P` processes · `F1`/`?` help · `Ctrl+Q`/`Ctrl+C` quit
- Mouse: wheel scrolls lists/outputs/details · left click selects/focuses
- Lists: arrow keys move selection · `PgUp`/`PgDn` page
- Projects: type to filter · `Esc` clears filter · `Enter` opens · `Space` result (newest session) · `Del` delete (confirm)
- Sessions: `Enter` opens · `Space` result (last Out) · `n` new session · `Del`/`Backspace` delete (confirm) · `Esc` back
- New Session: edit/paste · `Ctrl+Enter`/`Cmd+Enter` send · `Shift+Tab` switch engine · `F4` switch `Pipes`/`TTY` · `Esc` cancel
- Session Detail: `Enter` ToolOut (Tool call) · `f` fork/resume (Codex) · `o` result (last Out) · `c` visible context window · `Esc`/`Backspace` back
- Processes: `a` attach (TTY) · `Ctrl-]` detach · `s` stdout · `e` stderr · `l` log · `k` kill · `Enter` opens session (Codex only)

## License

MIT. See `LICENSE`.
