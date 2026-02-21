use crate::domain::{
    ProjectSummary, SessionEngine, TimelineItem, TimelineItemKind, compute_skill_metrics,
    detect_skill_loops, detect_skill_spans, index_projects,
};
use crate::infra::{LoadSessionTimelineError, load_session_timeline, scan_all_sessions};
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;

const DEFAULT_LIMIT: usize = 10;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CliInvocation {
    PrintHelp,
    PrintVersion,
    Tui { engine: Option<SessionEngine> },
    Command(CliCommand),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CliCommand {
    Projects {
        engine: Option<SessionEngine>,
    },
    Sessions {
        project_path: Option<PathBuf>,
        engine: Option<SessionEngine>,
        offset: usize,
        limit: usize,
        size: bool,
    },
    History {
        log_path: Option<PathBuf>,
        session_id: Option<String>,
        engine: Option<SessionEngine>,
        offset: usize,
        limit: usize,
        full: bool,
        size: bool,
    },
    Skills {
        log_path: Option<PathBuf>,
        engine: Option<SessionEngine>,
        json: bool,
        full: bool,
    },
    Update,
}

#[derive(Debug, Error)]
pub enum CliParseError {
    #[error("unknown subcommand: {0}")]
    UnknownSubcommand(String),

    #[error("unknown flag: {0}")]
    UnknownFlag(String),

    #[error("missing value for flag: {0}")]
    MissingFlagValue(String),

    #[error("invalid value for {flag}: {value}")]
    InvalidFlagValue { flag: String, value: String },

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

    let mut iter = args.iter().skip(1).peekable();
    let mut global_engine: Option<SessionEngine> = None;
    while let Some(arg) = iter.peek() {
        match arg.as_str() {
            "--engine" | "-e" => {
                let _ = iter.next();
                let value = iter
                    .next()
                    .ok_or_else(|| CliParseError::MissingFlagValue("--engine".to_string()))?;
                global_engine = parse_engine_flag("--engine", value)?;
            }
            "--" => {
                let _ = iter.next();
                break;
            }
            _ => break,
        }
    }

    let Some(subcommand) = iter.next() else {
        return Ok(CliInvocation::Tui {
            engine: global_engine,
        });
    };

    match subcommand.as_str() {
        "projects" => {
            let mut engine: Option<SessionEngine> = global_engine;

            let mut args = iter.peekable();
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--engine" | "-e" => {
                        let value = args.next().ok_or_else(|| {
                            CliParseError::MissingFlagValue("--engine".to_string())
                        })?;
                        engine = parse_engine_flag("--engine", value)?;
                    }
                    _ if arg.starts_with('-') => {
                        return Err(CliParseError::UnknownFlag(arg.to_string()));
                    }
                    _ => {
                        return Err(CliParseError::UnexpectedArgument(arg.to_string()));
                    }
                }
            }

            Ok(CliInvocation::Command(CliCommand::Projects { engine }))
        }
        "sessions" => {
            let mut project_path: Option<PathBuf> = None;
            let mut engine: Option<SessionEngine> = global_engine;
            let mut offset = 0usize;
            let mut limit = DEFAULT_LIMIT;
            let mut size = false;

            let mut args = iter.peekable();
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--engine" | "-e" => {
                        let value = args.next().ok_or_else(|| {
                            CliParseError::MissingFlagValue("--engine".to_string())
                        })?;
                        engine = parse_engine_flag("--engine", value)?;
                    }
                    "--limit" | "-l" => {
                        let value = args.next().ok_or_else(|| {
                            CliParseError::MissingFlagValue("--limit".to_string())
                        })?;
                        limit = parse_usize_flag("--limit", value)?;
                    }
                    "--offset" | "-o" => {
                        let value = args.next().ok_or_else(|| {
                            CliParseError::MissingFlagValue("--offset".to_string())
                        })?;
                        offset = parse_usize_flag("--offset", value)?;
                    }
                    "--size" => {
                        size = true;
                    }
                    _ if arg.starts_with('-') => {
                        return Err(CliParseError::UnknownFlag(arg.to_string()));
                    }
                    _ => {
                        if project_path.is_some() {
                            return Err(CliParseError::UnexpectedArgument(arg.to_string()));
                        }
                        project_path = Some(PathBuf::from(arg));
                    }
                }
            }

            Ok(CliInvocation::Command(CliCommand::Sessions {
                project_path,
                engine,
                offset,
                limit,
                size,
            }))
        }
        "history" => {
            let mut full = false;
            let mut size = false;
            let mut offset = 0usize;
            let mut limit = DEFAULT_LIMIT;
            let mut log_path: Option<PathBuf> = None;
            let mut session_id: Option<String> = None;
            let mut engine: Option<SessionEngine> = global_engine;

            let mut args = iter.peekable();
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--full" => {
                        full = true;
                    }
                    "--engine" | "-e" => {
                        let value = args.next().ok_or_else(|| {
                            CliParseError::MissingFlagValue("--engine".to_string())
                        })?;
                        engine = parse_engine_flag("--engine", value)?;
                    }
                    "--id" | "--session-id" => {
                        let value = args
                            .next()
                            .ok_or_else(|| CliParseError::MissingFlagValue("--id".to_string()))?;
                        session_id = Some((*value).to_string());
                    }
                    "--limit" | "-l" => {
                        let value = args.next().ok_or_else(|| {
                            CliParseError::MissingFlagValue("--limit".to_string())
                        })?;
                        limit = parse_usize_flag("--limit", value)?;
                    }
                    "--offset" | "-o" => {
                        let value = args.next().ok_or_else(|| {
                            CliParseError::MissingFlagValue("--offset".to_string())
                        })?;
                        offset = parse_usize_flag("--offset", value)?;
                    }
                    "--size" => {
                        size = true;
                    }
                    _ if arg.starts_with('-') => {
                        return Err(CliParseError::UnknownFlag(arg.to_string()));
                    }
                    _ => {
                        if looks_like_path(arg) {
                            if log_path.is_some() {
                                return Err(CliParseError::UnexpectedArgument(arg.to_string()));
                            }
                            log_path = Some(PathBuf::from(arg));
                            continue;
                        }

                        if session_id.is_some() {
                            return Err(CliParseError::UnexpectedArgument(arg.to_string()));
                        }
                        session_id = Some((*arg).to_string());
                    }
                }
            }

            Ok(CliInvocation::Command(CliCommand::History {
                log_path,
                session_id,
                engine,
                offset,
                limit,
                full,
                size,
            }))
        }
        "skills" => {
            let mut json = false;
            let mut full = false;
            let mut log_path: Option<PathBuf> = None;
            let mut engine: Option<SessionEngine> = global_engine;

            let mut args = iter.peekable();
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--json" => {
                        json = true;
                    }
                    "--full" => {
                        full = true;
                    }
                    "--engine" | "-e" => {
                        let value = args.next().ok_or_else(|| {
                            CliParseError::MissingFlagValue("--engine".to_string())
                        })?;
                        engine = parse_engine_flag("--engine", value)?;
                    }
                    _ if arg.starts_with('-') => {
                        return Err(CliParseError::UnknownFlag(arg.to_string()));
                    }
                    _ => {
                        if log_path.is_some() {
                            return Err(CliParseError::UnexpectedArgument(arg.to_string()));
                        }
                        log_path = Some(PathBuf::from(arg));
                    }
                }
            }

            Ok(CliInvocation::Command(CliCommand::Skills {
                log_path,
                engine,
                json,
                full,
            }))
        }
        "update" => {
            let rest = iter.collect::<Vec<_>>();
            if let Some(arg) = rest.first() {
                if arg.starts_with('-') {
                    return Err(CliParseError::UnknownFlag((*arg).to_string()));
                }
                return Err(CliParseError::UnexpectedArgument((*arg).to_string()));
            }
            Ok(CliInvocation::Command(CliCommand::Update))
        }
        other => Err(CliParseError::UnknownSubcommand(other.to_string())),
    }
}

#[derive(Debug, Error)]
pub enum CliRunError {
    #[error(transparent)]
    Scan(#[from] crate::infra::ScanError),

    #[error(transparent)]
    LoadTimeline(#[from] LoadSessionTimelineError),

    #[error(transparent)]
    PrepareSessionLog(#[from] crate::infra::PrepareSessionLogError),

    #[error("project not found: {0}\nHint: run `ccbox projects` and copy the full project path.")]
    ProjectNotFound(String),

    #[error("project has no sessions: {0}")]
    ProjectHasNoSessions(String),

    #[error("project has no sessions for engine {engine}: {project}")]
    ProjectHasNoSessionsForEngine { project: String, engine: String },

    #[error(
        "session not found: {0}\nHint: run `ccbox sessions <project-path>` and copy the session id column."
    )]
    SessionNotFound(String),

    #[error(
        "session id matches multiple sessions: {0}\nHint: pass a project directory before the session id, e.g. `ccbox history /path/to/project {0}`."
    )]
    SessionIdAmbiguous(String),

    #[error(
        "cannot combine session id with explicit log path: {0}\nHint: pass a project directory (or omit the path) and use --id."
    )]
    HistoryIdWithLogPath(String),

    #[error(transparent)]
    Update(#[from] crate::infra::UpdateError),

    #[error(transparent)]
    WriteOutput(#[from] io::Error),

    #[error("failed to resolve current directory: {0}")]
    CurrentDir(String),
}

struct CliUpdateNotice {
    cached_hint: Option<String>,
    rx: Option<Receiver<Option<String>>>,
    use_color: bool,
}

impl CliUpdateNotice {
    fn prepare() -> Self {
        let current = env!("CARGO_PKG_VERSION");
        let use_color = should_color_stderr();
        let Ok(state_dir) = crate::infra::resolve_ccbox_state_dir() else {
            return Self {
                cached_hint: None,
                rx: None,
                use_color,
            };
        };

        let cached_hint = crate::infra::load_update_check_cache(&state_dir)
            .ok()
            .flatten()
            .and_then(|(_checked_ms, latest_tag)| {
                let current_ver = crate::infra::Version::parse(current)?;
                let latest_ver = crate::infra::Version::parse(&latest_tag)?;
                if latest_ver > current_ver {
                    Some(format_update_hint(current, &latest_tag))
                } else {
                    None
                }
            });

        let refresh_needed = is_update_cache_stale(&state_dir);
        if !refresh_needed {
            return Self {
                cached_hint,
                rx: None,
                use_color,
            };
        }

        let (tx, rx) = channel::<Option<String>>();
        std::thread::spawn(move || {
            let info = crate::infra::fetch_latest_release_info(Duration::from_millis(800));
            let Ok(info) = info else {
                let _ = tx.send(None);
                return;
            };

            let _ = crate::infra::save_update_check_cache(&state_dir, &info.tag);

            let current_ver = crate::infra::Version::parse(current);
            if current_ver.is_some_and(|current_ver| info.version > current_ver) {
                let _ = tx.send(Some(format_update_hint(current, &info.tag)));
            } else {
                let _ = tx.send(None);
            }
        });

        Self {
            cached_hint,
            rx: Some(rx),
            use_color,
        }
    }

    fn write_hint(&mut self, err: &mut impl Write) -> io::Result<()> {
        let mut hint: Option<String> = None;
        if let Some(rx) = self.rx.as_ref() {
            if let Ok(message) = rx.try_recv() {
                hint = message;
            }
        }

        if hint.is_none() {
            hint = self.cached_hint.clone();
        }

        if let Some(hint) = hint {
            let rendered = if self.use_color {
                paint_bright_green(&hint)
            } else {
                hint
            };
            let _ = write_line(err, &rendered)?;
        }
        Ok(())
    }
}

fn format_update_hint(current: &str, latest_tag: &str) -> String {
    format!("Update available: v{current} -> {latest_tag}. Run `ccbox update`.")
}

fn paint_bright_green(text: &str) -> String {
    format!("\x1b[92m{text}\x1b[0m")
}

fn should_color_stderr() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if std::env::var("TERM").is_ok_and(|term| term == "dumb") {
        return false;
    }
    io::stderr().is_terminal()
}

fn update_cache_path(state_dir: &Path) -> PathBuf {
    state_dir.join("update_check.json")
}

fn is_update_cache_stale(state_dir: &Path) -> bool {
    const STALE_AFTER: Duration = Duration::from_secs(12 * 60 * 60);

    let Ok(Some((checked_unix_ms, _latest_tag))) = crate::infra::load_update_check_cache(state_dir)
    else {
        return true;
    };

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let now_ms = i64::try_from(now_ms).unwrap_or(i64::MAX);
    if checked_unix_ms <= 0 {
        return true;
    }
    let delta_ms = now_ms.saturating_sub(checked_unix_ms);
    let delta = Duration::from_millis(u64::try_from(delta_ms).unwrap_or(u64::MAX));
    delta >= STALE_AFTER
        || !fs::metadata(update_cache_path(state_dir)).is_ok_and(|meta| meta.is_file())
}

pub fn run(command: CliCommand, sessions_dir: &Path) -> Result<(), CliRunError> {
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    let stderr = io::stderr();
    let mut err = io::BufWriter::new(stderr.lock());

    let mut update_notice = match command {
        CliCommand::Update => CliUpdateNotice {
            cached_hint: None,
            rx: None,
            use_color: should_color_stderr(),
        },
        _ => CliUpdateNotice::prepare(),
    };

    match command {
        CliCommand::Projects { engine } => {
            let (projects, warnings, notice) = load_projects(sessions_dir)?;
            for project in projects {
                let session_count = project
                    .sessions
                    .iter()
                    .filter(|session| engine.is_none_or(|engine| session.engine == engine))
                    .count();
                if session_count == 0 {
                    continue;
                }
                let line = format!(
                    "{}\t{}\t{}",
                    project.name,
                    project.project_path.display(),
                    session_count
                );
                if !write_line(&mut out, &line)? {
                    return Ok(());
                }
            }
            if let Some(notice) = notice {
                if !write_line(&mut err, &notice)? {
                    return Ok(());
                }
            }
            if warnings > 0 && !write_line(&mut err, &format!("warnings: {warnings}"))? {
                return Ok(());
            }
            update_notice.write_hint(&mut err)?;
            Ok(())
        }
        CliCommand::Sessions {
            project_path,
            engine,
            offset,
            limit,
            size,
        } => {
            let (projects, warnings, notice) = load_projects(sessions_dir)?;
            let project = select_project(projects, project_path)?;

            let sessions = project
                .sessions
                .iter()
                .filter(|session| engine.is_none_or(|engine| session.engine == engine))
                .skip(offset)
                .take(limit);
            for session in sessions {
                let line = if size {
                    format!(
                        "{}\t{}\t{}\t{}\t{}",
                        session.meta.started_at_rfc3339,
                        session.meta.id,
                        session.title,
                        session.file_size_bytes,
                        session.log_path.display(),
                    )
                } else {
                    format!(
                        "{}\t{}\t{}\t{}",
                        session.meta.started_at_rfc3339,
                        session.meta.id,
                        session.title,
                        session.log_path.display(),
                    )
                };
                if !write_line(&mut out, &line)? {
                    return Ok(());
                }
            }
            if let Some(notice) = notice {
                if !write_line(&mut err, &notice)? {
                    return Ok(());
                }
            }
            if warnings > 0 && !write_line(&mut err, &format!("warnings: {warnings}"))? {
                return Ok(());
            }
            update_notice.write_hint(&mut err)?;
            Ok(())
        }
        CliCommand::History {
            log_path,
            session_id,
            engine,
            offset,
            limit,
            full,
            size,
        } => {
            let log_path =
                resolve_history_log_path(sessions_dir, &mut err, log_path, session_id, engine)?;

            let file_size_bytes = fs::metadata(&log_path).ok().map(|meta| meta.len());
            let timeline = load_session_timeline(&log_path)?;
            let total_items = timeline.items.len();
            let mut printed = 0usize;
            for item in timeline.items.iter().skip(offset).take(limit) {
                printed = printed.saturating_add(1);
                if !print_timeline_item(&mut out, item, full)? {
                    return Ok(());
                }
            }
            if size {
                let bytes = file_size_bytes
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "?".to_string());
                let line = format!(
                    "stats:\tbytes={bytes}\titems_total={total_items}\titems_printed={printed}\toffset={offset}\tlimit={limit}"
                );
                if !write_line(&mut err, &line)? {
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
            update_notice.write_hint(&mut err)?;
            Ok(())
        }
        CliCommand::Skills {
            log_path,
            engine,
            json,
            full,
        } => {
            let log_path =
                resolve_history_log_path(sessions_dir, &mut err, log_path, None, engine)?;
            let timeline = load_session_timeline(&log_path)?;

            let spans = detect_skill_spans(&timeline.items);
            let loops = detect_skill_loops(&spans);
            let metrics = spans
                .iter()
                .map(|span| compute_skill_metrics(span, &timeline.items))
                .collect::<Vec<_>>();

            if json {
                let payload =
                    build_skills_json_payload(&log_path, &timeline.items, &spans, &loops, &metrics);
                let rendered =
                    serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string());
                if !write_line(&mut out, &rendered)? {
                    return Ok(());
                }
            } else {
                print_skills_human(&mut out, &timeline.items, &spans, &loops, &metrics, full)?;
            }

            if timeline.warnings > 0
                && !write_line(&mut err, &format!("warnings: {}", timeline.warnings))?
            {
                return Ok(());
            }
            if timeline.truncated && !write_line(&mut err, "truncated: true")? {
                return Ok(());
            }
            update_notice.write_hint(&mut err)?;
            Ok(())
        }
        CliCommand::Update => match crate::infra::self_update()? {
            Some(update) => {
                let line = format!("updated:\tv{}\t->\t{}", update.current, update.latest_tag);
                write_line(&mut out, &line)?;
                Ok(())
            }
            None => {
                let line = format!("up-to-date:\tv{}", env!("CARGO_PKG_VERSION"));
                write_line(&mut out, &line)?;
                Ok(())
            }
        },
    }
}

fn print_skills_human(
    out: &mut impl Write,
    items: &[TimelineItem],
    spans: &[crate::domain::SkillSpan],
    loops: &[crate::domain::SkillLoop],
    metrics: &[crate::domain::SkillMetrics],
    full: bool,
) -> Result<(), CliRunError> {
    if spans.is_empty() {
        write_line(out, "No skill spans detected.")?;
        return Ok(());
    }

    let loop_count = loops.len();
    let loop_suffix = if loop_count == 1 { "" } else { "s" };
    write_line(
        out,
        &format!(
            "Skill spans: {} detected, {loop_count} loop{loop_suffix}",
            spans.len()
        ),
    )?;
    write_line(out, "")?;

    write_line(
        out,
        "  #  Skill                     Depth  Tools  Duration  Output",
    )?;
    for (idx, span) in spans.iter().enumerate() {
        let m = metrics.get(idx).cloned().unwrap_or_default();
        let duration = format_duration_ms(m.duration_ms);
        let output = format!("{} chars", format_commas_usize(m.output_chars));
        let loop_note = loop_note_for_span(idx, loops);
        let name = if let Some(note) = loop_note {
            format!("{} {note}", span.name)
        } else {
            span.name.clone()
        };

        write_line(
            out,
            &format!(
                "  {:>2}  {:<24} {:>5}  {:>5}  {:>8}  {:>10}",
                idx.saturating_add(1),
                truncate_end(&name, 24),
                span.depth,
                m.tool_calls,
                duration,
                output,
            ),
        )?;

        if full {
            let tool_calls = tool_call_summaries_for_span(span, items);
            if !tool_calls.is_empty() {
                write_line(out, &format!("      tools: {}", tool_calls.join(", ")))?;
            }
        }
    }

    if !loops.is_empty() {
        write_line(out, "")?;
        write_line(out, "Loops:")?;
        for entry in loops {
            let count = entry.span_indices.len();
            let indices = entry
                .span_indices
                .iter()
                .map(|idx| idx.saturating_add(1).to_string())
                .collect::<Vec<_>>()
                .join(", ");
            write_line(
                out,
                &format!(
                    "  - \"{}\" invoked {count}x consecutively (spans {indices})",
                    entry.name
                ),
            )?;
        }
    }

    let session_ms = session_duration_ms(items);
    let total_skill_ms = total_top_level_skill_duration_ms(spans, metrics);
    if let Some(session_ms) = session_ms {
        if session_ms > 0 {
            let pct = (total_skill_ms as f64 / session_ms as f64) * 100.0;
            write_line(out, "")?;
            write_line(
                out,
                &format!(
                    "Skill time: {} / {} session ({:.1}%)",
                    format_duration_ms(Some(total_skill_ms)),
                    format_duration_ms(Some(session_ms)),
                    pct
                ),
            )?;
        }
    }

    Ok(())
}

fn build_skills_json_payload(
    log_path: &Path,
    items: &[TimelineItem],
    spans: &[crate::domain::SkillSpan],
    loops: &[crate::domain::SkillLoop],
    metrics: &[crate::domain::SkillMetrics],
) -> serde_json::Value {
    let session_id = infer_session_id(log_path);
    let session_duration_ms = session_duration_ms(items);
    let total_skill_duration_ms = total_top_level_skill_duration_ms(spans, metrics);
    let skill_time_pct = session_duration_ms.and_then(|session_ms| {
        if session_ms > 0 {
            Some((total_skill_duration_ms as f64 / session_ms as f64) * 100.0)
        } else {
            None
        }
    });

    let spans_json = spans
        .iter()
        .enumerate()
        .map(|(idx, span)| {
            let m = metrics.get(idx).cloned().unwrap_or_default();
            serde_json::json!({
                "name": span.name.clone(),
                "depth": span.depth,
                "start_idx": span.start_idx,
                "end_idx": span.end_idx,
                "tool_calls": m.tool_calls,
                "duration_ms": m.duration_ms,
                "output_chars": m.output_chars,
            })
        })
        .collect::<Vec<_>>();

    let loops_json = loops
        .iter()
        .map(|entry| {
            serde_json::json!({
                "name": entry.name.clone(),
                "count": entry.span_indices.len(),
                "span_indices": entry.span_indices.clone(),
            })
        })
        .collect::<Vec<_>>();

    serde_json::json!({
        "session_id": session_id,
        "spans": spans_json,
        "loops": loops_json,
        "summary": {
            "total_spans": spans.len(),
            "total_skill_duration_ms": total_skill_duration_ms,
            "session_duration_ms": session_duration_ms,
            "skill_time_pct": skill_time_pct,
        }
    })
}

fn infer_session_id(log_path: &Path) -> String {
    if let Some(id) = extract_session_id_from_session_meta(log_path) {
        return id;
    }

    log_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| log_path.display().to_string())
}

fn extract_session_id_from_session_meta(path: &Path) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let reader = io::BufReader::new(file);
    for line_result in reader.lines().take(200) {
        let line = line_result.ok()?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.contains("\"type\":\"session_meta\"")
            || trimmed.contains("\"type\": \"session_meta\"")
        {
            if let Ok(meta) = crate::domain::parse_session_meta_line(trimmed) {
                return Some(meta.id);
            }
        }
    }
    None
}

fn session_duration_ms(items: &[TimelineItem]) -> Option<i64> {
    let min_ts = items.iter().filter_map(|item| item.timestamp_ms).min()?;
    let max_ts = items.iter().filter_map(|item| item.timestamp_ms).max()?;
    max_ts.checked_sub(min_ts)
}

fn total_top_level_skill_duration_ms(
    spans: &[crate::domain::SkillSpan],
    metrics: &[crate::domain::SkillMetrics],
) -> i64 {
    let mut total: i64 = 0;
    for (span, metric) in spans.iter().zip(metrics.iter()) {
        if span.depth != 0 {
            continue;
        }
        if let Some(ms) = metric.duration_ms {
            if ms > 0 {
                total = total.saturating_add(ms);
            }
        }
    }
    total
}

fn format_duration_ms(ms: Option<i64>) -> String {
    let Some(ms) = ms else {
        return "-".to_string();
    };
    if ms < 0 {
        return "-".to_string();
    }
    let value = u64::try_from(ms).unwrap_or(0);
    format_duration(Duration::from_millis(value))
}

fn loop_note_for_span(span_idx: usize, loops: &[crate::domain::SkillLoop]) -> Option<String> {
    for entry in loops {
        if entry.span_indices.contains(&span_idx) {
            let count = entry.span_indices.len();
            return Some(format!("[loop x{count}]"));
        }
    }
    None
}

fn tool_call_summaries_for_span(
    span: &crate::domain::SkillSpan,
    items: &[TimelineItem],
) -> Vec<String> {
    if items.is_empty() || span.start_idx >= items.len() {
        return Vec::new();
    }

    let mut end_idx = span
        .end_idx
        .unwrap_or_else(|| items.len().saturating_sub(1));
    end_idx = end_idx.min(items.len().saturating_sub(1));
    if end_idx <= span.start_idx {
        return Vec::new();
    }

    let mut out: Vec<String> = Vec::new();
    for item in items
        .iter()
        .take(end_idx.saturating_add(1))
        .skip(span.start_idx.saturating_add(1))
    {
        if item.kind == TimelineItemKind::ToolCall {
            out.push(item.summary.clone());
        }
    }
    if out.len() > 12 {
        let remaining = out.len().saturating_sub(12);
        out.truncate(12);
        out.push(format!("... (+{remaining} more)"));
    }
    out
}

fn truncate_end(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    for ch in text.chars() {
        let next = format!("{out}{ch}");
        if unicode_width::UnicodeWidthStr::width(next.as_str()) > width {
            break;
        }
        out.push(ch);
    }
    out
}

fn format_duration(duration: Duration) -> String {
    let total_ms = duration.as_millis() as u64;
    if total_ms < 1000 {
        return format!("{total_ms}ms");
    }
    if total_ms < 10_000 {
        let seconds = total_ms / 1000;
        let tenths = (total_ms % 1000) / 100;
        return format!("{seconds}.{tenths}s");
    }
    if total_ms < 60_000 {
        let seconds = total_ms / 1000;
        return format!("{seconds}s");
    }
    if total_ms < 3_600_000 {
        let total_s = total_ms / 1000;
        let minutes = total_s / 60;
        let seconds = total_s % 60;
        return format!("{minutes}m {seconds:02}s");
    }

    let total_m = total_ms / 60_000;
    let hours = total_m / 60;
    let minutes = total_m % 60;
    format!("{hours}h {minutes:02}m")
}

fn format_commas_usize(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (count, ch) in digits.chars().rev().enumerate() {
        if count > 0 && count % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

fn load_projects(
    sessions_dir: &Path,
) -> Result<(Vec<ProjectSummary>, usize, Option<String>), CliRunError> {
    let output = scan_all_sessions(sessions_dir);
    Ok((
        index_projects(&output.sessions),
        output.warnings.get(),
        output.notice,
    ))
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

fn parse_usize_flag(flag: &str, value: &str) -> Result<usize, CliParseError> {
    value
        .parse::<usize>()
        .map_err(|_| CliParseError::InvalidFlagValue {
            flag: flag.to_string(),
            value: value.to_string(),
        })
}

fn parse_engine_flag(flag: &str, value: &str) -> Result<Option<SessionEngine>, CliParseError> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "all" => Ok(None),
        "codex" | "cx" => Ok(Some(SessionEngine::Codex)),
        "claude" | "cl" => Ok(Some(SessionEngine::Claude)),
        "gemini" | "gm" => Ok(Some(SessionEngine::Gemini)),
        "opencode" | "open-code" | "open_code" | "oc" => Ok(Some(SessionEngine::OpenCode)),
        other => Err(CliParseError::InvalidFlagValue {
            flag: flag.to_string(),
            value: other.to_string(),
        }),
    }
}

fn engine_flag_value(engine: SessionEngine) -> &'static str {
    match engine {
        SessionEngine::Codex => "codex",
        SessionEngine::Claude => "claude",
        SessionEngine::Gemini => "gemini",
        SessionEngine::OpenCode => "opencode",
    }
}

fn looks_like_path(value: &str) -> bool {
    value == "."
        || value == ".."
        || value.starts_with("./")
        || value.starts_with("../")
        || value.starts_with('~')
        || value.contains('/')
        || value.contains('\\')
        || value.ends_with(".jsonl")
        || value.ends_with(".json")
        || Path::new(value).exists()
}

fn resolve_history_log_path(
    sessions_dir: &Path,
    err: &mut impl Write,
    log_path: Option<PathBuf>,
    session_id: Option<String>,
    engine: Option<SessionEngine>,
) -> Result<PathBuf, CliRunError> {
    match (log_path, session_id) {
        (Some(path), None) => {
            if !fs::metadata(&path).is_ok_and(|meta| meta.is_dir()) {
                return Ok(path);
            }

            let (projects, warnings, notice) = load_projects(sessions_dir)?;
            write_scan_notice(err, notice, warnings)?;
            let project = select_project(projects, Some(path))?;
            let session = project
                .sessions
                .iter()
                .find(|session| engine.is_none_or(|engine| session.engine == engine))
                .cloned()
                .ok_or_else(|| match engine {
                    Some(engine) => CliRunError::ProjectHasNoSessionsForEngine {
                        project: project.project_path.display().to_string(),
                        engine: engine_flag_value(engine).to_string(),
                    },
                    None => CliRunError::ProjectHasNoSessions(
                        project.project_path.display().to_string(),
                    ),
                })?;
            Ok(crate::infra::prepare_session_log_path(&session)?)
        }
        (Some(path), Some(session_id)) => {
            if !fs::metadata(&path).is_ok_and(|meta| meta.is_dir()) {
                return Err(CliRunError::HistoryIdWithLogPath(
                    path.display().to_string(),
                ));
            }

            let (projects, warnings, notice) = load_projects(sessions_dir)?;
            write_scan_notice(err, notice, warnings)?;
            let project = select_project(projects, Some(path))?;
            let session = project
                .sessions
                .iter()
                .find(|session| {
                    session.meta.id == session_id
                        && engine.is_none_or(|engine| session.engine == engine)
                })
                .cloned()
                .ok_or_else(|| {
                    CliRunError::SessionNotFound(format!(
                        "{} (project {})",
                        session_id,
                        project.project_path.display()
                    ))
                })?;
            Ok(crate::infra::prepare_session_log_path(&session)?)
        }
        (None, None) => {
            let (projects, warnings, notice) = load_projects(sessions_dir)?;
            write_scan_notice(err, notice, warnings)?;
            let project = select_project(projects, None)?;
            let session = project
                .sessions
                .iter()
                .find(|session| engine.is_none_or(|engine| session.engine == engine))
                .cloned()
                .ok_or_else(|| match engine {
                    Some(engine) => CliRunError::ProjectHasNoSessionsForEngine {
                        project: project.project_path.display().to_string(),
                        engine: engine_flag_value(engine).to_string(),
                    },
                    None => CliRunError::ProjectHasNoSessions(
                        project.project_path.display().to_string(),
                    ),
                })?;
            Ok(crate::infra::prepare_session_log_path(&session)?)
        }
        (None, Some(session_id)) => {
            let (projects, warnings, notice) = load_projects(sessions_dir)?;
            write_scan_notice(err, notice, warnings)?;

            let matches = projects
                .iter()
                .flat_map(|project| project.sessions.iter())
                .filter(|session| {
                    session.meta.id == session_id
                        && engine.is_none_or(|engine| session.engine == engine)
                })
                .cloned()
                .collect::<Vec<_>>();
            if matches.is_empty() {
                return Err(CliRunError::SessionNotFound(session_id));
            }
            if matches.len() > 1 {
                return Err(CliRunError::SessionIdAmbiguous(session_id));
            }
            let session = matches
                .into_iter()
                .next()
                .ok_or_else(|| CliRunError::SessionNotFound(session_id))?;

            Ok(crate::infra::prepare_session_log_path(&session)?)
        }
    }
}

fn write_scan_notice(
    err: &mut impl Write,
    notice: Option<String>,
    warnings: usize,
) -> Result<(), CliRunError> {
    if let Some(notice) = notice {
        let _ = write_line(err, &notice)?;
    }
    if warnings > 0 {
        let _ = write_line(err, &format!("warnings: {warnings}"))?;
    }
    Ok(())
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
        assert_eq!(parsed, CliInvocation::Tui { engine: None });
    }

    #[test]
    fn parse_help_flag_wins() {
        let parsed = parse_invocation(&args(&["ccbox", "projects", "--help"])).expect("parse");
        assert_eq!(parsed, CliInvocation::PrintHelp);
    }

    #[test]
    fn parse_engine_flag_before_subcommand_applies_to_tui() {
        let parsed = parse_invocation(&args(&["ccbox", "--engine", "claude"])).expect("parse");
        assert_eq!(
            parsed,
            CliInvocation::Tui {
                engine: Some(SessionEngine::Claude)
            }
        );
    }

    #[test]
    fn parse_engine_flag_before_subcommand_applies_to_projects() {
        let parsed =
            parse_invocation(&args(&["ccbox", "--engine", "claude", "projects"])).expect("parse");
        assert_eq!(
            parsed,
            CliInvocation::Command(CliCommand::Projects {
                engine: Some(SessionEngine::Claude)
            })
        );
    }

    #[test]
    fn parse_projects_command() {
        let parsed = parse_invocation(&args(&["ccbox", "projects"])).expect("parse");
        assert_eq!(
            parsed,
            CliInvocation::Command(CliCommand::Projects { engine: None })
        );
    }

    #[test]
    fn parse_sessions_command_defaults_to_current_dir_project() {
        let parsed = parse_invocation(&args(&["ccbox", "sessions"])).expect("parse");
        assert_eq!(
            parsed,
            CliInvocation::Command(CliCommand::Sessions {
                project_path: None,
                engine: None,
                offset: 0,
                limit: DEFAULT_LIMIT,
                size: false
            })
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
                session_id: None,
                engine: None,
                offset: 0,
                limit: DEFAULT_LIMIT,
                full: true,
                size: false
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
                session_id: None,
                engine: None,
                offset: 0,
                limit: DEFAULT_LIMIT,
                full: false,
                size: false
            })
        );
    }

    #[test]
    fn parse_sessions_supports_limit_offset_and_size_flags() {
        let parsed = parse_invocation(&args(&[
            "ccbox", "sessions", "--limit", "25", "--offset", "5", "--size",
        ]))
        .expect("parse");
        assert_eq!(
            parsed,
            CliInvocation::Command(CliCommand::Sessions {
                project_path: None,
                engine: None,
                offset: 5,
                limit: 25,
                size: true
            })
        );
    }

    #[test]
    fn parse_history_supports_limit_offset_and_size_flags() {
        let parsed = parse_invocation(&args(&[
            "ccbox", "history", "--limit", "25", "--offset", "5", "--size",
        ]))
        .expect("parse");
        assert_eq!(
            parsed,
            CliInvocation::Command(CliCommand::History {
                log_path: None,
                session_id: None,
                engine: None,
                offset: 5,
                limit: 25,
                full: false,
                size: true
            })
        );
    }

    #[test]
    fn parse_history_accepts_session_id_as_positional_argument() {
        let parsed = parse_invocation(&args(&["ccbox", "history", "019c754c-abc"])).expect("parse");
        assert_eq!(
            parsed,
            CliInvocation::Command(CliCommand::History {
                log_path: None,
                session_id: Some("019c754c-abc".to_string()),
                engine: None,
                offset: 0,
                limit: DEFAULT_LIMIT,
                full: false,
                size: false
            })
        );
    }

    #[test]
    fn parse_history_accepts_session_id_flag() {
        let parsed =
            parse_invocation(&args(&["ccbox", "history", "--id", "019c754c-abc"])).expect("parse");
        assert_eq!(
            parsed,
            CliInvocation::Command(CliCommand::History {
                log_path: None,
                session_id: Some("019c754c-abc".to_string()),
                engine: None,
                offset: 0,
                limit: DEFAULT_LIMIT,
                full: false,
                size: false
            })
        );
    }

    #[test]
    fn parse_projects_accepts_engine_flag() {
        let parsed =
            parse_invocation(&args(&["ccbox", "projects", "--engine", "claude"])).expect("parse");
        assert_eq!(
            parsed,
            CliInvocation::Command(CliCommand::Projects {
                engine: Some(SessionEngine::Claude)
            })
        );
    }

    #[test]
    fn parse_sessions_accepts_engine_flag() {
        let parsed =
            parse_invocation(&args(&["ccbox", "sessions", "--engine", "gemini"])).expect("parse");
        assert_eq!(
            parsed,
            CliInvocation::Command(CliCommand::Sessions {
                project_path: None,
                engine: Some(SessionEngine::Gemini),
                offset: 0,
                limit: DEFAULT_LIMIT,
                size: false
            })
        );
    }

    #[test]
    fn parse_history_accepts_engine_flag() {
        let parsed =
            parse_invocation(&args(&["ccbox", "history", "--engine", "opencode"])).expect("parse");
        assert_eq!(
            parsed,
            CliInvocation::Command(CliCommand::History {
                log_path: None,
                session_id: None,
                engine: Some(SessionEngine::OpenCode),
                offset: 0,
                limit: DEFAULT_LIMIT,
                full: false,
                size: false
            })
        );
    }

    #[test]
    fn parse_skills_defaults_to_current_dir_session() {
        let parsed = parse_invocation(&args(&["ccbox", "skills"])).expect("parse");
        assert_eq!(
            parsed,
            CliInvocation::Command(CliCommand::Skills {
                log_path: None,
                engine: None,
                json: false,
                full: false
            })
        );
    }

    #[test]
    fn parse_skills_accepts_json_full_engine_and_path() {
        let parsed = parse_invocation(&args(&[
            "ccbox",
            "skills",
            "--json",
            "--full",
            "--engine",
            "claude",
            "/tmp/project",
        ]))
        .expect("parse");
        assert_eq!(
            parsed,
            CliInvocation::Command(CliCommand::Skills {
                log_path: Some(PathBuf::from("/tmp/project")),
                engine: Some(SessionEngine::Claude),
                json: true,
                full: true
            })
        );
    }
}
