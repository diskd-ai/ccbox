use crate::domain::{ProjectSummary, SessionMeta, SessionSummary};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::SystemTime;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("invalid json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("missing required field: {0}")]
    MissingField(&'static str),
}

#[derive(Debug, Deserialize)]
struct SessionMetaLine {
    #[serde(rename = "type")]
    line_type: String,
    payload: SessionMetaPayload,
}

#[derive(Debug, Deserialize)]
struct SessionMetaPayload {
    id: String,
    timestamp: String,
    cwd: String,
}

pub fn parse_session_meta_line(line: &str) -> Result<SessionMeta, ParseError> {
    let parsed: SessionMetaLine = serde_json::from_str(line)?;
    if parsed.line_type != "session_meta" {
        return Err(ParseError::MissingField("type=session_meta"));
    }

    Ok(SessionMeta {
        id: parsed.payload.id,
        cwd: PathBuf::from(parsed.payload.cwd),
        started_at_rfc3339: parsed.payload.timestamp,
    })
}

#[derive(Debug, Deserialize)]
struct ResponseItemLine {
    #[serde(rename = "type")]
    line_type: String,
    payload: ResponseItemPayload,
}

#[derive(Debug, Deserialize)]
struct ResponseItemPayload {
    #[serde(rename = "type")]
    payload_type: String,
    role: Option<String>,
    content: Option<Vec<ContentItem>>,
}

#[derive(Debug, Deserialize)]
struct ContentItem {
    #[serde(rename = "type")]
    content_type: String,
    text: Option<String>,
}

pub fn parse_user_message_text(line: &str) -> Result<Option<String>, ParseError> {
    let parsed: ResponseItemLine = serde_json::from_str(line)?;
    if parsed.line_type != "response_item" {
        return Ok(None);
    }
    if parsed.payload.payload_type != "message" {
        return Ok(None);
    }
    if parsed.payload.role.as_deref() != Some("user") {
        return Ok(None);
    }

    let Some(content) = parsed.payload.content else {
        return Ok(None);
    };

    for item in content {
        if item.content_type == "input_text" {
            if let Some(text) = item.text {
                return Ok(Some(text));
            }
        }
    }

    Ok(None)
}

pub fn is_metadata_prompt(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with("# AGENTS.md instructions")
        || trimmed.starts_with("<environment_context>")
        || trimmed.starts_with("<INSTRUCTIONS>")
        || (trimmed.starts_with("<skill>") && trimmed.contains("</skill>"))
}

pub fn derive_title_from_user_text(text: &str) -> Option<String> {
    let first_line = text
        .lines()
        .map(|line| line.trim())
        .find(|line| !line.is_empty())?;
    Some(first_line.to_string())
}

pub fn index_projects(sessions: &[SessionSummary]) -> Vec<ProjectSummary> {
    let mut grouped: BTreeMap<PathBuf, Vec<SessionSummary>> = BTreeMap::new();
    for session in sessions {
        grouped
            .entry(session.meta.cwd.clone())
            .or_default()
            .push(session.clone());
    }

    let mut projects: Vec<ProjectSummary> = grouped
        .into_iter()
        .map(|(project_path, mut project_sessions)| {
            project_sessions
                .sort_by_key(|session| session.file_modified.unwrap_or(SystemTime::UNIX_EPOCH));
            project_sessions.reverse();
            let last_modified = project_sessions
                .first()
                .and_then(|session| session.file_modified);
            ProjectSummary {
                name: project_path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| project_path.display().to_string()),
                project_path,
                sessions: project_sessions,
                last_modified,
            }
        })
        .collect();

    projects.sort_by_key(|project| project.last_modified.unwrap_or(SystemTime::UNIX_EPOCH));
    projects.reverse();
    projects
}

pub fn make_session_summary(
    meta: SessionMeta,
    log_path: PathBuf,
    title: String,
    file_size_bytes: u64,
    file_modified: Option<SystemTime>,
) -> SessionSummary {
    SessionSummary {
        meta,
        log_path,
        title,
        file_size_bytes,
        file_modified,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_session_meta() {
        let line = r#"{"timestamp":"2026-02-18T21:45:57.762Z","type":"session_meta","payload":{"id":"abc","timestamp":"2026-02-18T21:39:39.022Z","cwd":"/tmp/project"}}"#;
        let meta = parse_session_meta_line(line).expect("meta");
        assert_eq!(meta.id, "abc");
        assert_eq!(meta.cwd.to_string_lossy(), "/tmp/project");
    }

    #[test]
    fn extracts_user_message_text() {
        let line = r#"{"timestamp":"2026-02-18T21:45:57.764Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hello\nworld"}]}}"#;
        let text = parse_user_message_text(line).expect("parse");
        assert_eq!(text, Some("hello\nworld".to_string()));
        assert_eq!(
            derive_title_from_user_text("hello\nworld"),
            Some("hello".to_string())
        );
    }

    #[test]
    fn detects_metadata_prompts() {
        assert!(is_metadata_prompt(
            "# AGENTS.md instructions for /x\n\n<INSTRUCTIONS>..."
        ));
        assert!(is_metadata_prompt(
            "<environment_context>\n  <cwd>/x</cwd>\n</environment_context>"
        ));
        assert!(is_metadata_prompt("<INSTRUCTIONS>\nfoo\n</INSTRUCTIONS>"));
        assert!(!is_metadata_prompt("do the thing"));
    }
}
