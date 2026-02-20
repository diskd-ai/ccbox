use crate::domain::SessionSummary;
use crate::infra::{
    ResolveClaudeProjectsDirError, ResolveGeminiRootDirError, ScanError, ScanWarningCount,
    resolve_claude_projects_dir, resolve_gemini_root_dir, scan_claude_projects_dir,
    scan_gemini_root_dir, scan_sessions_dir,
};
use std::path::Path;

#[derive(Clone, Debug)]
pub struct MultiEngineScanOutput {
    pub sessions: Vec<SessionSummary>,
    pub warnings: ScanWarningCount,
    pub notice: Option<String>,
}

pub fn scan_all_sessions(codex_sessions_dir: &Path) -> MultiEngineScanOutput {
    let (claude_projects_dir, claude_resolve_notice) = match resolve_claude_projects_dir() {
        Ok(dir) => (Some(dir), None),
        Err(ResolveClaudeProjectsDirError::HomeDirNotFound) => (
            None,
            Some("Claude projects dir disabled: home directory not found".to_string()),
        ),
    };

    let (gemini_root_dir, gemini_resolve_notice) = match resolve_gemini_root_dir() {
        Ok(dir) => (Some(dir), None),
        Err(ResolveGeminiRootDirError::HomeDirNotFound) => (
            None,
            Some("Gemini root dir disabled: home directory not found".to_string()),
        ),
    };

    scan_all_sessions_with_dirs(
        codex_sessions_dir,
        claude_projects_dir.as_deref(),
        claude_resolve_notice,
        gemini_root_dir.as_deref(),
        gemini_resolve_notice,
    )
}

fn scan_all_sessions_with_dirs(
    codex_sessions_dir: &Path,
    claude_projects_dir: Option<&Path>,
    claude_resolve_notice: Option<String>,
    gemini_root_dir: Option<&Path>,
    gemini_resolve_notice: Option<String>,
) -> MultiEngineScanOutput {
    let mut sessions: Vec<SessionSummary> = Vec::new();
    let mut warnings = 0usize;
    let mut notices: Vec<String> = Vec::new();

    match scan_sessions_dir(codex_sessions_dir) {
        Ok(output) => {
            warnings += output.warnings.get();
            sessions.extend(output.sessions);
        }
        Err(ScanError::SessionsDirMissing(path)) => {
            notices.push(format!("Codex sessions dir not found: {path}"));
        }
        Err(error) => {
            warnings = warnings.saturating_add(1);
            notices.push(format!("Failed to scan Codex sessions: {error}"));
        }
    }

    if let Some(notice) = claude_resolve_notice {
        notices.push(notice);
    }

    if let Some(projects_dir) = claude_projects_dir {
        let output = scan_claude_projects_dir(projects_dir);
        warnings += output.warnings.get();
        sessions.extend(output.sessions);
        if let Some(notice) = output.notice {
            notices.push(notice);
        }
    }

    if let Some(notice) = gemini_resolve_notice {
        notices.push(notice);
    }

    if let Some(root_dir) = gemini_root_dir {
        let output = scan_gemini_root_dir(root_dir);
        warnings += output.warnings.get();
        sessions.extend(output.sessions);
        if let Some(notice) = output.notice {
            notices.push(notice);
        }
    }

    MultiEngineScanOutput {
        sessions,
        warnings: ScanWarningCount::from(warnings),
        notice: join_notices(notices),
    }
}

fn join_notices(notices: Vec<String>) -> Option<String> {
    let text = notices
        .into_iter()
        .map(|notice| notice.trim().to_string())
        .filter(|notice| !notice.is_empty())
        .collect::<Vec<_>>()
        .join(" | ");
    if text.is_empty() { None } else { Some(text) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn scans_claude_when_codex_missing() {
        let dir = tempdir().expect("tempdir");
        let codex_sessions_dir = dir.path().join("missing-codex");

        let claude_root = dir.path().join("claude");
        let claude_projects = claude_root.join("projects");
        let key_dir = claude_projects.join("k");
        fs::create_dir_all(&key_dir).expect("create");

        let log_path = key_dir.join("s.jsonl");
        fs::write(
            &log_path,
            r#"{"type":"user","cwd":"/tmp/p","sessionId":"s","timestamp":"2026-02-19T00:00:00Z","message":{"content":"hello"}}"#,
        )
        .expect("write");

        let output = scan_all_sessions_with_dirs(
            &codex_sessions_dir,
            Some(&claude_projects),
            None,
            None,
            None,
        );

        assert_eq!(output.sessions.len(), 1);
        assert!(output.notice.is_some());
    }
}
