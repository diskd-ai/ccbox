use crate::domain::{
    ParsedLogLine, SessionTimeline, TimelineItem, TimelineItemKind, parse_log_value,
};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;
use thiserror::Error;

const MAX_TIMELINE_ITEMS: usize = 10_000;

#[derive(Debug, Error)]
pub enum LoadSessionTimelineError {
    #[error("failed to open session file: {0}")]
    OpenFile(#[from] io::Error),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LastAssistantOutput {
    pub output: Option<String>,
    pub warnings: usize,
}

#[derive(Debug, Error)]
pub enum LoadLastAssistantOutputError {
    #[error("failed to open session file: {0}")]
    OpenFile(#[from] io::Error),
}

pub fn load_last_assistant_output(
    path: &Path,
) -> Result<LastAssistantOutput, LoadLastAssistantOutputError> {
    if path.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::IsADirectory,
            format!("path is a directory: {}", path.display()),
        )
        .into());
    }

    match detect_log_format(path) {
        LogFormat::Gemini => {
            return Ok(super::gemini::load_gemini_last_assistant_output(path)?);
        }
        LogFormat::Claude => {
            return Ok(super::claude::load_claude_last_assistant_output(path)?);
        }
        LogFormat::Codex => {}
    }

    let file = File::open(path)?;
    let reader = BufReader::new(file);

    let mut warnings = 0usize;
    let mut current_turn_id: Option<String> = None;
    let mut last_output: Option<String> = None;

    for line_result in reader.lines() {
        let line = match line_result {
            Ok(line) => line,
            Err(_) => {
                warnings += 1;
                break;
            }
        };

        let value: serde_json::Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => {
                warnings += 1;
                continue;
            }
        };

        match parse_log_value(&value, current_turn_id.as_deref()) {
            ParsedLogLine::TurnContext(ctx) => {
                current_turn_id = Some(ctx.turn_id);
            }
            ParsedLogLine::TurnIdHint(turn_id) => {
                current_turn_id = Some(turn_id);
            }
            ParsedLogLine::Item(item) => {
                if item.kind == TimelineItemKind::Assistant {
                    last_output = Some(item.detail);
                }
            }
            ParsedLogLine::Ignore => {}
        }
    }

    Ok(LastAssistantOutput {
        output: last_output,
        warnings,
    })
}

pub fn load_session_timeline(path: &Path) -> Result<SessionTimeline, LoadSessionTimelineError> {
    if path.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::IsADirectory,
            format!("path is a directory: {}", path.display()),
        )
        .into());
    }

    match detect_log_format(path) {
        LogFormat::Gemini => {
            return Ok(super::gemini::load_gemini_session_timeline(path)?);
        }
        LogFormat::Claude => {
            return Ok(super::claude::load_claude_session_timeline(path)?);
        }
        LogFormat::Codex => {}
    }

    let file = File::open(path)?;
    let reader = BufReader::new(file);

    let mut warnings = 0usize;
    let mut truncated = false;

    let mut turn_contexts = BTreeMap::new();
    let mut turn_context_line_nos: BTreeMap<String, u64> = BTreeMap::new();
    let mut current_turn_id: Option<String> = None;
    let mut last_emitted_turn_id: Option<String> = None;
    let mut items: Vec<TimelineItem> = Vec::new();
    let mut last_user_prompt: Option<String> = None;
    let mut pending_aborted_prompt: Option<String> = None;
    let mut last_user_prompt_by_turn: BTreeMap<String, String> = BTreeMap::new();
    let mut last_token_count_fingerprint: Option<String> = None;
    let mut last_token_count_index: Option<usize> = None;

    let mut line_no: u64 = 0;
    for line_result in reader.lines() {
        let line = match line_result {
            Ok(line) => line,
            Err(_) => {
                warnings += 1;
                truncated = true;
                break;
            }
        };
        line_no = line_no.saturating_add(1);

        let value: serde_json::Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => {
                warnings += 1;
                continue;
            }
        };

        if value.get("type").and_then(|v| v.as_str()) == Some("event_msg") {
            let payload = value.get("payload").unwrap_or(&serde_json::Value::Null);
            let payload_type = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if payload_type == "turn_aborted" {
                pending_aborted_prompt = last_user_prompt.clone();
            }
        }

        match parse_log_value(&value, current_turn_id.as_deref()) {
            ParsedLogLine::TurnContext(ctx) => {
                current_turn_id = Some(ctx.turn_id.clone());
                turn_context_line_nos.insert(ctx.turn_id.clone(), line_no);
                turn_contexts.insert(ctx.turn_id.clone(), ctx);
            }
            ParsedLogLine::TurnIdHint(turn_id) => {
                current_turn_id = Some(turn_id);
            }
            ParsedLogLine::Item(mut item) => {
                item.source_line_no = Some(line_no);
                let is_token_count = item.kind == TimelineItemKind::TokenCount;
                let is_user_prompt = item.kind == TimelineItemKind::User;

                if is_token_count {
                    // Codex emits many duplicate token_count events (often identical 2–3x).
                    // Keep only one entry per unique token_count payload (the last occurrence).
                    let fingerprint = item.detail.clone();
                    if last_token_count_fingerprint.as_deref() == Some(fingerprint.as_str()) {
                        if let Some(index) = last_token_count_index {
                            if index < items.len()
                                && items[index].kind == TimelineItemKind::TokenCount
                            {
                                items.remove(index);
                            }
                        }
                    }
                    last_token_count_fingerprint = Some(fingerprint);
                    last_token_count_index = None;
                }

                if is_user_prompt {
                    let detail = item.detail.trim_end();
                    if let Some(turn_id) = item.turn_id.as_deref() {
                        if last_user_prompt_by_turn
                            .get(turn_id)
                            .is_some_and(|prev| prev.trim_end() == detail)
                        {
                            pending_aborted_prompt = None;
                            continue;
                        }
                    }
                    if pending_aborted_prompt
                        .as_deref()
                        .is_some_and(|prev| prev.trim_end() == detail)
                    {
                        pending_aborted_prompt = None;
                        continue;
                    }
                    pending_aborted_prompt = None;
                }

                if let Some(turn_id) = item.turn_id.as_deref() {
                    if last_emitted_turn_id.as_deref() != Some(turn_id) {
                        if items.len() >= MAX_TIMELINE_ITEMS {
                            truncated = true;
                            break;
                        }
                        items.push(make_turn_item(
                            turn_id,
                            turn_context_line_nos.get(turn_id).copied(),
                        ));
                        last_emitted_turn_id = Some(turn_id.to_string());
                    }
                }

                if items.len() >= MAX_TIMELINE_ITEMS {
                    truncated = true;
                    break;
                }
                items.push(item);
                if is_user_prompt {
                    if let Some(last) = items.last() {
                        let detail = last.detail.trim_end().to_string();
                        last_user_prompt = Some(detail.clone());
                        if let Some(turn_id) = last.turn_id.as_deref() {
                            last_user_prompt_by_turn.insert(turn_id.to_string(), detail);
                        }
                    }
                }

                if is_token_count {
                    last_token_count_index = Some(items.len().saturating_sub(1));
                }
            }
            ParsedLogLine::Ignore => {}
        }
    }

    Ok(SessionTimeline {
        items,
        turn_contexts,
        warnings,
        truncated,
    })
}

fn make_turn_item(turn_id: &str, source_line_no: Option<u64>) -> TimelineItem {
    TimelineItem {
        kind: TimelineItemKind::Turn,
        turn_id: Some(turn_id.to_string()),
        call_id: None,
        source_line_no,
        timestamp: None,
        timestamp_ms: None,
        summary: format!("Turn {}", short_id(turn_id)),
        detail: turn_id.to_string(),
    }
}

fn short_id(value: &str) -> String {
    let max = 8usize;
    value.chars().take(max).collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LogFormat {
    Codex,
    Claude,
    Gemini,
}

fn detect_log_format(path: &Path) -> LogFormat {
    if super::gemini::is_gemini_session_path(path) {
        return LogFormat::Gemini;
    }
    if looks_like_claude_jsonl(path) {
        return LogFormat::Claude;
    }
    LogFormat::Codex
}

fn looks_like_claude_jsonl(path: &Path) -> bool {
    for value in read_jsonl_values(path, 50) {
        let line_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if matches!(
            line_type,
            "user" | "assistant" | "summary" | "progress" | "file-history-snapshot"
        ) {
            return true;
        }
    }
    false
}

fn read_jsonl_values(path: &Path, limit: usize) -> Vec<serde_json::Value> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(_) => return Vec::new(),
    };
    let reader = BufReader::new(file);
    let mut out: Vec<serde_json::Value> = Vec::new();
    for line_result in reader.lines().take(limit.saturating_mul(2)) {
        let Ok(line) = line_result else {
            break;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            continue;
        };
        out.push(value);
        if out.len() >= limit {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io;
    use tempfile::tempdir;

    #[test]
    fn merges_duplicate_token_count_items_by_replacing_previous() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("session.jsonl");

        let lines = [
            serde_json::json!({
                "timestamp": "2026-02-18T22:00:00.000Z",
                "type": "turn_context",
                "payload": { "turn_id": "t1" }
            }),
            serde_json::json!({
                "timestamp": "2026-02-18T22:00:01.000Z",
                "type": "event_msg",
                "payload": {
                    "type": "token_count",
                    "info": {
                        "total_token_usage": { "total_tokens": 10 },
                        "last_token_usage": { "total_tokens": 10 }
                    }
                }
            }),
            serde_json::json!({
                "timestamp": "2026-02-18T22:00:02.000Z",
                "type": "response_item",
                "payload": { "type": "function_call_output", "call_id": "c1", "output": "ok" }
            }),
            serde_json::json!({
                "timestamp": "2026-02-18T22:00:03.000Z",
                "type": "event_msg",
                "payload": {
                    "type": "token_count",
                    "info": {
                        "total_token_usage": { "total_tokens": 10 },
                        "last_token_usage": { "total_tokens": 10 }
                    }
                }
            }),
            serde_json::json!({
                "timestamp": "2026-02-18T22:00:04.000Z",
                "type": "event_msg",
                "payload": {
                    "type": "token_count",
                    "info": {
                        "total_token_usage": { "total_tokens": 11 },
                        "last_token_usage": { "total_tokens": 1 }
                    }
                }
            }),
        ];

        let body = lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, body).expect("write");

        let timeline = load_session_timeline(&path).expect("load");
        let token_count_items = timeline
            .items
            .iter()
            .filter(|item| item.kind == TimelineItemKind::TokenCount)
            .collect::<Vec<_>>();

        assert_eq!(token_count_items.len(), 2);
        assert_eq!(token_count_items[0].summary, "tokens: total=10 last=10");
        assert_eq!(token_count_items[1].summary, "tokens: total=11 last=1");

        let tool_out_index = timeline
            .items
            .iter()
            .position(|item| item.kind == TimelineItemKind::ToolOutput)
            .expect("tool output");
        let first_token_index = timeline
            .items
            .iter()
            .position(|item| {
                item.kind == TimelineItemKind::TokenCount
                    && item.summary == "tokens: total=10 last=10"
            })
            .expect("token count");

        assert!(first_token_index > tool_out_index);
    }

    #[test]
    fn dedupes_retried_user_prompt_after_turn_aborted() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("session.jsonl");

        let lines = [
            serde_json::json!({
                "timestamp": "2026-02-18T22:00:00.000Z",
                "type": "turn_context",
                "payload": { "turn_id": "t1" }
            }),
            serde_json::json!({
                "timestamp": "2026-02-18T22:00:01.000Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "hello" }]
                }
            }),
            serde_json::json!({
                "timestamp": "2026-02-18T22:00:02.000Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": "ok" }]
                }
            }),
            serde_json::json!({
                "timestamp": "2026-02-18T22:00:03.000Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [{
                        "type": "input_text",
                        "text": "<turn_aborted>\nThe user interrupted.\n</turn_aborted>"
                    }]
                }
            }),
            serde_json::json!({
                "timestamp": "2026-02-18T22:00:03.500Z",
                "type": "event_msg",
                "payload": { "type": "turn_aborted", "turn_id": "t1" }
            }),
            serde_json::json!({
                "timestamp": "2026-02-18T22:00:04.000Z",
                "type": "event_msg",
                "payload": { "type": "task_started", "turn_id": "t2" }
            }),
            serde_json::json!({
                "timestamp": "2026-02-18T22:00:05.000Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "hello" }]
                }
            }),
            serde_json::json!({
                "timestamp": "2026-02-18T22:00:06.000Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": "ok again" }]
                }
            }),
        ];

        let body = lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, body).expect("write");

        let timeline = load_session_timeline(&path).expect("load");
        let user_items = timeline
            .items
            .iter()
            .filter(|item| item.kind == TimelineItemKind::User)
            .collect::<Vec<_>>();

        assert_eq!(user_items.len(), 1);
        assert_eq!(user_items[0].summary, "hello");
        assert_eq!(user_items[0].source_line_no, Some(2));
    }

    #[test]
    fn dedupes_duplicate_user_prompt_within_same_turn() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("session.jsonl");

        let lines = [
            serde_json::json!({
                "timestamp": "2026-02-18T22:00:00.000Z",
                "type": "turn_context",
                "payload": { "turn_id": "t1" }
            }),
            serde_json::json!({
                "timestamp": "2026-02-18T22:00:01.000Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "hello" }]
                }
            }),
            serde_json::json!({
                "timestamp": "2026-02-18T22:00:01.100Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "hello" }]
                }
            }),
            serde_json::json!({
                "timestamp": "2026-02-18T22:00:02.000Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": "ok" }]
                }
            }),
        ];

        let body = lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, body).expect("write");

        let timeline = load_session_timeline(&path).expect("load");
        let user_items = timeline
            .items
            .iter()
            .filter(|item| item.kind == TimelineItemKind::User)
            .collect::<Vec<_>>();

        assert_eq!(user_items.len(), 1);
        assert_eq!(user_items[0].summary, "hello");
        assert_eq!(user_items[0].source_line_no, Some(2));
    }

    #[test]
    fn sets_source_line_numbers_for_items_and_turn_markers() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("session.jsonl");

        let lines = [
            serde_json::json!({
                "timestamp": "2026-02-18T21:45:57.803Z",
                "type": "session_meta",
                "payload": {
                    "id": "s1",
                    "timestamp": "2026-02-18T21:45:57.803Z",
                    "cwd": "/tmp/project"
                }
            }),
            serde_json::json!({
                "timestamp": "2026-02-18T22:00:00.000Z",
                "type": "turn_context",
                "payload": { "turn_id": "t1", "cwd": "/tmp/project" }
            }),
            serde_json::json!({
                "timestamp": "2026-02-18T22:00:01.000Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "hello" }]
                }
            }),
            serde_json::json!({
                "timestamp": "2026-02-18T22:00:02.000Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": "ok" }]
                }
            }),
        ];

        let body = lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, body).expect("write");

        let timeline = load_session_timeline(&path).expect("load");

        let turn = timeline
            .items
            .iter()
            .find(|item| item.kind == TimelineItemKind::Turn)
            .expect("turn item");
        assert_eq!(turn.source_line_no, Some(2));

        let user = timeline
            .items
            .iter()
            .find(|item| item.kind == TimelineItemKind::User)
            .expect("user item");
        assert_eq!(user.source_line_no, Some(3));

        let out = timeline
            .items
            .iter()
            .find(|item| item.kind == TimelineItemKind::Assistant)
            .expect("assistant item");
        assert_eq!(out.source_line_no, Some(4));
    }

    #[test]
    fn load_session_timeline_errors_on_directory_path() {
        let dir = tempdir().expect("tempdir");

        let error = load_session_timeline(dir.path()).expect_err("error");
        match error {
            LoadSessionTimelineError::OpenFile(error) => {
                assert_eq!(error.kind(), io::ErrorKind::IsADirectory);
            }
        }
    }

    #[test]
    fn load_last_assistant_output_errors_on_directory_path() {
        let dir = tempdir().expect("tempdir");

        let error = load_last_assistant_output(dir.path()).expect_err("error");
        match error {
            LoadLastAssistantOutputError::OpenFile(error) => {
                assert_eq!(error.kind(), io::ErrorKind::IsADirectory);
            }
        }
    }

    #[test]
    fn loads_claude_timeline_when_queue_operation_precedes_messages() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("session.jsonl");

        let lines = [
            serde_json::json!({
                "type": "queue-operation",
                "operation": "dequeue",
                "timestamp": "2026-02-19T00:00:00Z",
                "sessionId": "s1",
            }),
            serde_json::json!({
                "type": "user",
                "timestamp": "2026-02-19T00:00:01Z",
                "message": { "content": "hello" }
            }),
        ];

        let body = lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, body).expect("write");

        let timeline = load_session_timeline(&path).expect("load");
        assert!(
            timeline
                .items
                .iter()
                .any(|item| item.kind == TimelineItemKind::User),
            "expected Claude user item"
        );
    }
}
