---
name: ccbox-insights
description: Analyze session logs with the ccbox CLI (projects/sessions/history) to identify erroneous tool calls and recurring tool failures, then propose additive instructions for AGENTS.md (project) and global agent instructions to prevent future call errors. Use when asked to run /ccbox:insights, diagnose tool-call errors, or improve agent UX with evidence-based recommendations.
---

# ccbox-insights

Use `ccbox` session logs to produce an evidence-based "insights" report focused on erroneous tool calls (failed commands, rejected actions, invalid tool inputs), plus copy-ready instruction snippets that reduce future failures.

## Requirements

- `ccbox` on your `$PATH`.
- Access to the local sessions directory scanned by `ccbox` (see `ccbox --help` if discovery looks empty).

## Quick start (single session)

1. Find the latest session for the current folder:

```bash
ccbox sessions --limit 5 --offset 0 --size
```

2. Inspect the latest session timeline (increase `--limit` if needed):

```bash
ccbox history --full --limit 200 --offset 0
```

## Workflow (recommended)

Follow a staged pipeline: collect -> filter -> summarize -> label -> aggregate -> synthesize -> propose instructions.

### Stage 0: Choose scope

- **Session**: one `.jsonl` log for deep root-cause analysis.
- **Project**: last N sessions for one project to find recurring failure patterns.
- **Global**: a sample across projects to find cross-project patterns.

### Stage 1: Collect evidence with `ccbox`

- Project discovery: `ccbox projects`
- Session listing: `ccbox sessions [project-path] --limit N --offset 0 --size`
- Timeline capture: `ccbox history [log-or-project] --full --limit N --offset 0`

When triaging quickly, scan the timeline for failure signals (examples): `error`, `failed`, `non-zero`, `rejected`, `permission`, `timeout`.

### Stage 2: Summarize long timelines (only if needed)

If the timeline is too large to analyze in one pass, summarize in chunks:

- Focus on: user request, tool calls, tool outputs/errors, and outcome.
- Preserve: tool names, command lines, error messages, and user feedback.
- Keep each chunk summary to 3-5 sentences.

### Stage 3: Extract per-session tool-call facets

For each session in scope, produce one JSON object matching `references/facets.md`.

Hard rules:

- Use only evidence from the session log; do not guess missing details.
- Separate "tool failed" from "wrong approach" (a tool can succeed but still be the wrong move).
- Count explicit user rejections as their own category (the tool did not fail; the action was declined).

### Stage 4: Aggregate and analyze

Aggregate the facet set to produce:

- Top failing tools and failure categories.
- Three root-cause themes with concrete evidence snippets.
- Repeated user constraints that should become standing instructions.
- Engine-neutral recommendations that reduce tool-call failures and improve UX.

Use `references/report.md` as the output template.

### Stage 5: Propose instruction updates (project + global)

Produce two additive sets of copy-ready snippets:

- **Project-level**: bullets to add to `AGENTS.md`.
- **Global**: bullets for the user's global agent instructions.

Guidelines:

- Do not include local paths, repository names, or one-off incident details.
- Prefer rules that prevent repeated errors (2+ occurrences) over one-time fixes.
- Each instruction should include: what to do, what to avoid, and why (1 sentence).

### Stage 6: Deliverables

Deliver:

- A concise insights report (Markdown).
- A proposed additive `AGENTS.md` snippet.
- A separate proposed global snippet.

## References

- Facet schema + taxonomy: `references/facets.md`
- Report template + instruction templates: `references/report.md`

