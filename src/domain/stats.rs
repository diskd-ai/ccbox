use crate::domain::{SessionMeta, TimelineItem, TimelineItemKind};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolUsage {
    pub name: String,
    pub calls: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileChange {
    pub path: String,
    pub operations: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionStats {
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
    pub duration_ms: Option<i64>,

    pub total_tokens: Option<u64>,
    pub last_tokens: Option<u64>,

    pub tool_calls_total: usize,
    pub tool_calls_success: usize,
    pub tool_calls_invalid: usize,
    pub tool_calls_error: usize,
    pub tool_calls_unknown: usize,
    pub tools_used: Vec<ToolUsage>,

    pub apply_patch_calls: usize,
    pub apply_patch_operations: usize,
    pub files_changed: Vec<FileChange>,
    pub lines_added: usize,
    pub lines_removed: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolOutputOutcome {
    Success,
    Invalid,
    Error,
    Unknown,
}

pub fn compute_session_stats(meta: &SessionMeta, items: &[TimelineItem]) -> SessionStats {
    let mut start_ms = items.iter().filter_map(|item| item.timestamp_ms).min();
    let end_ms = items.iter().filter_map(|item| item.timestamp_ms).max();
    if start_ms.is_none() {
        start_ms = parse_rfc3339_to_unix_ms(&meta.started_at_rfc3339);
    }

    let duration_ms = match (start_ms, end_ms) {
        (Some(start), Some(end)) if end >= start => Some(end - start),
        _ => None,
    };

    let (total_tokens, last_tokens) = compute_token_usage(items);

    let mut tool_calls_total = 0usize;
    let mut tool_calls_success = 0usize;
    let mut tool_calls_invalid = 0usize;
    let mut tool_calls_error = 0usize;
    let mut tool_calls_unknown = 0usize;
    let mut tools_used_counts: BTreeMap<String, usize> = BTreeMap::new();

    let mut apply_patch_calls = 0usize;
    let mut apply_patch_operations = 0usize;
    let mut files_changed_ops: BTreeMap<String, usize> = BTreeMap::new();
    let mut lines_added = 0usize;
    let mut lines_removed = 0usize;

    for (index, item) in items.iter().enumerate() {
        if item.kind != TimelineItemKind::ToolCall {
            continue;
        }

        tool_calls_total += 1;

        let tool_name = tool_name_from_summary(&item.summary);
        *tools_used_counts.entry(tool_name.clone()).or_insert(0) += 1;

        if is_apply_patch_tool(&tool_name) {
            apply_patch_calls += 1;
            let patch = item.detail.as_str();
            let (ops, files, adds, removes) = parse_apply_patch_stats(patch);
            apply_patch_operations += ops;
            lines_added = lines_added.saturating_add(adds);
            lines_removed = lines_removed.saturating_add(removes);
            for file in files {
                *files_changed_ops.entry(file).or_insert(0) += 1;
            }
        }

        let outcome = match item.call_id.as_deref() {
            Some(call_id) => find_tool_output_for_call(items, index, call_id)
                .map(|tool_out| classify_tool_output_detail(tool_out.detail.as_str()))
                .unwrap_or(ToolOutputOutcome::Unknown),
            None => ToolOutputOutcome::Unknown,
        };
        match outcome {
            ToolOutputOutcome::Success => tool_calls_success += 1,
            ToolOutputOutcome::Invalid => tool_calls_invalid += 1,
            ToolOutputOutcome::Error => tool_calls_error += 1,
            ToolOutputOutcome::Unknown => tool_calls_unknown += 1,
        }
    }

    let tools_used = sort_tool_usage(tools_used_counts);
    let files_changed = sort_file_changes(files_changed_ops);

    SessionStats {
        start_ms,
        end_ms,
        duration_ms,
        total_tokens,
        last_tokens,
        tool_calls_total,
        tool_calls_success,
        tool_calls_invalid,
        tool_calls_error,
        tool_calls_unknown,
        tools_used,
        apply_patch_calls,
        apply_patch_operations,
        files_changed,
        lines_added,
        lines_removed,
    }
}

fn compute_token_usage(items: &[TimelineItem]) -> (Option<u64>, Option<u64>) {
    let mut best_total: Option<u64> = None;
    let mut best_last: Option<u64> = None;

    for item in items {
        if item.kind != TimelineItemKind::TokenCount {
            continue;
        }
        let Some((total, last)) = parse_token_count_detail(item.detail.as_str()) else {
            continue;
        };

        match best_total {
            None => {
                best_total = Some(total);
                best_last = last;
            }
            Some(current) if total > current => {
                best_total = Some(total);
                best_last = last;
            }
            _ => {}
        }
    }

    (best_total, best_last)
}

fn parse_token_count_detail(detail: &str) -> Option<(u64, Option<u64>)> {
    let parsed: Value = serde_json::from_str(detail).ok()?;
    let total = parsed
        .get("total_token_usage")
        .and_then(|v| v.get("total_tokens"))
        .and_then(|v| v.as_u64())?;
    let last = parsed
        .get("last_token_usage")
        .and_then(|v| v.get("total_tokens"))
        .and_then(|v| v.as_u64());
    Some((total, last))
}

fn tool_name_from_summary(summary: &str) -> String {
    let trimmed = summary.trim();
    trimmed.strip_suffix("()").unwrap_or(trimmed).to_string()
}

fn is_apply_patch_tool(tool_name: &str) -> bool {
    tool_name.to_lowercase().contains("apply_patch")
}

fn parse_apply_patch_stats(patch: &str) -> (usize, Vec<String>, usize, usize) {
    let mut ops = 0usize;
    let mut files: BTreeSet<String> = BTreeSet::new();
    let mut added = 0usize;
    let mut removed = 0usize;

    for line in patch.lines() {
        if let Some(path) = line.strip_prefix("*** Add File: ") {
            ops += 1;
            files.insert(path.trim().to_string());
            continue;
        }
        if let Some(path) = line.strip_prefix("*** Update File: ") {
            ops += 1;
            files.insert(path.trim().to_string());
            continue;
        }
        if let Some(path) = line.strip_prefix("*** Delete File: ") {
            ops += 1;
            files.insert(path.trim().to_string());
            continue;
        }
        if let Some(path) = line.strip_prefix("*** Move to: ") {
            ops += 1;
            files.insert(path.trim().to_string());
            continue;
        }

        if line.starts_with("***") {
            continue;
        }

        if let Some(first) = line.chars().next() {
            match first {
                '+' => added += 1,
                '-' => removed += 1,
                _ => {}
            }
        }
    }

    (ops, files.into_iter().collect(), added, removed)
}

fn find_tool_output_for_call<'a>(
    items: &'a [TimelineItem],
    selected_index: usize,
    call_id: &str,
) -> Option<&'a TimelineItem> {
    if selected_index + 1 < items.len() {
        if let Some((_, item)) =
            items
                .iter()
                .enumerate()
                .skip(selected_index + 1)
                .find(|(_, item)| {
                    item.kind == TimelineItemKind::ToolOutput
                        && item.call_id.as_deref() == Some(call_id)
                })
        {
            return Some(item);
        }
    }

    items.iter().find(|item| {
        item.kind == TimelineItemKind::ToolOutput && item.call_id.as_deref() == Some(call_id)
    })
}

pub fn classify_tool_output_detail(detail: &str) -> ToolOutputOutcome {
    if let Some(code) = parse_exit_code(detail) {
        return if code == 0 {
            ToolOutputOutcome::Success
        } else {
            ToolOutputOutcome::Error
        };
    }

    let trimmed = detail.trim_start();
    if trimmed.starts_with("Success.") {
        return ToolOutputOutcome::Success;
    }

    let lower = trimmed.to_lowercase();
    if lower.contains("invalid tool call")
        || lower.contains("invalid tool")
        || lower.contains("unknown tool")
        || lower.contains("tool not found")
        || lower.contains("unrecognized tool")
        || lower.contains("unknown subcommand")
        || lower.contains("invalid argument")
        || lower.contains("unexpected argument")
        || lower.contains("unknown option")
        || lower.contains("unrecognized option")
    {
        return ToolOutputOutcome::Invalid;
    }

    if lower.contains("permission denied") || lower.contains("no such file or directory") {
        return ToolOutputOutcome::Error;
    }

    if lower.starts_with("error")
        || lower.starts_with("failed")
        || lower.contains("error:")
        || lower.contains("failed:")
    {
        return ToolOutputOutcome::Error;
    }

    ToolOutputOutcome::Unknown
}

fn parse_exit_code(detail: &str) -> Option<i32> {
    let marker = "Process exited with code ";
    let idx = detail.find(marker)?;
    let rest = detail.get(idx + marker.len()..)?;
    let digits = rest
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '-')
        .collect::<String>();
    digits.parse::<i32>().ok()
}

fn sort_tool_usage(counts: BTreeMap<String, usize>) -> Vec<ToolUsage> {
    let mut tools = counts
        .into_iter()
        .map(|(name, calls)| ToolUsage { name, calls })
        .collect::<Vec<_>>();
    tools.sort_by(|a, b| b.calls.cmp(&a.calls).then_with(|| a.name.cmp(&b.name)));
    tools
}

fn sort_file_changes(counts: BTreeMap<String, usize>) -> Vec<FileChange> {
    let mut files = counts
        .into_iter()
        .map(|(path, operations)| FileChange { path, operations })
        .collect::<Vec<_>>();
    files.sort_by(|a, b| {
        b.operations
            .cmp(&a.operations)
            .then_with(|| a.path.cmp(&b.path))
    });
    files
}

fn parse_rfc3339_to_unix_ms(value: &str) -> Option<i64> {
    let timestamp = OffsetDateTime::parse(value, &Rfc3339).ok()?;
    let ms: i128 = timestamp.unix_timestamp_nanos() / 1_000_000;
    i64::try_from(ms).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_apply_patch_files_and_line_counts() {
        let patch = r#"*** Begin Patch
*** Add File: a.txt
+hello
*** Update File: src/main.rs
@@
-old
+new
*** End Patch
"#;
        let (ops, files, added, removed) = parse_apply_patch_stats(patch);
        assert_eq!(ops, 2);
        assert!(files.contains(&"a.txt".to_string()));
        assert!(files.contains(&"src/main.rs".to_string()));
        assert_eq!(added, 2);
        assert_eq!(removed, 1);
    }

    #[test]
    fn classifies_exec_command_exit_codes() {
        assert_eq!(
            classify_tool_output_detail("Process exited with code 0\nok"),
            ToolOutputOutcome::Success
        );
        assert_eq!(
            classify_tool_output_detail("Process exited with code 2\nnope"),
            ToolOutputOutcome::Error
        );
    }

    #[test]
    fn classifies_invalid_tool_use() {
        assert_eq!(
            classify_tool_output_detail("Invalid tool call: nope"),
            ToolOutputOutcome::Invalid
        );
        assert_eq!(
            classify_tool_output_detail("Unknown tool: x"),
            ToolOutputOutcome::Invalid
        );
    }
}
