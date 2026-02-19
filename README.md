# ccbox

Rust TUI “box” for managing coding-agent sessions (Codex + Claude): browse local session logs now, and later reconstruct exactly what the agent did (files, tools, tokens).

## Status

Prototype features:
- Full-screen **Projects** → **Sessions** → **Session Detail** timeline
- “Online” dot (`●`) for recently modified projects/sessions
- Delete project/session logs with confirmation
- New Session prompt editor (`n`) that spawns Codex/Claude in the background
- Processes screen (`P`) for stdout/stderr/log viewing + killing spawned agents
- Auto-rescans when the sessions directory changes (file watcher)

## Install

Quick install from GitHub Releases (macOS/Linux):
```sh
/bin/bash -c "$(curl -fsSL -H 'Cache-Control: no-cache' https://raw.githubusercontent.com/diskd-ai/ccbox/main/scripts/install.sh)"
```

Homebrew (recommended):
```sh
brew tap diskd-ai/ccbox
brew install ccbox
```

From source:
```sh
git clone https://github.com/diskd-ai/ccbox.git
cd ccbox
cargo install --path .
```

## Screenshots

### Projects

![Projects screen showing local session-log projects with live “online” indicators and type-to-filter search.](assets/projects.png)

What’s happening / features:
- Browse all discovered projects under your sessions directory.
- Type to filter; `Esc` clears.
- `●` indicates a recently modified (“online”) project.

### Sessions

![Sessions screen showing a project’s sessions list with navigation, quick “result” access, and delete controls.](assets/sessions.png)

What’s happening / features:
- Browse sessions for the selected project and open them with `Enter`.
- `Space` jumps to the newest “last Out” result.
- Delete logs with `Del`/`Backspace` (with confirmation).

### Session Detail (timeline)

![Session Detail timeline screen showing chronological events with expandable details and context window controls.](assets/timeline.png)

What’s happening / features:
- Timeline view of the session: messages, outputs, and other events in order.
- `Enter` toggles expanded details; `c` adjusts the visible context window.
- `o` copies/opens the last “Out” result quickly.

### New Session

![New Session prompt editor screen showing a multi-line prompt with engine switching and send controls.](assets/new-session.png)

What’s happening / features:
- Paste/edit a prompt, then spawn Codex/Claude in the background.
- `Shift+Tab` switches engine; `Ctrl+Enter`/`Cmd+Enter` sends.

## Roadmap

1. Gemini support: sessions view + spawning.
2. Manage spawned sessions (processes) and their lifecycle.
3. Manage a tasks queue and assign tasks to an engine (Codex/Claude).
4. Support multi-session workflows (handoffs, context carry, session grouping).
5. Manage Ralph loops.
6. Loops planning and task decomposition routines.
7. Remote control via channels:
   - JSON-RPC
   - Telegram
   - Slack
   - WhatsApp
   - Email

## Run

```bash
cargo run
```

CLI mode (no TUI):

```bash
cargo run -- projects
cargo run -- sessions                     # defaults to current folder (or a parent folder) project
cargo run -- sessions "/path/to/project"
cargo run -- history                      # defaults to latest session in current folder project
cargo run -- history "/path/to/session.jsonl"
cargo run -- history "/path/to/session.jsonl" --full
```

CLI details:
- Auto-selects the project for the current folder (or nearest parent) when `project-path` is omitted.
- `projects` output: `project_name<TAB>project_path<TAB>session_count`
- `sessions` output: `started_at_rfc3339<TAB>session_id<TAB>title<TAB>log_path` (newest-first)
- `history` accepts a session `.jsonl` path or a **project directory**; if a directory is provided it selects that project’s latest session.
- `history` prints a readable timeline; `--full` includes long details (tool calls/outputs, full messages).
- Pipe-friendly output (handles broken pipes like `ccbox history | head`).
- Parse warnings and “truncated” notices are printed to stderr.

Optional overrides:
- `CODEX_SESSIONS_DIR` (defaults to `~/.codex/sessions`; Windows: `%USERPROFILE%\\.codex\\sessions`)

Notes:
- Spawning sessions requires `codex` on your `$PATH` (and `claude` if you switch engines).

## Skill (skills.sh)

Install the `ccbox` skill for agents:

```bash
npx skills add diskd-ai/ccbox --skill ccbox --global --yes
```

## Keybindings (prototype)

- Global: `Ctrl+R` rescan · `P` processes · `F1`/`?` help · `Ctrl+Q`/`Ctrl+C` quit
- Lists: arrow keys move selection · `PgUp`/`PgDn` page
- Projects: type to filter · `Esc` clears filter · `Enter` opens · `Space` result (newest session) · `Del` delete (confirm)
- Sessions: `Enter` opens · `Space` result (last Out) · `n` new session · `Del`/`Backspace` delete (confirm) · `Esc` back
- New Session: edit/paste · `Ctrl+Enter`/`Cmd+Enter` send · `Shift+Tab` switch engine · `Esc` cancel
- Session Detail: `Enter` toggles details · `o` result (last Out) · `c` visible context window · `Esc`/`Backspace` back
- Processes: `s` stdout · `e` stderr · `l` log · `k` kill · `Enter` opens session (Codex only)

## License

MIT. See `LICENSE`.
