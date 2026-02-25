use crate::domain::{
    TimelineItem, TimelineItemKind, derive_title_from_user_text, is_metadata_prompt,
    parse_rfc3339_to_unix_ms,
};
use serde_json::Value;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeminiUserLogEntry {
    pub session_id: String,
    pub timestamp: Option<String>,
    pub message: String,
}

pub fn parse_gemini_logs_entries(value: &Value) -> Vec<GeminiUserLogEntry> {
    let Some(items) = value.as_array() else {
        return Vec::new();
    };

    let mut out: Vec<GeminiUserLogEntry> = Vec::new();
    for item in items {
        if item.get("type").and_then(|v| v.as_str()) != Some("user") {
            continue;
        }
        let Some(session_id) = item.get("sessionId").and_then(|v| v.as_str()) else {
            continue;
        };
        let message = item.get("message").and_then(|v| v.as_str()).unwrap_or("");
        let message = message.trim_end();
        if message.trim().is_empty() {
            continue;
        }
        let timestamp = item
            .get("timestamp")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        out.push(GeminiUserLogEntry {
            session_id: session_id.to_string(),
            timestamp,
            message: message.to_string(),
        });
    }

    out
}

pub fn extract_gemini_session_id(value: &Value) -> Option<String> {
    value
        .get("sessionId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

pub fn extract_gemini_session_start_time(value: &Value) -> Option<String> {
    value
        .get("startTime")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

pub fn extract_gemini_first_user_message(value: &Value) -> Option<String> {
    let messages = value.get("messages").and_then(|v| v.as_array())?;

    for message in messages {
        if message.get("type").and_then(|v| v.as_str()) != Some("user") {
            continue;
        }
        let content = message
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let content = content.trim_end();
        if content.trim().is_empty() {
            continue;
        }
        if is_metadata_prompt(content) {
            continue;
        }
        return Some(content.to_string());
    }

    None
}

pub fn infer_gemini_title_from_session(value: &Value) -> Option<String> {
    let text = extract_gemini_first_user_message(value)?;
    derive_title_from_user_text(&text)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeminiTimelineParseOutput {
    pub items: Vec<TimelineItem>,
    pub warnings: usize,
}

pub fn parse_gemini_timeline_items(value: &Value) -> GeminiTimelineParseOutput {
    let Some(messages) = value.get("messages").and_then(|v| v.as_array()) else {
        let note = TimelineItem {
            kind: TimelineItemKind::Note,
            turn_id: None,
            call_id: None,
            source_line_no: None,
            timestamp: None,
            timestamp_ms: None,
            summary: "Gemini: missing messages".to_string(),
            detail: serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
        };
        return GeminiTimelineParseOutput {
            items: vec![note],
            warnings: 1,
        };
    };

    let mut warnings = 0usize;
    let mut items_out: Vec<TimelineItem> = Vec::new();

    for message in messages {
        let timestamp_raw = message.get("timestamp").and_then(|v| v.as_str());
        let timestamp = timestamp_raw.map(|s| s.to_string());
        let timestamp_ms = timestamp_raw.and_then(parse_rfc3339_to_unix_ms);

        let kind = message.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match kind {
            "user" => {
                if let Some(item) = parse_user_message(message, timestamp, timestamp_ms) {
                    items_out.push(item);
                }
            }
            "gemini" => {
                let parsed = parse_gemini_message(message, timestamp, timestamp_ms);
                warnings += parsed.warnings;
                items_out.extend(parsed.items);
            }
            "" => {}
            other => {
                warnings = warnings.saturating_add(1);
                items_out.push(TimelineItem {
                    kind: TimelineItemKind::Note,
                    turn_id: None,
                    call_id: None,
                    source_line_no: None,
                    timestamp,
                    timestamp_ms,
                    summary: format!("Gemini: {other}"),
                    detail: serde_json::to_string_pretty(message)
                        .unwrap_or_else(|_| message.to_string()),
                });
            }
        }
    }

    GeminiTimelineParseOutput {
        items: items_out,
        warnings,
    }
}

fn parse_user_message(
    value: &Value,
    timestamp: Option<String>,
    timestamp_ms: Option<i64>,
) -> Option<TimelineItem> {
    let content = value.get("content").and_then(|v| v.as_str()).unwrap_or("");
    let content = content.trim_end();
    if content.trim().is_empty() {
        return None;
    }

    let summary = derive_title_from_user_text(content).unwrap_or_else(|| "user".to_string());
    Some(TimelineItem {
        kind: TimelineItemKind::User,
        turn_id: None,
        call_id: None,
        source_line_no: None,
        timestamp,
        timestamp_ms,
        summary,
        detail: content.to_string(),
    })
}

fn parse_gemini_message(
    value: &Value,
    timestamp: Option<String>,
    timestamp_ms: Option<i64>,
) -> GeminiTimelineParseOutput {
    let mut warnings = 0usize;
    let mut items: Vec<TimelineItem> = Vec::new();

    let content = value.get("content").and_then(|v| v.as_str()).unwrap_or("");
    let content = content.trim_end();
    if !content.trim().is_empty() {
        let summary =
            derive_title_from_user_text(content).unwrap_or_else(|| "assistant".to_string());
        items.push(TimelineItem {
            kind: TimelineItemKind::Assistant,
            turn_id: None,
            call_id: None,
            source_line_no: None,
            timestamp: timestamp.clone(),
            timestamp_ms,
            summary,
            detail: content.to_string(),
        });
    }

    let thoughts = value.get("thoughts").and_then(|v| v.as_str()).unwrap_or("");
    let thoughts = thoughts.trim_end();
    if !thoughts.trim().is_empty() {
        let summary =
            derive_title_from_user_text(thoughts).unwrap_or_else(|| "thinking".to_string());
        items.push(TimelineItem {
            kind: TimelineItemKind::Thinking,
            turn_id: None,
            call_id: None,
            source_line_no: None,
            timestamp: timestamp.clone(),
            timestamp_ms,
            summary,
            detail: thoughts.to_string(),
        });
    }

    if let Some(tokens) = value.get("tokens") {
        if !tokens.is_null() {
            let (summary, detail) = format_tokens_item(tokens);
            items.push(TimelineItem {
                kind: TimelineItemKind::TokenCount,
                turn_id: None,
                call_id: None,
                source_line_no: None,
                timestamp: timestamp.clone(),
                timestamp_ms,
                summary,
                detail,
            });
        }
    }

    if let Some(tool_calls) = value.get("toolCalls").and_then(|v| v.as_array()) {
        for call in tool_calls {
            let parsed = parse_tool_call(call, timestamp.clone(), timestamp_ms);
            warnings += parsed.warnings;
            items.extend(parsed.items);
        }
    }

    GeminiTimelineParseOutput { items, warnings }
}

fn parse_tool_call(
    value: &Value,
    timestamp: Option<String>,
    timestamp_ms: Option<i64>,
) -> GeminiTimelineParseOutput {
    let mut warnings = 0usize;
    let mut items: Vec<TimelineItem> = Vec::new();

    let call_id = value
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let name = value.get("name").and_then(|v| v.as_str()).unwrap_or("tool");
    let args = value.get("args").unwrap_or(&Value::Null);

    let args_detail = serde_json::to_string_pretty(args).unwrap_or_else(|_| args.to_string());
    let (summary, detail) = if name == "activate_skill" {
        let skill_name = args
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        (
            "Skill()".to_string(),
            serde_json::json!({ "skill": skill_name }).to_string(),
        )
    } else {
        (format!("{name}()"), args_detail)
    };
    items.push(TimelineItem {
        kind: TimelineItemKind::ToolCall,
        turn_id: None,
        call_id: call_id.clone(),
        source_line_no: None,
        timestamp: timestamp.clone(),
        timestamp_ms,
        summary,
        detail,
    });

    if call_id.is_none() {
        warnings = warnings.saturating_add(1);
    }

    if let Some(result) = value.get("result") {
        if !result.is_null() {
            let detail =
                serde_json::to_string_pretty(result).unwrap_or_else(|_| result.to_string());
            let summary =
                derive_title_from_user_text(&detail).unwrap_or_else(|| "(tool output)".to_string());
            items.push(TimelineItem {
                kind: TimelineItemKind::ToolOutput,
                turn_id: None,
                call_id,
                source_line_no: None,
                timestamp,
                timestamp_ms,
                summary,
                detail,
            });
        }
    }

    GeminiTimelineParseOutput { items, warnings }
}

fn format_tokens_item(tokens: &Value) -> (String, String) {
    if let Some(n) = tokens.as_u64() {
        return (format!("tokens: {n}"), n.to_string());
    }
    if let Some(n) = tokens.as_i64() {
        return (format!("tokens: {n}"), n.to_string());
    }

    let detail = serde_json::to_string_pretty(tokens).unwrap_or_else(|_| tokens.to_string());
    ("tokens".to_string(), detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_user_assistant_thinking_tool_calls_and_tokens() {
        let json = serde_json::json!({
            "sessionId": "s1",
            "startTime": "2026-02-19T00:00:00Z",
            "messages": [
                { "type": "user", "timestamp": "2026-02-19T00:00:01Z", "id": "m1", "content": "hello\nworld" },
                {
                    "type": "gemini",
                    "timestamp": "2026-02-19T00:00:02Z",
                    "id": "m2",
                    "content": "ok",
                    "thoughts": "thinking...",
                    "tokens": { "total": 10 },
                    "toolCalls": [
                        { "id": "c1", "name": "exec_command", "args": { "cmd": "ls" }, "result": { "code": 0 } }
                    ]
                }
            ]
        });

        let parsed = parse_gemini_timeline_items(&json);
        assert!(
            parsed
                .items
                .iter()
                .any(|i| i.kind == TimelineItemKind::User)
        );
        assert!(
            parsed
                .items
                .iter()
                .any(|i| i.kind == TimelineItemKind::Assistant)
        );
        assert!(
            parsed
                .items
                .iter()
                .any(|i| i.kind == TimelineItemKind::Thinking)
        );
        assert!(
            parsed
                .items
                .iter()
                .any(|i| i.kind == TimelineItemKind::ToolCall)
        );
        assert!(
            parsed
                .items
                .iter()
                .any(|i| i.kind == TimelineItemKind::ToolOutput)
        );
        assert!(
            parsed
                .items
                .iter()
                .any(|i| i.kind == TimelineItemKind::TokenCount)
        );
    }

    #[test]
    fn rewrites_activate_skill_to_unified_skill_call() {
        let json = serde_json::json!({
            "messages": [
                {
                    "type": "gemini",
                    "timestamp": "2026-02-21T12:28:37.789Z",
                    "id": "m1",
                    "toolCalls": [
                        {
                            "id": "activate_skill_1",
                            "name": "activate_skill",
                            "args": { "name": "ccbox" },
                            "result": { "status": "success" }
                        }
                    ]
                }
            ]
        });

        let parsed = parse_gemini_timeline_items(&json);
        assert!(parsed.items.iter().any(|item| {
            item.kind == TimelineItemKind::ToolCall
                && item.summary == "Skill()"
                && item.detail.contains("ccbox")
        }));
    }

    #[test]
    fn unknown_message_variant_becomes_note() {
        let json = serde_json::json!({
            "messages": [{ "type": "weird", "timestamp": "2026-02-19T00:00:00Z" }]
        });

        let parsed = parse_gemini_timeline_items(&json);
        assert_eq!(parsed.items.len(), 1);
        assert_eq!(parsed.items[0].kind, TimelineItemKind::Note);
        assert!(parsed.warnings >= 1);
    }
}
