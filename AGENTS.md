# Repository Guidelines

## Project Structure & Module Organization

- `src/main.rs`: composition root (terminal setup + event loop).
- `src/domain/`: pure parsing/formatting + domain types (no I/O).
- `src/infra/`: adapters for filesystem/env (e.g., scanning Codex sessions).
- `src/app/`: state model + update logic (events -> next state + commands).
- `src/ui/`: Ratatui rendering.

Reference docs: `README.md`.

## Build, Test, and Development Commands

```bash
cargo run                      # run the TUI
cargo test                     # run unit tests
cargo fmt                      # format (rustfmt)
cargo clippy -- -D warnings    # lint (fail on warnings)
cargo build --release          # optimized build
```

Config example:

```bash
CODEX_SESSIONS_DIR=/path/to/sessions cargo run
```

## Coding Style & Naming Conventions

- Rust 2024 edition; keep a strict boundary: pure logic in `domain/`, side effects in `infra/`.
- Prefer small, single-purpose functions and explicit `Result<T, E>` errors (use `thiserror` for typed errors).
- Naming: types `PascalCase`, modules/functions `snake_case`, predicates `is_*` / `has_*`.

## Testing Guidelines

- Prefer deterministic unit tests colocated with code via `#[cfg(test)]` (see `src/domain/*`).
- Domain tests should avoid filesystem/network. Infra tests may use `tempfile` to create isolated directories.

## Commit & Pull Request Guidelines

- Git history currently contains a single commit (`Initial`), so conventions are not established yet.
- Suggested: imperative subjects (e.g., `Add sessions rescan command`), one logical change per commit.
- Branch naming: use hyphens, not slashes (e.g., `alexeus-session-browser`).
- PRs: explain the “why”, link issues, list verification steps (commands run), and include screenshots/recordings for UI changes.

## Agent Workspace

- Store design documents in `.agents/designs/`.
- Store task notes and follow-ups in `.agents/tasks/`.

## Release Guidelines

- Create a release only when explicitly requested by the maintainer (do not bump versions or push tags by default).
- Release only after code/doc changes are committed and pushed.
- For every release, update `CHANGELOG.md` (user-facing changes only; keep newest entry at the top; `### Changes` before `### Fixes`).
- For a new release: bump `Cargo.toml` version, then create and push an annotated tag `vX.Y.Z` (this triggers the GitHub Actions `Release` workflow).
- Verify the release completed and assets exist (`gh run watch …`, `gh release view vX.Y.Z`).
- After releasing, verify the self-update path works against the new release (`ccbox update`).

## System Menu Guidelines

- Add global actions to the System menu (see `src/app/mod.rs` `SYSTEM_MENU_ITEMS`).
- Prefer view-specific hotkeys for view-specific actions; update the view footer/help text when adding hotkeys.

## Security & Configuration Tips

- Codex session logs can include file paths, prompts, and token counts; treat them as sensitive and never commit data from `$HOME/.codex/sessions`.
- Use `CODEX_SESSIONS_DIR` to point the app at a redacted fixture directory when sharing repro steps.
