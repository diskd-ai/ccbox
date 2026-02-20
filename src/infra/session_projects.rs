use crate::domain::{SessionEngine, SessionSummary};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Clone, Debug, Default)]
pub struct SessionProjects {
    projects: BTreeMap<String, String>,
}

impl SessionProjects {
    pub fn project_for(&self, engine: SessionEngine, session_id: &str) -> Option<&str> {
        let key = session_project_key(engine, session_id);
        self.projects.get(&key).map(|s| s.as_str())
    }

    pub fn set(&mut self, engine: SessionEngine, session_id: &str, project_path: &str) {
        let key = session_project_key(engine, session_id);
        let project_path = project_path.trim();
        if project_path.is_empty() {
            self.projects.remove(&key);
        } else {
            self.projects.insert(key, project_path.to_string());
        }
    }
}

#[derive(Debug, Error)]
pub enum LoadSessionProjectsError {
    #[error("failed to read session projects: {0}")]
    Read(#[from] io::Error),

    #[error("failed to parse session projects: {0}")]
    Parse(#[from] serde_json::Error),
}

#[derive(Debug, Error)]
pub enum SaveSessionProjectsError {
    #[error("failed to encode session projects: {0}")]
    Encode(#[from] serde_json::Error),

    #[error("failed to write session projects: {0}")]
    Write(#[from] io::Error),
}

#[derive(Debug, Error)]
pub enum SetSessionProjectError {
    #[error(transparent)]
    Load(#[from] LoadSessionProjectsError),

    #[error(transparent)]
    Save(#[from] SaveSessionProjectsError),
}

fn session_projects_path(state_dir: &Path) -> PathBuf {
    state_dir.join("session_projects.json")
}

pub fn load_session_projects(
    state_dir: &Path,
) -> Result<SessionProjects, LoadSessionProjectsError> {
    let path = session_projects_path(state_dir);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(SessionProjects::default());
        }
        Err(error) => return Err(error.into()),
    };

    let file: SessionProjectsFile = serde_json::from_str(&raw)?;
    Ok(SessionProjects {
        projects: file.projects,
    })
}

pub fn save_session_projects(
    state_dir: &Path,
    projects: &SessionProjects,
) -> Result<(), SaveSessionProjectsError> {
    fs::create_dir_all(state_dir)?;

    let path = session_projects_path(state_dir);
    let tmp = path.with_extension("json.tmp");
    let file = SessionProjectsFile {
        version: 1,
        projects: projects.projects.clone(),
    };
    let text = serde_json::to_string_pretty(&file)?;
    fs::write(&tmp, text)?;
    fs::rename(tmp, path)?;
    Ok(())
}

pub fn set_session_project(
    state_dir: &Path,
    engine: SessionEngine,
    session_id: &str,
    project_path: &str,
) -> Result<(), SetSessionProjectError> {
    let mut projects = load_session_projects(state_dir)?;
    projects.set(engine, session_id, project_path);
    save_session_projects(state_dir, &projects)?;
    Ok(())
}

pub fn apply_session_projects(sessions: &mut [SessionSummary], projects: &SessionProjects) {
    for session in sessions {
        if let Some(path) = projects.project_for(session.engine, &session.meta.id) {
            let trimmed = path.trim();
            if trimmed.is_empty() {
                continue;
            }
            session.meta.cwd = PathBuf::from(trimmed);
        }
    }
}

pub fn session_project_key(engine: SessionEngine, session_id: &str) -> String {
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
struct SessionProjectsFile {
    version: u32,
    projects: BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{SessionMeta, make_session_summary};
    use std::time::SystemTime;
    use tempfile::tempdir;

    #[test]
    fn round_trips_projects_and_applies_cwd() {
        let dir = tempdir().expect("tempdir");
        let state = dir.path();

        set_session_project(state, SessionEngine::Codex, "s1", "/tmp/next").expect("set");

        let loaded = load_session_projects(state).expect("load");
        assert_eq!(
            loaded.project_for(SessionEngine::Codex, "s1"),
            Some("/tmp/next")
        );

        let mut sessions = vec![make_session_summary(
            SessionMeta {
                id: "s1".to_string(),
                cwd: PathBuf::from("/tmp/old"),
                started_at_rfc3339: "2026-02-20T00:00:00Z".to_string(),
            },
            PathBuf::from("/tmp/log.jsonl"),
            "auto".to_string(),
            0,
            Some(SystemTime::now()),
            SessionEngine::Codex,
        )];

        apply_session_projects(&mut sessions, &loaded);
        assert_eq!(sessions[0].meta.cwd, PathBuf::from("/tmp/next"));
    }

    #[test]
    fn empty_path_clears_override() {
        let dir = tempdir().expect("tempdir");
        let state = dir.path();

        set_session_project(state, SessionEngine::Codex, "s1", "/tmp/next").expect("set");
        set_session_project(state, SessionEngine::Codex, "s1", "").expect("clear");

        let loaded = load_session_projects(state).expect("load");
        assert_eq!(loaded.project_for(SessionEngine::Codex, "s1"), None);
    }
}
