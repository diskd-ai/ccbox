use crate::domain::{TimelineItem, TimelineItemKind, parse_rfc3339_to_unix_ms};
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize)]
pub struct ClaudeSessionsIndex {
    #[serde(rename = "originalPath")]
    pub original_path: Option<String>,

    #[serde(default)]
    pub entries: Vec<ClaudeSessionsIndexEntry>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ClaudeSessionsIndexEntry {
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,

    #[serde(rename = "fullPath")]
    pub full_path: Option<PathBuf>,

    #[serde(default)]
    pub created: Option<String>,

    #[serde(default)]
    pub modified: Option<String>,

    #[serde(default)]
    pub summary: Option<String>,

    #[serde(rename = "firstPrompt", default)]
    pub first_prompt: Option<String>,

    #[serde(rename = "projectPath", default)]
    pub project_path: Option<String>,
}

pub fn parse_claude_sessions_index(text: &str) -> Result<ClaudeSessionsIndex, serde_json::Error> {
    serde_json::from_str(text)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaudeSessionMetaHint {
    pub cwd: Option<PathBuf>,
    pub session_id: Option<String>,
    pub timestamp: Option<String>,
    pub summary: Option<String>,
    pub first_prompt: Option<String>,
}

pub fn extract_claude_session_meta_hint(value: &Value) -> ClaudeSessionMetaHint {
    let cwd = value
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .or_else(|| {
            value
                .get("projectPath")
                .and_then(|v| v.as_str())
                .map(PathBuf::from)
        });
    let session_id = value
        .get("sessionId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let timestamp = value
        .get("timestamp")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let summary = value
        .get("summary")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let first_prompt = value
        .get("firstPrompt")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    ClaudeSessionMetaHint {
        cwd,
        session_id,
        timestamp,
        summary,
        first_prompt,
    }
}

pub fn parse_claude_user_message_text(value: &Value) -> Option<String> {
    if value.get("type").and_then(|v| v.as_str()) != Some("user") {
        return None;
    }

    let message = value.get("message").unwrap_or(&Value::Null);
    let content = message.get("content").unwrap_or(&Value::Null);
    let text = extract_text_blocks(content);
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

pub fn parse_claude_timeline_items(value: &Value, source_line_no: u64) -> Vec<TimelineItem> {
    let timestamp_raw = value.get("timestamp").and_then(|v| v.as_str());
    let timestamp = timestamp_raw.map(|s| s.to_string());
    let timestamp_ms = timestamp_raw.and_then(parse_rfc3339_to_unix_ms);

    let kind = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match kind {
        "user" => parse_user_items(value, timestamp, timestamp_ms, source_line_no),
        "assistant" => parse_assistant_items(value, timestamp, timestamp_ms, source_line_no),
        "summary" => parse_summary_item(value, timestamp, timestamp_ms, source_line_no)
            .into_iter()
            .collect(),
        "file-history-snapshot" | "progress" => Vec::new(),
        "" => Vec::new(),
        other => vec![TimelineItem {
            kind: TimelineItemKind::Note,
            turn_id: None,
            call_id: None,
            source_line_no: Some(source_line_no),
            timestamp,
            timestamp_ms,
            summary: format!("Claude: {other}"),
            detail: serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
        }],
    }
}

fn parse_user_items(
    value: &Value,
    timestamp: Option<String>,
    timestamp_ms: Option<i64>,
    source_line_no: u64,
) -> Vec<TimelineItem> {
    let message = value.get("message").unwrap_or(&Value::Null);
    let content = message.get("content").unwrap_or(&Value::Null);

    // Claude tool results are sometimes stored as user records with `tool_result` blocks.
    if let Some(items) = content.as_array() {
        let mut out: Vec<TimelineItem> = Vec::new();
        for block in items {
            let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
            match block_type {
                "text" => {
                    let text = block.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    let text = text.trim_end();
                    if text.is_empty() {
                        continue;
                    }
                    out.push(TimelineItem {
                        kind: TimelineItemKind::User,
                        turn_id: None,
                        call_id: None,
                        source_line_no: Some(source_line_no),
                        timestamp: timestamp.clone(),
                        timestamp_ms,
                        summary: first_non_empty_line(text).unwrap_or_else(|| "user".to_string()),
                        detail: text.to_string(),
                    });
                }
                "tool_result" => {
                    let call_id = block
                        .get("tool_use_id")
                        .or_else(|| block.get("toolUseId"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    let detail = format_tool_result_detail(block);
                    out.push(TimelineItem {
                        kind: TimelineItemKind::ToolOutput,
                        turn_id: None,
                        call_id,
                        source_line_no: Some(source_line_no),
                        timestamp: timestamp.clone(),
                        timestamp_ms,
                        summary: first_non_empty_line(&detail)
                            .unwrap_or_else(|| "tool output".to_string()),
                        detail,
                    });
                }
                "" => {}
                other => {
                    out.push(TimelineItem {
                        kind: TimelineItemKind::Note,
                        turn_id: None,
                        call_id: None,
                        source_line_no: Some(source_line_no),
                        timestamp: timestamp.clone(),
                        timestamp_ms,
                        summary: format!("Claude user: {other}"),
                        detail: serde_json::to_string_pretty(block)
                            .unwrap_or_else(|_| block.to_string()),
                    });
                }
            }
        }
        return out;
    }

    let text = extract_text_blocks(content).trim_end().to_string();
    if text.trim().is_empty() {
        return Vec::new();
    }
    vec![TimelineItem {
        kind: TimelineItemKind::User,
        turn_id: None,
        call_id: None,
        source_line_no: Some(source_line_no),
        timestamp,
        timestamp_ms,
        summary: first_non_empty_line(&text).unwrap_or_else(|| "user".to_string()),
        detail: text,
    }]
}

fn parse_assistant_items(
    value: &Value,
    timestamp: Option<String>,
    timestamp_ms: Option<i64>,
    source_line_no: u64,
) -> Vec<TimelineItem> {
    let message = value.get("message").unwrap_or(&Value::Null);
    let content = message.get("content").unwrap_or(&Value::Null);

    let Some(items) = content.as_array() else {
        let text = extract_text_blocks(content).trim_end().to_string();
        if text.trim().is_empty() {
            return Vec::new();
        }
        return vec![TimelineItem {
            kind: TimelineItemKind::Assistant,
            turn_id: None,
            call_id: None,
            source_line_no: Some(source_line_no),
            timestamp,
            timestamp_ms,
            summary: first_non_empty_line(&text).unwrap_or_else(|| "assistant".to_string()),
            detail: text,
        }];
    };

    let mut out: Vec<TimelineItem> = Vec::new();
    for block in items {
        let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match block_type {
            "text" => {
                let text = block.get("text").and_then(|v| v.as_str()).unwrap_or("");
                let text = text.trim_end();
                if text.is_empty() {
                    continue;
                }
                out.push(TimelineItem {
                    kind: TimelineItemKind::Assistant,
                    turn_id: None,
                    call_id: None,
                    source_line_no: Some(source_line_no),
                    timestamp: timestamp.clone(),
                    timestamp_ms,
                    summary: first_non_empty_line(text).unwrap_or_else(|| "assistant".to_string()),
                    detail: text.to_string(),
                });
            }
            "thinking" => {
                let thinking = block
                    .get("thinking")
                    .or_else(|| block.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let thinking = thinking.trim_end();
                if thinking.is_empty() {
                    continue;
                }
                out.push(TimelineItem {
                    kind: TimelineItemKind::Thinking,
                    turn_id: None,
                    call_id: None,
                    source_line_no: Some(source_line_no),
                    timestamp: timestamp.clone(),
                    timestamp_ms,
                    summary: first_non_empty_line(thinking)
                        .unwrap_or_else(|| "thinking".to_string()),
                    detail: thinking.to_string(),
                });
            }
            "tool_use" => {
                let call_id = block
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("tool");
                let input = block.get("input").unwrap_or(&Value::Null);
                let detail =
                    serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string());
                out.push(TimelineItem {
                    kind: TimelineItemKind::ToolCall,
                    turn_id: None,
                    call_id,
                    source_line_no: Some(source_line_no),
                    timestamp: timestamp.clone(),
                    timestamp_ms,
                    summary: format!("{name}()"),
                    detail,
                });
            }
            "" => {}
            other => out.push(TimelineItem {
                kind: TimelineItemKind::Note,
                turn_id: None,
                call_id: None,
                source_line_no: Some(source_line_no),
                timestamp: timestamp.clone(),
                timestamp_ms,
                summary: format!("Claude assistant: {other}"),
                detail: serde_json::to_string_pretty(block).unwrap_or_else(|_| block.to_string()),
            }),
        }
    }

    out
}

fn parse_summary_item(
    value: &Value,
    timestamp: Option<String>,
    timestamp_ms: Option<i64>,
    source_line_no: u64,
) -> Option<TimelineItem> {
    let summary = value
        .get("summary")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim_end()
        .to_string();
    if summary.trim().is_empty() {
        return None;
    }
    Some(TimelineItem {
        kind: TimelineItemKind::Note,
        turn_id: None,
        call_id: None,
        source_line_no: Some(source_line_no),
        timestamp,
        timestamp_ms,
        summary: "Claude summary".to_string(),
        detail: summary,
    })
}

fn extract_text_blocks(value: &Value) -> String {
    match value {
        Value::String(text) => text.to_string(),
        Value::Array(items) => items
            .iter()
            .filter_map(|block| {
                if block.get("type").and_then(|v| v.as_str()) == Some("text") {
                    return block.get("text").and_then(|v| v.as_str());
                }
                None
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn format_tool_result_detail(block: &Value) -> String {
    // Prefer plain string content, but fall back to pretty JSON for structured results.
    let content = block.get("content").unwrap_or(&Value::Null);
    if let Some(text) = content.as_str() {
        return text.trim_end().to_string();
    }
    serde_json::to_string_pretty(content).unwrap_or_else(|_| content.to_string())
}

fn first_non_empty_line(text: &str) -> Option<String> {
    text.lines()
        .map(|line| line.trim())
        .find(|line| !line.is_empty())
        .map(|line| line.to_string())
}

pub fn has_path_component(path: &Path, component: &str) -> bool {
    path.components()
        .any(|c| c.as_os_str() == std::ffi::OsStr::new(component))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sessions_index_entries() {
        let json = r#"{
            "originalPath": "/tmp/project",
            "entries": [
                {
                    "sessionId": "s1",
                    "fullPath": "/tmp/log.jsonl",
                    "created": "2026-02-19T00:00:00Z",
                    "modified": "2026-02-19T00:01:00Z",
                    "summary": "hello",
                    "firstPrompt": "hello world"
                }
            ]
        }"#;
        let parsed = parse_claude_sessions_index(json).expect("parse");
        assert_eq!(parsed.original_path.as_deref(), Some("/tmp/project"));
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].session_id.as_deref(), Some("s1"));
        assert_eq!(parsed.entries[0].summary.as_deref(), Some("hello"));
    }

    #[test]
    fn parses_tool_use_and_result_with_call_ids() {
        let tool_use = serde_json::json!({
            "type": "assistant",
            "timestamp": "2026-02-19T00:00:00Z",
            "message": {
                "content": [
                    { "type": "tool_use", "id": "toolu_1", "name": "Bash", "input": { "cmd": "ls" } }
                ]
            }
        });
        let items = parse_claude_timeline_items(&tool_use, 2);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, TimelineItemKind::ToolCall);
        assert_eq!(items[0].call_id.as_deref(), Some("toolu_1"));

        let tool_out = serde_json::json!({
            "type": "user",
            "timestamp": "2026-02-19T00:00:01Z",
            "message": {
                "content": [
                    { "type": "tool_result", "tool_use_id": "toolu_1", "content": "ok" }
                ]
            }
        });
        let out_items = parse_claude_timeline_items(&tool_out, 3);
        assert_eq!(out_items.len(), 1);
        assert_eq!(out_items[0].kind, TimelineItemKind::ToolOutput);
        assert_eq!(out_items[0].call_id.as_deref(), Some("toolu_1"));
        assert_eq!(out_items[0].detail, "ok");
    }

    #[test]
    fn extracts_user_text_from_string_content() {
        let json = serde_json::json!({
            "type": "user",
            "message": { "content": "hello\nworld" }
        });
        assert_eq!(
            parse_claude_user_message_text(&json),
            Some("hello\nworld".to_string())
        );
    }

    #[test]
    fn has_path_component_matches_hidden_claude_dir() {
        let path = PathBuf::from("/Users/a/.claude/projects/x/y.jsonl");
        assert!(has_path_component(&path, ".claude"));
        assert!(!has_path_component(&path, ".codex"));
    }
}
