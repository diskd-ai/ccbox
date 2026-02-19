Codex Cyber Design Doc
======================

Context and motivation
----------------------
Codex CLI stores raw session logs on disk, but there is no dedicated local tool to browse them and (later) reconstruct exactly what happened in a session (file paths, tool calls, and token usage). This project builds a Rust TUI that indexes those logs from the local filesystem and presents them as navigable “projects → sessions → (future) session detail” views.

Goals:
- Read Codex session logs from disk (default: `$HOME/.codex/sessions`).
- Provide a full-screen TUI with navigation matching the reference screenshots:
  - projects list (home screen)
  - sessions list per project
- Keep parsing resilient to unknown/new fields (forward-compatible).
- Keep parsing and indexing logic pure and unit-testable; keep I/O at the edges.

Non-goals for first implementation (v1):
- Reconstructing the full “every token” timeline view.
- Full-fidelity rendering of assistant output and tool-call transcripts (v1 focuses on a message timeline with summaries; rich detail views are future work).
- Parity with Codex’s “recent projects” metrics (worktrees, sessions count badges, etc.).

Implementation considerations
-----------------------------
- Local-first and privacy: reads from local files only; no network calls required.
- Performance: session files are JSONL; individual lines can be extremely large (e.g., base instructions). Readers stream line-by-line and avoid loading whole files.
- Robustness: malformed/unexpected lines do not crash the app; errors become values and are surfaced as warnings or an error view.
- Purity boundary:
  - domain: pure parsing/title derivation/indexing
  - infra: filesystem traversal, file I/O
  - UI: rendering and input handling
- UI stack:
  - `ratatui` for layout and widgets
  - `crossterm` for terminal input/output
  - `ansi-to-tui` reserved for future “observer/session detail” views that render ANSI logs as `ratatui::text::Text`

High-level behavior
-------------------
- On startup:
  - Resolves sessions directory:
    - `CODEX_SESSIONS_DIR` if set
    - otherwise `$HOME/.codex/sessions`
  - Scans session files recursively and builds an in-memory index:
    - session summaries grouped by project (project key = session `cwd`)
    - projects sorted by most-recent session
  - Renders the Projects view.
- In Projects view:
  - Shows a filter input (search) and a list of projects.
  - Updates the displayed list as the user types (case-insensitive substring match).
  - Opens Sessions view for the selected project on `Enter`.
- In Sessions view:
  - Shows a list of sessions (newest first) for the selected project.
  - Allows selection movement with arrow keys.
  - Opens Session Detail view for the selected session on `Enter`.
  - Opens New Session view on `n`.
  - Deletes the selected session log on `Del` or `Backspace` with a confirmation dialog.
  - Returns to Projects view on `Esc`.
- In New Session view:
  - Shows a multiline prompt editor.
  - Default engine is Codex; `Shift+Tab` switches engines.
  - `Ctrl+Enter` / `Cmd+Enter` spawns a background agent process and returns to Sessions view.
  - `Esc` cancels and returns to Sessions view.
- In Session Detail view:
  - Shows a chronological list of “timeline items” reconstructed from the raw JSONL stream (messages, reasoning, tool calls, tool outputs, token counts).
  - Opens a “Visible Context” overlay window for the currently selected turn via a hotkey (not shown by default).
  - Returns to Sessions view on `Esc` or `Backspace`.
- Processes:
  - When one or more background agent processes are running, the footer shows a green `P●`.
  - Press `P` to open the Processes view.
- In Processes view:
  - Shows a list of spawned processes (Codex/Claude).
  - `s` opens stdout, `e` opens stderr, `l` opens combined log output.
  - `k` kills the selected process.
  - `Enter` opens Session Detail for the associated Codex session once its on-disk log path is discovered.
- Rescan:
  - `Ctrl+R` triggers a full rescan and replaces the in-memory index.
- Auto-rescan (watcher):
  - Watches the sessions directory and triggers a debounced rescan on `.jsonl` changes.
  - Preserves the current view and selection where possible.
- Quit:
  - `Ctrl+Q` or `Ctrl+C` exits.

Discovery rules
--------------
Observed on-disk structure:
- `$HOME/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`

Notes:
- The `YYYY/MM/DD` directory is based on local date, while `session_meta.payload.timestamp` is UTC (`...Z`), so date boundaries can differ by ±1 day.
- `ccbox` stores spawned-process logs under `$HOME/.codex/sessions/.ccbox/processes/pN/` (not treated as sessions).

Discovery algorithm (v1):
1. Walk the sessions directory recursively.
2. Consider files with `.jsonl` extension.
3. Read the first line and parse it as JSON:
   - Accept the file only if the first line is a `session_meta` event.
4. Extract `payload.cwd` as the project key (project path).
5. Derive a human title by scanning a bounded number of subsequent lines for a “real” user prompt.

Data format and validation
--------------------------
Each session file is a JSONL stream of events. v1 requires only a small subset:

- Session meta (first line; required to accept a file):
  - `type == "session_meta"`
  - `payload.id`: session id
  - `payload.timestamp`: session start timestamp (RFC3339 string)
  - `payload.cwd`: working directory (project key)

- User message text (for title derivation):
  - `type == "response_item"`
  - `payload.type == "message"`
  - `payload.role == "user"`
  - `payload.content[]` contains an item with `type == "input_text"` and a `text` field

Validation rules (v1):
- Any file that fails `session_meta` parsing is skipped and increments a warnings counter.
- Any line that fails user message parsing is ignored for title derivation (the session remains valid).

Additional events used for context reconstruction (Session Detail view)
----------------------------------------------------------------------
Session Detail view derives “timeline items” from these observed event shapes:

- Turn boundary / context injection:
  - `type == "turn_context"`
  - `payload.turn_id`: identifier used to group timeline items into turns
  - `payload.cwd`, `payload.model`, `payload.personality`
  - `payload.approval_policy`
  - `payload.sandbox_policy.type`
  - `payload.collaboration_mode.settings.developer_instructions` (long string)
  - `payload.user_instructions` (long string; often includes AGENTS + environment context)

- Assistant “thinking” (summaries):
  - `type == "response_item"`
  - `payload.type == "reasoning"`
  - `payload.summary[]` with entries like `{ "type": "summary_text", "text": "..." }`

- Assistant output:
  - `type == "response_item"`
  - `payload.type == "message"`
  - `payload.role == "assistant"`
  - `payload.content[]` includes items like `{ "type": "output_text", "text": "..." }`

- User messages (already used for titles, also appear in the timeline):
  - `type == "response_item"`
  - `payload.type == "message"`
  - `payload.role == "user"`
  - `payload.content[]` includes items like `{ "type": "input_text", "text": "..." }`

- Tool calls and their outputs:
  - `type == "response_item"`, `payload.type == "function_call"` with:
    - `payload.name`, `payload.arguments`, `payload.call_id`
  - `type == "response_item"`, `payload.type == "function_call_output"` with:
    - `payload.call_id`, `payload.output` (string)
  - `type == "response_item"`, `payload.type == "custom_tool_call"` with:
    - `payload.name`, `payload.input`, `payload.call_id`, `payload.status`
  - `type == "response_item"`, `payload.type == "custom_tool_call_output"` with:
    - `payload.call_id`, `payload.output` (JSON-as-string containing `output` + `metadata`)

- Token usage:
  - `type == "event_msg"`
  - `payload.type == "token_count"`
  - `payload.info.total_token_usage` and `payload.info.last_token_usage` include:
    - `input_tokens`, `cached_input_tokens`, `output_tokens`, `reasoning_output_tokens`, `total_tokens`

Title derivation (v1)
---------------------
Title is the first non-empty line of the first user message that is not metadata-only.

Metadata-only prompts skipped:
- `# AGENTS.md instructions …`
- `<environment_context> … </environment_context>`
- `<INSTRUCTIONS> … </INSTRUCTIONS>`
- `<skill> … </skill>`

Bounded scan:
- Scans at most 250 lines after the meta line for a title candidate.
- Falls back to `(untitled)` if none is found.

UI scaffolding
--------------
UI architecture is a thin shell over an immutable state machine with pure rendering.

Module boundaries (current scaffolding):
- `src/domain/*`: pure parsing, title derivation, project/session indexing
- `src/infra/*`: filesystem traversal and streaming JSONL reads
- `src/app/*`: model/update/command types (state machine)
- `src/ui/*`: `ratatui` rendering functions
- `src/main.rs`: composition root + terminal setup + event loop

Background agent processes (v1.2)
---------------------------------
The TUI can spawn new agent runs as separate OS processes and capture their output to disk for monitoring.

On-disk structure (under the sessions dir):
- `$SESSIONS_DIR/.ccbox/processes/pN/`
  - `prompt.txt` (initial prompt)
  - `stdout.log`
  - `stderr.log`
  - `process.log` (combined stdout/stderr with prefixes)
  - `last_message.txt` (Codex only; written by `--output-last-message`)

Spawning:
- Codex:
  - Runs `codex exec --full-auto --json --output-last-message <last_message.txt> -C <project> -`
  - Writes the prompt to stdin.
  - Sets `CODEX_SESSIONS_DIR=$SESSIONS_DIR` so the resulting session is persisted under the indexed directory.
- Claude:
  - Runs `claude --dangerously-skip-permissions --verbose --output-format stream-json -p <prompt>`
  - Does not produce a Codex session log (no session association in v1.2).

Session association (Codex only):
- Parses stdout JSONL until it sees a `session_meta` event, extracting:
  - `payload.id` (session id)
  - `payload.timestamp` (UTC RFC3339)
- Locates the on-disk `.jsonl` session file by searching the `YYYY/MM/DD` directory for the UTC day and adjacent days for a filename containing the session id.

Monitoring UI:
- While any process is running, the footer shows `P●`.
- `P` opens Processes view; `s`/`e`/`l` opens the corresponding log viewer.
- The output viewer tails the file on open and then appends new bytes while the view is visible.

Terminal loop (v1):
- Enters raw mode and alternate screen.
- Renders the current model every tick and on input.
- Polls for events (keyboard, resize).
- Converts input into `AppEvent`, applies `update(model, event) -> (model, command)`.
- Executes `AppCommand` at the boundary:
  - `Rescan` runs a scan and rebuilds `AppData`
  - `Quit` exits the loop

Views and widgets (v1):
- Projects view:
  - Top: search bar (a bordered `Paragraph`)
  - Middle: projects list (`List` with highlight + selection state)
  - Rows show an “online” indicator (`●`) when the most-recent session was modified recently
  - Bottom: footer with warnings, key hints, and `P●` when processes are running
- Sessions view:
  - Top: header bar with current project name/path
  - Middle: session list (`List` with highlight + selection state)
  - Rows show an “online” indicator (`●`) when the session is being written to recently
  - Bottom: footer with warnings, key hints, and `P●` when processes are running
- New Session view (v1.2):
  - Top: header bar with project name/path and a short hint
  - Middle: multiline prompt editor (supports paste)
  - Bottom: footer shows key hints plus current engine (blue) and `P●` when processes are running
- Session Detail view (v1.1):
  - Top: header bar showing session id + started-at + log file + size
  - Main: timeline list + details panel (side-by-side when wide, stacked when narrow)
  - Timeline rows include right-aligned time offset and tool-call duration columns
  - Bottom: footer with key hints and parse warnings
  - Overlay (hotkey, v1.1):
    - “Visible Context” window derived from `turn_context` for the selected timeline item’s turn
- Processes view (v1.2):
  - List of spawned agent processes with a running dot (`●`)
  - Right columns show process status and started time
- Process Output view (v1.2):
  - Scrollable output viewer for stdout/stderr/combined logs (live updates while open)
- Help overlay (modal, v1.1):
  - `F1` or `?` toggles a centered window that lists keybindings and view-specific hints
- Error view:
  - Full-screen panel describing the failure to load sessions
  - Key hints for retry/quit

Layout and navigation (v1):
- Selection movement:
  - arrow keys: move selection (up/down by 1)
- Projects:
  - typing filters projects; `Esc` clears the filter
  - `Del` (and `Backspace` when the filter is empty) opens “Delete Project Logs” confirmation
- Delete confirmation dialogs:
  - `←`/`→` toggles Cancel/Delete
  - `Enter` confirms the current choice
  - `Esc` cancels; `y`/`n` are shortcuts
- Sessions:
  - selection moves by 1 with arrow keys
  - `n` opens New Session
  - `Del`/`Backspace` opens “Delete Session Log” confirmation
  - `Esc` returns to Projects
- New Session:
  - multiline editor supports paste and arrow-key movement
  - `Shift+Tab` switches engines (Codex/Claude)
  - `Ctrl+Enter` / `Cmd+Enter` sends; `Esc` cancels
- Session Detail selection moves by 1 with arrow keys; `Enter` toggles summary/detail for the selected item.
- Session Detail overlay:
  - `c` toggles the “Visible Context” window for the currently selected item (if turn context is available)
  - `Esc` closes the overlay if open, otherwise navigates back
- Processes:
  - `P` opens Processes view
  - `s`/`e`/`l` opens stdout/stderr/log output viewers
  - `k` kills the selected process
  - `Esc` closes Processes/output viewers
- Help overlay:
  - `F1` or `?` toggles Help; `Esc` closes Help if open
- Resizing updates stored terminal size; selection remains stable.

ASCII preview (wireframes)
--------------------------
Projects view (full-screen “recent projects” list, based on the reference screenshot):

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│  Search projects...                                                          │
│  [ channels-hub / Downloads / mcp-hub / openclaw ... ]                        │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ▸ ● channels-hub  ·  ~/.../mono/channels-hub  ·  388 sessions  ·  just now  │
│      Downloads     ·  ~/Downloads              ·  1 session    ·  4h ago     │
│      mcp-hub       ·  ~/.../mono/mcp-hub       ·  140 sessions ·  6h ago     │
│      openclaw      ·  ~/src/openclaw           ·  3 sessions   ·  22h ago    │
│                                                                              │
├──────────────────────────────────────────────────────────────────────────────┤
│  Keys: arrows=move  Enter=open  Del=delete  Esc=clear  P=processes  Ctrl+R   │
│        rescan  Ctrl+Q/Ctrl+C=quit  F1/?=help  ·  warn:N                      │
└──────────────────────────────────────────────────────────────────────────────┘
```

Sessions view (project selected → list of sessions, based on the reference screenshot):

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│  Sessions · mcp-hub (/Users/.../mono/mcp-hub)                                 │
│  140 sessions · newest first                                                  │
├──────────────────────────────────────────────────────────────────────────────┤
│  TODAY                                                                       │
│                                                                              │
│  ▸ ● /clear                                                   · 6h ago       │
│    create API.md with detailed API description ...            · 7h ago       │
│    plz rewrite README.md in style of ../channels-hub          · 11h ago      │
│    ## Project Context Project: mcp-hub Language: TypeScript…  · 17h ago      │
│    ...                                                                        │
│                                                                              │
├──────────────────────────────────────────────────────────────────────────────┤
│  Keys: arrows=move  Enter=open  n=new  Del/Backspace=delete  Esc=back         │
│        P=processes  Ctrl+R=rescan  Ctrl+Q/Ctrl+C=quit  F1/?=help  ·  warn:N   │
└──────────────────────────────────────────────────────────────────────────────┘
```

New Session view (spawn a background agent from a project’s Sessions list):

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│  New Session · mcp-hub (/Users/.../mono/mcp-hub)                               │
│  Write a prompt, then press Ctrl+Enter/Cmd+Enter to send.                     │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  Type or paste a prompt…                                                     │
│                                                                              │
│                                                                              │
├──────────────────────────────────────────────────────────────────────────────┤
│  Keys: edit text  Ctrl+Enter/Cmd+Enter=send  Esc=cancel  Engine: Codex        │
│        (Shift+Tab)  ·  P●                                                    │
└──────────────────────────────────────────────────────────────────────────────┘
```

Session Detail view (selected session → messages/timeline; context is a hotkey overlay, based on Image #1):

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│  Session · 019c72b1…  ·  2026-02-18 21:39:39Z  ·  rollout-2026-02-18T…jsonl   │
│  Keys: arrows=move  Enter=toggle details  c=context  Esc=back/close  Ctrl+R   │
├──────────────────────────────────────────────────────────────────────────────┤
│  TIMELINE                                                                     │
│                                                                              │
│  Turn 019c72b7…                                                              │
│   - UserMessage  (# AGENTS…)                                                 │
│   - UserMessage  (<env_ctx>)                                                 │
│   - UserMessage  (/init …)                                                   │
│   - Thinking     (Creating…)                                                 │
│   - ToolCall     (exec_command)                                              │
│   - ToolOutput   (ls -la …)                                                  │
│   - ToolCall     (apply_patch)                                               │
│   - Output       (Initialized…)                                              │
│                                                                              │
│  Turn 019c72c0…                                                              │
│   - ToolCall     (cargo test)                                                │
│   - TokenCount   (total=…)                                                   │
│   - Output       (Updated …)                                                 │
│                                                                              │
│  (Press `c` to open Visible Context for the selected item’s turn)             │
├──────────────────────────────────────────────────────────────────────────────┤
│  warnings: N  ·  timeline items: M  ·  parsed turns: K                         │
└──────────────────────────────────────────────────────────────────────────────┘
```

Visible Context overlay (opened via `c` on a timeline item, based on Image #1):

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│  Session · 019c72b1…                                                          │
│  Keys: arrows=move  Enter=toggle details  c=context  Esc=back/close  Ctrl+R   │
│                                                                              │
│        ┌──────────────────────────────────────────────────────────────┐      │
│        │ Visible Context (Turn 019c72b7…)                              │      │
│        │                                                              │      │
│        │ cwd: /Users/alexeus/diskd/ccbox                               │      │
│        │ model: gpt-5.2  ·  sandbox: danger-full-access                │      │
│        │ approval: never  ·  personality: pragmatic                    │      │
│        │ user_instructions: 47,171 chars                               │      │
│        │ developer_instructions: present                               │      │
│        │ truncation_policy: bytes/10000                                │      │
│        │                                                              │      │
│        │ (future) diff vs previous turn: +files, +tools, +tokens, …    │      │
│        └──────────────────────────────────────────────────────────────┘      │
│                                                                              │
└──────────────────────────────────────────────────────────────────────────────┘
```

Processes view (global `P` hotkey; shows background agents):

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│  Processes                                                                    │
│  2 process(es)  ·  running: 1                                                 │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  ▸ ● p1  Codex   pid 12345  create API.md…                     · running · 1m │
│      p2  Claude  pid 23456  rewrite README…                     · exit 0 · 5m │
│                                                                              │
├──────────────────────────────────────────────────────────────────────────────┤
│  Keys: arrows=move  Enter=session  s=stdout  e=stderr  l=log  k=kill  Esc=back│
│        Ctrl+Q/Ctrl+C=quit  F1/?=help  ·  P●                                   │
└──────────────────────────────────────────────────────────────────────────────┘
```

Process output view (stdout/stderr/log; live updates while open):

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│  Process · p1 · Codex · pid 12345 · stdout                                    │
│  file: stdout.log                                                             │
├──────────────────────────────────────────────────────────────────────────────┤
│  {"type":"session_meta", ...}                                                 │
│  {"type":"turn_context", ...}                                                 │
│  {"type":"response_item", ...}                                                │
│  ...                                                                          │
├──────────────────────────────────────────────────────────────────────────────┤
│  Keys: arrows=scroll  s=stdout  e=stderr  l=log  k=kill  Esc=back  ·  P●      │
└──────────────────────────────────────────────────────────────────────────────┘
```

State management
----------------
The app is modeled as a small explicit state machine:
- `View = Projects(ProjectsView) | Sessions(SessionsView) | NewSession(NewSessionView) | SessionDetail(SessionDetailView) | Processes(ProcessesView) | ProcessOutput(ProcessOutputView) | Error`
- Additional modal/ephemeral state lives on the model (not inside `View`):
  - help overlay open/closed
  - delete confirmation dialogs
  - spawned process list + statuses
- `ProjectsView` state:
  - `query: String`
  - `filtered_indices: Vec<usize>` (computed from `query`)
  - `selected: usize` (selection index within the filtered set)
- `SessionsView` state:
  - `project_path: PathBuf` (selected project key)
  - `session_selected: usize`
- `NewSessionView` state (v1.2):
  - `engine: Codex | Claude`
  - multiline editor buffer + cursor
- `SessionDetailView` state (v1.1):
  - `selected_item: usize`
  - `show_details: bool` (toggle: summary-only vs expanded detail)
  - `context_overlay_open: bool`
- `ProcessesView` state (v1.2):
  - `selected: usize`
- `ProcessOutputView` state (v1.2):
  - `process_id: String`
  - `kind: stdout | stderr | log`
  - `scroll: u16`

Update contract:
- `update(model, event) -> (next_model, command)`
- `command` is one of:
  - `None`
  - `Rescan`
  - `Quit`
  - `OpenSessionDetail` (load + parse the selected session JSONL)
  - `DeleteProjectLogs` / `DeleteSessionLog`
  - `SpawnAgentSession` (starts a background agent process)
  - `KillProcess` / `OpenProcessOutput`
  - `OpenSessionDetailByLogPath` (from a running process)

Rendering / Output model
------------------------
Rendering is pure with respect to the current model:
- `ui::render(frame, model)` draws the entire screen for the active view.

Current visuals are intentionally “ASCII prototype”:
- bordered blocks with consistent padding
- simple list layout with right-aligned secondary columns (size/modified/status)
- selection highlight (yellow + `▸`)
- semantic colors (online `●`, engine hint, timeline kind labels)

Error handling and UX
---------------------
- If the resolved sessions directory does not exist:
  - The app enters Error view and displays:
    - resolved path
    - error text
    - how to override using `CODEX_SESSIONS_DIR`
- If a session file cannot be read or parsed:
  - The file is skipped; a warning counter increments and is shown in the footer.
- The app avoids panics for expected malformed input; failures are either non-fatal warnings or a typed load error.

Update cadence / Lifecycle
--------------------------
- Index loads at startup.
- `Ctrl+R` performs a full rescan and replaces the in-memory index.
- Future: optional incremental refresh beyond full rescan.

Future-proofing
---------------
- Parsing is tolerant:
  - unknown fields are ignored
  - missing optional fields become “unknown”
- Indexing is separated from rendering so the app can later add:
  - cached summaries (avoid rescanning titles)
  - deeper session detail (full tool args/output, file path diffs, token-level accounting)
  - observer view rendering ANSI logs via `ansi-to-tui`

Implementation outline
----------------------
1. Implement discovery and streaming JSONL reading for `session_meta` and title derivation.
2. Build an in-memory index: `ProjectSummary { sessions: Vec<SessionSummary> }`, sorted by recency.
3. Implement a `ratatui` app shell:
   - terminal setup/restore
   - event loop
   - state machine update + boundary commands
4. Implement Projects and Sessions views:
   - search + projects list
   - sessions list + navigation
5. Add unit tests for pure parsing/title rules.
6. Implement Session Detail view:
   - stream-parse JSONL into turns and timeline items
   - show timeline list; provide Visible Context as an overlay window (hotkey)
   - include token_count summaries where present
7. Add safe deletes:
   - confirmation dialogs for project/session deletes
   - infra deletion guard to prevent deleting outside the sessions dir
8. Add background agent spawning and monitoring:
   - New Session prompt editor + engine switch
   - ProcessManager that captures stdout/stderr into log files
   - Processes and Process Output views for monitoring/killing agents

Testing approach
----------------
- Unit tests (pure):
  - parse `session_meta` from a JSON line
  - extract `role=user` message text from a JSON line
  - metadata-prompt detection and title derivation
  - parse `turn_context` into a turn-context summary
  - parse tool call and tool output lines into timeline item summaries
- Unit tests (infra):
  - find spawned Codex session log path even when the sessions directory date is ±1 day from the UTC `session_meta` timestamp
- Manual:
  - run against real `$HOME/.codex/sessions` and verify navigation and rescans.
  - spawn a New Session and verify `P●` appears, Processes view opens, and logs update live.
  - delete a project/session and verify confirmation + on-disk deletion.

Acceptance criteria
-------------------
- Given a valid sessions directory, running `cargo run` opens a full-screen TUI and shows a projects list.
- Given a search query, typing updates the projects list in-place and selection stays within the filtered results.
- Given a selected project, pressing `Enter` opens a sessions list for that project.
- Given Projects view, pressing `Del` (or `Backspace` with an empty filter) opens a delete confirmation and never deletes without confirmation.
- Given Sessions view, pressing `Esc` returns to Projects view.
- Given Sessions view, pressing `Del` or `Backspace` opens a delete confirmation for the selected session log.
- Given Sessions view, pressing `n` opens New Session view.
- Given New Session view, pressing `Shift+Tab` switches engines (Codex/Claude).
- Given New Session view, pressing `Ctrl+Enter` / `Cmd+Enter` spawns a background agent process and returns to Sessions view.
- Given a selected session, pressing `Enter` opens Session Detail view and shows a timeline list of reconstructed items.
- Given Session Detail view, pressing `c` opens a Visible Context overlay for the selected item’s turn when `turn_context` is available.
- Given the overlay is open, pressing `Esc` closes it without leaving Session Detail view.
- Given Session Detail view, pressing `Esc` or `Backspace` returns to Sessions view.
- Given a running background process, the footer shows `P●`.
- Given any view, pressing `P` opens Processes view.
- Given Processes view, pressing `s`/`e`/`l` opens stdout/stderr/log output, and pressing `k` kills the selected process.
- Given any time, pressing `Ctrl+R` triggers a rescan and updates the visible projects/sessions.
- Given the sessions directory changes (new/updated `.jsonl`), the app auto-rescans and updates the index within a few seconds.
- Given any time, pressing `Ctrl+Q` or `Ctrl+C` exits cleanly and restores the terminal.
- Given unreadable/malformed session files, the app does not crash and increments a visible warnings counter.
- Given a missing sessions directory, the app shows an Error view that includes the resolved path and `CODEX_SESSIONS_DIR` override hint.
