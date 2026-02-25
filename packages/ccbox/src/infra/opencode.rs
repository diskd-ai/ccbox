use crate::domain::{SessionEngine, SessionMeta, SessionSummary, make_session_summary};
use crate::infra::{ResolveCcboxStateDirError, ScanWarningCount, resolve_ccbox_state_dir};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

#[derive(Clone, Debug)]
pub struct OpenCodeScanOutput {
    pub sessions: Vec<SessionSummary>,
    pub warnings: ScanWarningCount,
    pub notice: Option<String>,
}

#[derive(Debug, Error)]
pub enum ResolveOpenCodeDbPathError {
    #[error("home directory not found")]
    HomeDirNotFound,
}

pub fn resolve_opencode_db_path() -> Result<PathBuf, ResolveOpenCodeDbPathError> {
    if let Some(override_path) = std::env::var_os("CCBOX_OPENCODE_DB_PATH") {
        return Ok(PathBuf::from(override_path));
    }

    if let Some(xdg_data_home) = std::env::var_os("XDG_DATA_HOME") {
        return Ok(PathBuf::from(xdg_data_home)
            .join("opencode")
            .join("opencode.db"));
    }

    let Some(home) = dirs::home_dir() else {
        return Err(ResolveOpenCodeDbPathError::HomeDirNotFound);
    };

    Ok(home
        .join(".local")
        .join("share")
        .join("opencode")
        .join("opencode.db"))
}

pub fn scan_opencode_db(db_path: &Path) -> OpenCodeScanOutput {
    let state_dir = match resolve_ccbox_state_dir() {
        Ok(dir) => dir,
        Err(ResolveCcboxStateDirError::HomeDirNotFound) => {
            return OpenCodeScanOutput {
                sessions: Vec::new(),
                warnings: ScanWarningCount::from(0usize),
                notice: Some("OpenCode disabled: home directory not found".to_string()),
            };
        }
    };

    scan_opencode_db_with_state_dir(db_path, &state_dir)
}

pub fn scan_opencode_db_with_state_dir(db_path: &Path, state_dir: &Path) -> OpenCodeScanOutput {
    if !db_path.exists() {
        return OpenCodeScanOutput {
            sessions: Vec::new(),
            warnings: ScanWarningCount::from(0usize),
            notice: Some(format!(
                "OpenCode DB not found: {} (set CCBOX_OPENCODE_DB_PATH to override)",
                db_path.display()
            )),
        };
    }

    let conn = match open_db_readonly(db_path) {
        Ok(conn) => conn,
        Err(error) => {
            return OpenCodeScanOutput {
                sessions: Vec::new(),
                warnings: ScanWarningCount::from(0usize),
                notice: Some(format!(
                    "OpenCode DB is not readable: {} ({error})",
                    db_path.display()
                )),
            };
        }
    };

    let mut sessions: Vec<SessionSummary> = Vec::new();
    let mut warnings = 0usize;

    let sql = r#"
        SELECT
            s.id,
            s.title,
            s.directory,
            s.time_created,
            s.time_updated,
            p.worktree
        FROM session s
        JOIN project p ON p.id = s.project_id
        WHERE s.time_archived IS NULL
        ORDER BY s.time_updated DESC, s.id DESC
    "#;

    let mut stmt = match conn.prepare(sql) {
        Ok(stmt) => stmt,
        Err(_) => {
            return OpenCodeScanOutput {
                sessions: Vec::new(),
                warnings: ScanWarningCount::from(1usize),
                notice: Some("OpenCode DB has an unexpected schema.".to_string()),
            };
        }
    };

    let rows = stmt
        .query_map([], |row| {
            let id: String = row.get(0)?;
            let title: String = row.get(1)?;
            let directory: String = row.get(2)?;
            let time_created: i64 = row.get(3)?;
            let time_updated: i64 = row.get(4)?;
            let worktree: String = row.get(5)?;
            Ok((id, title, directory, time_created, time_updated, worktree))
        })
        .ok();

    let Some(rows) = rows else {
        return OpenCodeScanOutput {
            sessions: Vec::new(),
            warnings: ScanWarningCount::from(1usize),
            notice: Some("Failed to read OpenCode sessions.".to_string()),
        };
    };

    for row in rows {
        let (id, title, directory, time_created, time_updated, worktree) = match row {
            Ok(row) => row,
            Err(_) => {
                warnings += 1;
                continue;
            }
        };

        let started_at_rfc3339 = unix_ms_to_rfc3339(time_created).unwrap_or_else(now_rfc3339);
        let file_modified = unix_ms_to_system_time(time_updated);

        let cwd = if worktree.trim().is_empty() {
            PathBuf::from(directory.clone())
        } else {
            PathBuf::from(worktree)
        };

        let log_path = opencode_session_cache_path(state_dir, id.as_str());
        let file_size_bytes = fs::metadata(&log_path).ok().map(|m| m.len()).unwrap_or(0);

        sessions.push(make_session_summary(
            SessionMeta {
                id,
                cwd,
                started_at_rfc3339,
            },
            log_path,
            title,
            file_size_bytes,
            file_modified,
            SessionEngine::OpenCode,
        ));
    }

    OpenCodeScanOutput {
        sessions,
        warnings: ScanWarningCount::from(warnings),
        notice: None,
    }
}

fn open_db_readonly(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let _ = conn.busy_timeout(Duration::from_millis(250));
    Ok(conn)
}

pub fn opencode_session_cache_path(state_dir: &Path, session_id: &str) -> PathBuf {
    state_dir
        .join("opencode")
        .join("sessions")
        .join(format!("{session_id}.jsonl"))
}

fn unix_ms_to_rfc3339(ms: i64) -> Option<String> {
    let nanos: i128 = i128::from(ms).saturating_mul(1_000_000);
    let timestamp = OffsetDateTime::from_unix_timestamp_nanos(nanos).ok()?;
    timestamp.format(&Rfc3339).ok()
}

fn now_rfc3339() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "".to_string())
}

fn unix_ms_to_system_time(ms: i64) -> Option<SystemTime> {
    let ms_u64 = u64::try_from(ms).ok()?;
    UNIX_EPOCH.checked_add(Duration::from_millis(ms_u64))
}

#[derive(Debug, Error)]
pub enum PrepareSessionLogError {
    #[error(transparent)]
    ResolveOpenCodeDbPath(#[from] ResolveOpenCodeDbPathError),

    #[error(transparent)]
    ResolveCcboxStateDir(#[from] ResolveCcboxStateDirError),

    #[error("OpenCode DB not found: {0}")]
    DbMissing(String),

    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("failed to parse OpenCode JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("failed to write OpenCode cache: {0}")]
    Io(#[from] std::io::Error),

    #[error("OpenCode session not found: {0}")]
    SessionNotFound(String),
}

pub fn prepare_session_log_path(
    session: &SessionSummary,
) -> Result<PathBuf, PrepareSessionLogError> {
    if session.engine != SessionEngine::OpenCode {
        return Ok(session.log_path.clone());
    }

    let db_path = resolve_opencode_db_path()?;
    if !db_path.exists() {
        return Err(PrepareSessionLogError::DbMissing(
            db_path.display().to_string(),
        ));
    }
    let state_dir = resolve_ccbox_state_dir()?;
    let cache_path = opencode_session_cache_path(&state_dir, &session.meta.id);
    ensure_opencode_session_cache(&db_path, &cache_path, &session.meta.id)?;
    Ok(cache_path)
}

fn ensure_opencode_session_cache(
    db_path: &Path,
    cache_path: &Path,
    session_id: &str,
) -> Result<(), PrepareSessionLogError> {
    let conn = open_db_readonly(db_path)?;

    let session_row = load_session_row(&conn, session_id)?;

    if cache_is_fresh(cache_path, session_row.time_updated_ms)? {
        return Ok(());
    }

    let data = load_messages_and_parts(&conn, session_id)?;
    let values = build_codex_jsonl_values(&session_row, &data.messages, &data.parts_by_message);

    if let Some(parent) = cache_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let tmp_path = cache_path.with_extension("jsonl.tmp");
    {
        let mut file = fs::File::create(&tmp_path)?;
        for value in values {
            let line = serde_json::to_string(&value)?;
            writeln!(file, "{line}")?;
        }
        file.flush()?;
    }
    fs::rename(tmp_path, cache_path)?;
    Ok(())
}

#[derive(Clone, Debug)]
struct OpenCodeSessionRow {
    session_id: String,
    title: String,
    directory: String,
    worktree: String,
    time_created_ms: i64,
    time_updated_ms: i64,
}

#[derive(Clone, Debug)]
struct OpenCodeMessageRow {
    id: String,
    time_created_ms: i64,
    data: Value,
}

type OpenCodePartsByMessage = BTreeMap<String, Vec<OpenCodePartRow>>;

#[derive(Clone, Debug)]
struct OpenCodePartRow {
    data: Value,
}

#[derive(Clone, Debug)]
struct OpenCodeMessagesAndParts {
    messages: Vec<OpenCodeMessageRow>,
    parts_by_message: OpenCodePartsByMessage,
}

fn load_session_row(
    conn: &Connection,
    session_id: &str,
) -> Result<OpenCodeSessionRow, PrepareSessionLogError> {
    let sql = r#"
        SELECT
            s.id,
            s.title,
            s.directory,
            s.time_created,
            s.time_updated,
            p.worktree
        FROM session s
        JOIN project p ON p.id = s.project_id
        WHERE s.id = ?1
        LIMIT 1
    "#;

    let mut stmt = conn.prepare(sql)?;
    let row = stmt.query_row([session_id], |row| {
        let id: String = row.get(0)?;
        let title: String = row.get(1)?;
        let directory: String = row.get(2)?;
        let time_created_ms: i64 = row.get(3)?;
        let time_updated_ms: i64 = row.get(4)?;
        let worktree: String = row.get(5)?;
        Ok(OpenCodeSessionRow {
            session_id: id,
            title,
            directory,
            worktree,
            time_created_ms,
            time_updated_ms,
        })
    });

    match row {
        Ok(row) => Ok(row),
        Err(rusqlite::Error::QueryReturnedNoRows) => Err(PrepareSessionLogError::SessionNotFound(
            session_id.to_string(),
        )),
        Err(error) => Err(error.into()),
    }
}

fn cache_is_fresh(
    path: &Path,
    expected_time_updated_ms: i64,
) -> Result<bool, PrepareSessionLogError> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };

    let mut reader = BufReader::new(file);
    let mut first_line = String::new();
    let bytes = reader.read_line(&mut first_line)?;
    if bytes == 0 {
        return Ok(false);
    }

    let value: Value = serde_json::from_str(first_line.trim_end())?;
    if value.get("type").and_then(|v| v.as_str()) != Some("session_meta") {
        return Ok(false);
    }

    let payload = value.get("payload").unwrap_or(&Value::Null);
    let cached = payload
        .get("opencode_time_updated_ms")
        .and_then(|v| v.as_i64());
    Ok(cached == Some(expected_time_updated_ms))
}

fn load_messages_and_parts(
    conn: &Connection,
    session_id: &str,
) -> Result<OpenCodeMessagesAndParts, PrepareSessionLogError> {
    let mut messages: Vec<OpenCodeMessageRow> = Vec::new();
    let mut parts_by_message: OpenCodePartsByMessage = BTreeMap::new();

    {
        let mut stmt = conn.prepare(
            r#"
            SELECT id, time_created, data
            FROM message
            WHERE session_id = ?1
            ORDER BY time_created ASC, id ASC
        "#,
        )?;
        let rows = stmt.query_map([session_id], |row| {
            let id: String = row.get(0)?;
            let time_created_ms: i64 = row.get(1)?;
            let raw: String = row.get(2)?;
            Ok((id, time_created_ms, raw))
        })?;

        for row in rows {
            let (id, time_created_ms, raw) = row?;
            let data: Value = match serde_json::from_str(&raw) {
                Ok(data) => data,
                Err(_) => continue,
            };
            messages.push(OpenCodeMessageRow {
                id,
                time_created_ms,
                data,
            });
        }
    }

    {
        let mut stmt = conn.prepare(
            r#"
            SELECT message_id, data
            FROM part
            WHERE session_id = ?1
            ORDER BY time_created ASC, message_id ASC, id ASC
        "#,
        )?;
        let rows = stmt.query_map([session_id], |row| {
            let message_id: String = row.get(0)?;
            let raw: String = row.get(1)?;
            Ok((message_id, raw))
        })?;

        for row in rows {
            let (message_id, raw) = row?;
            let data: Value = match serde_json::from_str(&raw) {
                Ok(data) => data,
                Err(_) => continue,
            };
            let part = OpenCodePartRow { data };
            parts_by_message.entry(message_id).or_default().push(part);
        }
    }

    Ok(OpenCodeMessagesAndParts {
        messages,
        parts_by_message,
    })
}

fn build_codex_jsonl_values(
    session: &OpenCodeSessionRow,
    messages: &[OpenCodeMessageRow],
    parts_by_message: &OpenCodePartsByMessage,
) -> Vec<Value> {
    let mut lines: Vec<Value> = Vec::new();

    let started_at = unix_ms_to_rfc3339(session.time_created_ms).unwrap_or_else(now_rfc3339);
    let meta_cwd = if session.worktree.trim().is_empty() {
        &session.directory
    } else {
        &session.worktree
    };
    lines.push(serde_json::json!({
        "timestamp": started_at,
        "type": "session_meta",
        "payload": {
            "id": &session.session_id,
            "timestamp": started_at,
            "cwd": meta_cwd,
            "opencode_time_updated_ms": session.time_updated_ms,
            "opencode_directory": &session.directory,
            "opencode_title": &session.title,
        }
    }));

    let mut assistants_by_parent: BTreeMap<String, Vec<&OpenCodeMessageRow>> = BTreeMap::new();
    let mut user_messages: Vec<&OpenCodeMessageRow> = Vec::new();

    for message in messages {
        let role = message
            .data
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if role == "user" {
            user_messages.push(message);
            continue;
        }
        if role == "assistant" {
            if let Some(parent_id) = message
                .data
                .get("parentID")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
            {
                assistants_by_parent
                    .entry(parent_id)
                    .or_default()
                    .push(message);
            }
        }
    }

    let mut running_total_tokens: u64 = 0;

    for user in user_messages {
        let turn_id = user.id.clone();
        let ts = unix_ms_to_rfc3339(user.time_created_ms).unwrap_or_else(now_rfc3339);

        let model = user
            .data
            .get("model")
            .and_then(|v| v.as_object())
            .and_then(|model| {
                let provider = model.get("providerID")?.as_str()?;
                let id = model.get("modelID")?.as_str()?;
                Some(format!("{provider}/{id}"))
            });

        lines.push(serde_json::json!({
            "timestamp": ts,
            "type": "turn_context",
            "payload": {
                "turn_id": turn_id,
                "cwd": &session.directory,
                "model": model,
            }
        }));

        let user_text = join_user_parts(parts_by_message.get(&user.id));
        if !user_text.trim().is_empty() {
            lines.push(serde_json::json!({
                "timestamp": ts,
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [{ "type": "input_text", "text": user_text }],
                }
            }));
        }

        if let Some(mut assistants) = assistants_by_parent.remove(&user.id) {
            assistants.sort_by_key(|msg| (msg.time_created_ms, msg.id.clone()));
            for assistant in assistants {
                let assistant_ts =
                    unix_ms_to_rfc3339(assistant.time_created_ms).unwrap_or_else(now_rfc3339);
                for value in assistant_message_lines(
                    assistant,
                    parts_by_message.get(&assistant.id),
                    &assistant_ts,
                    &mut running_total_tokens,
                ) {
                    lines.push(value);
                }
            }
        }
    }

    lines
}

fn join_user_parts(parts: Option<&Vec<OpenCodePartRow>>) -> String {
    let Some(parts) = parts else {
        return "".to_string();
    };

    let mut text = String::new();
    let mut attachments: Vec<String> = Vec::new();

    for part in parts {
        let kind = part.data.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match kind {
            "text" => {
                if let Some(chunk) = part.data.get("text").and_then(|v| v.as_str()) {
                    text.push_str(chunk);
                }
            }
            "file" => {
                let mime = part.data.get("mime").and_then(|v| v.as_str()).unwrap_or("");
                let url = part.data.get("url").and_then(|v| v.as_str()).unwrap_or("");
                let filename = part
                    .data
                    .get("filename")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let label = if !filename.trim().is_empty() {
                    format!("[file] {mime} {filename} {url}")
                } else {
                    format!("[file] {mime} {url}")
                };
                let label = label.trim().to_string();
                attachments.push(label);
            }
            _ => {}
        }
    }

    if !attachments.is_empty() {
        if !text.trim().is_empty() {
            text.push('\n');
            text.push('\n');
        }
        text.push_str(&attachments.join("\n"));
    }

    text
}

fn assistant_message_lines(
    assistant: &OpenCodeMessageRow,
    parts: Option<&Vec<OpenCodePartRow>>,
    default_ts: &str,
    running_total_tokens: &mut u64,
) -> Vec<Value> {
    let mut lines: Vec<Value> = Vec::new();
    let Some(parts) = parts else {
        return lines;
    };

    let mut output_text = String::new();

    for part in parts {
        let kind = part.data.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match kind {
            "reasoning" => {
                let text = part.data.get("text").and_then(|v| v.as_str()).unwrap_or("");
                if text.trim().is_empty() {
                    continue;
                }
                let ts = part_timestamp_rfc3339(&part.data, default_ts);
                lines.push(serde_json::json!({
                    "timestamp": ts,
                    "type": "response_item",
                    "payload": {
                        "type": "reasoning",
                        "summary": [{ "type": "summary_text", "text": text }],
                    }
                }));
            }
            "tool" => {
                let tool = part
                    .data
                    .get("tool")
                    .and_then(|v| v.as_str())
                    .unwrap_or("tool");
                let call_id = part
                    .data
                    .get("callID")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let state = part.data.get("state").unwrap_or(&Value::Null);
                let status = state.get("status").and_then(|v| v.as_str()).unwrap_or("");
                let (call_ts, out_ts) = tool_timestamps_rfc3339(state, default_ts);

                let arguments = tool_arguments_string(state);
                lines.push(serde_json::json!({
                    "timestamp": call_ts,
                    "type": "response_item",
                    "payload": {
                        "type": "function_call",
                        "name": tool,
                        "call_id": call_id,
                        "arguments": arguments,
                    }
                }));

                if status == "completed" {
                    let output = state.get("output").and_then(|v| v.as_str()).unwrap_or("");
                    if !output.trim().is_empty() {
                        lines.push(serde_json::json!({
                            "timestamp": out_ts,
                            "type": "response_item",
                            "payload": {
                                "type": "function_call_output",
                                "call_id": call_id,
                                "output": output,
                            }
                        }));
                    }
                } else if status == "error" {
                    let error = state.get("error").and_then(|v| v.as_str()).unwrap_or("");
                    if !error.trim().is_empty() {
                        lines.push(serde_json::json!({
                            "timestamp": out_ts,
                            "type": "response_item",
                            "payload": {
                                "type": "function_call_output",
                                "call_id": call_id,
                                "output": format!("error: {error}"),
                            }
                        }));
                    }
                }
            }
            "text" => {
                if let Some(chunk) = part.data.get("text").and_then(|v| v.as_str()) {
                    output_text.push_str(chunk);
                }
            }
            _ => {}
        }
    }

    if !output_text.trim().is_empty() {
        lines.push(serde_json::json!({
            "timestamp": default_ts,
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": output_text }],
            }
        }));
    }

    if let Some(last_tokens) = assistant_total_tokens(&assistant.data) {
        *running_total_tokens = running_total_tokens.saturating_add(last_tokens);
        let info = serde_json::json!({
            "total_token_usage": { "total_tokens": *running_total_tokens },
            "last_token_usage": { "total_tokens": last_tokens },
            "opencode": {
                "providerID": assistant.data.get("providerID").and_then(|v| v.as_str()),
                "modelID": assistant.data.get("modelID").and_then(|v| v.as_str()),
                "tokens": assistant.data.get("tokens"),
                "cost": assistant.data.get("cost"),
            }
        });

        lines.push(serde_json::json!({
            "timestamp": default_ts,
            "type": "event_msg",
            "payload": { "type": "token_count", "info": info },
        }));
    }

    lines
}

fn part_timestamp_rfc3339(part: &Value, fallback: &str) -> String {
    let start_ms = part
        .get("time")
        .and_then(|t| t.get("start"))
        .and_then(|v| v.as_i64());
    start_ms
        .and_then(unix_ms_to_rfc3339)
        .unwrap_or_else(|| fallback.to_string())
}

fn tool_timestamps_rfc3339(state: &Value, fallback: &str) -> (String, String) {
    let time = state.get("time").unwrap_or(&Value::Null);
    let start_ms = time.get("start").and_then(|v| v.as_i64());
    let end_ms = time.get("end").and_then(|v| v.as_i64()).or(start_ms);

    let start = start_ms
        .and_then(unix_ms_to_rfc3339)
        .unwrap_or_else(|| fallback.to_string());
    let end = end_ms
        .and_then(unix_ms_to_rfc3339)
        .unwrap_or_else(|| fallback.to_string());
    (start, end)
}

fn tool_arguments_string(state: &Value) -> String {
    if let Some(raw) = state.get("raw").and_then(|v| v.as_str()) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }

    if let Some(input) = state.get("input") {
        return serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string());
    }

    "".to_string()
}

fn assistant_total_tokens(value: &Value) -> Option<u64> {
    let tokens = value.get("tokens")?;
    if let Some(total) = tokens.get("total").and_then(|v| v.as_u64()) {
        return Some(total);
    }
    let input = tokens.get("input").and_then(|v| v.as_u64()).unwrap_or(0);
    let output = tokens.get("output").and_then(|v| v.as_u64()).unwrap_or(0);
    let reasoning = tokens
        .get("reasoning")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let cache = tokens.get("cache").unwrap_or(&Value::Null);
    let cache_read = cache.get("read").and_then(|v| v.as_u64()).unwrap_or(0);
    let cache_write = cache.get("write").and_then(|v| v.as_u64()).unwrap_or(0);
    Some(input + output + reasoning + cache_read + cache_write)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use tempfile::tempdir;

    fn create_minimal_db(path: &Path) -> Connection {
        let conn = Connection::open(path).expect("open");
        conn.execute_batch(
            r#"
            CREATE TABLE project (
              id TEXT PRIMARY KEY,
              worktree TEXT NOT NULL,
              name TEXT,
              time_created INTEGER NOT NULL,
              time_updated INTEGER NOT NULL,
              sandboxes TEXT NOT NULL
            );
            CREATE TABLE session (
              id TEXT PRIMARY KEY,
              project_id TEXT NOT NULL,
              parent_id TEXT,
              slug TEXT NOT NULL,
              directory TEXT NOT NULL,
              title TEXT NOT NULL,
              version TEXT NOT NULL,
              time_created INTEGER NOT NULL,
              time_updated INTEGER NOT NULL,
              time_archived INTEGER
            );
        "#,
        )
        .expect("schema");
        conn
    }

    #[test]
    fn scans_sessions_from_sqlite() {
        let dir = tempdir().expect("tempdir");
        let db_path = dir.path().join("opencode.db");
        let conn = create_minimal_db(&db_path);

        conn.execute(
            "INSERT INTO project (id, worktree, name, time_created, time_updated, sandboxes) VALUES (?1, ?2, NULL, 1, 1, '[]')",
            params!["p1", "/tmp/worktree"],
        )
        .expect("project");

        conn.execute(
            "INSERT INTO session (id, project_id, parent_id, slug, directory, title, version, time_created, time_updated, time_archived) VALUES (?1, ?2, NULL, 's', '/tmp/worktree', 't', 'v', 10, 20, NULL)",
            params!["s1", "p1"],
        )
        .expect("session");

        let state_dir = dir.path().join("state");
        fs::create_dir_all(&state_dir).expect("state");

        let output = scan_opencode_db_with_state_dir(&db_path, &state_dir);
        assert_eq!(output.warnings.get(), 0);
        assert_eq!(output.sessions.len(), 1);
        assert_eq!(output.sessions[0].engine, SessionEngine::OpenCode);
        assert_eq!(output.sessions[0].meta.id, "s1");
        assert_eq!(output.sessions[0].meta.cwd, PathBuf::from("/tmp/worktree"));
        assert!(
            output.sessions[0]
                .log_path
                .ends_with("opencode/sessions/s1.jsonl")
        );
    }

    #[test]
    fn materializes_a_codex_jsonl_cache_for_a_session() {
        let dir = tempdir().expect("tempdir");
        let db_path = dir.path().join("opencode.db");
        let conn = create_minimal_db(&db_path);

        conn.execute(
            "INSERT INTO project (id, worktree, name, time_created, time_updated, sandboxes) VALUES (?1, ?2, NULL, 1, 1, '[]')",
            params!["p1", "/tmp/worktree"],
        )
        .expect("project");

        conn.execute(
            "INSERT INTO session (id, project_id, parent_id, slug, directory, title, version, time_created, time_updated, time_archived) VALUES (?1, ?2, NULL, 's', '/tmp/worktree', 't', 'v', 10, 20, NULL)",
            params!["s1", "p1"],
        )
        .expect("session");

        conn.execute_batch(
            r#"
            CREATE TABLE message (
              id TEXT PRIMARY KEY,
              session_id TEXT NOT NULL,
              time_created INTEGER NOT NULL,
              time_updated INTEGER NOT NULL,
              data TEXT NOT NULL
            );
            CREATE TABLE part (
              id TEXT PRIMARY KEY,
              message_id TEXT NOT NULL,
              session_id TEXT NOT NULL,
              time_created INTEGER NOT NULL,
              time_updated INTEGER NOT NULL,
              data TEXT NOT NULL
            );
        "#,
        )
        .expect("message schema");

        let user_data = serde_json::json!({
            "role": "user",
            "time": { "created": 1000 },
            "model": { "providerID": "opencode", "modelID": "m" }
        });
        conn.execute(
            "INSERT INTO message (id, session_id, time_created, time_updated, data) VALUES (?1, ?2, 1000, 1000, ?3)",
            params!["m1", "s1", user_data.to_string()],
        )
        .expect("user msg");

        let assistant_data = serde_json::json!({
            "role": "assistant",
            "time": { "created": 1100, "completed": 1200 },
            "parentID": "m1",
            "providerID": "opencode",
            "modelID": "m",
            "cost": 0,
            "tokens": { "total": 10, "input": 1, "output": 2, "reasoning": 0, "cache": { "read": 3, "write": 4 } }
        });
        conn.execute(
            "INSERT INTO message (id, session_id, time_created, time_updated, data) VALUES (?1, ?2, 1100, 1100, ?3)",
            params!["m2", "s1", assistant_data.to_string()],
        )
        .expect("assistant msg");

        let user_part = serde_json::json!({ "type": "text", "text": "hello" });
        conn.execute(
            "INSERT INTO part (id, message_id, session_id, time_created, time_updated, data) VALUES (?1, ?2, ?3, 1000, 1000, ?4)",
            params!["p1", "m1", "s1", user_part.to_string()],
        )
        .expect("user part");

        let reasoning_part = serde_json::json!({
            "type": "reasoning",
            "text": "think",
            "time": { "start": 1100, "end": 1110 }
        });
        conn.execute(
            "INSERT INTO part (id, message_id, session_id, time_created, time_updated, data) VALUES (?1, ?2, ?3, 1100, 1100, ?4)",
            params!["p2", "m2", "s1", reasoning_part.to_string()],
        )
        .expect("reasoning part");

        let tool_part = serde_json::json!({
            "type": "tool",
            "callID": "c1",
            "tool": "exec_command",
            "state": {
                "status": "completed",
                "input": { "cmd": "ls" },
                "output": "ok",
                "title": "",
                "metadata": {},
                "time": { "start": 1120, "end": 1130 }
            }
        });
        conn.execute(
            "INSERT INTO part (id, message_id, session_id, time_created, time_updated, data) VALUES (?1, ?2, ?3, 1120, 1120, ?4)",
            params!["p3", "m2", "s1", tool_part.to_string()],
        )
        .expect("tool part");

        let text_part = serde_json::json!({ "type": "text", "text": "done" });
        conn.execute(
            "INSERT INTO part (id, message_id, session_id, time_created, time_updated, data) VALUES (?1, ?2, ?3, 1140, 1140, ?4)",
            params!["p4", "m2", "s1", text_part.to_string()],
        )
        .expect("text part");

        let state_dir = dir.path().join("state");
        fs::create_dir_all(&state_dir).expect("state");
        let cache_path = opencode_session_cache_path(&state_dir, "s1");

        ensure_opencode_session_cache(&db_path, &cache_path, "s1").expect("ensure");
        let timeline = crate::infra::load_session_timeline(&cache_path).expect("timeline");

        let kinds = timeline
            .items
            .iter()
            .map(|item| item.kind)
            .collect::<Vec<_>>();
        assert!(kinds.contains(&crate::domain::TimelineItemKind::Turn));
        assert!(kinds.contains(&crate::domain::TimelineItemKind::User));
        assert!(kinds.contains(&crate::domain::TimelineItemKind::Thinking));
        assert!(kinds.contains(&crate::domain::TimelineItemKind::ToolCall));
        assert!(kinds.contains(&crate::domain::TimelineItemKind::ToolOutput));
        assert!(kinds.contains(&crate::domain::TimelineItemKind::Assistant));
        assert!(kinds.contains(&crate::domain::TimelineItemKind::TokenCount));
    }
}
