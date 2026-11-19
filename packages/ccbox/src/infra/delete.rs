use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct DeleteOutcome {
    pub deleted: usize,
    pub failed: usize,
    pub skipped_outside_sessions_dir: usize,
}

pub fn delete_session_logs_with_roots(
    allowed_roots: &[PathBuf],
    log_paths: &[PathBuf],
) -> DeleteOutcome {
    let mut deleted = 0usize;
    let mut failed = 0usize;
    let mut skipped_outside_sessions_dir = 0usize;

    let allowed_roots_canonical = allowed_roots
        .iter()
        .map(|root| fs::canonicalize(root).unwrap_or_else(|_| root.clone()))
        .collect::<Vec<_>>();

    for path in log_paths {
        let path_canonical = fs::canonicalize(path).unwrap_or_else(|_| path.clone());
        let is_inside_allowed_root = allowed_roots_canonical
            .iter()
            .any(|root| path_canonical.starts_with(root));
        if !is_inside_allowed_root {
            skipped_outside_sessions_dir += 1;
            continue;
        }

        match fs::remove_file(path) {
            Ok(()) => deleted += 1,
            Err(_) => failed += 1,
        }
    }

    DeleteOutcome {
        deleted,
        failed,
        skipped_outside_sessions_dir,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn deletes_file_inside_single_root() {
        let dir = tempdir().expect("tempdir");
        let sessions_dir = dir.path().join("sessions");
        fs::create_dir_all(&sessions_dir).expect("create sessions dir");
        let log_path = sessions_dir.join("s.jsonl");
        fs::write(&log_path, "{}").expect("write log");

        let outcome = delete_session_logs_with_roots(
            std::slice::from_ref(&sessions_dir),
            std::slice::from_ref(&log_path),
        );

        assert_eq!(outcome.deleted, 1);
        assert_eq!(outcome.failed, 0);
        assert_eq!(outcome.skipped_outside_sessions_dir, 0);
        assert!(!log_path.exists());
    }

    #[test]
    fn deletes_files_inside_multiple_roots() {
        let dir = tempdir().expect("tempdir");
        let codex_dir = dir.path().join("codex");
        let claude_dir = dir.path().join("claude");
        fs::create_dir_all(&codex_dir).expect("create codex");
        fs::create_dir_all(&claude_dir).expect("create claude");

        let codex_log = codex_dir.join("codex.jsonl");
        let claude_log = claude_dir.join("claude.jsonl");
        fs::write(&codex_log, "{}").expect("write codex");
        fs::write(&claude_log, "{}").expect("write claude");

        let outcome = delete_session_logs_with_roots(
            &[codex_dir.clone(), claude_dir.clone()],
            &[codex_log.clone(), claude_log.clone()],
        );

        assert_eq!(outcome.deleted, 2);
        assert_eq!(outcome.failed, 0);
        assert_eq!(outcome.skipped_outside_sessions_dir, 0);
        assert!(!codex_log.exists());
        assert!(!claude_log.exists());
    }

    #[test]
    fn skips_file_outside_allowed_roots() {
        let dir = tempdir().expect("tempdir");
        let allowed_dir = dir.path().join("allowed");
        let outside_dir = dir.path().join("outside");
        fs::create_dir_all(&allowed_dir).expect("create allowed");
        fs::create_dir_all(&outside_dir).expect("create outside");

        let outside_log = outside_dir.join("outside.jsonl");
        fs::write(&outside_log, "{}").expect("write outside");

        let outcome = delete_session_logs_with_roots(
            std::slice::from_ref(&allowed_dir),
            std::slice::from_ref(&outside_log),
        );

        assert_eq!(outcome.deleted, 0);
        assert_eq!(outcome.failed, 0);
        assert_eq!(outcome.skipped_outside_sessions_dir, 1);
        assert!(outside_log.exists());
    }
}
