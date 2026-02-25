use crate::domain::{SessionEngine, SessionSummary};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Clone, Debug, Default)]
pub struct SessionAliases {
    aliases: BTreeMap<String, String>,
}

impl SessionAliases {
    pub fn title_for(&self, engine: SessionEngine, session_id: &str) -> Option<&str> {
        let key = session_alias_key(engine, session_id);
        self.aliases.get(&key).map(|s| s.as_str())
    }

    pub fn set(&mut self, engine: SessionEngine, session_id: &str, title: &str) {
        let key = session_alias_key(engine, session_id);
        let title = title.trim();
        if title.is_empty() {
            self.aliases.remove(&key);
        } else {
            self.aliases.insert(key, title.to_string());
        }
    }
}

#[derive(Debug, Error)]
pub enum LoadSessionAliasesError {
    #[error("failed to read session aliases: {0}")]
    Read(#[from] io::Error),

    #[error("failed to parse session aliases: {0}")]
    Parse(#[from] serde_json::Error),
}

#[derive(Debug, Error)]
pub enum SaveSessionAliasesError {
    #[error("failed to encode session aliases: {0}")]
    Encode(#[from] serde_json::Error),

    #[error("failed to write session aliases: {0}")]
    Write(#[from] io::Error),
}

#[derive(Debug, Error)]
pub enum SetSessionAliasError {
    #[error(transparent)]
    Load(#[from] LoadSessionAliasesError),

    #[error(transparent)]
    Save(#[from] SaveSessionAliasesError),
}

fn session_aliases_path(state_dir: &Path) -> PathBuf {
    state_dir.join("session_aliases.json")
}

pub fn load_session_aliases(state_dir: &Path) -> Result<SessionAliases, LoadSessionAliasesError> {
    let path = session_aliases_path(state_dir);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(SessionAliases::default());
        }
        Err(error) => return Err(error.into()),
    };

    let file: SessionAliasesFile = serde_json::from_str(&raw)?;
    Ok(SessionAliases {
        aliases: file.aliases,
    })
}

pub fn save_session_aliases(
    state_dir: &Path,
    aliases: &SessionAliases,
) -> Result<(), SaveSessionAliasesError> {
    fs::create_dir_all(state_dir)?;

    let path = session_aliases_path(state_dir);
    let tmp = path.with_extension("json.tmp");
    let file = SessionAliasesFile {
        version: 1,
        aliases: aliases.aliases.clone(),
    };
    let text = serde_json::to_string_pretty(&file)?;
    fs::write(&tmp, text)?;
    fs::rename(tmp, path)?;
    Ok(())
}

pub fn set_session_alias(
    state_dir: &Path,
    engine: SessionEngine,
    session_id: &str,
    title: &str,
) -> Result<(), SetSessionAliasError> {
    let mut aliases = load_session_aliases(state_dir)?;
    aliases.set(engine, session_id, title);
    save_session_aliases(state_dir, &aliases)?;
    Ok(())
}

pub fn apply_session_aliases(sessions: &mut [SessionSummary], aliases: &SessionAliases) {
    for session in sessions {
        if let Some(title) = aliases.title_for(session.engine, &session.meta.id) {
            if !title.trim().is_empty() {
                session.title = title.to_string();
            }
        }
    }
}

pub fn session_alias_key(engine: SessionEngine, session_id: &str) -> String {
    format!("{}:{}", engine_prefix(engine), session_id)
}

fn engine_prefix(engine: SessionEngine) -> &'static str {
    match engine {
        SessionEngine::Codex => "codex",
        SessionEngine::Claude => "claude",
        SessionEngine::Gemini => "gemini",
        SessionEngine::OpenCode => "opencode",
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SessionAliasesFile {
    version: u32,
    aliases: BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{SessionMeta, make_session_summary};
    use std::time::SystemTime;
    use tempfile::tempdir;

    #[test]
    fn round_trips_aliases_and_applies_titles() {
        let dir = tempdir().expect("tempdir");
        let state = dir.path();

        set_session_alias(state, SessionEngine::Codex, "s1", "My title").expect("set");

        let loaded = load_session_aliases(state).expect("load");
        assert_eq!(
            loaded.title_for(SessionEngine::Codex, "s1"),
            Some("My title")
        );

        let mut sessions = vec![make_session_summary(
            SessionMeta {
                id: "s1".to_string(),
                cwd: PathBuf::from("/tmp/p"),
                started_at_rfc3339: "2026-02-20T00:00:00Z".to_string(),
            },
            PathBuf::from("/tmp/log.jsonl"),
            "auto".to_string(),
            0,
            Some(SystemTime::now()),
            SessionEngine::Codex,
        )];

        apply_session_aliases(&mut sessions, &loaded);
        assert_eq!(sessions[0].title, "My title");
    }

    #[test]
    fn empty_title_clears_alias() {
        let dir = tempdir().expect("tempdir");
        let state = dir.path();

        set_session_alias(state, SessionEngine::Codex, "s1", "t").expect("set");
        set_session_alias(state, SessionEngine::Codex, "s1", "").expect("clear");

        let loaded = load_session_aliases(state).expect("load");
        assert_eq!(loaded.title_for(SessionEngine::Codex, "s1"), None);
    }
}
