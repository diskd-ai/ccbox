# ccbox CLI reference (sessions inspection)

## Sessions directory

`ccbox` scans the Codex sessions directory:

- If `CODEX_SESSIONS_DIR` is set, use it.
- Otherwise, default to `~/.codex/sessions` (platform-specific home directory).

## Commands

### `ccbox projects`

List discovered projects.

Output: **tab-separated** columns

1. `project_name`
2. `project_path` (absolute)
3. `session_count` (integer)

### `ccbox sessions [project-path]`

List sessions for a project.

- If `project-path` is omitted, select the project matching the current folder (or the nearest parent folder with sessions).
- Sessions are printed newest-first.

Output: **tab-separated** columns

1. `started_at_rfc3339`
2. `session_id`
3. `title` (first non-metadata user prompt line when available)
4. `log_path` (absolute `.jsonl` path)

### `ccbox history [log-path] [--full]`

Print a readable session timeline.

- If `log-path` is omitted, select the latest session for the current folder project (if it exists).
- `--full` prints the full detail body for each timeline item (tool outputs, long assistant messages, etc.).

Output: plain text (pipe-friendly), grouped by turns.

