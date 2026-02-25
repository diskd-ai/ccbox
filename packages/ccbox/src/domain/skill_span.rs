use crate::domain::{TimelineItem, TimelineItemKind, is_metadata_prompt};

/// A contiguous range of timeline items belonging to a single skill invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillSpan {
    /// Skill name extracted from the Skill() tool call input (e.g., "commit").
    pub name: String,
    /// Index of the Skill() ToolCall item in the timeline.
    pub start_idx: usize,
    /// Index of the last item in the span (inclusive). None if the skill never completed.
    pub end_idx: Option<usize>,
    /// call_id linking the Skill() ToolCall to its ToolOutput (when available).
    pub call_id: String,
    /// Nesting depth (0 = top-level, 1 = called by another skill, etc.).
    pub depth: u8,
    /// Index of the parent SkillSpan in the spans list, if this is a nested invocation.
    pub parent_span_idx: Option<usize>,
}

/// Aggregated metrics for a single skill span.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SkillMetrics {
    /// Number of ToolCall items inside this span (excluding the Skill() call itself).
    pub tool_calls: u32,
    /// Number of ToolOutput items inside this span.
    pub tool_outputs: u32,
    /// Duration from the Skill() call timestamp to the span end timestamp, in milliseconds.
    pub duration_ms: Option<i64>,
    /// Total characters in all ToolOutput detail texts within the span.
    pub output_chars: usize,
}

/// Loop detection result for a session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillLoop {
    /// Skill name that loops.
    pub name: String,
    /// Indices into the spans list of consecutive invocations of the same skill.
    pub span_indices: Vec<usize>,
}

pub fn extract_skill_name(detail_json: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(detail_json).ok()?;
    value
        .get("skill")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

pub fn extract_codex_skill_name(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    if !trimmed.starts_with("<skill>") {
        return None;
    }

    let name_start = trimmed.find("<name>")? + "<name>".len();
    let name_end = trimmed[name_start..].find("</name>")? + name_start;
    let name = trimmed[name_start..name_end].trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn is_skill_call(item: &TimelineItem) -> bool {
    item.kind == TimelineItemKind::ToolCall && item.summary == "Skill()"
}

pub fn detect_skill_spans(items: &[TimelineItem]) -> Vec<SkillSpan> {
    let mut spans: Vec<SkillSpan> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();

    for (idx, item) in items.iter().enumerate() {
        if is_skill_call(item) {
            if item.call_id.is_none() {
                close_all_open_spans(&mut spans, &mut stack, idx);
            }

            let name = extract_skill_name(&item.detail).unwrap_or_else(|| "unknown".to_string());
            let call_id = item.call_id.clone().unwrap_or_default();

            let parent_span_idx = stack.last().copied();
            let depth = u8::try_from(stack.len()).unwrap_or(u8::MAX);

            let span_idx = spans.len();
            spans.push(SkillSpan {
                name,
                start_idx: idx,
                end_idx: None,
                call_id,
                depth,
                parent_span_idx,
            });
            stack.push(span_idx);
            continue;
        }

        if item.kind == TimelineItemKind::User && !is_metadata_prompt(&item.detail) {
            close_all_open_spans(&mut spans, &mut stack, idx);
        }
    }

    spans
}

fn close_all_open_spans(spans: &mut [SkillSpan], stack: &mut Vec<usize>, idx: usize) {
    if stack.is_empty() {
        return;
    }
    let end_idx = idx.saturating_sub(1);
    while let Some(span_idx) = stack.pop() {
        if let Some(span) = spans.get_mut(span_idx) {
            if span.end_idx.is_none() {
                span.end_idx = Some(end_idx);
            }
        }
    }
}

pub fn compute_skill_metrics(span: &SkillSpan, items: &[TimelineItem]) -> SkillMetrics {
    if items.is_empty() {
        return SkillMetrics::default();
    }
    if span.start_idx >= items.len() {
        return SkillMetrics::default();
    }

    let mut end_idx = span
        .end_idx
        .unwrap_or_else(|| items.len().saturating_sub(1));
    end_idx = end_idx.min(items.len().saturating_sub(1));
    if end_idx < span.start_idx {
        end_idx = span.start_idx;
    }

    let mut metrics = SkillMetrics::default();

    for (i, item) in items
        .iter()
        .enumerate()
        .skip(span.start_idx)
        .take(end_idx.saturating_sub(span.start_idx).saturating_add(1))
    {
        match item.kind {
            TimelineItemKind::ToolCall => {
                if i != span.start_idx {
                    metrics.tool_calls = metrics.tool_calls.saturating_add(1);
                }
            }
            TimelineItemKind::ToolOutput => {
                metrics.tool_outputs = metrics.tool_outputs.saturating_add(1);
                metrics.output_chars = metrics
                    .output_chars
                    .saturating_add(item.detail.chars().count());
            }
            _ => {}
        }
    }

    let start_ts = items.get(span.start_idx).and_then(|item| item.timestamp_ms);
    let end_ts = items.get(end_idx).and_then(|item| item.timestamp_ms);
    metrics.duration_ms = match (start_ts, end_ts) {
        (Some(start), Some(end)) => end.checked_sub(start),
        _ => None,
    };

    metrics
}

pub fn detect_skill_loops(spans: &[SkillSpan]) -> Vec<SkillLoop> {
    let mut out: Vec<SkillLoop> = Vec::new();
    let mut current_name: Option<&str> = None;
    let mut current_indices: Vec<usize> = Vec::new();

    for (idx, span) in spans.iter().enumerate() {
        if span.depth != 0 {
            continue;
        }

        match current_name {
            Some(name) if name == span.name => {
                current_indices.push(idx);
            }
            _ => {
                if current_indices.len() >= 2 {
                    if let Some(name) = current_name {
                        out.push(SkillLoop {
                            name: name.to_string(),
                            span_indices: current_indices.clone(),
                        });
                    }
                }
                current_name = Some(&span.name);
                current_indices.clear();
                current_indices.push(idx);
            }
        }
    }

    if current_indices.len() >= 2 {
        if let Some(name) = current_name {
            out.push(SkillLoop {
                name: name.to_string(),
                span_indices: current_indices,
            });
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_item(kind: TimelineItemKind, summary: &str, detail: &str) -> TimelineItem {
        TimelineItem {
            kind,
            turn_id: None,
            call_id: None,
            source_line_no: None,
            timestamp: None,
            timestamp_ms: None,
            summary: summary.to_string(),
            detail: detail.to_string(),
        }
    }

    #[test]
    fn extract_skill_name_reads_skill_field() {
        assert_eq!(
            extract_skill_name(r#"{"skill":"commit"}"#),
            Some("commit".to_string())
        );
        assert_eq!(
            extract_skill_name(r#"{"skill":"assemblyai-cli","args":"file.ogg"}"#),
            Some("assemblyai-cli".to_string())
        );
        assert_eq!(extract_skill_name("not json"), None);
        assert_eq!(extract_skill_name(r#"{"args":"x"}"#), None);
    }

    #[test]
    fn extract_codex_skill_name_reads_name_tag() {
        let text = "<skill>\n<name>ccbox</name>\n<path>/x</path>\n</skill>";
        assert_eq!(extract_codex_skill_name(text), Some("ccbox".to_string()));
        assert_eq!(extract_codex_skill_name("hello world"), None);
    }

    #[test]
    fn detect_skill_spans_returns_empty_for_no_skills() {
        let items = vec![make_item(TimelineItemKind::User, "user", "hello")];
        let spans = detect_skill_spans(&items);
        assert!(spans.is_empty());
    }

    #[test]
    fn detects_single_span_closed_by_next_user_message() {
        let mut skill = make_item(
            TimelineItemKind::ToolCall,
            "Skill()",
            r#"{"skill":"commit"}"#,
        );
        skill.call_id = Some("toolu_1".to_string());

        let items = vec![
            skill,
            make_item(TimelineItemKind::ToolCall, "Bash()", r#"{"cmd":"ls"}"#),
            make_item(TimelineItemKind::ToolOutput, "ok", "done"),
            make_item(TimelineItemKind::User, "user", "next task"),
        ];

        let spans = detect_skill_spans(&items);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].name, "commit");
        assert_eq!(spans[0].start_idx, 0);
        assert_eq!(spans[0].end_idx, Some(2));
        assert_eq!(spans[0].depth, 0);
        assert_eq!(spans[0].parent_span_idx, None);
    }

    #[test]
    fn detects_nested_span() {
        let mut outer = make_item(
            TimelineItemKind::ToolCall,
            "Skill()",
            r#"{"skill":"commit"}"#,
        );
        outer.call_id = Some("toolu_outer".to_string());
        let mut inner = make_item(
            TimelineItemKind::ToolCall,
            "Skill()",
            r#"{"skill":"code-review"}"#,
        );
        inner.call_id = Some("toolu_inner".to_string());

        let items = vec![
            outer,
            make_item(
                TimelineItemKind::ToolCall,
                "Bash()",
                r#"{"cmd":"git status"}"#,
            ),
            inner,
            make_item(TimelineItemKind::ToolCall, "Bash()", r#"{"cmd":"rg foo"}"#),
            make_item(TimelineItemKind::User, "user", "done"),
        ];

        let spans = detect_skill_spans(&items);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].depth, 0);
        assert_eq!(spans[0].parent_span_idx, None);
        assert_eq!(spans[1].depth, 1);
        assert_eq!(spans[1].parent_span_idx, Some(0));
    }

    #[test]
    fn detects_loops_over_consecutive_top_level_spans() {
        let spans = vec![
            SkillSpan {
                name: "commit".to_string(),
                start_idx: 0,
                end_idx: Some(2),
                call_id: "c1".to_string(),
                depth: 0,
                parent_span_idx: None,
            },
            SkillSpan {
                name: "assemblyai-cli".to_string(),
                start_idx: 1,
                end_idx: Some(1),
                call_id: "c2".to_string(),
                depth: 1,
                parent_span_idx: Some(0),
            },
            SkillSpan {
                name: "commit".to_string(),
                start_idx: 5,
                end_idx: Some(6),
                call_id: "c3".to_string(),
                depth: 0,
                parent_span_idx: None,
            },
        ];

        let loops = detect_skill_loops(&spans);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].name, "commit");
        assert_eq!(loops[0].span_indices, vec![0, 2]);
    }

    #[test]
    fn compute_skill_metrics_counts_tool_calls_outputs_and_duration() {
        let mut skill = make_item(
            TimelineItemKind::ToolCall,
            "Skill()",
            r#"{"skill":"commit"}"#,
        );
        skill.timestamp_ms = Some(1_000);

        let mut tool_call = make_item(TimelineItemKind::ToolCall, "Bash()", "{}");
        tool_call.timestamp_ms = Some(2_000);
        let mut tool_out = make_item(TimelineItemKind::ToolOutput, "ok", "hello");
        tool_out.timestamp_ms = Some(3_500);

        let span = SkillSpan {
            name: "commit".to_string(),
            start_idx: 0,
            end_idx: Some(2),
            call_id: "c1".to_string(),
            depth: 0,
            parent_span_idx: None,
        };

        let items = vec![skill, tool_call, tool_out];
        let metrics = compute_skill_metrics(&span, &items);
        assert_eq!(metrics.tool_calls, 1);
        assert_eq!(metrics.tool_outputs, 1);
        assert_eq!(metrics.output_chars, 5);
        assert_eq!(metrics.duration_ms, Some(2_500));
    }

    #[test]
    fn user_metadata_prompt_does_not_close_spans() {
        let mut skill = make_item(
            TimelineItemKind::ToolCall,
            "Skill()",
            r#"{"skill":"commit"}"#,
        );
        skill.call_id = Some("toolu_1".to_string());

        let items = vec![
            skill,
            make_item(
                TimelineItemKind::User,
                "user",
                "<skill>\n<name>ccbox</name>\n</skill>",
            ),
            make_item(TimelineItemKind::ToolCall, "Bash()", "{}"),
            make_item(TimelineItemKind::User, "user", "next"),
        ];

        let spans = detect_skill_spans(&items);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].end_idx, Some(2));
    }
}
