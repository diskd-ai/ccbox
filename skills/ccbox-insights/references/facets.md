# Tool-call facets (per session)

Goal: label each session with a small, consistent JSON object so tool-call failures can be aggregated across sessions without losing evidence.

## Normalization rules

- Stay engine-neutral: do not reference specific assistants, vendors, or model names.
- Stay repo-neutral: do not include local paths or repo names in summaries or recommendations.
- Evidence first: every counted failure should have at least one short evidence snippet.

## Tool groups (optional, recommended)

Map tool names from logs into a small stable set for aggregation:

- `exec` (shell/command execution)
- `edit` (patch/apply/edit file)
- `read` (file reads)
- `search` (repo search)
- `fetch` (HTTP fetch)
- `browser` (interactive browsing)
- `other`

If a tool does not fit, keep its raw name in `tool_failures[*].tool`.

## Failure taxonomy

Use these categories for tool-call failures:

- `invalid_tool_input`: invalid args/schema; tool invocation rejected before running.
- `tool_not_available`: tool missing, not installed, not on PATH, or disabled.
- `user_rejected_action`: user explicitly declined a proposed tool action.
- `permission_denied`: permissions or access denied.
- `path_not_found`: file/directory not found.
- `auth_or_secret_missing`: missing key/token/credential.
- `network_error`: DNS/TLS/connectivity failures.
- `timeout_or_hang`: timeouts, hangs, or long-running operations aborted.
- `command_failed`: runtime failure or non-zero exit (for exec-like tools).
- `conflicting_instructions`: failure caused by constraints that prohibit the required action.
- `partial_or_truncated`: incomplete outputs that prevent correct follow-through.
- `wrong_tool_or_scope`: tool succeeded but was the wrong move (scope mismatch, wrong target).
- `unknown`: cannot classify from available evidence.

## Facet schema (JSON)

Return one JSON object per session:

```json
{
  "session_label": "Short label for this session",
  "underlying_goal": "What the user fundamentally wanted",
  "session_type": "single_task | multi_task | iterative_refinement | exploration | quick_question",
  "outcome": "fully_achieved | mostly_achieved | partially_achieved | not_achieved | unclear_from_log",
  "user_satisfaction": "happy | satisfied | likely_satisfied | dissatisfied | frustrated | unsure",
  "lessons_learned": [
    {
      "scope": "project | global",
      "rule": "One sentence: what to do next time",
      "why": "One sentence: why this rule exists",
      "evidence": "1 short line copied from the log output",
      "confidence": "low | medium | high"
    }
  ],
  "tool_groups_used": { "exec": 0, "edit": 0, "read": 0, "search": 0, "fetch": 0, "browser": 0, "other": 0 },
  "tool_failure_counts": { "category_name": 0 },
  "tool_failures": [
    {
      "tool": "exec | edit | read | search | fetch | browser | other | <raw tool name>",
      "category": "invalid_tool_input | tool_not_available | user_rejected_action | permission_denied | path_not_found | auth_or_secret_missing | network_error | timeout_or_hang | command_failed | conflicting_instructions | partial_or_truncated | wrong_tool_or_scope | unknown",
      "evidence": "1 short line copied from the log output",
      "skill_context": "Optional: active skill name if known (from `ccbox skills --json`)"
    }
  ],
  "root_cause_hypothesis": "One sentence; must be consistent with evidence",
  "brief_summary": "One sentence: what was attempted and what went wrong/right"
}
```

## Counting guidelines

- `tool_groups_used`: count tool calls (attempts), not successes.
- `tool_failure_counts`: count failures by taxonomy category.
- If a single tool call produces multiple distinct failures (rare), prefer the earliest/root failure.
- If the user declines a tool action, count it as `user_rejected_action` and capture the rejection phrase as evidence.
