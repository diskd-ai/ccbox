use crate::domain::{ForkContext, ForkCut, SessionSummary, TimelineItem, TimelineItemKind};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ForkSelectionError {
    #[error(
        "Fork is available on Turn/User/Out/ToolOut records. Select a neighboring record and retry."
    )]
    UnsupportedKind,

    #[error("Selected record has no source line number (unsupported log format).")]
    MissingSourceLine,
}

pub fn fork_context_from_timeline_item(
    session: &SessionSummary,
    item: &TimelineItem,
) -> Result<ForkContext, ForkSelectionError> {
    let source_line_no = item
        .source_line_no
        .ok_or(ForkSelectionError::MissingSourceLine)?;

    let cut = match item.kind {
        TimelineItemKind::Turn => ForkCut::BeforeLine {
            line_no: source_line_no,
        },
        TimelineItemKind::User | TimelineItemKind::Assistant | TimelineItemKind::ToolOutput => {
            ForkCut::AfterLine {
                line_no: source_line_no,
            }
        }
        TimelineItemKind::Thinking
        | TimelineItemKind::ToolCall
        | TimelineItemKind::TokenCount
        | TimelineItemKind::Note => return Err(ForkSelectionError::UnsupportedKind),
    };

    let label = match item.kind {
        TimelineItemKind::Turn => item.summary.clone(),
        TimelineItemKind::User => format!("User: {}", item.summary),
        TimelineItemKind::Assistant => format!("Out: {}", item.summary),
        TimelineItemKind::ToolOutput => format!("ToolOut: {}", item.summary),
        TimelineItemKind::Thinking
        | TimelineItemKind::ToolCall
        | TimelineItemKind::TokenCount
        | TimelineItemKind::Note => item.summary.clone(),
    };

    Ok(ForkContext {
        parent_session_id: session.meta.id.clone(),
        parent_log_path: session.log_path.clone(),
        project_path: session.meta.cwd.clone(),
        cut,
        label,
    })
}

pub fn default_fork_prompt(fork: &ForkContext) -> String {
    format!("Continue from: {}", fork.label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{SessionEngine, SessionMeta};
    use std::path::PathBuf;
    use std::time::SystemTime;

    fn make_session() -> SessionSummary {
        SessionSummary {
            engine: SessionEngine::Codex,
            meta: SessionMeta {
                id: "019c72c9-e13d-71b3-b853-5ff79aa22102".to_string(),
                cwd: PathBuf::from("/tmp/project"),
                started_at_rfc3339: "2026-02-18T21:45:57.803Z".to_string(),
            },
            log_path: PathBuf::from("/tmp/session.jsonl"),
            title: "t".to_string(),
            file_size_bytes: 0,
            file_modified: Some(SystemTime::now()),
        }
    }

    #[test]
    fn derives_before_cut_for_turn_item() {
        let session = make_session();
        let item = TimelineItem {
            kind: TimelineItemKind::Turn,
            turn_id: Some("t1".to_string()),
            call_id: None,
            source_line_no: Some(12),
            timestamp: None,
            timestamp_ms: None,
            summary: "Turn t1".to_string(),
            detail: "t1".to_string(),
        };

        let fork = fork_context_from_timeline_item(&session, &item).expect("fork");
        assert_eq!(fork.cut, ForkCut::BeforeLine { line_no: 12 });
    }

    #[test]
    fn derives_after_cut_for_user_item() {
        let session = make_session();
        let item = TimelineItem {
            kind: TimelineItemKind::User,
            turn_id: Some("t1".to_string()),
            call_id: None,
            source_line_no: Some(13),
            timestamp: None,
            timestamp_ms: None,
            summary: "hello".to_string(),
            detail: "hello".to_string(),
        };

        let fork = fork_context_from_timeline_item(&session, &item).expect("fork");
        assert_eq!(fork.cut, ForkCut::AfterLine { line_no: 13 });
    }

    #[test]
    fn rejects_non_forkable_kind() {
        let session = make_session();
        let item = TimelineItem {
            kind: TimelineItemKind::ToolCall,
            turn_id: Some("t1".to_string()),
            call_id: Some("c1".to_string()),
            source_line_no: Some(14),
            timestamp: None,
            timestamp_ms: None,
            summary: "tool()".to_string(),
            detail: "{}".to_string(),
        };

        let error = fork_context_from_timeline_item(&session, &item).expect_err("error");
        assert!(matches!(error, ForkSelectionError::UnsupportedKind));
    }

    #[test]
    fn seeds_default_prompt_from_label() {
        let mut fork = ForkContext {
            parent_session_id: "p".to_string(),
            parent_log_path: PathBuf::from("a"),
            project_path: PathBuf::from("b"),
            cut: ForkCut::AfterLine { line_no: 2 },
            label: "Out: ok".to_string(),
        };
        assert_eq!(default_fork_prompt(&fork), "Continue from: Out: ok");
        fork.label = "Turn t1".to_string();
        assert_eq!(default_fork_prompt(&fork), "Continue from: Turn t1");
    }
}
