# ccbox-insights Skill

> **Install:** `npx skills add diskd-ai/ccbox --skill ccbox-insights --global --yes` | [skills.sh](https://skills.sh)

Analyze session logs with the `ccbox` CLI and produce an evidence-based "insights" report: what tools failed, why they failed, and copy-ready instruction snippets that reduce future tool-call errors.

This skill uses `ccbox` directly (no Python scripts).

---

## Scope & Purpose

This skill helps an agent:

* Inspect recent sessions by project and engine (Codex, Claude, Gemini, OpenCode)
* Identify invalid tool usage vs real tool/runtime failures
* Find recurring failure patterns across multiple sessions
* Propose additive instructions for project `AGENTS.md` and global agent instructions

---

## Requirements

* `ccbox` on your `$PATH`.
* Local access to the sessions directories scanned by `ccbox` (see `ccbox --help` if discovery looks empty).

---

## Quick Reference

```bash
ccbox projects
ccbox sessions --limit 20 --offset 0 --size
ccbox sessions --engine claude --limit 20 --offset 0 --size
ccbox history --full --limit 200 --offset 0
```

---

## Outputs

The expected deliverables are:

* A concise insights report (Markdown)
* A proposed additive `AGENTS.md` snippet (project-level)
* A proposed global snippet (engine-neutral rules)

---

## Skill Structure

```
skills/ccbox-insights/
  SKILL.md
  README.md
  agents/
    openai.yaml
  references/
    facets.md
    report.md
```

---

## Resources

* **Full skill reference**: [SKILL.md](SKILL.md)
* **Facet schema + taxonomy**: [references/facets.md](references/facets.md)
* **Report template**: [references/report.md](references/report.md)

---

## License

MIT

