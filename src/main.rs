mod app;
mod cli;
mod domain;
mod infra;
mod ui;

use crate::app::ProcessOutputKind;
use crate::app::{AppCommand, AppEvent, AppModel};
use crate::cli::CliInvocation;
use crate::domain::{compute_session_stats, make_session_summary, parse_session_meta_line};
use crate::infra::{
    KillProcessError, ProcessExit, ProcessManager, ProcessSignal, SessionIndex, WatchSignal,
    delete_session_logs, load_last_assistant_output, load_session_index, load_session_timeline,
    read_from_offset, read_tail, refresh_session_index, resolve_ccbox_state_dir,
    resolve_sessions_dir, save_session_index, scan_sessions_dir, watch_sessions_dir,
};
use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::size as terminal_size;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use crossterm::{ExecutableCommand, execute};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::{self, Stdout, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};
use std::time::{Duration, Instant};
use thiserror::Error;

#[derive(Debug, Error)]
enum MainError {
    #[error(transparent)]
    App(#[from] crate::app::AppError),

    #[error(transparent)]
    Cli(#[from] crate::cli::CliRunError),
}

#[derive(Clone, Debug)]
enum UpdateSignal {
    UpdateAvailable { latest_tag: String },
}

#[derive(Clone, Debug)]
struct SessionIndexRequest {
    sessions: Vec<crate::domain::SessionSummary>,
}

#[derive(Clone, Debug)]
enum SessionIndexSignal {
    Updated { index: Arc<SessionIndex> },
}

fn main() {
    if let Err(error) = run_main() {
        let mut err = io::stderr().lock();
        let _ = writeln!(err, "{error}");
        std::process::exit(1);
    }
}

fn run_main() -> Result<(), MainError> {
    let args = std::env::args().collect::<Vec<_>>();
    let invocation = match crate::cli::parse_invocation(&args) {
        Ok(invocation) => invocation,
        Err(error) => {
            let mut err = io::stderr().lock();
            let _ = writeln!(err, "{error}");
            let _ = writeln!(err);
            print_help();
            std::process::exit(2);
        }
    };

    match invocation {
        CliInvocation::PrintHelp => {
            print_help();
            Ok(())
        }
        CliInvocation::PrintVersion => {
            let mut out = io::stdout().lock();
            let _ = writeln!(out, "{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        CliInvocation::Tui => Ok(run_tui()?),
        CliInvocation::Command(command) => {
            let sessions_dir = resolve_sessions_dir().map_err(app::AppError::from)?;
            crate::cli::run(command, &sessions_dir)?;
            Ok(())
        }
    }
}

fn print_help() {
    let text = format!(
        "{name} — manage coding-agent sessions (Codex + Claude)\n\nUSAGE:\n  {name}                          Start the TUI\n  {name} projects                 List discovered projects\n  {name} sessions [project-path]  List sessions (defaults to current folder)\n  {name} history [log|project]    Print timeline (defaults to latest for current folder)\n  {name} update                   Self-update from GitHub Releases (macOS/Linux)\n  {name} --help | --version\n\nSESSIONS FLAGS:\n  --limit N   Max sessions to print (default: 10)\n  --offset N  Skip first N sessions (default: 0)\n  --size      Include file size bytes column\n\nHISTORY FLAGS:\n  --limit N   Max timeline items to print (default: 10)\n  --offset N  Skip first N timeline items (default: 0)\n  --full      Include full details (tool outputs, long messages)\n  --size      Print stats to stderr (bytes + item counts)\n\nOUTPUT:\n  projects: project_name<TAB>project_path<TAB>session_count\n  sessions: started_at<TAB>session_id<TAB>title<TAB>log_path  (with --size adds file_size_bytes before log_path)\n\nENV:\n  CODEX_SESSIONS_DIR  Override sessions dir (default: ~/.codex/sessions; Windows: %USERPROFILE%\\.codex\\sessions)\n",
        name = env!("CARGO_PKG_NAME")
    );
    let mut out = io::stdout().lock();
    let _ = write!(out, "{text}");
}

fn run_tui() -> Result<(), crate::app::AppError> {
    let sessions_dir = resolve_sessions_dir()?;
    let initial_data = match scan_sessions_dir(&sessions_dir) {
        Ok(output) => {
            app::build_index_from_sessions(sessions_dir.clone(), output.sessions, output.warnings)
        }
        Err(error) => app::AppData::from_load_error(sessions_dir.clone(), error.to_string()),
    };

    let mut model = AppModel::new(initial_data);
    let mut terminal = setup_terminal()?;
    if let Ok((width, height)) = terminal_size() {
        model = model.with_terminal_size(width, height);
    }
    let result = run(&mut terminal, &mut model);
    restore_terminal(&mut terminal)?;
    result
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>, app::AppError> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    let _ = stdout.execute(EnableBracketedPaste);
    let _ = stdout.execute(PushKeyboardEnhancementFlags(
        KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES,
    ));
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
) -> Result<(), app::AppError> {
    disable_raw_mode()?;
    let _ = execute!(
        terminal.backend_mut(),
        DisableBracketedPaste,
        PopKeyboardEnhancementFlags
    );
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn run(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    model: &mut AppModel,
) -> Result<(), app::AppError> {
    let (update_tx, update_rx) = channel::<UpdateSignal>();
    spawn_update_check(update_tx);

    let (session_index_req_tx, session_index_rx) = match resolve_ccbox_state_dir() {
        Ok(state_dir) => {
            match load_session_index(&state_dir) {
                Ok(index) => model.session_index = Arc::new(index),
                Err(error) => {
                    model.session_index = Arc::new(SessionIndex::default());
                    *model = model
                        .with_notice(Some(format!("Index cache reset (failed to load): {error}")));
                }
            }

            let (req_tx, req_rx) = channel::<SessionIndexRequest>();
            let (tx, rx) = channel::<SessionIndexSignal>();
            spawn_session_indexer(req_rx, tx, state_dir, model.session_index.clone());
            request_session_index_refresh(&req_tx, model);
            (Some(req_tx), Some(rx))
        }
        Err(error) => {
            model.session_index = Arc::new(SessionIndex::default());
            *model = model.with_notice(Some(format!("Index cache disabled: {error}")));
            (None, None)
        }
    };

    let watcher = match watch_sessions_dir(&model.data.sessions_dir) {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            *model = model.with_notice(Some(format!(
                "Auto-rescan disabled: {error} (Ctrl+R to rescan)"
            )));
            None
        }
    };
    let debounce = Duration::from_millis(900);
    let max_delay = Duration::from_secs(5);
    let mut pending_rescan = false;
    let mut first_change_at: Option<Instant> = None;
    let mut rescan_deadline: Option<Instant> = None;

    let (process_tx, process_rx) = channel::<ProcessSignal>();
    let mut process_manager = match ProcessManager::new(model.data.sessions_dir.clone(), process_tx)
    {
        Ok(manager) => Some(manager),
        Err(error) => {
            *model = model.with_notice(Some(format!("Process manager disabled: {error}")));
            None
        }
    };

    loop {
        while let Ok(signal) = update_rx.try_recv() {
            match signal {
                UpdateSignal::UpdateAvailable { latest_tag } => {
                    model.update_hint = Some(format!(
                        "Update available: v{} -> {latest_tag}. Run `ccbox update`.",
                        env!("CARGO_PKG_VERSION")
                    ));
                }
            }
        }

        if let Some(rx) = &session_index_rx {
            while let Ok(signal) = rx.try_recv() {
                match signal {
                    SessionIndexSignal::Updated { index } => {
                        model.session_index = index;
                        refresh_open_project_stats_overlay(model);
                    }
                }
            }
        }

        if let Some(watcher) = &watcher {
            while let Some(signal) = watcher.try_recv() {
                match signal {
                    WatchSignal::Changed => {
                        let now = Instant::now();
                        pending_rescan = true;
                        rescan_deadline = Some(now + debounce);
                        if first_change_at.is_none() {
                            first_change_at = Some(now);
                        }
                    }
                    WatchSignal::Error(message) => {
                        *model = model.with_notice(Some(format!("Watcher error: {message}")));
                    }
                }
            }
        }

        while let Ok(signal) = process_rx.try_recv() {
            apply_process_signal(model, signal);
        }

        if let Some(manager) = process_manager.as_mut() {
            for exit in manager.poll_exits() {
                apply_process_exit(model, exit);
            }
        }

        refresh_process_output_view(model);

        if pending_rescan {
            let now = Instant::now();
            let due_by_debounce = rescan_deadline.is_some_and(|due| now >= due);
            let due_by_max_delay =
                first_change_at.is_some_and(|first| now.duration_since(first) >= max_delay);
            if due_by_debounce || due_by_max_delay {
                pending_rescan = false;
                first_change_at = None;
                rescan_deadline = None;

                let sessions_dir = model.data.sessions_dir.clone();
                let new_data = match scan_sessions_dir(&sessions_dir) {
                    Ok(output) => app::build_index_from_sessions(
                        sessions_dir.clone(),
                        output.sessions,
                        output.warnings,
                    ),
                    Err(error) => app::AppData::from_load_error(sessions_dir, error.to_string()),
                };
                let prior_notice = model.notice.clone();
                let updated = model.with_data(new_data);
                let notice = if prior_notice.is_some() {
                    prior_notice
                } else {
                    Some("Auto-rescanned.".to_string())
                };
                *model = updated.with_notice(notice);
                refresh_open_project_stats_overlay(model);
                request_session_index_refresh_optional(&session_index_req_tx, model);
            }
        }

        terminal.draw(|frame| ui::render(frame, model))?;

        if event::poll(Duration::from_millis(200))? {
            match event::read()? {
                Event::Key(key) => {
                    let (next, command) = app::update(model.clone(), AppEvent::Key(key));
                    *model = next;
                    match command {
                        AppCommand::None => {}
                        AppCommand::Quit => return Ok(()),
                        AppCommand::Rescan => {
                            let sessions_dir = model.data.sessions_dir.clone();
                            let new_data = match scan_sessions_dir(&sessions_dir) {
                                Ok(output) => app::build_index_from_sessions(
                                    sessions_dir.clone(),
                                    output.sessions,
                                    output.warnings,
                                ),
                                Err(error) => {
                                    app::AppData::from_load_error(sessions_dir, error.to_string())
                                }
                            };
                            *model = model.with_data(new_data);
                            refresh_open_project_stats_overlay(model);
                            request_session_index_refresh_optional(&session_index_req_tx, model);
                        }
                        AppCommand::OpenSessionDetail {
                            from_sessions,
                            session,
                        } => match load_session_timeline(&session.log_path) {
                            Ok(timeline) => {
                                *model = model.open_session_detail(
                                    from_sessions,
                                    session,
                                    timeline.items,
                                    timeline.turn_contexts,
                                    timeline.warnings,
                                    timeline.truncated,
                                );
                            }
                            Err(error) => {
                                *model = model
                                    .with_notice(Some(format!("Failed to load session: {error}")));
                            }
                        },
                        AppCommand::OpenSessionStats { session } => {
                            match load_session_timeline(&session.log_path) {
                                Ok(timeline) => {
                                    let stats =
                                        compute_session_stats(&session.meta, &timeline.items);
                                    model.session_stats_overlay =
                                        Some(crate::app::SessionStatsOverlay {
                                            session,
                                            stats,
                                            scroll: 0,
                                        });
                                    model.help_open = false;
                                    model.system_menu = None;
                                }
                                Err(error) => {
                                    *model = model.with_notice(Some(format!(
                                        "Failed to load stats: {error}"
                                    )));
                                }
                            }
                        }
                        AppCommand::OpenSessionResultPreview { session } => {
                            match load_last_assistant_output(&session.log_path) {
                                Ok(result) => {
                                    let output = result.output.unwrap_or_else(|| {
                                        "(No assistant output found.)".to_string()
                                    });
                                    model.session_result_preview =
                                        Some(crate::app::SessionResultPreviewOverlay {
                                            session_title: session.title,
                                            output,
                                            scroll: 0,
                                        });
                                }
                                Err(error) => {
                                    *model = model.with_notice(Some(format!(
                                        "Failed to load result: {error}"
                                    )));
                                }
                            }
                        }
                        AppCommand::DeleteProjectLogs { project_path } => {
                            let Some(project) = model
                                .data
                                .projects
                                .iter()
                                .find(|project| project.project_path == project_path)
                            else {
                                *model = model.with_notice(Some("Project not found.".to_string()));
                                continue;
                            };

                            let log_paths = project
                                .sessions
                                .iter()
                                .map(|session| session.log_path.clone())
                                .collect::<Vec<_>>();
                            let outcome = delete_session_logs(&model.data.sessions_dir, &log_paths);

                            pending_rescan = false;
                            first_change_at = None;
                            rescan_deadline = None;

                            let sessions_dir = model.data.sessions_dir.clone();
                            let new_data = match scan_sessions_dir(&sessions_dir) {
                                Ok(output) => app::build_index_from_sessions(
                                    sessions_dir.clone(),
                                    output.sessions,
                                    output.warnings,
                                ),
                                Err(error) => {
                                    app::AppData::from_load_error(sessions_dir, error.to_string())
                                }
                            };
                            *model = model.with_data(new_data);
                            refresh_open_project_stats_overlay(model);
                            request_session_index_refresh_optional(&session_index_req_tx, model);

                            let mut message =
                                format!("Deleted {} session log(s).", outcome.deleted);
                            if outcome.failed > 0 {
                                message.push_str(&format!(" {} failed.", outcome.failed));
                            }
                            if outcome.skipped_outside_sessions_dir > 0 {
                                message.push_str(&format!(
                                    " {} skipped (outside sessions dir).",
                                    outcome.skipped_outside_sessions_dir
                                ));
                            }
                            *model = model.with_notice(Some(message));
                        }
                        AppCommand::DeleteSessionLog { log_path } => {
                            let outcome =
                                delete_session_logs(&model.data.sessions_dir, &[log_path.clone()]);

                            pending_rescan = false;
                            first_change_at = None;
                            rescan_deadline = None;

                            let sessions_dir = model.data.sessions_dir.clone();
                            let new_data = match scan_sessions_dir(&sessions_dir) {
                                Ok(output) => app::build_index_from_sessions(
                                    sessions_dir.clone(),
                                    output.sessions,
                                    output.warnings,
                                ),
                                Err(error) => {
                                    app::AppData::from_load_error(sessions_dir, error.to_string())
                                }
                            };
                            *model = model.with_data(new_data);
                            refresh_open_project_stats_overlay(model);
                            request_session_index_refresh_optional(&session_index_req_tx, model);

                            let mut message =
                                format!("Deleted {} session log(s).", outcome.deleted);
                            if outcome.failed > 0 {
                                message.push_str(&format!(" {} failed.", outcome.failed));
                            }
                            if outcome.skipped_outside_sessions_dir > 0 {
                                message.push_str(&format!(
                                    " {} skipped (outside sessions dir).",
                                    outcome.skipped_outside_sessions_dir
                                ));
                            }
                            *model = model.with_notice(Some(message));
                        }
                        AppCommand::SpawnAgentSession {
                            engine,
                            project_path,
                            prompt,
                        } => {
                            let Some(manager) = process_manager.as_mut() else {
                                *model = model
                                    .with_notice(Some("Process spawning is disabled.".to_string()));
                                continue;
                            };

                            match manager.spawn_agent_process(engine, &project_path, &prompt) {
                                Ok(spawned) => {
                                    model.processes.push(crate::app::ProcessInfo {
                                        id: spawned.id.clone(),
                                        pid: spawned.pid,
                                        engine: spawned.engine,
                                        project_path: spawned.project_path.clone(),
                                        prompt_preview: spawned.prompt_preview.clone(),
                                        started_at: spawned.started_at,
                                        status: crate::app::ProcessStatus::Running,
                                        session_id: None,
                                        session_log_path: None,
                                        stdout_path: spawned.stdout_path.clone(),
                                        stderr_path: spawned.stderr_path.clone(),
                                        log_path: spawned.log_path.clone(),
                                    });
                                    *model = model.with_notice(Some(format!(
                                        "Spawned {} ({})",
                                        spawned.engine.label(),
                                        spawned.id
                                    )));
                                }
                                Err(error) => {
                                    *model = model.with_notice(Some(format!(
                                        "Failed to spawn process: {error}"
                                    )));
                                }
                            }
                        }
                        AppCommand::KillProcess { process_id } => {
                            let Some(manager) = process_manager.as_mut() else {
                                *model = model
                                    .with_notice(Some("Process manager disabled.".to_string()));
                                continue;
                            };

                            match manager.kill(&process_id) {
                                Ok(()) => {
                                    if let Some(process) = model
                                        .processes
                                        .iter_mut()
                                        .find(|process| process.id == process_id)
                                    {
                                        process.status = crate::app::ProcessStatus::Killed;
                                    }
                                    *model =
                                        model.with_notice(Some(format!("Killed {process_id}.")));
                                }
                                Err(KillProcessError::NotFound) => {
                                    *model = model.with_notice(Some(format!(
                                        "Process not running: {process_id}"
                                    )));
                                }
                                Err(error) => {
                                    *model = model.with_notice(Some(format!(
                                        "Failed to kill {process_id}: {error}"
                                    )));
                                }
                            }
                        }
                        AppCommand::OpenProcessOutput { process_id, kind } => {
                            open_process_output_view(model, &process_id, kind);
                        }
                        AppCommand::OpenSessionDetailByLogPath {
                            project_path,
                            log_path,
                        } => {
                            open_session_detail_by_log_path(model, project_path, log_path);
                        }
                    }
                }
                Event::Paste(text) => {
                    let (next, _command) = app::update(model.clone(), AppEvent::Paste(text));
                    *model = next;
                }
                Event::Resize(width, height) => {
                    *model = model.with_terminal_size(width, height);
                }
                _ => {}
            }
        }
    }
}

fn spawn_update_check(tx: Sender<UpdateSignal>) {
    std::thread::spawn(move || {
        let current = env!("CARGO_PKG_VERSION");
        let Ok(Some(update)) = crate::infra::check_for_update(current) else {
            return;
        };
        let _ = tx.send(UpdateSignal::UpdateAvailable {
            latest_tag: update.latest_tag,
        });
    });
}

fn spawn_session_indexer(
    rx: std::sync::mpsc::Receiver<SessionIndexRequest>,
    tx: Sender<SessionIndexSignal>,
    state_dir: PathBuf,
    initial: Arc<SessionIndex>,
) {
    std::thread::spawn(move || {
        let mut current = initial;
        loop {
            let request = match rx.recv() {
                Ok(request) => request,
                Err(_) => return,
            };

            let mut sessions = request.sessions;
            while let Ok(next) = rx.try_recv() {
                sessions = next.sessions;
            }

            let next = Arc::new(refresh_session_index(&sessions, current.as_ref()));
            let _ = save_session_index(&state_dir, next.as_ref());
            current = next.clone();
            let _ = tx.send(SessionIndexSignal::Updated { index: next });
        }
    });
}

fn request_session_index_refresh(tx: &Sender<SessionIndexRequest>, model: &AppModel) {
    let sessions = model
        .data
        .projects
        .iter()
        .flat_map(|project| project.sessions.iter().cloned())
        .collect::<Vec<_>>();
    let _ = tx.send(SessionIndexRequest { sessions });
}

fn request_session_index_refresh_optional(
    tx: &Option<Sender<SessionIndexRequest>>,
    model: &AppModel,
) {
    let Some(tx) = tx else {
        return;
    };
    request_session_index_refresh(tx, model);
}

fn refresh_open_project_stats_overlay(model: &mut AppModel) {
    let Some(current) = model.project_stats_overlay.clone() else {
        return;
    };
    let Some(project) = model
        .data
        .projects
        .iter()
        .find(|project| project.project_path == current.project_path)
    else {
        model.project_stats_overlay = None;
        return;
    };

    let mut refreshed =
        crate::app::ProjectStatsOverlay::from_project(project, model.session_index.as_ref());
    refreshed.scroll = current.scroll;
    model.project_stats_overlay = Some(refreshed);
}

fn apply_process_signal(model: &mut AppModel, signal: ProcessSignal) {
    match signal {
        ProcessSignal::SessionMeta {
            process_id,
            session_id,
        } => {
            if let Some(process) = model
                .processes
                .iter_mut()
                .find(|process| process.id == process_id)
            {
                process.session_id = Some(session_id);
            }
        }
        ProcessSignal::SessionLogPath {
            process_id,
            log_path,
        } => {
            if let Some(process) = model
                .processes
                .iter_mut()
                .find(|process| process.id == process_id)
            {
                process.session_log_path = Some(log_path);
            }
        }
    }
}

fn apply_process_exit(model: &mut AppModel, exit: ProcessExit) {
    if let Some(process) = model
        .processes
        .iter_mut()
        .find(|process| process.id == exit.process_id)
    {
        if process.status == crate::app::ProcessStatus::Running {
            process.status = crate::app::ProcessStatus::Exited(exit.exit_code);
        }
    }
}

fn open_process_output_view(model: &mut AppModel, process_id: &str, kind: ProcessOutputKind) {
    let Some(process) = model
        .processes
        .iter()
        .find(|process| process.id == process_id)
        .cloned()
    else {
        *model = model.with_notice(Some("Process not found.".to_string()));
        return;
    };

    let file_path = match kind {
        ProcessOutputKind::Stdout => process.stdout_path.clone(),
        ProcessOutputKind::Stderr => process.stderr_path.clone(),
        ProcessOutputKind::Log => process.log_path.clone(),
    };

    let return_to = match &model.view {
        crate::app::View::ProcessOutput(output) => output.return_to.clone(),
        _ => Box::new(model.view.clone()),
    };

    let (buffer, file_offset) = match read_tail(&file_path, 200_000) {
        Ok((text, offset)) => (text, offset),
        Err(error) => {
            let message = format!("Failed to read output: {error}");
            *model = model.with_notice(Some(message));
            (String::new(), 0)
        }
    };

    model.view = crate::app::View::ProcessOutput(crate::app::ProcessOutputView {
        return_to,
        process_id: process.id,
        kind,
        file_path,
        buffer: Arc::new(buffer),
        file_offset,
        scroll: 0,
    });
}

fn refresh_process_output_view(model: &mut AppModel) {
    let crate::app::View::ProcessOutput(output_view) = &mut model.view else {
        return;
    };

    let Ok((delta, next_offset)) =
        read_from_offset(&output_view.file_path, output_view.file_offset, 16_384)
    else {
        return;
    };
    if delta.is_empty() {
        return;
    }

    let mut next = String::new();
    next.push_str(output_view.buffer.as_str());
    next.push_str(&delta);
    next = trim_string_to_max_bytes(next, 260_000);

    output_view.buffer = Arc::new(next);
    output_view.file_offset = next_offset;
}

fn trim_string_to_max_bytes(mut value: String, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value;
    }

    let mut start = value.len().saturating_sub(max_bytes);
    while start < value.len() && !value.is_char_boundary(start) {
        start = start.saturating_add(1);
    }
    if start >= value.len() {
        return value;
    }
    value.split_off(start)
}

fn open_session_detail_by_log_path(model: &mut AppModel, project_path: PathBuf, log_path: PathBuf) {
    let session = model
        .data
        .projects
        .iter()
        .find(|project| project.project_path == project_path)
        .and_then(|project| project.sessions.iter().find(|s| s.log_path == log_path))
        .cloned()
        .or_else(|| build_session_summary_from_log_path(&log_path));

    let Some(session) = session else {
        *model = model.with_notice(Some("Failed to load session meta.".to_string()));
        return;
    };

    match load_session_timeline(&log_path) {
        Ok(timeline) => {
            let session_count = model
                .data
                .projects
                .iter()
                .find(|project| project.project_path == project_path)
                .map(|project| project.sessions.len())
                .unwrap_or(0);
            let from_sessions = crate::app::SessionsView::new(project_path, session_count);
            *model = model.open_session_detail(
                from_sessions,
                session,
                timeline.items,
                timeline.turn_contexts,
                timeline.warnings,
                timeline.truncated,
            );
        }
        Err(error) => {
            *model = model.with_notice(Some(format!("Failed to load session: {error}")));
        }
    }
}

fn build_session_summary_from_log_path(
    log_path: &std::path::Path,
) -> Option<crate::domain::SessionSummary> {
    use std::io::BufRead;
    let file = std::fs::File::open(log_path).ok()?;
    let metadata = file.metadata().ok()?;
    let file_size_bytes = metadata.len();
    let file_modified = metadata.modified().ok();

    let mut reader = std::io::BufReader::new(file);
    let mut first_line = String::new();
    let bytes = reader.read_line(&mut first_line).ok()?;
    if bytes == 0 {
        return None;
    }

    let meta = parse_session_meta_line(first_line.trim_end()).ok()?;
    let title = log_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "(spawned)".to_string());

    Some(make_session_summary(
        meta,
        log_path.to_path_buf(),
        title,
        file_size_bytes,
        file_modified,
    ))
}
