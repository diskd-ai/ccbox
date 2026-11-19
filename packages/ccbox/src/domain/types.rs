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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionEngine {
    Codex,
    Claude,
    Gemini,
    OpenCode,
}

impl SessionEngine {
    pub fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude",
            Self::Gemini => "Gemini",
            Self::OpenCode => "OpenCode",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpawnIoMode {
    Pipes,
    Tty,
}

impl SpawnIoMode {
    pub fn toggle(self) -> Self {
        match self {
            Self::Pipes => Self::Tty,
            Self::Tty => Self::Pipes,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Pipes => "Pipes",
            Self::Tty => "TTY",
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
    pub engine: SessionEngine,
    pub meta: SessionMeta,
    pub log_path: PathBuf,
    pub title: String,
    pub file_size_bytes: u64,
    pub file_modified: Option<SystemTime>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionSource {
    FileLog { path: PathBuf },
    ProviderNative { key: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionKey {
    pub provider: SessionEngine,
    pub id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionRecord {
    pub key: SessionKey,
    pub source: SessionSource,
    pub project_cwd: PathBuf,
    pub started_at_rfc3339: String,
    pub title: String,
    pub size_bytes: u64,
    pub modified_at: Option<SystemTime>,
}

impl SessionRecord {
    pub fn from_summary(summary: &SessionSummary) -> Self {
        let source = match summary.engine {
            SessionEngine::OpenCode => SessionSource::ProviderNative {
                key: summary.meta.id.clone(),
            },
            SessionEngine::Codex | SessionEngine::Claude | SessionEngine::Gemini => {
                SessionSource::FileLog {
                    path: summary.log_path.clone(),
                }
            }
        };

        Self {
            key: SessionKey {
                provider: summary.engine,
                id: summary.meta.id.clone(),
            },
            source,
            project_cwd: summary.meta.cwd.clone(),
            started_at_rfc3339: summary.meta.started_at_rfc3339.clone(),
            title: summary.title.clone(),
            size_bytes: summary.file_size_bytes,
            modified_at: summary.file_modified,
        }
    }
}

pub type ProjectIndex = Vec<ProjectSummary>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectSummary {
    pub name: String,
    pub project_path: PathBuf,
    pub sessions: Vec<SessionSummary>,
    pub last_modified: Option<SystemTime>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForkCut {
    BeforeLine { line_no: u64 },
    AfterLine { line_no: u64 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForkContext {
    pub parent_session_id: String,
    pub parent_log_path: PathBuf,
    pub project_path: PathBuf,
    pub cut: ForkCut,
    pub label: String,
}
