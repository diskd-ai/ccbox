use crate::domain::{
    ParsedLogLine, SessionEngine, SessionSummary, TimelineItemKind, ToolOutputOutcome,
    classify_tool_output_detail, parse_claude_timeline_items, parse_gemini_timeline_items,
    parse_log_value,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ResolveCcboxStateDirError {
    #[error("home directory not found")]
    HomeDirNotFound,
}

pub fn resolve_ccbox_state_dir() -> Result<PathBuf, ResolveCcboxStateDirError> {
    let Some(home) = dirs::home_dir() else {
        return Err(ResolveCcboxStateDirError::HomeDirNotFound);
    };
    Ok(home.join(".ccbox"))
}

#[derive(Clone, Debug, Default)]
pub struct SessionIndex {
    entries: BTreeMap<PathBuf, SessionIndexEntry>,
}

impl SessionIndex {
    pub fn total_tokens(&self, log_path: &Path) -> Option<u64> {
        self.entries
            .get(log_path)
            .and_then(|entry| entry.total_tokens)
    }

    pub fn tool_failures(&self, log_path: &Path) -> Option<ToolFailureCounts> {
        let entry = self.entries.get(log_path)?;
        Some(ToolFailureCounts {
            invalid: entry.tool_calls_invalid?,
            error: entry.tool_calls_error?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct SessionIndexEntry {
    pub size_bytes: u64,
    pub modified_unix_ms: Option<i64>,
    pub total_tokens: Option<u64>,
    pub last_tokens: Option<u64>,
    pub tool_calls_invalid: Option<u32>,
    pub tool_calls_error: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ToolFailureCounts {
    pub invalid: u32,
    pub error: u32,
}

impl ToolFailureCounts {
    pub fn total(self) -> u32 {
        self.invalid.saturating_add(self.error)
    }
}

#[derive(Debug, Error)]
pub enum LoadSessionIndexError {
    #[error("failed to read session index: {0}")]
    Read(#[from] io::Error),

    #[error("failed to parse session index: {0}")]
    Parse(#[from] serde_json::Error),
}

#[derive(Debug, Error)]
pub enum SaveSessionIndexError {
    #[error("failed to encode session index: {0}")]
    Encode(#[from] serde_json::Error),

    #[error("failed to write session index: {0}")]
    Write(#[from] io::Error),
}

fn session_index_path(state_dir: &Path) -> PathBuf {
    state_dir.join("session_index.json")
}

pub fn load_session_index(state_dir: &Path) -> Result<SessionIndex, LoadSessionIndexError> {
    let path = session_index_path(state_dir);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(SessionIndex::default());
        }
        Err(error) => return Err(error.into()),
    };

    let file: SessionIndexFile = serde_json::from_str(&raw)?;
    Ok(file.into_index())
}

pub fn save_session_index(
    state_dir: &Path,
    index: &SessionIndex,
) -> Result<(), SaveSessionIndexError> {
    fs::create_dir_all(state_dir)?;
    let path = session_index_path(state_dir);
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(&SessionIndexFile::from_index(index))?;
    fs::write(&tmp, text)?;
    fs::rename(tmp, path)?;
    Ok(())
}

pub fn refresh_session_index(sessions: &[SessionSummary], prior: &SessionIndex) -> SessionIndex {
    let mut next_entries: BTreeMap<PathBuf, SessionIndexEntry> = BTreeMap::new();
    for session in sessions {
        let log_path = session.log_path.clone();
        let size_bytes = session.file_size_bytes;
        let modified_unix_ms = session.file_modified.and_then(system_time_to_unix_ms);

        let reuse = prior.entries.get(&log_path).is_some_and(|entry| {
            entry.size_bytes == size_bytes && entry.modified_unix_ms == modified_unix_ms
        });
        if reuse {
            if let Some(entry) = prior.entries.get(&log_path).cloned() {
                next_entries.insert(log_path, entry);
                continue;
            }
        }

        let (total_tokens, last_tokens) = if session.engine == SessionEngine::Codex {
            extract_last_token_usage(&session.log_path)
        } else {
            (None, None)
        };

        let (tool_calls_invalid, tool_calls_error) =
            extract_tool_failure_counts(&session.log_path, session.engine);
        next_entries.insert(
            log_path,
            SessionIndexEntry {
                size_bytes,
                modified_unix_ms,
                total_tokens,
                last_tokens,
                tool_calls_invalid,
                tool_calls_error,
            },
        );
    }

    SessionIndex {
        entries: next_entries,
    }
}

fn system_time_to_unix_ms(value: SystemTime) -> Option<i64> {
    let delta = value.duration_since(UNIX_EPOCH).ok()?;
    i64::try_from(delta.as_millis()).ok()
}

fn extract_last_token_usage(path: &Path) -> (Option<u64>, Option<u64>) {
    const SMALL_TAIL_BYTES: usize = 256 * 1024;
    const LARGE_TAIL_BYTES: usize = 2 * 1024 * 1024;

    if let Ok((tail, _size)) = super::read_tail(path, SMALL_TAIL_BYTES) {
        if let Some((total, last)) = find_last_token_usage_in_text(&tail) {
            return (Some(total), last);
        }
    }

    if let Ok((tail, _size)) = super::read_tail(path, LARGE_TAIL_BYTES) {
        if let Some((total, last)) = find_last_token_usage_in_text(&tail) {
            return (Some(total), last);
        }
    }

    (None, None)
}

fn extract_tool_failure_counts(path: &Path, engine: SessionEngine) -> (Option<u32>, Option<u32>) {
    match engine {
        SessionEngine::Codex | SessionEngine::Claude | SessionEngine::OpenCode => {
            extract_tool_failure_counts_jsonl_tail(path, engine)
        }
        SessionEngine::Gemini => extract_tool_failure_counts_gemini_json(path),
    }
}

fn extract_tool_failure_counts_jsonl_tail(
    path: &Path,
    engine: SessionEngine,
) -> (Option<u32>, Option<u32>) {
    const SMALL_TAIL_BYTES: usize = 512 * 1024;
    const LARGE_TAIL_BYTES: usize = 4 * 1024 * 1024;

    let counts = super::read_tail(path, SMALL_TAIL_BYTES)
        .ok()
        .map(|(tail, _size)| scan_tool_failures_jsonl(&tail, engine))
        .or_else(|| {
            super::read_tail(path, LARGE_TAIL_BYTES)
                .ok()
                .map(|(tail, _size)| scan_tool_failures_jsonl(&tail, engine))
        });

    match counts {
        Some((invalid, error)) => (Some(invalid), Some(error)),
        None => (None, None),
    }
}

fn scan_tool_failures_jsonl(text: &str, engine: SessionEngine) -> (u32, u32) {
    let mut invalid = 0u32;
    let mut error = 0u32;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };

        match engine {
            SessionEngine::Codex | SessionEngine::OpenCode => match parse_log_value(&value, None) {
                ParsedLogLine::Item(item) if item.kind == TimelineItemKind::ToolOutput => {
                    match classify_tool_output_detail(item.detail.as_str()) {
                        ToolOutputOutcome::Invalid => {
                            invalid = invalid.saturating_add(1);
                        }
                        ToolOutputOutcome::Error => {
                            error = error.saturating_add(1);
                        }
                        ToolOutputOutcome::Success | ToolOutputOutcome::Unknown => {}
                    }
                }
                _ => {}
            },
            SessionEngine::Claude => {
                for item in parse_claude_timeline_items(&value, 0) {
                    if item.kind != TimelineItemKind::ToolOutput {
                        continue;
                    }
                    match classify_tool_output_detail(item.detail.as_str()) {
                        ToolOutputOutcome::Invalid => invalid = invalid.saturating_add(1),
                        ToolOutputOutcome::Error => error = error.saturating_add(1),
                        ToolOutputOutcome::Success | ToolOutputOutcome::Unknown => {}
                    }
                }
            }
            SessionEngine::Gemini => {}
        }
    }

    (invalid, error)
}

fn extract_tool_failure_counts_gemini_json(path: &Path) -> (Option<u32>, Option<u32>) {
    const MAX_GEMINI_BYTES: u64 = 10 * 1024 * 1024;

    let Ok(meta) = fs::metadata(path) else {
        return (None, None);
    };
    let size = meta.len();
    if size > MAX_GEMINI_BYTES {
        return (None, None);
    }

    let Ok(text) = fs::read_to_string(path) else {
        return (None, None);
    };
    let Ok(value) = serde_json::from_str::<Value>(&text) else {
        return (None, None);
    };
    let parsed = parse_gemini_timeline_items(&value);

    let mut invalid = 0u32;
    let mut error = 0u32;
    for item in parsed.items {
        if item.kind != TimelineItemKind::ToolOutput {
            continue;
        }
        match classify_tool_output_detail(item.detail.as_str()) {
            ToolOutputOutcome::Invalid => invalid = invalid.saturating_add(1),
            ToolOutputOutcome::Error => error = error.saturating_add(1),
            ToolOutputOutcome::Success | ToolOutputOutcome::Unknown => {}
        }
    }

    (Some(invalid), Some(error))
}

fn find_last_token_usage_in_text(text: &str) -> Option<(u64, Option<u64>)> {
    for line in text.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let value: Value = serde_json::from_str(trimmed).ok()?;
        if let Some(tokens) = parse_token_usage_value(&value) {
            return Some(tokens);
        }
    }
    None
}

fn parse_token_usage_value(value: &Value) -> Option<(u64, Option<u64>)> {
    if value.get("type").and_then(|v| v.as_str()) != Some("event_msg") {
        return None;
    }

    let payload = value.get("payload")?;
    if payload.get("type").and_then(|v| v.as_str()) != Some("token_count") {
        return None;
    }

    let info = payload.get("info")?;
    if info.is_null() {
        return None;
    }

    let total = info
        .get("total_token_usage")
        .and_then(|v| v.get("total_tokens"))
        .and_then(|v| v.as_u64())?;
    let last = info
        .get("last_token_usage")
        .and_then(|v| v.get("total_tokens"))
        .and_then(|v| v.as_u64());
    Some((total, last))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SessionIndexFile {
    version: u32,
    entries: Vec<SessionIndexFileEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SessionIndexFileEntry {
    log_path: PathBuf,
    size_bytes: u64,
    modified_unix_ms: Option<i64>,
    total_tokens: Option<u64>,
    last_tokens: Option<u64>,
    #[serde(default)]
    tool_calls_invalid: Option<u32>,
    #[serde(default)]
    tool_calls_error: Option<u32>,
}

impl SessionIndexFile {
    fn from_index(index: &SessionIndex) -> Self {
        let entries = index
            .entries
            .iter()
            .map(|(log_path, entry)| SessionIndexFileEntry {
                log_path: log_path.clone(),
                size_bytes: entry.size_bytes,
                modified_unix_ms: entry.modified_unix_ms,
                total_tokens: entry.total_tokens,
                last_tokens: entry.last_tokens,
                tool_calls_invalid: entry.tool_calls_invalid,
                tool_calls_error: entry.tool_calls_error,
            })
            .collect();

        Self {
            version: 2,
            entries,
        }
    }

    fn into_index(self) -> SessionIndex {
        let mut entries = BTreeMap::new();
        for entry in self.entries {
            entries.insert(
                entry.log_path,
                SessionIndexEntry {
                    size_bytes: entry.size_bytes,
                    modified_unix_ms: entry.modified_unix_ms,
                    total_tokens: entry.total_tokens,
                    last_tokens: entry.last_tokens,
                    tool_calls_invalid: entry.tool_calls_invalid,
                    tool_calls_error: entry.tool_calls_error,
                },
            );
        }
        SessionIndex { entries }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{SessionEngine, SessionMeta, SessionSummary};
    use tempfile::tempdir;

    #[test]
    fn refresh_session_index_extracts_tool_failure_counts_from_codex_jsonl_tail() {
        let dir = tempdir().expect("tempdir");
        let log_path = dir.path().join("session.jsonl");

        let lines = [
            serde_json::json!({
                "type": "response_item",
                "timestamp": "2026-01-01T00:00:00Z",
                "payload": {
                    "type": "function_call_output",
                    "call_id": "c1",
                    "output": "Process exited with code 0\nok"
                }
            })
            .to_string(),
            serde_json::json!({
                "type": "response_item",
                "timestamp": "2026-01-01T00:00:01Z",
                "payload": {
                    "type": "function_call_output",
                    "call_id": "c2",
                    "output": "Process exited with code 2\nnope"
                }
            })
            .to_string(),
            "not-json".to_string(),
            serde_json::json!({
                "type": "response_item",
                "timestamp": "2026-01-01T00:00:02Z",
                "payload": {
                    "type": "function_call_output",
                    "call_id": "c3",
                    "output": "Invalid tool call: nope"
                }
            })
            .to_string(),
        ];
        std::fs::write(&log_path, lines.join("\n")).expect("write log");

        let meta_fs = std::fs::metadata(&log_path).expect("metadata");

        let session = SessionSummary {
            engine: SessionEngine::Codex,
            meta: SessionMeta {
                id: "s1".to_string(),
                cwd: dir.path().to_path_buf(),
                started_at_rfc3339: "2026-01-01T00:00:00Z".to_string(),
            },
            log_path: log_path.clone(),
            title: "test".to_string(),
            file_size_bytes: meta_fs.len(),
            file_modified: meta_fs.modified().ok(),
        };

        let index = refresh_session_index(&[session], &SessionIndex::default());
        let failures = index.tool_failures(&log_path).expect("tool failures");
        assert_eq!(failures.invalid, 1);
        assert_eq!(failures.error, 1);
    }
}
