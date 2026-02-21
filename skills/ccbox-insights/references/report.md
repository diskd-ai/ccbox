# Report template (aggregated)

Produce a Markdown report from a set of per-session facets.

## Output structure

- **Scope**: session | project | global (and the selection method used)
- **At a glance**:
  - What's working
  - What's causing tool-call errors (split: assistant-side vs user-side)
  - Quick wins to try
  - Ambitious workflows (for more capable systems)
- **Failure hotspots**: top tools and top failure categories
- **Root-cause themes**: 3 themes with evidence snippets
- **Recommendations**: engine-neutral UX improvements that reduce tool-call failures
- **Instruction proposals**:
  - `AGENTS.md` additions (project-level)
  - Global instruction additions (separate)

## Failure hotspots (format)

Use a compact table:

| Tool group | Failure category | Count | Typical evidence |
| --- | --- | ---: | --- |
| exec | command_failed | 6 | "..." |

## Root-cause themes (format)

For each theme:

- Name (3-6 words)
- Description (1-2 sentences)
- Evidence (2-3 short snippets)
- Prevention (1-2 concrete rules)

## Instruction proposals (format)

Write copy-ready bullets. Keep them additive and generalized.

### `AGENTS.md` additions (project-level)

- Rule: ...
  - Why: ...

### Global instruction additions

- Rule: ...
  - Why: ...

## Copyable prompt scaffolds (optional)

Include 1-2 prompts that reduce tool-call errors, for example:

- "Before running tools, restate the goal, list assumptions, and ask up to 3 clarifying questions if anything is ambiguous."
- "Propose a minimal plan and wait for confirmation before destructive or high-impact tool calls."

