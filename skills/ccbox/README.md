# ccbox Skill

> **Install:** `npx skills add diskd-ai/ccbox --skill ccbox --global --yes` | [skills.sh](https://skills.sh)

Inspect and summarize local Codex/Claude session logs via the `ccbox` CLI (`projects`, `sessions`, `history`).

---

## Scope & Purpose

This skill helps an agent:

* List discovered projects under `CODEX_SESSIONS_DIR`
* List sessions for a project (defaults to the current folder project)
* Print session history/timeline (defaults to the latest session for the current folder project)
* Produce concise "what happened" insights based on evidence from `ccbox history --full`

---

## When to Use This Skill

**Triggers:**

* Asked to summarize an agent run or explain what happened in a session
* Mentions of `ccbox`, Codex sessions, `.codex/sessions`, or `.jsonl` session logs
* Need to find the latest session for the current repo/folder and inspect its history

---

## Requirements

* `ccbox` on your `$PATH`.
* Local access to the Codex sessions directory:
  * `CODEX_SESSIONS_DIR` (preferred override), or
  * default `~/.codex/sessions`

---

## Quick Reference

```bash
ccbox projects
ccbox sessions
ccbox history
ccbox history --full
ccbox sessions --limit 50 --offset 0 --size
ccbox history --limit 200 --offset 0 --full --size
```

---

## Common workflows

### Find the latest session for the current folder

```bash
ccbox sessions          # auto-selects current folder project
ccbox history --full    # auto-selects latest session in that project
```

### Inspect a specific project

```bash
ccbox projects
ccbox sessions "/abs/path/to/project"
ccbox history "/abs/path/to/project" --full
```

### Inspect a specific session log

```bash
ccbox history "/abs/path/to/session.jsonl" --full
```

---

## Output formats (pipe-friendly)

- `ccbox projects` prints: `project_name<TAB>project_path<TAB>session_count`
- `ccbox sessions` prints: `started_at_rfc3339<TAB>session_id<TAB>title<TAB>log_path`

---

## Skill Structure

```
skills/ccbox/
  SKILL.md
  README.md
  agents/
    openai.yaml
  references/
    cli.md
    insights.md
```

---

## Resources

* **Full skill reference**: [SKILL.md](SKILL.md)
* **CLI details**: [references/cli.md](references/cli.md)
* **Insights checklist**: [references/insights.md](references/insights.md)

---

## License

MIT
