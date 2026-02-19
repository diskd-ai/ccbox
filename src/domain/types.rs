use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentEngine {
    Codex,
    Claude,
}

impl AgentEngine {
    pub fn toggle(self) -> Self {
        match self {
            Self::Codex => Self::Claude,
            Self::Claude => Self::Codex,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionMeta {
    pub id: String,
    pub cwd: PathBuf,
    pub started_at_rfc3339: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionSummary {
    pub meta: SessionMeta,
    pub log_path: PathBuf,
    pub title: String,
    pub file_size_bytes: u64,
    pub file_modified: Option<SystemTime>,
}

pub type ProjectIndex = Vec<ProjectSummary>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectSummary {
    pub name: String,
    pub project_path: PathBuf,
    pub sessions: Vec<SessionSummary>,
    pub last_modified: Option<SystemTime>,
}
