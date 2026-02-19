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

fn should_trigger_rescan(event: &notify::Event) -> bool {
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }
    if event.paths.is_empty() {
        return true;
    }

    event
        .paths
        .iter()
        .any(|path| path.extension().and_then(|ext| ext.to_str()) == Some("jsonl"))
}
