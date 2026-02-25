use crate::domain::ForkCut;
use serde_json::Value;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForkedSession {
    pub session_id: String,
    pub log_path: PathBuf,
}

#[derive(Debug, Error)]
pub enum ForkSessionError {
    #[error("failed to open parent session log: {0}")]
    OpenParent(#[from] io::Error),

    #[error("parent session log is empty")]
    EmptyParent,

    #[error("failed to parse parent session_meta json: {0}")]
    ParseParentMeta(#[from] serde_json::Error),

    #[error("parent session_meta is missing field: {0}")]
    ParentMetaMissing(String),

    #[error("parent session_meta has unexpected type")]
    ParentMetaUnexpectedType,

    #[error("failed to format timestamp: {0}")]
    FormatTimestamp(String),

    #[error("failed to create fork log directory: {0}")]
    CreateForkDir(io::Error),

    #[error("failed to create fork log file: {0}")]
    CreateForkFile(io::Error),

    #[error("failed to write fork log: {0}")]
    WriteFork(io::Error),

    #[error("fork cut line is out of range: {line_no}")]
    CutOutOfRange { line_no: u64 },
}

pub fn fork_codex_session_log_at_cut(
    sessions_dir: &Path,
    parent_log_path: &Path,
    cut: ForkCut,
) -> Result<ForkedSession, ForkSessionError> {
    let now_utc = OffsetDateTime::now_utc();
    let now_rfc3339 = now_utc
        .format(&Rfc3339)
        .map_err(|error| ForkSessionError::FormatTimestamp(error.to_string()))?;

    let parent = File::open(parent_log_path)?;
    let mut reader = BufReader::new(parent);

    let mut meta_line = String::new();
    let bytes = reader
        .read_line(&mut meta_line)
        .map_err(ForkSessionError::OpenParent)?;
    if bytes == 0 {
        return Err(ForkSessionError::EmptyParent);
    }

    let mut meta: Value = serde_json::from_str(meta_line.trim_end())?;
    if meta.get("type").and_then(|value| value.as_str()) != Some("session_meta") {
        return Err(ForkSessionError::ParentMetaUnexpectedType);
    }

    let payload = meta
        .get_mut("payload")
        .ok_or_else(|| ForkSessionError::ParentMetaMissing("payload".to_string()))?;
    if !payload.is_object() {
        return Err(ForkSessionError::ParentMetaMissing("payload".to_string()));
    }
    let cwd = payload
        .get("cwd")
        .and_then(|value| value.as_str())
        .ok_or_else(|| ForkSessionError::ParentMetaMissing("payload.cwd".to_string()))?
        .to_string();

    let session_id = Uuid::now_v7().to_string();
    set_json_string(&mut meta, &["timestamp"], &now_rfc3339)?;
    set_json_string(&mut meta, &["payload", "id"], &session_id)?;
    set_json_string(&mut meta, &["payload", "timestamp"], &now_rfc3339)?;
    set_json_string(&mut meta, &["payload", "cwd"], &cwd)?;

    let (year, month, day, file_stamp) = local_file_timestamp_parts(now_utc);
    let day_dir = sessions_dir
        .join(format!("{year:04}"))
        .join(format!("{month:02}"))
        .join(format!("{day:02}"));
    fs::create_dir_all(&day_dir).map_err(ForkSessionError::CreateForkDir)?;

    let log_path = day_dir.join(format!("rollout-{file_stamp}-{session_id}.jsonl"));
    let fork_file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&log_path)
        .map_err(ForkSessionError::CreateForkFile)?;

    let mut writer = BufWriter::new(fork_file);
    let meta_out = serde_json::to_string(&meta).map_err(ForkSessionError::ParseParentMeta)?;
    writer
        .write_all(meta_out.as_bytes())
        .and_then(|()| writer.write_all(b"\n"))
        .map_err(ForkSessionError::WriteFork)?;

    copy_parent_prefix(&mut reader, &mut writer, cut)?;
    writer.flush().map_err(ForkSessionError::WriteFork)?;

    Ok(ForkedSession {
        session_id,
        log_path,
    })
}

fn set_json_string(value: &mut Value, path: &[&str], text: &str) -> Result<(), ForkSessionError> {
    let Some((last, prefix)) = path.split_last() else {
        return Err(ForkSessionError::ParentMetaMissing("path".to_string()));
    };

    let mut cursor = value;
    for key in prefix {
        cursor = cursor
            .get_mut(*key)
            .ok_or_else(|| ForkSessionError::ParentMetaMissing((*key).to_string()))?;
    }

    let object = cursor
        .as_object_mut()
        .ok_or_else(|| ForkSessionError::ParentMetaMissing((*last).to_string()))?;
    object.insert((*last).to_string(), Value::String(text.to_string()));
    Ok(())
}

fn copy_parent_prefix(
    reader: &mut dyn BufRead,
    writer: &mut dyn Write,
    cut: ForkCut,
) -> Result<(), ForkSessionError> {
    let target_line_no = match cut {
        ForkCut::BeforeLine { line_no } | ForkCut::AfterLine { line_no } => line_no,
    };
    if matches!(cut, ForkCut::BeforeLine { .. }) && target_line_no <= 2 {
        return Ok(());
    }

    let mut current_line_no: u64 = 1;
    let mut line = String::new();
    let mut reached = target_line_no <= 1;

    while !reached {
        line.clear();
        let bytes = reader
            .read_line(&mut line)
            .map_err(ForkSessionError::OpenParent)?;
        if bytes == 0 {
            break;
        }
        current_line_no = current_line_no.saturating_add(1);

        match cut {
            ForkCut::AfterLine { line_no } => {
                if current_line_no > line_no {
                    reached = true;
                    break;
                }
                writer
                    .write_all(line.as_bytes())
                    .map_err(ForkSessionError::WriteFork)?;
                if current_line_no == line_no {
                    reached = true;
                    break;
                }
            }
            ForkCut::BeforeLine { line_no } => {
                if current_line_no >= line_no {
                    reached = true;
                    break;
                }
                writer
                    .write_all(line.as_bytes())
                    .map_err(ForkSessionError::WriteFork)?;
            }
        }
    }

    if !reached {
        return Err(ForkSessionError::CutOutOfRange {
            line_no: target_line_no,
        });
    }

    Ok(())
}

fn local_file_timestamp_parts(now_utc: OffsetDateTime) -> (i32, u8, u8, String) {
    #[cfg(unix)]
    {
        if let Some(parts) = unix_local_parts(now_utc) {
            let stamp = format!(
                "{:04}-{:02}-{:02}T{:02}-{:02}-{:02}",
                parts.year, parts.month, parts.day, parts.hour, parts.minute, parts.second
            );
            return (parts.year, parts.month, parts.day, stamp);
        }
    }

    let year = now_utc.year();
    let month = now_utc.month() as u8;
    let day = now_utc.day();
    let stamp = format!(
        "{:04}-{:02}-{:02}T{:02}-{:02}-{:02}",
        year,
        month,
        day,
        now_utc.hour(),
        now_utc.minute(),
        now_utc.second()
    );
    (year, month, day, stamp)
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug)]
struct UnixLocalParts {
    year: i32,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
}

#[cfg(unix)]
fn unix_local_parts(now_utc: OffsetDateTime) -> Option<UnixLocalParts> {
    use std::mem::MaybeUninit;

    #[cfg(target_pointer_width = "64")]
    let seconds: libc::time_t = now_utc.unix_timestamp();

    #[cfg(target_pointer_width = "32")]
    let seconds: libc::time_t = {
        let unix_seconds = now_utc.unix_timestamp();
        if unix_seconds > i32::MAX as i64 {
            i32::MAX as libc::time_t
        } else if unix_seconds < i32::MIN as i64 {
            i32::MIN as libc::time_t
        } else {
            unix_seconds as libc::time_t
        }
    };

    let mut tm = MaybeUninit::<libc::tm>::uninit();
    let tm_ptr = unsafe { libc::localtime_r(&seconds, tm.as_mut_ptr()) };
    if tm_ptr.is_null() {
        return None;
    }
    let tm = unsafe { tm.assume_init() };

    let year = tm.tm_year + 1900;
    let month = (tm.tm_mon + 1).try_into().ok()?;
    let day = tm.tm_mday.try_into().ok()?;
    let hour = tm.tm_hour.try_into().ok()?;
    let minute = tm.tm_min.try_into().ok()?;
    let second = tm.tm_sec.try_into().ok()?;

    Some(UnixLocalParts {
        year,
        month,
        day,
        hour,
        minute,
        second,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::load_session_timeline;
    use std::fs;
    use tempfile::tempdir;

    fn write_lines(path: &Path, lines: &[Value]) {
        let body = lines
            .iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(path, body).expect("write");
    }

    #[test]
    fn forks_after_line_inclusive() {
        let dir = tempdir().expect("tempdir");
        let sessions_dir = dir.path().join("sessions");
        let parent_path = dir.path().join("parent.jsonl");

        let parent_lines = vec![
            serde_json::json!({
                "timestamp": "2026-02-18T21:45:57.803Z",
                "type": "session_meta",
                "payload": {
                    "id": "parent",
                    "timestamp": "2026-02-18T21:45:57.803Z",
                    "cwd": "/tmp/project"
                }
            }),
            serde_json::json!({
                "timestamp": "2026-02-18T21:45:58.000Z",
                "type": "turn_context",
                "payload": { "turn_id": "t1", "cwd": "/tmp/project" }
            }),
            serde_json::json!({
                "timestamp": "2026-02-18T21:45:59.000Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "hello" }]
                }
            }),
            serde_json::json!({
                "timestamp": "2026-02-18T21:46:00.000Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": "ok" }]
                }
            }),
        ];
        write_lines(&parent_path, &parent_lines);

        let forked = fork_codex_session_log_at_cut(
            &sessions_dir,
            &parent_path,
            ForkCut::AfterLine { line_no: 3 },
        )
        .expect("fork");

        let content = fs::read_to_string(&forked.log_path).expect("read fork");
        let fork_lines = content.lines().collect::<Vec<_>>();
        assert_eq!(fork_lines.len(), 3);

        let meta: Value = serde_json::from_str(fork_lines[0]).expect("meta");
        assert_eq!(
            meta.get("type").and_then(|v| v.as_str()),
            Some("session_meta")
        );
        assert_eq!(
            meta.get("payload")
                .and_then(|v| v.get("cwd"))
                .and_then(|v| v.as_str()),
            Some("/tmp/project")
        );
        assert_ne!(
            meta.get("payload")
                .and_then(|v| v.get("id"))
                .and_then(|v| v.as_str()),
            Some("parent")
        );

        assert_eq!(fork_lines[1], parent_lines[1].to_string());
        assert_eq!(fork_lines[2], parent_lines[2].to_string());

        let timeline = load_session_timeline(&forked.log_path).expect("timeline");
        assert!(
            timeline
                .items
                .iter()
                .any(|item| item.kind == crate::domain::TimelineItemKind::User)
        );
    }

    #[test]
    fn forks_before_line_exclusive() {
        let dir = tempdir().expect("tempdir");
        let sessions_dir = dir.path().join("sessions");
        let parent_path = dir.path().join("parent.jsonl");

        let parent_lines = vec![
            serde_json::json!({
                "timestamp": "2026-02-18T21:45:57.803Z",
                "type": "session_meta",
                "payload": {
                    "id": "parent",
                    "timestamp": "2026-02-18T21:45:57.803Z",
                    "cwd": "/tmp/project"
                }
            }),
            serde_json::json!({ "type": "turn_context", "payload": { "turn_id": "t1" } }),
            serde_json::json!({ "type": "event_msg", "payload": { "type": "token_count", "info": { "total_token_usage": { "total_tokens": 1 } } } }),
        ];
        write_lines(&parent_path, &parent_lines);

        let forked = fork_codex_session_log_at_cut(
            &sessions_dir,
            &parent_path,
            ForkCut::BeforeLine { line_no: 3 },
        )
        .expect("fork");

        let content = fs::read_to_string(&forked.log_path).expect("read fork");
        let fork_lines = content.lines().collect::<Vec<_>>();
        assert_eq!(fork_lines.len(), 2);
        assert_eq!(fork_lines[1], parent_lines[1].to_string());
    }

    #[test]
    fn errors_on_out_of_range_cut() {
        let dir = tempdir().expect("tempdir");
        let sessions_dir = dir.path().join("sessions");
        let parent_path = dir.path().join("parent.jsonl");

        let parent_lines = vec![
            serde_json::json!({
                "timestamp": "2026-02-18T21:45:57.803Z",
                "type": "session_meta",
                "payload": {
                    "id": "parent",
                    "timestamp": "2026-02-18T21:45:57.803Z",
                    "cwd": "/tmp/project"
                }
            }),
            serde_json::json!({ "type": "turn_context", "payload": { "turn_id": "t1" } }),
        ];
        write_lines(&parent_path, &parent_lines);

        let error = fork_codex_session_log_at_cut(
            &sessions_dir,
            &parent_path,
            ForkCut::AfterLine { line_no: 5 },
        )
        .expect_err("error");

        assert!(matches!(
            error,
            ForkSessionError::CutOutOfRange { line_no: 5 }
        ));
    }
}
