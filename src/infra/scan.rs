use crate::domain::{
    SessionSummary, derive_title_from_user_text, is_metadata_prompt, make_session_summary,
    parse_session_meta_line, parse_user_message_text,
};
use dirs::home_dir;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use thiserror::Error;
use walkdir::WalkDir;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScanWarningCount(usize);

impl From<usize> for ScanWarningCount {
    fn from(value: usize) -> Self {
        Self(value)
    }
}

impl ScanWarningCount {
    pub fn get(&self) -> usize {
        self.0
    }
}

#[derive(Debug, Error)]
pub enum ScanError {
    #[error("sessions directory does not exist: {0}")]
    SessionsDirMissing(String),

    #[error("failed to read session file: {0}")]
    ReadFile(String),
}

#[derive(Debug, Error)]
pub enum ResolveSessionsDirError {
    #[error("home directory not found")]
    HomeDirNotFound,
}

pub fn resolve_sessions_dir() -> Result<PathBuf, ResolveSessionsDirError> {
    if let Some(override_dir) = std::env::var_os("CODEX_SESSIONS_DIR") {
        return Ok(PathBuf::from(override_dir));
    }

    let Some(home) = home_dir() else {
        return Err(ResolveSessionsDirError::HomeDirNotFound);
    };

    Ok(home.join(".codex").join("sessions"))
}

#[derive(Clone, Debug)]
pub struct ScanOutput {
    pub sessions: Vec<SessionSummary>,
    pub warnings: ScanWarningCount,
}

pub fn scan_sessions_dir(sessions_dir: &Path) -> Result<ScanOutput, ScanError> {
    if !sessions_dir.exists() {
        return Err(ScanError::SessionsDirMissing(
            sessions_dir.display().to_string(),
        ));
    }

    let mut warnings = 0usize;
    let mut sessions: Vec<SessionSummary> = Vec::new();

    let walker = WalkDir::new(sessions_dir).follow_links(false).into_iter();
    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_error) => {
                warnings += 1;
                continue;
            }
        };

        if !entry.file_type().is_file() {
            continue;
        }
        if entry.path().extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }

        match scan_session_file(entry.path()) {
            Ok(summary) => sessions.push(summary),
            Err(_) => warnings += 1,
        }
    }

    Ok(ScanOutput {
        sessions,
        warnings: ScanWarningCount::from(warnings),
    })
}

const MAX_TITLE_SCAN_LINES: usize = 250;

fn scan_session_file(path: &Path) -> Result<SessionSummary, ScanError> {
    let file = File::open(path)
        .map_err(|error| ScanError::ReadFile(format!("{}: {error}", path.display())))?;
    let metadata = file
        .metadata()
        .map_err(|error| ScanError::ReadFile(format!("{}: {error}", path.display())))?;
    let file_size_bytes = metadata.len();
    let file_modified = metadata.modified().ok();

    let mut reader = BufReader::new(file);
    let mut first_line = String::new();
    let bytes = reader
        .read_line(&mut first_line)
        .map_err(|error| ScanError::ReadFile(format!("{}: {error}", path.display())))?;
    if bytes == 0 {
        return Err(ScanError::ReadFile(format!(
            "{}: empty file",
            path.display()
        )));
    }

    let meta = parse_session_meta_line(first_line.trim_end()).map_err(|error| {
        ScanError::ReadFile(format!(
            "{}: failed to parse session_meta: {error}",
            path.display()
        ))
    })?;

    let mut title: Option<String> = None;
    for _ in 0..MAX_TITLE_SCAN_LINES {
        let mut line = String::new();
        let bytes = reader
            .read_line(&mut line)
            .map_err(|error| ScanError::ReadFile(format!("{}: {error}", path.display())))?;
        if bytes == 0 {
            break;
        }
        let Ok(Some(text)) = parse_user_message_text(line.trim_end()) else {
            continue;
        };
        if is_metadata_prompt(&text) {
            continue;
        }
        if let Some(candidate) = derive_title_from_user_text(&text) {
            title = Some(candidate);
            break;
        }
    }

    let display_title = title.unwrap_or_else(|| "(untitled)".to_string());

    Ok(make_session_summary(
        meta,
        path.to_path_buf(),
        display_title,
        file_size_bytes,
        file_modified,
    ))
}
