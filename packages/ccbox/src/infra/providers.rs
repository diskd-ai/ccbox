use crate::domain::{SessionEngine, SessionRecord, SessionSource, SessionSummary, SessionTimeline};
use crate::infra::{
    ArchiveOpenCodeSessionsError, DeleteOutcome, LastAssistantOutput, ScanError,
    archive_opencode_sessions, delete_session_logs_with_roots, load_claude_last_assistant_output,
    load_claude_session_timeline, load_gemini_last_assistant_output, load_gemini_session_timeline,
    load_last_assistant_output, load_session_timeline, prepare_session_log_path,
    resolve_ccbox_state_dir, resolve_claude_projects_dir, resolve_gemini_root_dir,
    resolve_opencode_db_path, scan_claude_projects_dir, scan_gemini_root_dir, scan_opencode_db,
    scan_sessions_dir,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Clone, Debug, Default)]
pub struct ProviderScanAggregate {
    pub sessions: Vec<SessionSummary>,
    pub warnings: usize,
    pub notices: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ProviderScanResult {
    pub sessions: Vec<SessionSummary>,
    pub warnings: usize,
    pub notice: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ProviderDeleteOutcome {
    pub deleted: usize,
    pub archived: usize,
    pub failed: usize,
    pub skipped_outside_allowed_roots: usize,
    pub missing: usize,
}

impl ProviderDeleteOutcome {
    pub fn merge_in(&mut self, other: &Self) {
        self.deleted = self.deleted.saturating_add(other.deleted);
        self.archived = self.archived.saturating_add(other.archived);
        self.failed = self.failed.saturating_add(other.failed);
        self.skipped_outside_allowed_roots = self
            .skipped_outside_allowed_roots
            .saturating_add(other.skipped_outside_allowed_roots);
        self.missing = self.missing.saturating_add(other.missing);
    }

    pub fn total_removed(&self) -> usize {
        self.deleted.saturating_add(self.archived)
    }
}

#[derive(Clone, Debug, Default)]
pub struct ProviderDeleteReport {
    pub outcome: ProviderDeleteOutcome,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderWatchTargetKind {
    Directory,
    SqliteFamily,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderWatchTarget {
    pub provider: SessionEngine,
    pub kind: ProviderWatchTargetKind,
    pub path: PathBuf,
}

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("{0}")]
    Message(String),
}

pub trait SessionProvider: Send + Sync {
    fn id(&self) -> SessionEngine;
    fn scan(&self) -> ProviderScanResult;
    fn load_timeline(&self, session: &SessionSummary) -> Result<SessionTimeline, ProviderError>;
    fn load_last_output(
        &self,
        session: &SessionSummary,
    ) -> Result<LastAssistantOutput, ProviderError>;
    fn delete_sessions(
        &self,
        sessions: &[SessionSummary],
    ) -> Result<ProviderDeleteOutcome, ProviderError>;
    fn watch_targets(&self) -> Vec<ProviderWatchTarget>;
}

pub struct ProviderRegistry {
    providers: Vec<Box<dyn SessionProvider>>,
}

impl ProviderRegistry {
    pub fn new_default(codex_sessions_dir: &Path) -> Self {
        let providers: Vec<Box<dyn SessionProvider>> = vec![
            Box::new(CodexProvider::new(codex_sessions_dir)),
            Box::new(ClaudeProvider),
            Box::new(GeminiProvider),
            Box::new(OpenCodeProvider),
        ];
        Self { providers }
    }

    pub fn scan_all(&self) -> ProviderScanAggregate {
        let mut aggregate = ProviderScanAggregate::default();
        for provider in &self.providers {
            let result = provider.scan();
            aggregate.warnings = aggregate.warnings.saturating_add(result.warnings);
            aggregate.sessions.extend(result.sessions);
            if let Some(notice) = result.notice {
                let trimmed = notice.trim();
                if !trimmed.is_empty() {
                    aggregate.notices.push(trimmed.to_string());
                }
            }
        }
        aggregate
    }

    pub fn load_timeline(
        &self,
        session: &SessionSummary,
    ) -> Result<SessionTimeline, ProviderError> {
        let provider = self.provider_for_engine(session.engine)?;
        provider.load_timeline(session)
    }

    pub fn load_last_output(
        &self,
        session: &SessionSummary,
    ) -> Result<LastAssistantOutput, ProviderError> {
        let provider = self.provider_for_engine(session.engine)?;
        provider.load_last_output(session)
    }

    pub fn delete_sessions(&self, sessions: &[SessionSummary]) -> ProviderDeleteReport {
        let mut report = ProviderDeleteReport::default();

        for provider in &self.providers {
            let engine = provider.id();
            let engine_sessions = sessions
                .iter()
                .filter(|session| session.engine == engine)
                .cloned()
                .collect::<Vec<_>>();
            if engine_sessions.is_empty() {
                continue;
            }

            match provider.delete_sessions(&engine_sessions) {
                Ok(outcome) => report.outcome.merge_in(&outcome),
                Err(error) => {
                    report.outcome.failed =
                        report.outcome.failed.saturating_add(engine_sessions.len());
                    report
                        .errors
                        .push(format!("{} delete failed: {}", engine.label(), error));
                }
            }
        }

        report
    }

    pub fn watch_targets(&self) -> Vec<ProviderWatchTarget> {
        let mut targets = Vec::new();
        for provider in &self.providers {
            targets.extend(provider.watch_targets());
        }
        targets
    }

    fn provider_for_engine(
        &self,
        engine: SessionEngine,
    ) -> Result<&dyn SessionProvider, ProviderError> {
        self.providers
            .iter()
            .find(|provider| provider.id() == engine)
            .map(|provider| provider.as_ref())
            .ok_or_else(|| ProviderError::Message(format!("Unknown provider: {}", engine.label())))
    }
}

pub fn scan_all_with_providers(codex_sessions_dir: &Path) -> ProviderScanAggregate {
    ProviderRegistry::new_default(codex_sessions_dir).scan_all()
}

pub fn load_timeline_for_session(
    codex_sessions_dir: &Path,
    session: &SessionSummary,
) -> Result<SessionTimeline, ProviderError> {
    ProviderRegistry::new_default(codex_sessions_dir).load_timeline(session)
}

pub fn load_last_output_for_session(
    codex_sessions_dir: &Path,
    session: &SessionSummary,
) -> Result<LastAssistantOutput, ProviderError> {
    ProviderRegistry::new_default(codex_sessions_dir).load_last_output(session)
}

pub fn collect_allowed_file_roots(codex_sessions_dir: &Path) -> Vec<PathBuf> {
    let registry = ProviderRegistry::new_default(codex_sessions_dir);
    let mut roots = registry
        .watch_targets()
        .into_iter()
        .filter_map(|target| match target.kind {
            ProviderWatchTargetKind::Directory => Some(target.path),
            ProviderWatchTargetKind::SqliteFamily => None,
        })
        .collect::<Vec<_>>();

    if let Ok(state_dir) = resolve_ccbox_state_dir() {
        roots.push(state_dir.join("opencode").join("sessions"));
    }

    roots.sort();
    roots.dedup();
    roots
}

pub fn delete_sessions_by_log_paths(
    codex_sessions_dir: &Path,
    all_sessions: &[SessionSummary],
    requested_log_paths: &[PathBuf],
) -> ProviderDeleteReport {
    let mut session_by_log_path: BTreeMap<PathBuf, SessionSummary> = BTreeMap::new();
    for session in all_sessions {
        session_by_log_path.insert(session.log_path.clone(), session.clone());
    }

    let mut selected_sessions: Vec<SessionSummary> = Vec::new();
    let mut selected_keys: BTreeSet<String> = BTreeSet::new();
    let mut unmatched_log_paths: Vec<PathBuf> = Vec::new();

    for log_path in requested_log_paths {
        if let Some(session) = session_by_log_path.get(log_path) {
            let record = SessionRecord::from_summary(session);
            let key = format!("{}:{}", record.key.provider.label(), record.key.id);
            if selected_keys.insert(key) {
                selected_sessions.push(session.clone());
            }

            if let SessionSource::FileLog { path } = record.source
                && path != *log_path
            {
                unmatched_log_paths.push(path);
            }
        } else {
            unmatched_log_paths.push(log_path.clone());
        }
    }

    let registry = ProviderRegistry::new_default(codex_sessions_dir);
    let mut report = registry.delete_sessions(&selected_sessions);

    if !unmatched_log_paths.is_empty() {
        let roots = collect_allowed_file_roots(codex_sessions_dir);
        let file_outcome = delete_session_logs_with_roots(&roots, &unmatched_log_paths);
        report.outcome.merge_in(&ProviderDeleteOutcome {
            deleted: file_outcome.deleted,
            archived: 0,
            failed: file_outcome.failed,
            skipped_outside_allowed_roots: file_outcome.skipped_outside_sessions_dir,
            missing: 0,
        });
    }

    report
}

struct CodexProvider {
    sessions_dir: PathBuf,
}

impl CodexProvider {
    fn new(sessions_dir: &Path) -> Self {
        Self {
            sessions_dir: sessions_dir.to_path_buf(),
        }
    }
}

impl SessionProvider for CodexProvider {
    fn id(&self) -> SessionEngine {
        SessionEngine::Codex
    }

    fn scan(&self) -> ProviderScanResult {
        match scan_sessions_dir(&self.sessions_dir) {
            Ok(output) => ProviderScanResult {
                sessions: output.sessions,
                warnings: output.warnings.get(),
                notice: None,
            },
            Err(ScanError::SessionsDirMissing(path)) => ProviderScanResult {
                sessions: Vec::new(),
                warnings: 0,
                notice: Some(format!("Codex sessions dir not found: {path}")),
            },
            Err(error) => ProviderScanResult {
                sessions: Vec::new(),
                warnings: 1,
                notice: Some(format!("Failed to scan Codex sessions: {error}")),
            },
        }
    }

    fn load_timeline(&self, session: &SessionSummary) -> Result<SessionTimeline, ProviderError> {
        load_session_timeline(&session.log_path).map_err(|error| {
            ProviderError::Message(format!(
                "Failed to load {} timeline: {error}",
                self.id().label()
            ))
        })
    }

    fn load_last_output(
        &self,
        session: &SessionSummary,
    ) -> Result<LastAssistantOutput, ProviderError> {
        load_last_assistant_output(&session.log_path).map_err(|error| {
            ProviderError::Message(format!(
                "Failed to load {} result: {error}",
                self.id().label()
            ))
        })
    }

    fn delete_sessions(
        &self,
        sessions: &[SessionSummary],
    ) -> Result<ProviderDeleteOutcome, ProviderError> {
        let log_paths = sessions
            .iter()
            .map(|session| session.log_path.clone())
            .collect::<Vec<_>>();
        let outcome =
            delete_session_logs_with_roots(std::slice::from_ref(&self.sessions_dir), &log_paths);
        Ok(ProviderDeleteOutcome {
            deleted: outcome.deleted,
            archived: 0,
            failed: outcome.failed,
            skipped_outside_allowed_roots: outcome.skipped_outside_sessions_dir,
            missing: 0,
        })
    }

    fn watch_targets(&self) -> Vec<ProviderWatchTarget> {
        vec![ProviderWatchTarget {
            provider: self.id(),
            kind: ProviderWatchTargetKind::Directory,
            path: self.sessions_dir.clone(),
        }]
    }
}

struct ClaudeProvider;

impl SessionProvider for ClaudeProvider {
    fn id(&self) -> SessionEngine {
        SessionEngine::Claude
    }

    fn scan(&self) -> ProviderScanResult {
        let projects_dir = match resolve_claude_projects_dir() {
            Ok(path) => path,
            Err(error) => {
                return ProviderScanResult {
                    sessions: Vec::new(),
                    warnings: 0,
                    notice: Some(format!("Claude projects dir disabled: {error}")),
                };
            }
        };

        let output = scan_claude_projects_dir(&projects_dir);
        ProviderScanResult {
            sessions: output.sessions,
            warnings: output.warnings.get(),
            notice: output.notice,
        }
    }

    fn load_timeline(&self, session: &SessionSummary) -> Result<SessionTimeline, ProviderError> {
        load_claude_session_timeline(&session.log_path).map_err(|error| {
            ProviderError::Message(format!(
                "Failed to load {} timeline: {error}",
                self.id().label()
            ))
        })
    }

    fn load_last_output(
        &self,
        session: &SessionSummary,
    ) -> Result<LastAssistantOutput, ProviderError> {
        load_claude_last_assistant_output(&session.log_path).map_err(|error| {
            ProviderError::Message(format!(
                "Failed to load {} result: {error}",
                self.id().label()
            ))
        })
    }

    fn delete_sessions(
        &self,
        sessions: &[SessionSummary],
    ) -> Result<ProviderDeleteOutcome, ProviderError> {
        let projects_dir = resolve_claude_projects_dir().map_err(|error| {
            ProviderError::Message(format!("Claude delete unavailable: {error}"))
        })?;
        let log_paths = sessions
            .iter()
            .map(|session| session.log_path.clone())
            .collect::<Vec<_>>();
        let outcome =
            delete_session_logs_with_roots(std::slice::from_ref(&projects_dir), &log_paths);
        Ok(ProviderDeleteOutcome {
            deleted: outcome.deleted,
            archived: 0,
            failed: outcome.failed,
            skipped_outside_allowed_roots: outcome.skipped_outside_sessions_dir,
            missing: 0,
        })
    }

    fn watch_targets(&self) -> Vec<ProviderWatchTarget> {
        match resolve_claude_projects_dir() {
            Ok(path) => vec![ProviderWatchTarget {
                provider: self.id(),
                kind: ProviderWatchTargetKind::Directory,
                path,
            }],
            Err(_) => Vec::new(),
        }
    }
}

struct GeminiProvider;

impl SessionProvider for GeminiProvider {
    fn id(&self) -> SessionEngine {
        SessionEngine::Gemini
    }

    fn scan(&self) -> ProviderScanResult {
        let root_dir = match resolve_gemini_root_dir() {
            Ok(path) => path,
            Err(error) => {
                return ProviderScanResult {
                    sessions: Vec::new(),
                    warnings: 0,
                    notice: Some(format!("Gemini root dir disabled: {error}")),
                };
            }
        };

        let output = scan_gemini_root_dir(&root_dir);
        ProviderScanResult {
            sessions: output.sessions,
            warnings: output.warnings.get(),
            notice: output.notice,
        }
    }

    fn load_timeline(&self, session: &SessionSummary) -> Result<SessionTimeline, ProviderError> {
        load_gemini_session_timeline(&session.log_path).map_err(|error| {
            ProviderError::Message(format!(
                "Failed to load {} timeline: {error}",
                self.id().label()
            ))
        })
    }

    fn load_last_output(
        &self,
        session: &SessionSummary,
    ) -> Result<LastAssistantOutput, ProviderError> {
        load_gemini_last_assistant_output(&session.log_path).map_err(|error| {
            ProviderError::Message(format!(
                "Failed to load {} result: {error}",
                self.id().label()
            ))
        })
    }

    fn delete_sessions(
        &self,
        sessions: &[SessionSummary],
    ) -> Result<ProviderDeleteOutcome, ProviderError> {
        let root_dir = resolve_gemini_root_dir().map_err(|error| {
            ProviderError::Message(format!("Gemini delete unavailable: {error}"))
        })?;
        let log_paths = sessions
            .iter()
            .map(|session| session.log_path.clone())
            .collect::<Vec<_>>();
        let outcome = delete_session_logs_with_roots(&[root_dir.join("tmp")], &log_paths);
        Ok(ProviderDeleteOutcome {
            deleted: outcome.deleted,
            archived: 0,
            failed: outcome.failed,
            skipped_outside_allowed_roots: outcome.skipped_outside_sessions_dir,
            missing: 0,
        })
    }

    fn watch_targets(&self) -> Vec<ProviderWatchTarget> {
        match resolve_gemini_root_dir() {
            Ok(root_dir) => vec![ProviderWatchTarget {
                provider: self.id(),
                kind: ProviderWatchTargetKind::Directory,
                path: root_dir.join("tmp"),
            }],
            Err(_) => Vec::new(),
        }
    }
}

struct OpenCodeProvider;

impl SessionProvider for OpenCodeProvider {
    fn id(&self) -> SessionEngine {
        SessionEngine::OpenCode
    }

    fn scan(&self) -> ProviderScanResult {
        let db_path = match resolve_opencode_db_path() {
            Ok(path) => path,
            Err(error) => {
                return ProviderScanResult {
                    sessions: Vec::new(),
                    warnings: 0,
                    notice: Some(format!("OpenCode DB disabled: {error}")),
                };
            }
        };

        let output = scan_opencode_db(&db_path);
        ProviderScanResult {
            sessions: output.sessions,
            warnings: output.warnings.get(),
            notice: output.notice,
        }
    }

    fn load_timeline(&self, session: &SessionSummary) -> Result<SessionTimeline, ProviderError> {
        let log_path = prepare_session_log_path(session).map_err(|error| {
            ProviderError::Message(format!("Failed to prepare OpenCode log: {error}"))
        })?;
        load_session_timeline(&log_path).map_err(|error| {
            ProviderError::Message(format!(
                "Failed to load {} timeline: {error}",
                self.id().label()
            ))
        })
    }

    fn load_last_output(
        &self,
        session: &SessionSummary,
    ) -> Result<LastAssistantOutput, ProviderError> {
        let log_path = prepare_session_log_path(session).map_err(|error| {
            ProviderError::Message(format!("Failed to prepare OpenCode log: {error}"))
        })?;
        load_last_assistant_output(&log_path).map_err(|error| {
            ProviderError::Message(format!(
                "Failed to load {} result: {error}",
                self.id().label()
            ))
        })
    }

    fn delete_sessions(
        &self,
        sessions: &[SessionSummary],
    ) -> Result<ProviderDeleteOutcome, ProviderError> {
        let mut unique_ids: BTreeSet<String> = BTreeSet::new();
        let mut log_paths: Vec<PathBuf> = Vec::new();
        for session in sessions {
            unique_ids.insert(session.meta.id.clone());
            if session.log_path.exists() {
                log_paths.push(session.log_path.clone());
            }
        }

        let archive_outcome =
            archive_opencode_sessions(&unique_ids.into_iter().collect::<Vec<_>>())
                .map_err(|error| ProviderError::Message(format_archive_error(error)))?;

        let cache_delete_outcome = match resolve_ccbox_state_dir() {
            Ok(state_dir) => {
                let cache_root = state_dir.join("opencode").join("sessions");
                delete_session_logs_with_roots(&[cache_root], &log_paths)
            }
            Err(_) => DeleteOutcome {
                deleted: 0,
                failed: 0,
                skipped_outside_sessions_dir: 0,
            },
        };

        Ok(ProviderDeleteOutcome {
            deleted: cache_delete_outcome.deleted,
            archived: archive_outcome.archived,
            failed: cache_delete_outcome.failed,
            skipped_outside_allowed_roots: cache_delete_outcome.skipped_outside_sessions_dir,
            missing: archive_outcome.missing,
        })
    }

    fn watch_targets(&self) -> Vec<ProviderWatchTarget> {
        match resolve_opencode_db_path() {
            Ok(path) => vec![ProviderWatchTarget {
                provider: self.id(),
                kind: ProviderWatchTargetKind::SqliteFamily,
                path,
            }],
            Err(_) => Vec::new(),
        }
    }
}

fn format_archive_error(error: ArchiveOpenCodeSessionsError) -> String {
    match error {
        ArchiveOpenCodeSessionsError::ResolveOpenCodeDbPath(inner) => inner.to_string(),
        ArchiveOpenCodeSessionsError::DbMissing(path) => format!("OpenCode DB not found: {path}"),
        ArchiveOpenCodeSessionsError::Sqlite(inner) => format!("SQLite error: {inner}"),
    }
}

pub fn notices_to_text(notices: Vec<String>) -> Option<String> {
    let text = notices
        .into_iter()
        .map(|notice| notice.trim().to_string())
        .filter(|notice| !notice.is_empty())
        .collect::<Vec<_>>()
        .join(" | ");
    if text.is_empty() { None } else { Some(text) }
}
