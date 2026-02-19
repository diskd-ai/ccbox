# Insights workflow (session review)

Use this checklist to produce a concise, evidence-based summary of an agent run.

## Checklist

1. Identify the scope:
   - project-level overview (many sessions), or
   - one specific session (one `.jsonl` log).
2. Establish the facts with `ccbox`:
   - `ccbox sessions` (current folder) or `ccbox sessions "/abs/project/path"`.
   - `ccbox history --full` (latest in current folder) or `ccbox history "/abs/log.jsonl" --full`.
3. Extract the core story:
   - What the user asked for (first USER prompt line).
   - What the agent actually did (tool call sequence + key outputs).
   - What changed (files and config visible in outputs).
   - What failed or looks risky (errors, warnings, retries, partial edits).
4. Propose next actions:
   - concrete verification steps (commands to run, tests, checks),
   - missing info/questions,
   - follow-up tasks.

## Report template

Use this structure (Markdown-friendly):

- **Session**: id/title/log path (and project path if relevant)
- **Goal**: 1 sentence of what the user wanted
- **What happened**: 3-6 bullets in chronological order
- **Tools used**: top tool calls + what they were for
- **Files changed**: list (if any)
- **Errors / warnings**: what failed and where
- **Next steps**: exact commands or tasks to validate/fix

