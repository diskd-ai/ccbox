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

## Roadmap

1. Manage spawned sessions (processes) and their lifecycle.
2. Manage a tasks queue and assign tasks to an engine (Codex/Claude).
3. Support multi-session workflows (handoffs, context carry, session grouping).
4. Manage Ralph loops.
5. Loops planning and task decomposition routines.
6. Remote control via channels:
   - JSON-RPC
   - Telegram
   - Slack
   - WhatsApp
   - Email

## Run

```bash
cargo run
```

Optional overrides:
- `CODEX_SESSIONS_DIR` (defaults to `$HOME/.codex/sessions`)

Notes:
- Spawning sessions requires `codex` on your `$PATH` (and `claude` if you switch engines).

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
