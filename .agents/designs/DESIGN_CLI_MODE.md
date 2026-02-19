ccbox CLI Mode Design Doc
=========================

Context and motivation
----------------------
`ccbox` is currently a TUI-first tool. For quick inspection (and for scripting/automation), it is useful to run `ccbox` without a full-screen UI and instead use a command-line interface to:
- list discovered projects
- list sessions for a project
- print a session’s history/timeline

Goals:
- Keep the current behavior unchanged: running `ccbox` with no args launches the TUI.
- Add CLI subcommands to:
  - list projects discovered under the sessions directory
  - list sessions for a given project
  - print session history from a `.jsonl` session log file
- Reuse existing parsing/indexing/timeline derivation logic:
  - domain stays pure (no I/O)
  - infra performs filesystem reads
  - entrypoint chooses TUI vs CLI based on args
- Make CLI output pipe-friendly (no ANSI required; stable text output).

Non-goals for first implementation (v1):
- JSON output mode, rich formatting, paging, or interactive selection.
- Mutating operations from the CLI (delete logs, spawn processes, kill processes).
- Advanced filtering/search beyond selecting a single project path.
- Cross-project queries (e.g., “sessions --all”) beyond what is required.

Implementation considerations
-----------------------------
- Keep changes additive: introduce a small `cli` module and route to it from `main`.
- Avoid adding heavy dependencies (e.g., `clap`) for v1; implement a small, typed args parser.
- Preserve the purity boundary:
  - CLI argument parsing is pure and unit-testable.
  - CLI commands call existing infra functions (`scan_sessions_dir`, `load_session_timeline`) and format output.
- Error handling is explicit:
  - unknown command/flags produce a clear usage message and a non-zero exit
  - missing projects/sessions produce a clear message and a non-zero exit

High-level behavior
-------------------
- `ccbox` with no args starts the TUI (current behavior).
- `ccbox --help` prints usage for both TUI and CLI modes.
- `ccbox --version` prints the package version (current behavior).
- `ccbox <subcommand> ...` runs the CLI command and exits.

CLI interface
-------------
Subcommands (v1):
- `ccbox projects`
  - Scans the sessions directory and prints one line per project.
  - Each line includes: project name, project path, session count.
- `ccbox sessions [project-path]`
  - If `project-path` is omitted, selects the project matching the current folder (or the nearest parent folder with sessions).
  - Scans the sessions directory, finds the matching project, and prints sessions newest-first.
  - Each line includes: started timestamp, session id, title, log path.
- `ccbox history [session-log-path] [--full]`
  - Loads the session timeline and prints a readable history.
  - If `session-log-path` is omitted, selects the latest session for the current folder project (if it exists).
  - Default output prints one line per timeline item: kind + summary (plus optional timestamp/turn id when present).
  - With `--full`, prints the item detail after each summary line.

Output model
------------
- Output is plain UTF-8 text to stdout.
- Errors are printed to stderr with a short, actionable message.
- The CLI does not change terminal modes (no raw mode, no alternate screen).

Error handling and UX
---------------------
Error categories:
- User errors (bad args, missing required parameters):
  - Print usage summary + specific error (e.g., “missing project-path”).
- Data errors (sessions dir missing, unreadable files, parse warnings):
  - If sessions dir is missing: print the resolved sessions dir and how to override (`CODEX_SESSIONS_DIR`).
  - If parsing produces warnings: print a trailing “warnings: N” line to stderr.
- Session history errors:
  - If the log path is unreadable: print the path and the OS error.
  - If the timeline is truncated: print a trailing “truncated: true” line to stderr.

Update cadence / Lifecycle
--------------------------
- CLI commands perform a one-time scan/load and exit.
- No file watching or auto-rescan in CLI mode (TUI keeps watcher behavior).

Future-proofing
---------------
- The CLI command enum is designed to add new read-only commands without changing existing ones.
- A future JSON output mode can be introduced behind an explicit flag (e.g., `--json`) without breaking the default text output.
- If argument parsing grows, the internal ADT stays stable and can be backed by a parser library later.

Implementation outline
----------------------
1. Add `src/cli/mod.rs`:
   - `CliCommand` ADT
   - pure `parse_args(Vec<String>) -> Result<CliInvocation, CliParseError>`
   - `run_cli(command, sessions_dir)` that performs infra calls and prints output
2. Update `src/main.rs`:
   - route to CLI mode before terminal setup
   - keep existing TUI path unchanged
3. Add unit tests for CLI arg parsing and basic formatting.
4. Update `README.md` with CLI usage examples (projects/sessions/history).

Testing approach
----------------
- Unit tests (pure):
  - CLI argument parsing (valid/invalid inputs)
  - formatting helpers (stable output for known inputs)
- Manual tests:
  - `ccbox projects` prints projects for a known fixture sessions dir.
  - `ccbox sessions <path>` prints sessions and exits 0.
  - `ccbox history <log>` prints timeline; `--full` prints details.

Acceptance criteria
-------------------
- Given no args, `ccbox` starts the existing TUI as before.
- Given `ccbox projects`, the command prints at least one line per discovered project and exits 0.
- Given `ccbox sessions <project-path>` where `<project-path>` matches a discovered project, the command prints that project’s sessions newest-first and exits 0.
- Given `ccbox history <session-log-path>`, the command prints a readable history derived from the existing timeline parser and exits 0.
- Given an unknown subcommand, `ccbox` prints a usage hint and exits non-zero.
