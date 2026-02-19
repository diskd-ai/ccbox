use serde_json::Value;
use std::collections::BTreeMap;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimelineItemKind {
    Turn,
    User,
    Assistant,
    Thinking,
    ToolCall,
    ToolOutput,
    TokenCount,
    Note,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelineItem {
    pub kind: TimelineItemKind,
    pub turn_id: Option<String>,
    pub call_id: Option<String>,
    /// 1-based JSONL line number in the source log, when this item was derived from a concrete
    /// source line.
    ///
    /// Synthetic items (like `Turn`) may also carry a source line reference (e.g. the associated
    /// `turn_context` line).
    pub source_line_no: Option<u64>,
    pub timestamp: Option<String>,
    pub timestamp_ms: Option<i64>,
    pub summary: String,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnContextSummary {
    pub turn_id: String,
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub personality: Option<String>,
    pub approval_policy: Option<String>,
    pub sandbox_policy: Option<String>,
    pub user_instructions_len: Option<usize>,
    pub developer_instructions_len: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionTimeline {
    pub items: Vec<TimelineItem>,
    pub turn_contexts: BTreeMap<String, TurnContextSummary>,
    pub warnings: usize,
    pub truncated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParsedLogLine {
    TurnContext(TurnContextSummary),
    TurnIdHint(String),
    Item(TimelineItem),
    Ignore,
}

pub fn parse_log_value(value: &Value, current_turn_id: Option<&str>) -> ParsedLogLine {
    let timestamp_raw = value.get("timestamp").and_then(|v| v.as_str());
    let timestamp = timestamp_raw.map(str::to_string);
    let timestamp_ms = timestamp_raw.and_then(parse_rfc3339_to_unix_ms);
    let line_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match line_type {
        "turn_context" => parse_turn_context_value(value),
        "event_msg" => parse_event_msg_value(value, current_turn_id, timestamp, timestamp_ms),
        "response_item" => {
            parse_response_item_value(value, current_turn_id, timestamp, timestamp_ms)
        }
        _ => ParsedLogLine::Ignore,
    }
}

fn parse_turn_context_value(value: &Value) -> ParsedLogLine {
    let payload = value.get("payload").unwrap_or(&Value::Null);
    let Some(turn_id) = payload.get("turn_id").and_then(|v| v.as_str()) else {
        return ParsedLogLine::Ignore;
    };

    let cwd = payload
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let model = payload
        .get("model")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let personality = payload
        .get("personality")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let approval_policy = payload
        .get("approval_policy")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let sandbox_policy = payload
        .get("sandbox_policy")
        .and_then(|v| v.get("type"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let user_instructions_len = payload
        .get("user_instructions")
        .and_then(|v| v.as_str())
        .map(|s| s.len());
    let developer_instructions_len = payload
        .get("collaboration_mode")
        .and_then(|v| v.get("settings"))
        .and_then(|v| v.get("developer_instructions"))
        .and_then(|v| v.as_str())
        .map(|s| s.len());

    let context = TurnContextSummary {
        turn_id: turn_id.to_string(),
        cwd,
        model,
        personality,
        approval_policy,
        sandbox_policy,
        user_instructions_len,
        developer_instructions_len,
    };

    ParsedLogLine::TurnContext(context)
}

fn parse_event_msg_value(
    value: &Value,
    current_turn_id: Option<&str>,
    timestamp: Option<String>,
    timestamp_ms: Option<i64>,
) -> ParsedLogLine {
    let payload = value.get("payload").unwrap_or(&Value::Null);
    let payload_type = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");

    if payload_type == "task_started" {
        if let Some(turn_id) = payload.get("turn_id").and_then(|v| v.as_str()) {
            return ParsedLogLine::TurnIdHint(turn_id.to_string());
        }
    }

    // Codex duplicates user prompts as both:
    // - `event_msg` payload `type=user_message`, and
    // - `response_item` payload `type=message role=user`.
    //
    // We only keep the `response_item` form to avoid duplicated first-user messages in the
    // timeline. The `response_item` variant also participates in the normal message parsing
    // flow (metadata prompt filtering, etc.).
    if payload_type == "user_message" {
        return ParsedLogLine::Ignore;
    }

    if payload_type == "token_count" {
        let Some(info) = payload.get("info") else {
            return ParsedLogLine::Ignore;
        };
        if info.is_null() {
            return ParsedLogLine::Ignore;
        }
        let total = info
            .get("total_token_usage")
            .and_then(|v| v.get("total_tokens"))
            .and_then(|v| v.as_u64());
        let last = info
            .get("last_token_usage")
            .and_then(|v| v.get("total_tokens"))
            .and_then(|v| v.as_u64());
        let summary = match (total, last) {
            (Some(total), Some(last)) => format!("tokens: total={total} last={last}"),
            (Some(total), None) => format!("tokens: total={total}"),
            _ => "tokens".to_string(),
        };
        let detail = serde_json::to_string_pretty(info).unwrap_or_else(|_| info.to_string());
        return ParsedLogLine::Item(TimelineItem {
            kind: TimelineItemKind::TokenCount,
            turn_id: current_turn_id.map(str::to_string),
            call_id: None,
            source_line_no: None,
            timestamp,
            timestamp_ms,
            summary,
            detail,
        });
    }

    ParsedLogLine::Ignore
}

fn parse_response_item_value(
    value: &Value,
    current_turn_id: Option<&str>,
    timestamp: Option<String>,
    timestamp_ms: Option<i64>,
) -> ParsedLogLine {
    let payload = value.get("payload").unwrap_or(&Value::Null);
    let payload_type = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");

    match payload_type {
        "reasoning" => {
            let mut parts: Vec<String> = Vec::new();
            if let Some(summary) = payload.get("summary").and_then(|v| v.as_array()) {
                for entry in summary {
                    if entry.get("type").and_then(|v| v.as_str()) == Some("summary_text") {
                        if let Some(text) = entry.get("text").and_then(|v| v.as_str()) {
                            if !text.trim().is_empty() {
                                parts.push(text.trim().to_string());
                            }
                        }
                    }
                }
            }

            if parts.is_empty() {
                return ParsedLogLine::Ignore;
            }
            let detail = parts.join("\n\n");
            let summary = first_non_empty_line(&detail).unwrap_or_else(|| "thinking".to_string());
            ParsedLogLine::Item(TimelineItem {
                kind: TimelineItemKind::Thinking,
                turn_id: current_turn_id.map(str::to_string),
                call_id: None,
                source_line_no: None,
                timestamp,
                timestamp_ms,
                summary,
                detail,
            })
        }
        "message" => parse_message_item(payload, current_turn_id, timestamp, timestamp_ms),
        "function_call" => parse_function_call(payload, current_turn_id, timestamp, timestamp_ms),
        "function_call_output" => {
            parse_function_call_output(payload, current_turn_id, timestamp, timestamp_ms)
        }
        "custom_tool_call" => {
            parse_custom_tool_call(payload, current_turn_id, timestamp, timestamp_ms)
        }
        "custom_tool_call_output" => {
            parse_custom_tool_call_output(payload, current_turn_id, timestamp, timestamp_ms)
        }
        _ => ParsedLogLine::Ignore,
    }
}

fn parse_message_item(
    payload: &Value,
    current_turn_id: Option<&str>,
    timestamp: Option<String>,
    timestamp_ms: Option<i64>,
) -> ParsedLogLine {
    let role = payload.get("role").and_then(|v| v.as_str()).unwrap_or("");

    let Some(content) = payload.get("content").and_then(|v| v.as_array()) else {
        return ParsedLogLine::Ignore;
    };

    let mut texts: Vec<String> = Vec::new();
    for item in content {
        if item.get("type").and_then(|v| v.as_str()) == Some("input_text")
            || item.get("type").and_then(|v| v.as_str()) == Some("output_text")
        {
            if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                texts.push(text.to_string());
            }
        }
    }

    let joined = texts.join("\n");
    if joined.trim().is_empty() {
        return ParsedLogLine::Ignore;
    }

    if role == "user" && super::is_metadata_prompt(&joined) {
        return ParsedLogLine::Ignore;
    }
    if role == "developer" {
        return ParsedLogLine::Ignore;
    }

    let kind = match role {
        "assistant" => TimelineItemKind::Assistant,
        "user" => TimelineItemKind::User,
        _ => TimelineItemKind::Note,
    };

    let summary = first_non_empty_line(&joined).unwrap_or_else(|| "(message)".to_string());
    ParsedLogLine::Item(TimelineItem {
        kind,
        turn_id: current_turn_id.map(str::to_string),
        call_id: None,
        source_line_no: None,
        timestamp,
        timestamp_ms,
        summary,
        detail: joined,
    })
}

fn parse_function_call(
    payload: &Value,
    current_turn_id: Option<&str>,
    timestamp: Option<String>,
    timestamp_ms: Option<i64>,
) -> ParsedLogLine {
    let name = payload
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("function_call");
    let call_id = payload
        .get("call_id")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let arguments = payload
        .get("arguments")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    ParsedLogLine::Item(TimelineItem {
        kind: TimelineItemKind::ToolCall,
        turn_id: current_turn_id.map(str::to_string),
        call_id,
        source_line_no: None,
        timestamp,
        timestamp_ms,
        summary: format!("{name}()"),
        detail: arguments,
    })
}

fn parse_function_call_output(
    payload: &Value,
    current_turn_id: Option<&str>,
    timestamp: Option<String>,
    timestamp_ms: Option<i64>,
) -> ParsedLogLine {
    let call_id = payload
        .get("call_id")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let output = payload
        .get("output")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if output.trim().is_empty() {
        return ParsedLogLine::Ignore;
    }
    let summary = first_non_empty_line(&output).unwrap_or_else(|| "(tool output)".to_string());
    ParsedLogLine::Item(TimelineItem {
        kind: TimelineItemKind::ToolOutput,
        turn_id: current_turn_id.map(str::to_string),
        call_id,
        source_line_no: None,
        timestamp,
        timestamp_ms,
        summary,
        detail: output,
    })
}

fn parse_custom_tool_call(
    payload: &Value,
    current_turn_id: Option<&str>,
    timestamp: Option<String>,
    timestamp_ms: Option<i64>,
) -> ParsedLogLine {
    let name = payload
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("tool_call");
    let call_id = payload
        .get("call_id")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let input = payload
        .get("input")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    ParsedLogLine::Item(TimelineItem {
        kind: TimelineItemKind::ToolCall,
        turn_id: current_turn_id.map(str::to_string),
        call_id,
        source_line_no: None,
        timestamp,
        timestamp_ms,
        summary: name.to_string(),
        detail: input,
    })
}

fn parse_custom_tool_call_output(
    payload: &Value,
    current_turn_id: Option<&str>,
    timestamp: Option<String>,
    timestamp_ms: Option<i64>,
) -> ParsedLogLine {
    let call_id = payload
        .get("call_id")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let raw = payload
        .get("output")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if raw.trim().is_empty() {
        return ParsedLogLine::Ignore;
    }

    let (summary, detail) = match serde_json::from_str::<Value>(&raw) {
        Ok(json) => {
            let output_text = json.get("output").and_then(|v| v.as_str()).unwrap_or(&raw);
            let summary =
                first_non_empty_line(output_text).unwrap_or_else(|| "(tool output)".to_string());
            (summary, output_text.to_string())
        }
        Err(_) => {
            let summary = first_non_empty_line(&raw).unwrap_or_else(|| "(tool output)".to_string());
            (summary, raw.clone())
        }
    };

    ParsedLogLine::Item(TimelineItem {
        kind: TimelineItemKind::ToolOutput,
        turn_id: current_turn_id.map(str::to_string),
        call_id,
        source_line_no: None,
        timestamp,
        timestamp_ms,
        summary,
        detail,
    })
}

fn first_non_empty_line(text: &str) -> Option<String> {
    text.lines()
        .map(|line| line.trim())
        .find(|line| !line.is_empty())
        .map(|line| line.to_string())
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
    fn parses_turn_context_summary() {
        let json = serde_json::json!({
            "timestamp": "2026-02-18T21:45:57.803Z",
            "type": "turn_context",
            "payload": {
                "turn_id": "t1",
                "cwd": "/tmp/x",
                "approval_policy": "never",
                "sandbox_policy": { "type": "danger-full-access" },
                "model": "gpt-5.2",
                "personality": "pragmatic",
                "user_instructions": "abc",
                "collaboration_mode": { "settings": { "developer_instructions": "def" } }
            }
        });
        let parsed = parse_log_value(&json, None);
        match parsed {
            ParsedLogLine::TurnContext(ctx) => {
                assert_eq!(ctx.turn_id, "t1");
                assert_eq!(ctx.cwd.as_deref(), Some("/tmp/x"));
                assert_eq!(ctx.model.as_deref(), Some("gpt-5.2"));
                assert_eq!(ctx.sandbox_policy.as_deref(), Some("danger-full-access"));
                assert_eq!(ctx.user_instructions_len, Some(3));
                assert_eq!(ctx.developer_instructions_len, Some(3));
            }
            other => panic!("unexpected parse result: {other:?}"),
        }
    }

    #[test]
    fn ignores_user_message_event_to_avoid_duplicates() {
        let json = serde_json::json!({
            "timestamp": "2026-02-18T21:45:57.766Z",
            "type": "event_msg",
            "payload": { "type": "user_message", "message": "hello\nworld", "images": [] }
        });
        let parsed = parse_log_value(&json, Some("t1"));
        assert_eq!(parsed, ParsedLogLine::Ignore);
    }

    #[test]
    fn parses_user_message_response_item() {
        let json = serde_json::json!({
            "timestamp": "2026-02-18T21:45:57.766Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": "hello\nworld" }]
            }
        });
        let parsed = parse_log_value(&json, Some("t1"));
        match parsed {
            ParsedLogLine::Item(item) => {
                assert_eq!(item.kind, TimelineItemKind::User);
                assert_eq!(item.turn_id.as_deref(), Some("t1"));
                assert!(item.timestamp_ms.is_some());
                assert_eq!(item.summary, "hello");
                assert_eq!(item.detail, "hello\nworld");
            }
            other => panic!("unexpected parse result: {other:?}"),
        }
    }
}
