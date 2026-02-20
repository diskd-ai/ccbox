use crate::domain::{
    ClaudeSessionsIndexEntry, SessionMeta, SessionSummary, SessionTimeline, TimelineItemKind,
    derive_title_from_user_text, extract_claude_session_meta_hint, is_metadata_prompt,
    make_session_summary, parse_claude_sessions_index, parse_claude_timeline_items,
    parse_claude_user_message_text,
};
use crate::infra::{LastAssistantOutput, ScanWarningCount};
use dirs::home_dir;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

#[derive(Clone, Debug)]
pub struct ClaudeScanOutput {
    pub sessions: Vec<SessionSummary>,
    pub warnings: ScanWarningCount,
    pub notice: Option<String>,
}

const MAX_TIMELINE_ITEMS: usize = 10_000;

pub fn load_claude_last_assistant_output(path: &Path) -> io::Result<LastAssistantOutput> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);

    let mut warnings = 0usize;
    let mut last_output: Option<String> = None;

    let mut line_no: u64 = 0;
    for line_result in reader.lines() {
        let line = match line_result {
            Ok(line) => line,
            Err(_) => {
                warnings += 1;
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

        for item in parse_claude_timeline_items(&value, line_no) {
            if item.kind == TimelineItemKind::Assistant {
                last_output = Some(item.detail);
            }
        }
    }

    Ok(LastAssistantOutput {
        output: last_output,
        warnings,
    })
}

pub fn load_claude_session_timeline(path: &Path) -> io::Result<SessionTimeline> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);

    let mut warnings = 0usize;
    let mut truncated = false;
    let mut items = Vec::new();

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

        let next_items = parse_claude_timeline_items(&value, line_no);
        for item in next_items {
            if items.len() >= MAX_TIMELINE_ITEMS {
                truncated = true;
                break;
            }
            items.push(item);
        }

        if truncated {
            break;
        }
    }

    Ok(SessionTimeline {
        items,
        turn_contexts: BTreeMap::new(),
        warnings,
        truncated,
    })
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveClaudeProjectsDirError {
    #[error("home directory not found")]
    HomeDirNotFound,
}

pub fn resolve_claude_projects_dir() -> Result<PathBuf, ResolveClaudeProjectsDirError> {
    if let Some(override_dir) = std::env::var_os("CLAUDE_PROJECTS_DIR") {
        return Ok(PathBuf::from(override_dir));
    }

    let Some(home) = home_dir() else {
        return Err(ResolveClaudeProjectsDirError::HomeDirNotFound);
    };

    Ok(home.join(".claude").join("projects"))
}

pub fn scan_claude_projects_dir(projects_dir: &Path) -> ClaudeScanOutput {
    if !projects_dir.exists() {
        return ClaudeScanOutput {
            sessions: Vec::new(),
            warnings: ScanWarningCount::from(0usize),
            notice: Some(format!(
                "Claude projects dir not found: {}",
                projects_dir.display()
            )),
        };
    }

    let Ok(entries) = fs::read_dir(projects_dir) else {
        return ClaudeScanOutput {
            sessions: Vec::new(),
            warnings: ScanWarningCount::from(0usize),
            notice: Some(format!(
                "Claude projects dir is not readable: {}",
                projects_dir.display()
            )),
        };
    };

    let mut warnings = 0usize;
    let mut sessions: Vec<SessionSummary> = Vec::new();

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                warnings += 1;
                continue;
            }
        };

        let Ok(file_type) = entry.file_type() else {
            warnings += 1;
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }

        let project_key_dir = entry.path();
        let output = scan_claude_project_key_dir(&project_key_dir);
        warnings += output.warnings;
        sessions.extend(output.sessions);
    }

    ClaudeScanOutput {
        sessions,
        warnings: ScanWarningCount::from(warnings),
        notice: None,
    }
}

struct ScanProjectKeyOutput {
    sessions: Vec<SessionSummary>,
    warnings: usize,
}

fn scan_claude_project_key_dir(project_key_dir: &Path) -> ScanProjectKeyOutput {
    let sessions_index_path = project_key_dir.join("sessions-index.json");
    if sessions_index_path.is_file() {
        let parsed = fs::read_to_string(&sessions_index_path)
            .ok()
            .and_then(|text| parse_claude_sessions_index(&text).ok());
        if let Some(index) = parsed {
            return scan_project_key_from_sessions_index(project_key_dir, index);
        }

        let mut fallback = scan_project_key_from_jsonl_files(project_key_dir);
        fallback.warnings = fallback.warnings.saturating_add(1);
        return fallback;
    }

    scan_project_key_from_jsonl_files(project_key_dir)
}

fn scan_project_key_from_sessions_index(
    project_key_dir: &Path,
    index: crate::domain::ClaudeSessionsIndex,
) -> ScanProjectKeyOutput {
    let mut sessions: Vec<SessionSummary> = Vec::new();
    let mut warnings = 0usize;

    let index_project_path = index.original_path.map(PathBuf::from);
    for entry in index.entries {
        match summary_from_index_entry(project_key_dir, &index_project_path, &entry) {
            Ok(summary) => sessions.push(summary),
            Err(_) => warnings += 1,
        }
    }

    if sessions.is_empty() {
        let fallback = scan_project_key_from_jsonl_files(project_key_dir);
        warnings += fallback.warnings;
        sessions.extend(fallback.sessions);
    }

    ScanProjectKeyOutput { sessions, warnings }
}

fn summary_from_index_entry(
    project_key_dir: &Path,
    index_project_path: &Option<PathBuf>,
    entry: &ClaudeSessionsIndexEntry,
) -> Result<SessionSummary, ()> {
    let full_path = entry.full_path.as_ref().ok_or(())?;
    let log_path = if full_path.is_absolute() {
        full_path.clone()
    } else {
        project_key_dir.join(full_path)
    };

    let metadata = fs::metadata(&log_path).map_err(|_| ())?;
    let file_size_bytes = metadata.len();
    let file_modified = metadata.modified().ok();

    let crate::domain::ClaudeSessionMetaHint {
        cwd: hint_cwd,
        session_id: hint_session_id,
        timestamp: hint_timestamp,
        summary: _,
        first_prompt: _,
    } = scan_claude_session_file_meta_hint(&log_path);
    let cwd = entry
        .project_path
        .as_ref()
        .map(PathBuf::from)
        .or_else(|| index_project_path.clone())
        .or_else(|| hint_cwd.clone())
        .ok_or(())?;

    let started_at_rfc3339 = entry
        .created
        .as_ref()
        .filter(|s| !s.trim().is_empty())
        .cloned()
        .or_else(|| {
            entry
                .modified
                .as_ref()
                .filter(|s| !s.trim().is_empty())
                .cloned()
        })
        .or_else(|| hint_timestamp.clone())
        .or_else(|| file_modified.and_then(system_time_to_rfc3339))
        .unwrap_or_else(|| {
            OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .unwrap_or_default()
        });

    let session_id = entry
        .session_id
        .as_ref()
        .filter(|s| !s.trim().is_empty())
        .cloned()
        .or_else(|| hint_session_id.clone())
        .or_else(|| file_stem_string(&log_path))
        .unwrap_or_else(|| "(unknown)".to_string());

    let title = entry
        .summary
        .as_ref()
        .filter(|s| !s.trim().is_empty())
        .cloned()
        .or_else(|| {
            entry
                .first_prompt
                .as_ref()
                .and_then(|text| derive_title_from_user_text(text))
        })
        .or_else(|| scan_claude_session_file_title_hint(&log_path))
        .unwrap_or_else(|| "(untitled)".to_string());

    Ok(make_session_summary(
        SessionMeta {
            id: session_id,
            cwd,
            started_at_rfc3339,
        },
        log_path,
        title,
        file_size_bytes,
        file_modified,
    ))
}

fn scan_project_key_from_jsonl_files(project_key_dir: &Path) -> ScanProjectKeyOutput {
    let mut sessions: Vec<SessionSummary> = Vec::new();
    let mut warnings = 0usize;

    let entries = match fs::read_dir(project_key_dir) {
        Ok(entries) => entries,
        Err(_) => {
            return ScanProjectKeyOutput {
                sessions,
                warnings: 1,
            };
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                warnings += 1;
                continue;
            }
        };

        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }

        match scan_claude_session_file(&path) {
            Ok(summary) => sessions.push(summary),
            Err(_) => warnings += 1,
        }
    }

    ScanProjectKeyOutput { sessions, warnings }
}

const MAX_META_SCAN_LINES: usize = 250;
const MAX_META_SCAN_BYTES: usize = 512 * 1024;

fn scan_claude_session_file_meta_hint(path: &Path) -> crate::domain::ClaudeSessionMetaHint {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(_) => {
            return crate::domain::ClaudeSessionMetaHint {
                cwd: None,
                session_id: None,
                timestamp: None,
                summary: None,
                first_prompt: None,
            };
        }
    };
    let mut reader = BufReader::new(file);

    let mut bytes_read = 0usize;
    for _ in 0..MAX_META_SCAN_LINES {
        let mut line = String::new();
        let bytes = match reader.read_line(&mut line) {
            Ok(bytes) => bytes,
            Err(_) => {
                return crate::domain::ClaudeSessionMetaHint {
                    cwd: None,
                    session_id: None,
                    timestamp: None,
                    summary: None,
                    first_prompt: None,
                };
            }
        };
        if bytes == 0 {
            break;
        }
        bytes_read = bytes_read.saturating_add(bytes);
        if bytes_read > MAX_META_SCAN_BYTES {
            break;
        }

        let value: serde_json::Value = match serde_json::from_str(line.trim_end()) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let hint = extract_claude_session_meta_hint(&value);
        if hint.cwd.is_some() || hint.session_id.is_some() || hint.timestamp.is_some() {
            return hint;
        }
    }

    crate::domain::ClaudeSessionMetaHint {
        cwd: None,
        session_id: None,
        timestamp: None,
        summary: None,
        first_prompt: None,
    }
}

fn scan_claude_session_file_title_hint(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(file);

    let mut bytes_read = 0usize;
    for _ in 0..MAX_META_SCAN_LINES {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line).ok()?;
        if bytes == 0 {
            break;
        }
        bytes_read = bytes_read.saturating_add(bytes);
        if bytes_read > MAX_META_SCAN_BYTES {
            break;
        }

        let value: serde_json::Value = serde_json::from_str(line.trim_end()).ok()?;
        let Some(text) = parse_claude_user_message_text(&value) else {
            continue;
        };
        if is_metadata_prompt(&text) {
            continue;
        }
        if let Some(candidate) = derive_title_from_user_text(&text) {
            return Some(candidate);
        }
    }

    None
}

fn scan_claude_session_file(path: &Path) -> Result<SessionSummary, ()> {
    let metadata = fs::metadata(path).map_err(|_| ())?;
    let file_size_bytes = metadata.len();
    let file_modified = metadata.modified().ok();

    let file = File::open(path).map_err(|_| ())?;
    let mut reader = BufReader::new(file);

    let mut cwd: Option<PathBuf> = None;
    let mut session_id: Option<String> = None;
    let mut timestamp: Option<String> = None;
    let mut title: Option<String> = None;

    let mut bytes_read = 0usize;
    for _ in 0..MAX_META_SCAN_LINES {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line).map_err(|_| ())?;
        if bytes == 0 {
            break;
        }

        bytes_read = bytes_read.saturating_add(bytes);
        if bytes_read > MAX_META_SCAN_BYTES {
            break;
        }

        let value: serde_json::Value = match serde_json::from_str(line.trim_end()) {
            Ok(value) => value,
            Err(_) => continue,
        };

        let hint = extract_claude_session_meta_hint(&value);
        if cwd.is_none() {
            cwd = hint.cwd;
        }
        if session_id.is_none() {
            session_id = hint.session_id;
        }
        if timestamp.is_none() {
            timestamp = hint.timestamp;
        }

        if title.is_none() {
            if let Some(text) = parse_claude_user_message_text(&value) {
                if !is_metadata_prompt(&text) {
                    title = derive_title_from_user_text(&text);
                }
            }
        }

        if cwd.is_some() && session_id.is_some() && timestamp.is_some() && title.is_some() {
            break;
        }
    }

    let cwd = cwd.ok_or(())?;
    let session_id = session_id
        .or_else(|| file_stem_string(path))
        .unwrap_or_else(|| "(unknown)".to_string());
    let started_at_rfc3339 = timestamp
        .or_else(|| file_modified.and_then(system_time_to_rfc3339))
        .unwrap_or_else(|| {
            OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .unwrap_or_default()
        });
    let display_title = title.unwrap_or_else(|| "(untitled)".to_string());

    Ok(make_session_summary(
        SessionMeta {
            id: session_id,
            cwd,
            started_at_rfc3339,
        },
        path.to_path_buf(),
        display_title,
        file_size_bytes,
        file_modified,
    ))
}

fn file_stem_string(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string())
}

fn system_time_to_rfc3339(value: SystemTime) -> Option<String> {
    let timestamp = OffsetDateTime::from(value);
    timestamp.format(&Rfc3339).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn scans_sessions_index_fast_path() {
        let dir = tempdir().expect("tempdir");
        let projects_dir = dir.path().join("projects");
        let key_dir = projects_dir.join("k1");
        fs::create_dir_all(&key_dir).expect("create");

        let log_path = key_dir.join("s1.jsonl");
        fs::write(
            &log_path,
            r#"{"type":"user","cwd":"/tmp/p1","sessionId":"s1","timestamp":"2026-02-19T00:00:00Z","message":{"content":"hello"}}"#,
        )
        .expect("write log");

        let index_path = key_dir.join("sessions-index.json");
        let index_json = format!(
            r#"{{
  "originalPath": "/tmp/p1",
  "entries": [
    {{
      "sessionId": "s1",
      "fullPath": "{}",
      "created": "2026-02-19T00:00:00Z",
      "summary": "hello from index"
    }}
  ]
}}"#,
            log_path.display()
        );
        fs::write(&index_path, index_json).expect("write index");

        let output = scan_claude_projects_dir(&projects_dir);
        assert!(output.notice.is_none());
        assert_eq!(output.warnings.get(), 0);
        assert_eq!(output.sessions.len(), 1);
        assert_eq!(output.sessions[0].meta.cwd, PathBuf::from("/tmp/p1"));
        assert_eq!(output.sessions[0].meta.id, "s1");
        assert_eq!(output.sessions[0].title, "hello from index");
    }

    #[test]
    fn scans_jsonl_fallback_path_without_index() {
        let dir = tempdir().expect("tempdir");
        let projects_dir = dir.path().join("projects");
        let key_dir = projects_dir.join("k2");
        fs::create_dir_all(&key_dir).expect("create");

        let log_path = key_dir.join("s2.jsonl");
        let mut file = File::create(&log_path).expect("create log");
        writeln!(
            file,
            r#"{{"type":"user","cwd":"/tmp/p2","sessionId":"s2","timestamp":"2026-02-19T00:00:00Z","message":{{"content":"first"}}}}"#
        )
        .expect("write");

        let output = scan_claude_projects_dir(&projects_dir);
        assert_eq!(output.warnings.get(), 0);
        assert_eq!(output.sessions.len(), 1);
        assert_eq!(output.sessions[0].meta.cwd, PathBuf::from("/tmp/p2"));
        assert_eq!(output.sessions[0].meta.id, "s2");
        assert_eq!(output.sessions[0].title, "first");
    }

    #[test]
    fn missing_projects_dir_returns_notice() {
        let dir = tempdir().expect("tempdir");
        let projects_dir = dir.path().join("missing");
        let output = scan_claude_projects_dir(&projects_dir);
        assert!(output.sessions.is_empty());
        assert!(output.notice.is_some());
    }
}
