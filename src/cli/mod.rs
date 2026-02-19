use crate::domain::{ProjectSummary, TimelineItem, TimelineItemKind, index_projects};
use crate::infra::{LoadSessionTimelineError, ScanError, load_session_timeline, scan_sessions_dir};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CliInvocation {
    PrintHelp,
    PrintVersion,
    Tui,
    Command(CliCommand),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CliCommand {
    Projects,
    Sessions {
        project_path: Option<PathBuf>,
    },
    History {
        log_path: Option<PathBuf>,
        full: bool,
    },
}

#[derive(Debug, Error)]
pub enum CliParseError {
    #[error("unknown subcommand: {0}")]
    UnknownSubcommand(String),

    #[error("unknown flag: {0}")]
    UnknownFlag(String),

    #[error("unexpected argument: {0}")]
    UnexpectedArgument(String),
}

pub fn parse_invocation(args: &[String]) -> Result<CliInvocation, CliParseError> {
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        return Ok(CliInvocation::PrintHelp);
    }
    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        return Ok(CliInvocation::PrintVersion);
    }

    let mut iter = args.iter().skip(1);
    let Some(subcommand) = iter.next() else {
        return Ok(CliInvocation::Tui);
    };

    match subcommand.as_str() {
        "projects" => {
            let rest = iter.collect::<Vec<_>>();
            if let Some(arg) = rest.first() {
                if arg.starts_with('-') {
                    return Err(CliParseError::UnknownFlag((*arg).to_string()));
                }
                return Err(CliParseError::UnexpectedArgument((*arg).to_string()));
            }
            Ok(CliInvocation::Command(CliCommand::Projects))
        }
        "sessions" => {
            let mut project_path: Option<PathBuf> = None;
            for arg in iter {
                if arg.starts_with('-') {
                    return Err(CliParseError::UnknownFlag(arg.to_string()));
                }
                if project_path.is_some() {
                    return Err(CliParseError::UnexpectedArgument(arg.to_string()));
                }
                project_path = Some(PathBuf::from(arg));
            }

            Ok(CliInvocation::Command(CliCommand::Sessions {
                project_path,
            }))
        }
        "history" => {
            let mut full = false;
            let mut log_path: Option<PathBuf> = None;

            for arg in iter {
                if arg == "--full" {
                    full = true;
                    continue;
                }
                if arg.starts_with('-') {
                    return Err(CliParseError::UnknownFlag(arg.to_string()));
                }
                if log_path.is_some() {
                    return Err(CliParseError::UnexpectedArgument(arg.to_string()));
                }
                log_path = Some(PathBuf::from(arg));
            }

            Ok(CliInvocation::Command(CliCommand::History {
                log_path,
                full,
            }))
        }
        other => Err(CliParseError::UnknownSubcommand(other.to_string())),
    }
}

#[derive(Debug, Error)]
pub enum CliRunError {
    #[error("sessions directory does not exist: {0}\nHint: set CODEX_SESSIONS_DIR to override.")]
    SessionsDirMissing(String),

    #[error(transparent)]
    Scan(#[from] ScanError),

    #[error(transparent)]
    LoadTimeline(#[from] LoadSessionTimelineError),

    #[error("project not found: {0}\nHint: run `ccbox projects` and copy the full project path.")]
    ProjectNotFound(String),

    #[error("project has no sessions: {0}")]
    ProjectHasNoSessions(String),

    #[error(transparent)]
    WriteOutput(#[from] io::Error),

    #[error("failed to resolve current directory: {0}")]
    CurrentDir(String),
}

pub fn run(command: CliCommand, sessions_dir: &Path) -> Result<(), CliRunError> {
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    let stderr = io::stderr();
    let mut err = io::BufWriter::new(stderr.lock());

    match command {
        CliCommand::Projects => {
            let (projects, warnings) = load_projects(sessions_dir)?;
            for project in projects {
                let line = format!(
                    "{}\t{}\t{}",
                    project.name,
                    project.project_path.display(),
                    project.sessions.len()
                );
                if !write_line(&mut out, &line)? {
                    return Ok(());
                }
            }
            if warnings > 0 && !write_line(&mut err, &format!("warnings: {warnings}"))? {
                return Ok(());
            }
            Ok(())
        }
        CliCommand::Sessions { project_path } => {
            let (projects, warnings) = load_projects(sessions_dir)?;
            let project = select_project(projects, project_path)?;

            for session in project.sessions {
                let line = format!(
                    "{}\t{}\t{}\t{}",
                    session.meta.started_at_rfc3339,
                    session.meta.id,
                    session.title,
                    session.log_path.display(),
                );
                if !write_line(&mut out, &line)? {
                    return Ok(());
                }
            }
            if warnings > 0 && !write_line(&mut err, &format!("warnings: {warnings}"))? {
                return Ok(());
            }
            Ok(())
        }
        CliCommand::History { log_path, full } => {
            let log_path = match log_path {
                Some(path) if fs::metadata(&path).is_ok_and(|meta| meta.is_dir()) => {
                    let (projects, warnings) = load_projects(sessions_dir)?;
                    if warnings > 0 && !write_line(&mut err, &format!("warnings: {warnings}"))? {
                        return Ok(());
                    }
                    let project = select_project(projects, Some(path))?;
                    project
                        .sessions
                        .first()
                        .map(|session| session.log_path.clone())
                        .ok_or_else(|| {
                            CliRunError::ProjectHasNoSessions(
                                project.project_path.display().to_string(),
                            )
                        })?
                }
                Some(path) => path,
                None => {
                    let (projects, warnings) = load_projects(sessions_dir)?;
                    if warnings > 0 && !write_line(&mut err, &format!("warnings: {warnings}"))? {
                        return Ok(());
                    }
                    let project = select_project(projects, None)?;
                    project
                        .sessions
                        .first()
                        .map(|session| session.log_path.clone())
                        .ok_or_else(|| {
                            CliRunError::ProjectHasNoSessions(
                                project.project_path.display().to_string(),
                            )
                        })?
                }
            };

            let timeline = load_session_timeline(&log_path)?;
            for item in timeline.items {
                if !print_timeline_item(&mut out, &item, full)? {
                    return Ok(());
                }
            }
            if timeline.warnings > 0
                && !write_line(&mut err, &format!("warnings: {}", timeline.warnings))?
            {
                return Ok(());
            }
            if timeline.truncated && !write_line(&mut err, "truncated: true")? {
                return Ok(());
            }
            Ok(())
        }
    }
}

fn load_projects(sessions_dir: &Path) -> Result<(Vec<ProjectSummary>, usize), CliRunError> {
    match scan_sessions_dir(sessions_dir) {
        Ok(output) => Ok((index_projects(&output.sessions), output.warnings.get())),
        Err(ScanError::SessionsDirMissing(path)) => Err(CliRunError::SessionsDirMissing(path)),
        Err(error) => Err(CliRunError::Scan(error)),
    }
}

fn select_project(
    projects: Vec<ProjectSummary>,
    requested: Option<PathBuf>,
) -> Result<ProjectSummary, CliRunError> {
    let base_dir =
        std::env::current_dir().map_err(|error| CliRunError::CurrentDir(error.to_string()))?;

    let requested = requested.map(|path| {
        if path.is_absolute() {
            path
        } else {
            base_dir.join(path)
        }
    });

    let candidates: Vec<PathBuf> = match requested.as_ref() {
        Some(path) => path.ancestors().map(|path| path.to_path_buf()).collect(),
        None => base_dir
            .ancestors()
            .map(|path| path.to_path_buf())
            .collect(),
    };

    let canonical_projects = projects
        .iter()
        .map(|project| project.project_path.canonicalize().ok())
        .collect::<Vec<_>>();

    for candidate in candidates {
        let canonical_candidate = candidate.canonicalize().ok();
        for (idx, project) in projects.iter().enumerate() {
            if project.project_path == candidate {
                return Ok(project.clone());
            }
            if let (Some(project_canon), Some(candidate_canon)) = (
                canonical_projects.get(idx).and_then(|value| value.as_ref()),
                canonical_candidate.as_ref(),
            ) {
                if project_canon == candidate_canon {
                    return Ok(project.clone());
                }
            }
        }
    }

    let attempted = requested.unwrap_or(base_dir);
    Err(CliRunError::ProjectNotFound(
        attempted.display().to_string(),
    ))
}

fn print_timeline_item(out: &mut impl Write, item: &TimelineItem, full: bool) -> io::Result<bool> {
    if item.kind == TimelineItemKind::Turn {
        if !write_line(out, "")? {
            return Ok(false);
        }
        if !write_line(out, &format!("== {} ==", item.summary))? {
            return Ok(false);
        }
        return Ok(true);
    }

    let kind = kind_label(item.kind);
    let timestamp = item.timestamp.as_deref().unwrap_or("");
    let turn_id = item.turn_id.as_deref().unwrap_or("");

    let line = match (timestamp.is_empty(), turn_id.is_empty()) {
        (true, true) => format!("{kind}: {}", item.summary),
        (false, true) => format!("[{timestamp}] {kind}: {}", item.summary),
        (true, false) => format!("[{}] {kind}: {}", short_id(turn_id), item.summary),
        (false, false) => format!(
            "[{timestamp}] [{}] {kind}: {}",
            short_id(turn_id),
            item.summary
        ),
    };
    if !write_line(out, &line)? {
        return Ok(false);
    }

    if full {
        let detail = item.detail.trim_end();
        if !detail.is_empty() {
            for line in detail.lines() {
                if !write_line(out, &format!("  {line}"))? {
                    return Ok(false);
                }
            }
        }
        if !write_line(out, "")? {
            return Ok(false);
        }
    }

    Ok(true)
}

fn kind_label(kind: TimelineItemKind) -> &'static str {
    match kind {
        TimelineItemKind::Turn => "TURN",
        TimelineItemKind::User => "USER",
        TimelineItemKind::Assistant => "ASSISTANT",
        TimelineItemKind::Thinking => "THINKING",
        TimelineItemKind::ToolCall => "TOOL",
        TimelineItemKind::ToolOutput => "TOOL_OUT",
        TimelineItemKind::TokenCount => "TOKENS",
        TimelineItemKind::Note => "NOTE",
    }
}

fn short_id(value: &str) -> String {
    let max = 8usize;
    value.chars().take(max).collect()
}

fn write_line(out: &mut impl Write, line: &str) -> io::Result<bool> {
    match writeln!(out, "{line}") {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_string()).collect()
    }

    #[test]
    fn parse_defaults_to_tui_when_no_args() {
        let parsed = parse_invocation(&args(&["ccbox"])).expect("parse");
        assert_eq!(parsed, CliInvocation::Tui);
    }

    #[test]
    fn parse_help_flag_wins() {
        let parsed = parse_invocation(&args(&["ccbox", "projects", "--help"])).expect("parse");
        assert_eq!(parsed, CliInvocation::PrintHelp);
    }

    #[test]
    fn parse_projects_command() {
        let parsed = parse_invocation(&args(&["ccbox", "projects"])).expect("parse");
        assert_eq!(parsed, CliInvocation::Command(CliCommand::Projects));
    }

    #[test]
    fn parse_sessions_command_defaults_to_current_dir_project() {
        let parsed = parse_invocation(&args(&["ccbox", "sessions"])).expect("parse");
        assert_eq!(
            parsed,
            CliInvocation::Command(CliCommand::Sessions { project_path: None })
        );
    }

    #[test]
    fn parse_history_command_supports_full_flag() {
        let parsed = parse_invocation(&args(&["ccbox", "history", "--full", "/tmp/session.jsonl"]))
            .expect("parse");
        assert_eq!(
            parsed,
            CliInvocation::Command(CliCommand::History {
                log_path: Some(PathBuf::from("/tmp/session.jsonl")),
                full: true
            })
        );
    }

    #[test]
    fn parse_history_command_defaults_to_current_dir_session() {
        let parsed = parse_invocation(&args(&["ccbox", "history"])).expect("parse");
        assert_eq!(
            parsed,
            CliInvocation::Command(CliCommand::History {
                log_path: None,
                full: false
            })
        );
    }
}
