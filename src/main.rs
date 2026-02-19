mod app;
mod domain;
mod infra;
mod ui;

use crate::app::{AppCommand, AppEvent, AppModel};
use crate::app::ProcessOutputKind;
use crate::domain::{make_session_summary, parse_session_meta_line};
use crate::infra::{
    KillProcessError, ProcessExit, ProcessManager, ProcessSignal, WatchSignal,
    delete_session_logs, load_last_assistant_output, load_session_timeline, read_from_offset,
    read_tail, resolve_sessions_dir, scan_sessions_dir, watch_sessions_dir,
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
use std::io::{self, Stdout};
use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn main() -> Result<(), app::AppError> {
    if should_print_help() {
        print_help();
        return Ok(());
    }
    if should_print_version() {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

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

fn should_print_help() -> bool {
    std::env::args().any(|arg| arg == "--help" || arg == "-h")
}

fn should_print_version() -> bool {
    std::env::args().any(|arg| arg == "--version" || arg == "-V")
}

fn print_help() {
    println!(
        "{name} — manage coding-agent sessions (Codex + Claude)\n\nUSAGE:\n  {name}\n  {name} --help\n  {name} --version\n\nENV:\n  CODEX_SESSIONS_DIR  Override the sessions directory (default: $HOME/.codex/sessions)\n",
        name = env!("CARGO_PKG_NAME")
    );
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
            *model = model.with_notice(Some(format!(
                "Process manager disabled: {error}"
            )));
            None
        }
    };

    loop {
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
                        AppCommand::OpenSessionResultPreview { session } => {
                            match load_last_assistant_output(&session.log_path) {
                                Ok(result) => {
                                    let output = result
                                        .output
                                        .unwrap_or_else(|| "(No assistant output found.)".to_string());
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
                                Err(error) => app::AppData::from_load_error(
                                    sessions_dir,
                                    error.to_string(),
                                ),
                            };
                            *model = model.with_data(new_data);

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
                                Err(error) => app::AppData::from_load_error(
                                    sessions_dir,
                                    error.to_string(),
                                ),
                            };
                            *model = model.with_data(new_data);

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
                                *model = model.with_notice(Some(
                                    "Process spawning is disabled.".to_string(),
                                ));
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
                                *model =
                                    model.with_notice(Some("Process manager disabled.".to_string()));
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
                                    *model = model.with_notice(Some(format!("Killed {process_id}.")));
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
        ProcessSignal::SessionLogPath { process_id, log_path } => {
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

    let Ok((delta, next_offset)) = read_from_offset(&output_view.file_path, output_view.file_offset, 16_384) else {
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
            let from_sessions = crate::app::SessionsView::new(project_path);
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

fn build_session_summary_from_log_path(log_path: &std::path::Path) -> Option<crate::domain::SessionSummary> {
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
