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
- Pagination: defaults to `--limit 10 --offset 0`. Use `--limit N` and `--offset N` to paginate.
- `--size` adds a `file_size_bytes` column.

Output: **tab-separated** columns

1. `started_at_rfc3339`
2. `session_id`
3. `title` (first non-metadata user prompt line when available)
4. `log_path` (absolute `.jsonl` path)

With `--size`, output columns are:

1. `started_at_rfc3339`
2. `session_id`
3. `title`
4. `file_size_bytes` (integer)
5. `log_path`

### `ccbox history [log-path] [--full] [--limit N] [--offset N] [--size]`

Print a readable session timeline.

- If `log-path` is omitted, select the latest session for the current folder project (if it exists).
- If `log-path` is a directory, treat it as a project path and select that project’s latest session.
- Pagination: defaults to `--limit 10 --offset 0`. Use `--limit N` and `--offset N` to paginate.
- `--full` prints the full detail body for each timeline item (tool outputs, long assistant messages, etc.).
- `--size` prints a stats line to stderr: file bytes + item counts for the current pagination window.

Output: plain text (pipe-friendly), grouped by turns.
