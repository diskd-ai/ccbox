use crate::domain::SessionSummary;
use crate::infra::{
    ScanWarningCount, apply_session_aliases, apply_session_projects, load_session_aliases,
    load_session_projects, notices_to_text, resolve_ccbox_state_dir, scan_all_with_providers,
};
use std::path::Path;

#[derive(Clone, Debug)]
pub struct MultiEngineScanOutput {
    pub sessions: Vec<SessionSummary>,
    pub warnings: ScanWarningCount,
    pub notice: Option<String>,
}

pub fn scan_all_sessions(codex_sessions_dir: &Path) -> MultiEngineScanOutput {
    let scan = scan_all_with_providers(codex_sessions_dir);
    let mut sessions = scan.sessions;
    let mut warnings = scan.warnings;
    let mut notices = scan.notices;

    if let Ok(state_dir) = resolve_ccbox_state_dir() {
        match load_session_aliases(&state_dir) {
            Ok(aliases) => apply_session_aliases(&mut sessions, &aliases),
            Err(error) => {
                warnings = warnings.saturating_add(1);
                notices.push(format!("Failed to load session aliases: {error}"));
            }
        }

        match load_session_projects(&state_dir) {
            Ok(projects) => apply_session_projects(&mut sessions, &projects),
            Err(error) => {
                warnings = warnings.saturating_add(1);
                notices.push(format!("Failed to load session projects: {error}"));
            }
        }
    }

    MultiEngineScanOutput {
        sessions,
        warnings: ScanWarningCount::from(warnings),
        notice: notices_to_text(notices),
    }
}
