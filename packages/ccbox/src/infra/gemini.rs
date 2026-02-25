use crate::domain::{
    GeminiTimelineParseOutput, GeminiUserLogEntry, SessionEngine, SessionMeta, SessionSummary,
    SessionTimeline, derive_title_from_user_text, extract_gemini_first_user_message,
    extract_gemini_session_id, extract_gemini_session_start_time, infer_gemini_title_from_session,
    is_metadata_prompt, make_session_summary, parse_gemini_logs_entries,
    parse_gemini_timeline_items,
};
use crate::infra::{LastAssistantOutput, ScanWarningCount};
use dirs::home_dir;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

#[derive(Clone, Debug)]
pub struct GeminiScanOutput {
    pub sessions: Vec<SessionSummary>,
    pub warnings: ScanWarningCount,
    pub notice: Option<String>,
}

const MAX_TIMELINE_ITEMS: usize = 10_000;

#[derive(Debug, thiserror::Error)]
pub enum ResolveGeminiRootDirError {
    #[error("home directory not found")]
    HomeDirNotFound,
}

pub fn resolve_gemini_root_dir() -> Result<PathBuf, ResolveGeminiRootDirError> {
    if let Some(override_dir) = std::env::var_os("CCBOX_GEMINI_DIR") {
        return Ok(PathBuf::from(override_dir));
    }

    let Some(home) = home_dir() else {
        return Err(ResolveGeminiRootDirError::HomeDirNotFound);
    };

    Ok(home.join(".gemini"))
}

pub fn scan_gemini_root_dir(root: &Path) -> GeminiScanOutput {
    let tmp_dir = root.join("tmp");
    if !tmp_dir.exists() {
        return GeminiScanOutput {
            sessions: Vec::new(),
            warnings: ScanWarningCount::from(0usize),
            notice: Some(format!(
                "Gemini tmp dir not found: {} (set CCBOX_GEMINI_DIR to override)",
                tmp_dir.display()
            )),
        };
    }

    let Ok(entries) = fs::read_dir(&tmp_dir) else {
        return GeminiScanOutput {
            sessions: Vec::new(),
            warnings: ScanWarningCount::from(0usize),
            notice: Some(format!(
                "Gemini tmp dir is not readable: {} (set CCBOX_GEMINI_DIR to override)",
                tmp_dir.display()
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

        let project_dir = entry.path();
        let Some(hash) = project_dir
            .file_name()
            .and_then(|name| name.to_str())
            .map(|s| s.to_string())
        else {
            continue;
        };
        if !is_project_hash_dir(&hash) {
            continue;
        }

        let output = scan_gemini_project_dir(&project_dir);
        warnings += output.warnings;
        sessions.extend(output.sessions);
    }

    GeminiScanOutput {
        sessions,
        warnings: ScanWarningCount::from(warnings),
        notice: None,
    }
}

struct ScanGeminiProjectOutput {
    sessions: Vec<SessionSummary>,
    warnings: usize,
}

fn scan_gemini_project_dir(project_dir: &Path) -> ScanGeminiProjectOutput {
    let mut warnings = 0usize;
    let mut sessions: Vec<SessionSummary> = Vec::new();

    let logs_path = project_dir.join("logs.json");
    let logs_entries: Vec<GeminiUserLogEntry> = match fs::read_to_string(&logs_path) {
        Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(value) => parse_gemini_logs_entries(&value),
            Err(_) => {
                warnings += 1;
                Vec::new()
            }
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(_) => {
            warnings += 1;
            Vec::new()
        }
    };

    let hints = build_log_hints(&logs_entries);

    let chats_dir = project_dir.join("chats");
    let Ok(entries) = fs::read_dir(&chats_dir) else {
        if chats_dir.exists() {
            warnings += 1;
        }
        return ScanGeminiProjectOutput { sessions, warnings };
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
        if !is_gemini_session_path(&path) {
            continue;
        }

        match scan_gemini_session_file(project_dir, &path, &hints) {
            Ok(summary) => sessions.push(summary),
            Err(_) => warnings += 1,
        }
    }

    ScanGeminiProjectOutput { sessions, warnings }
}

#[derive(Clone, Debug)]
struct GeminiLogHints {
    first_message_by_session: BTreeMap<String, String>,
    first_timestamp_by_session: BTreeMap<String, String>,
    session_id_by_prefix8: BTreeMap<String, Option<String>>,
}

fn build_log_hints(entries: &[GeminiUserLogEntry]) -> GeminiLogHints {
    let mut first_message_by_session: BTreeMap<String, String> = BTreeMap::new();
    let mut first_timestamp_by_session: BTreeMap<String, String> = BTreeMap::new();
    let mut session_id_by_prefix8: BTreeMap<String, Option<String>> = BTreeMap::new();

    for entry in entries {
        if !first_message_by_session.contains_key(&entry.session_id) {
            first_message_by_session.insert(entry.session_id.clone(), entry.message.clone());
        }
        if let Some(ts) = entry.timestamp.as_ref() {
            if !first_timestamp_by_session.contains_key(&entry.session_id) {
                first_timestamp_by_session.insert(entry.session_id.clone(), ts.clone());
            }
        }

        let prefix = entry.session_id.chars().take(8).collect::<String>();
        if prefix.len() != 8 {
            continue;
        }
        match session_id_by_prefix8.get(&prefix) {
            None => {
                session_id_by_prefix8.insert(prefix, Some(entry.session_id.clone()));
            }
            Some(Some(existing)) if existing == &entry.session_id => {}
            Some(_) => {
                session_id_by_prefix8.insert(prefix, None);
            }
        }
    }

    GeminiLogHints {
        first_message_by_session,
        first_timestamp_by_session,
        session_id_by_prefix8,
    }
}

fn scan_gemini_session_file(
    project_dir: &Path,
    path: &Path,
    hints: &GeminiLogHints,
) -> Result<SessionSummary, ()> {
    let metadata = fs::metadata(path).map_err(|_| ())?;
    let file_size_bytes = metadata.len();
    let file_modified = metadata.modified().ok();

    let project_cwd = project_dir.to_path_buf();

    let prefix = session_id_prefix_from_file_name(path);
    let mut session_id = prefix
        .as_ref()
        .and_then(|p| hints.session_id_by_prefix8.get(p))
        .and_then(|opt| opt.clone())
        .or_else(|| prefix.clone())
        .or_else(|| file_stem_string(path))
        .unwrap_or_else(|| "(unknown)".to_string());

    let mut started_at = hints.first_timestamp_by_session.get(&session_id).cloned();

    let mut title = hints
        .first_message_by_session
        .get(&session_id)
        .cloned()
        .and_then(|text| (!is_metadata_prompt(&text)).then_some(text))
        .and_then(|text| derive_title_from_user_text(&text));

    let needs_parse = title.is_none()
        || started_at.is_none()
        || session_id.len() == 8
        || !hints.first_message_by_session.contains_key(&session_id);
    if needs_parse {
        if let Ok(text) = fs::read_to_string(path) {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(parsed_id) = extract_gemini_session_id(&value) {
                    session_id = parsed_id;
                }

                if started_at.is_none() {
                    started_at = hints
                        .first_timestamp_by_session
                        .get(&session_id)
                        .cloned()
                        .or_else(|| extract_gemini_session_start_time(&value));
                }

                if title.is_none() {
                    title = hints
                        .first_message_by_session
                        .get(&session_id)
                        .cloned()
                        .and_then(|text| (!is_metadata_prompt(&text)).then_some(text))
                        .and_then(|text| derive_title_from_user_text(&text))
                        .or_else(|| infer_gemini_title_from_session(&value))
                        .or_else(|| {
                            extract_gemini_first_user_message(&value)
                                .and_then(|text| derive_title_from_user_text(&text))
                        });
                }
            }
        }
    }

    let started_at_rfc3339 = started_at
        .or_else(|| file_modified.and_then(system_time_to_rfc3339))
        .unwrap_or_else(|| {
            OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .unwrap_or_default()
        });

    let title = title.unwrap_or_else(|| "(untitled)".to_string());

    Ok(make_session_summary(
        SessionMeta {
            id: session_id,
            cwd: project_cwd,
            started_at_rfc3339,
        },
        path.to_path_buf(),
        title,
        file_size_bytes,
        file_modified,
        SessionEngine::Gemini,
    ))
}

pub fn load_gemini_session_timeline(path: &Path) -> io::Result<SessionTimeline> {
    let file = File::open(path)?;
    let value: serde_json::Value = serde_json::from_reader(file)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

    let GeminiTimelineParseOutput {
        mut items,
        warnings,
    } = parse_gemini_timeline_items(&value);
    let truncated = items.len() > MAX_TIMELINE_ITEMS;
    if truncated {
        items.truncate(MAX_TIMELINE_ITEMS);
    }

    Ok(SessionTimeline {
        items,
        turn_contexts: BTreeMap::new(),
        warnings,
        truncated,
    })
}

pub fn load_gemini_last_assistant_output(path: &Path) -> io::Result<LastAssistantOutput> {
    let file = File::open(path)?;
    let value: serde_json::Value = serde_json::from_reader(file)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

    let GeminiTimelineParseOutput { items, warnings } = parse_gemini_timeline_items(&value);
    let last = items
        .into_iter()
        .rev()
        .find(|item| item.kind == crate::domain::TimelineItemKind::Assistant)
        .map(|item| item.detail);

    Ok(LastAssistantOutput {
        output: last,
        warnings,
    })
}

fn is_project_hash_dir(value: &str) -> bool {
    if value.len() != 64 {
        return false;
    }
    value.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'))
}

pub fn is_gemini_session_path(path: &Path) -> bool {
    if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
        return false;
    }
    if path.file_name().and_then(|n| n.to_str()).is_none() {
        return false;
    }
    let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if !file_name.starts_with("session-") {
        return false;
    }
    path.parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        == Some("chats")
}

fn session_id_prefix_from_file_name(path: &Path) -> Option<String> {
    let stem = file_stem_string(path)?;
    let last = stem.rsplit('-').next()?;
    if last.is_empty() {
        None
    } else {
        Some(last.to_string())
    }
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
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn scans_sessions_from_tmp_hash_projects() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("gemini");
        let tmp = root.join("tmp");
        let hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let project_dir = tmp.join(hash);
        let chats_dir = project_dir.join("chats");
        fs::create_dir_all(&chats_dir).expect("create");

        fs::write(
            project_dir.join("logs.json"),
            serde_json::json!([
                {
                    "type": "user",
                    "timestamp": "2026-02-19T00:00:00Z",
                    "sessionId": "deadbeef-0000-0000-0000-000000000000",
                    "messageId": "m1",
                    "message": "hello"
                }
            ])
            .to_string(),
        )
        .expect("write logs");

        fs::write(
            chats_dir.join("session-2026-02-19T00-00-deadbeef.json"),
            serde_json::json!({
                "sessionId": "deadbeef-0000-0000-0000-000000000000",
                "startTime": "2026-02-19T00:00:00Z",
                "messages": []
            })
            .to_string(),
        )
        .expect("write session");

        let output = scan_gemini_root_dir(&root);
        assert_eq!(output.sessions.len(), 1);
        assert_eq!(output.sessions[0].engine, SessionEngine::Gemini);
        assert_eq!(output.sessions[0].meta.cwd, project_dir);
    }

    #[test]
    fn ignores_non_hash_entries_under_tmp() {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("gemini");
        fs::create_dir_all(root.join("tmp").join("bin")).expect("create");

        let output = scan_gemini_root_dir(&root);
        assert!(output.sessions.is_empty());
    }
}
