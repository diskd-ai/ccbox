use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct DeleteOutcome {
    pub deleted: usize,
    pub failed: usize,
    pub skipped_outside_sessions_dir: usize,
}

pub fn delete_session_logs(sessions_dir: &Path, log_paths: &[PathBuf]) -> DeleteOutcome {
    let mut deleted = 0usize;
    let mut failed = 0usize;
    let mut skipped_outside_sessions_dir = 0usize;

    let sessions_dir_canonical =
        fs::canonicalize(sessions_dir).unwrap_or_else(|_| sessions_dir.to_path_buf());

    for path in log_paths {
        let path_canonical = fs::canonicalize(path).unwrap_or_else(|_| path.clone());
        if !path_canonical.starts_with(&sessions_dir_canonical) {
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
