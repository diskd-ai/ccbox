use notify::event::EventKind;
use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::sync::mpsc::{Receiver, channel};
use thiserror::Error;

#[derive(Clone, Debug)]
pub enum WatchSignal {
    Changed,
    Error(String),
}

#[derive(Debug)]
pub struct SessionsDirWatcher {
    _watcher: RecommendedWatcher,
    rx: Receiver<WatchSignal>,
}

impl SessionsDirWatcher {
    pub fn try_recv(&self) -> Option<WatchSignal> {
        self.rx.try_recv().ok()
    }
}

#[derive(Debug)]
pub struct SessionFileWatcher {
    _watcher: RecommendedWatcher,
    rx: Receiver<WatchSignal>,
}

impl SessionFileWatcher {
    pub fn try_recv(&self) -> Option<WatchSignal> {
        self.rx.try_recv().ok()
    }
}

#[derive(Debug, Error)]
pub enum WatchSessionsDirError {
    #[error("watch error: {0}")]
    Notify(#[from] notify::Error),
}

pub fn watch_sessions_dir(path: &Path) -> Result<SessionsDirWatcher, WatchSessionsDirError> {
    let (tx, rx) = channel::<WatchSignal>();

    let mut watcher = RecommendedWatcher::new(
        move |res: notify::Result<notify::Event>| match res {
            Ok(event) => {
                if should_trigger_rescan(&event) {
                    let _ = tx.send(WatchSignal::Changed);
                }
            }
            Err(error) => {
                let _ = tx.send(WatchSignal::Error(error.to_string()));
            }
        },
        Config::default(),
    )?;

    watcher.watch(path, RecursiveMode::Recursive)?;

    Ok(SessionsDirWatcher {
        _watcher: watcher,
        rx,
    })
}

pub fn watch_sqlite_db_family(
    db_path: &Path,
) -> Result<Option<SessionsDirWatcher>, WatchSessionsDirError> {
    let Some(parent_dir) = db_path.parent() else {
        return Ok(None);
    };
    let Some(db_file_name) = db_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|s| s.to_string())
    else {
        return Ok(None);
    };

    let wal_file_name = format!("{db_file_name}-wal");
    let shm_file_name = format!("{db_file_name}-shm");

    let (tx, rx) = channel::<WatchSignal>();

    let mut watcher = RecommendedWatcher::new(
        move |res: notify::Result<notify::Event>| match res {
            Ok(event) => {
                if matches!(event.kind, EventKind::Access(_)) {
                    return;
                }
                if event.paths.is_empty() {
                    let _ = tx.send(WatchSignal::Changed);
                    return;
                }

                let should_trigger = event.paths.iter().any(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| {
                            name == db_file_name || name == wal_file_name || name == shm_file_name
                        })
                });

                if should_trigger {
                    let _ = tx.send(WatchSignal::Changed);
                }
            }
            Err(error) => {
                let _ = tx.send(WatchSignal::Error(error.to_string()));
            }
        },
        Config::default(),
    )?;

    watcher.watch(parent_dir, RecursiveMode::NonRecursive)?;

    Ok(Some(SessionsDirWatcher {
        _watcher: watcher,
        rx,
    }))
}

#[derive(Debug, Error)]
pub enum WatchSessionFileError {
    #[error("watch error: {0}")]
    Notify(#[from] notify::Error),
}

pub fn watch_session_file(path: &Path) -> Result<SessionFileWatcher, WatchSessionFileError> {
    let (tx, rx) = channel::<WatchSignal>();

    let mut watcher = RecommendedWatcher::new(
        move |res: notify::Result<notify::Event>| match res {
            Ok(event) => {
                if should_trigger_session_reload(&event) {
                    let _ = tx.send(WatchSignal::Changed);
                }
            }
            Err(error) => {
                let _ = tx.send(WatchSignal::Error(error.to_string()));
            }
        },
        Config::default(),
    )?;

    watcher.watch(path, RecursiveMode::NonRecursive)?;

    Ok(SessionFileWatcher {
        _watcher: watcher,
        rx,
    })
}

fn should_trigger_rescan(event: &notify::Event) -> bool {
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }
    if event.paths.is_empty() {
        return true;
    }

    event.paths.iter().any(|path| {
        let extension = path.extension().and_then(|ext| ext.to_str());
        if extension == Some("jsonl") {
            return true;
        }

        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            return false;
        };

        if file_name == "sessions-index.json" || file_name == "logs.json" {
            return true;
        }

        extension == Some("json") && file_name.starts_with("session-")
    })
}

fn should_trigger_session_reload(event: &notify::Event) -> bool {
    !matches!(event.kind, EventKind::Access(_))
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{AccessKind, ModifyKind};
    use std::path::PathBuf;

    fn event(kind: EventKind, paths: Vec<&str>) -> notify::Event {
        notify::Event {
            kind,
            paths: paths.into_iter().map(PathBuf::from).collect(),
            attrs: notify::event::EventAttributes::default(),
        }
    }

    #[test]
    fn rescan_ignores_access_events() {
        let event = event(
            EventKind::Access(AccessKind::Any),
            vec!["/tmp/sessions/a.jsonl"],
        );
        assert!(!should_trigger_rescan(&event));
    }

    #[test]
    fn rescan_triggers_for_jsonl_logs() {
        let event = event(
            EventKind::Modify(ModifyKind::Any),
            vec!["/tmp/sessions/a.jsonl"],
        );
        assert!(should_trigger_rescan(&event));
    }

    #[test]
    fn rescan_triggers_for_claude_sessions_index() {
        let event = event(
            EventKind::Modify(ModifyKind::Any),
            vec!["/tmp/projects/sessions-index.json"],
        );
        assert!(should_trigger_rescan(&event));
    }

    #[test]
    fn rescan_triggers_for_gemini_logs_json() {
        let event = event(
            EventKind::Modify(ModifyKind::Any),
            vec!["/tmp/gemini/tmp/hash/logs.json"],
        );
        assert!(should_trigger_rescan(&event));
    }

    #[test]
    fn rescan_triggers_for_gemini_session_json_files() {
        let event = event(
            EventKind::Modify(ModifyKind::Any),
            vec!["/tmp/gemini/tmp/hash/chats/session-abc.json"],
        );
        assert!(should_trigger_rescan(&event));
    }

    #[test]
    fn rescan_does_not_trigger_for_unrelated_json_files() {
        let event = event(
            EventKind::Modify(ModifyKind::Any),
            vec!["/tmp/gemini/tmp/hash/other.json"],
        );
        assert!(!should_trigger_rescan(&event));
    }
}
