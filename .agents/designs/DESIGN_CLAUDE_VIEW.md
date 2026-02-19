Claude Sessions View Design Doc
===============================

Context and motivation
----------------------
`ccbox` currently indexes and renders Codex sessions from `~/.codex/sessions`. Claude Code stores
its session logs under `~/.claude`, and users want the same browsing experience (projects →
sessions → timelines) for Claude sessions.

Goals:
- Add a **separate Claude view** in the TUI (do not mix with Codex v1) that supports:
  - Projects list (derived from Claude sessions)
  - Sessions list per project
  - Session Detail timeline (timeline + details panes)
- Discover Claude sessions from `~/.claude/projects/**.jsonl`.
- Parse enough metadata to build the existing `ProjectSummary`/`SessionSummary` models:
  - `cwd` (project path)
  - `sessionId` (id)
  - `timestamp` (started_at)
  - `title` (inferred; see “Infer session names” roadmap item)
- Preserve functional architecture boundaries:
  - **domain**: pure parsing/formatting and typed events
  - **infra**: filesystem discovery + reads
  - **app/ui**: state + rendering
- Keep UX consistent with existing views (type-to-filter, `Enter` to open, paging, help).

Non-goals for first implementation (v1):
- Unified cross-engine browsing/search (Codex + Claude combined).
- Mutating Claude data (delete/rename/edit logs).
- Perfect parity of all Codex-specific timeline items (e.g., tool call/out linking) if Claude logs
  do not represent them similarly.
- Background process management changes (detached TTY “screen/tmux” is a separate roadmap item).

Implementation considerations
-----------------------------
- Claude’s log format is newline-delimited JSON with multiple record kinds; parsing must be
  tolerant and forward-compatible (unknown lines become “Note” items).
- Avoid relying on encoded project folder names; prefer `cwd` inside log records as the source of
  truth for the project path.
- Minimize duplication by reusing:
  - existing `ProjectSummary`/`SessionSummary`
  - existing timeline UI (List + Details) and stats window where possible
- Keep all I/O at the edges (infra). Domain parsing returns `Result` and never panics on bad input.

Discovery rules
---------------
- Claude sessions live under:
  - `~/.claude/projects/<project-key>/*.jsonl` (main session logs)
  - optionally: `~/.claude/projects/<project-key>/<sessionId>/subagents/*.jsonl`
- For v1, treat each `*.jsonl` file in `projects/<project-key>/` as a session log.
- To build session metadata, scan each session file and pick the first record that contains:
  - `cwd` (string path)
  - `sessionId` (string)
  - `timestamp` (RFC3339 / ISO-8601 string)
- Group sessions into projects by the parsed `cwd` path (project path).
  - Project name is `basename(cwd)` (same approach as Codex projects).

Data format and validation
--------------------------
Observed record shapes (examples from real logs):
- `{"type":"user", ... "cwd": "...", "sessionId":"...", "message":{"role":"user","content":"..."}, "timestamp":"..."}`
- `{"type":"assistant", ... "message":{"role":"assistant","content":[{"type":"text","text":"..."}]}}`
- `{"type":"file-history-snapshot", ...}`

Parsing approach:
- Parse each line as JSON. If it fails, keep a “Note” timeline item with the raw line.
- For known record kinds, extract a minimal typed event:
  - User text
  - Assistant text
  - Assistant thinking (optional separate timeline kind)
  - Other/unknown → Note
- Timestamp handling:
  - Prefer the record `timestamp` when present.
  - Convert to both the raw string (for display) and `timestamp_ms` (for sorting/offset math).

High-level behavior
-------------------
- The main menu includes a way to enter the Claude view (exact placement is a UI decision, but it
  is accessible without leaving the TUI).
- Claude Projects screen:
  - Lists projects discovered from Claude sessions (derived from `cwd`)
  - Supports type-to-filter and `Esc` to clear filter
  - Shows session counts and “online” dot based on file modified time (same policy as Codex)
- Claude Sessions screen:
  - Lists sessions for the selected project, newest-first
  - `Enter` opens Session Detail
- Claude Session Detail screen:
  - Shows a timeline list + details pane (same UI as Codex Session Detail)
  - Uses the same focus model (`Tab` switches focus; focused pane highlighted; scrollbars shown
    when content overflows)

Error handling and UX
---------------------
- If `~/.claude` does not exist or is unreadable:
  - Show an in-app notice explaining the expected location and how to proceed.
  - Do not crash; allow returning to other views.
- If individual session files are unreadable or malformed:
  - Skip the session with a warning counter (surface in footer).
  - Do not block the rest of the scan.

Update cadence / Lifecycle
--------------------------
- Claude data is refreshed on:
  - app start (initial scan)
  - `Ctrl+R` rescan
  - file-watcher events (optional v1; acceptable to keep watcher Codex-only in v1)

Future-proofing
---------------
- Keep Claude parsing isolated (`domain/claude_*`, `infra/claude_*`) so adding Gemini or unified
  search does not require rewriting the Codex pipeline.
- Prefer a generic “engine source” abstraction that can later support:
  - Codex sessions dir
  - Claude sessions dir
  - Gemini sessions dir
  - merged “All” view and unified search index

Implementation outline
----------------------
1. Domain
   - Add a Claude log parser that:
     - extracts `SessionMeta` and inferred title
     - converts records into `TimelineItem` values
2. Infra
   - Add a Claude scanner that walks `~/.claude/projects` and returns `Vec<SessionSummary>`
   - Reuse existing `index_projects` grouping by `cwd`
3. App
   - Add new views for Claude (projects/sessions/detail) and navigation wiring
   - Ensure global actions still work (`Ctrl+R`, help, processes)
4. UI
   - Reuse list/detail rendering with Claude-specific headers/titles
5. Docs
   - Update README roadmap and usage notes as needed.

Testing approach
----------------
- Unit tests (domain):
  - Parse representative Claude JSONL lines into typed events (user/assistant/thinking)
  - Ensure unknown records become “Note” without panicking
- Integration tests (infra):
  - Scan a temporary `projects/` directory containing fixture `.jsonl` files
  - Verify project grouping, sorting newest-first, and file size/mtime extraction

Acceptance criteria
-------------------
- Given a populated `~/.claude/projects`, `ccbox` shows a Claude Projects list derived from `cwd`.
- Given a selected Claude project, `ccbox` lists sessions newest-first and opens a session with
  `Enter`.
- Session Detail renders a timeline and details pane for Claude sessions and supports `Tab`
  focus switching + scrolling.
- Malformed lines or unknown record types do not crash the app and instead appear as “Note” items
  or are counted as warnings.
