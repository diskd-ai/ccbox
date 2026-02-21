mod app;
mod cli;
mod domain;
mod infra;
mod ui;

use crate::app::ProcessOutputKind;
use crate::app::{AppCommand, AppEvent, AppModel};
use crate::cli::CliInvocation;
use crate::domain::{
    compute_session_stats, derive_task_title, format_task_spawn_prompt, make_session_summary,
    parse_session_meta_line,
};
use crate::infra::{
    AttachTtyError, KillProcessError, ProcessExit, ProcessManager, ProcessSignal, ResizeTtyError,
    ResolveClaudeProjectsDirError, ResolveGeminiRootDirError, ResolveOpenCodeDbPathError,
    SessionIndex, TaskStore, WatchSignal, WriteTtyError, delete_session_logs,
    fork_codex_session_log_at_cut, load_last_assistant_output, load_session_index,
    load_session_timeline, read_from_offset, read_tail, refresh_session_index,
    resolve_ccbox_state_dir, resolve_claude_projects_dir, resolve_gemini_root_dir,
    resolve_opencode_db_path, resolve_sessions_dir, save_session_index, scan_all_sessions,
    set_session_alias, set_session_project, watch_session_file, watch_sessions_dir,
    watch_sqlite_db_family,
};
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyEventKind, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
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

#[derive(Debug)]
enum SessionDetailTimelineSignal {
    Loaded {
        log_path: PathBuf,
        result: Result<crate::domain::SessionTimeline, String>,
    },
}

#[derive(Debug)]
enum SessionsDirScanSignal {
    Scanned {
        data: crate::app::AppData,
        notice: Option<String>,
    },
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
        CliInvocation::Tui { engine } => Ok(run_tui(engine)?),
        CliInvocation::Command(command) => {
            let sessions_dir = resolve_sessions_dir().map_err(app::AppError::from)?;
            crate::cli::run(command, &sessions_dir)?;
            Ok(())
        }
    }
}

fn print_help() {
    let text = format!(
        "{name} — manage coding-agent sessions (Codex + Claude + Gemini + OpenCode)\n\nUSAGE:\n  {name} [--engine ENGINE]                Start the TUI\n  {name} projects [--engine ENGINE]       List discovered projects\n  {name} sessions [project-path] [--engine ENGINE]  List sessions (defaults to current folder)\n  {name} history [log|project] [session-id] [--engine ENGINE]  Print timeline (defaults to latest for current folder)\n  {name} skills [log|project] [session-id] [--engine ENGINE] [--json] [--full]  Analyze skill spans (defaults to latest for current folder)\n  {name} update                           Self-update from GitHub Releases (macOS/Linux)\n  {name} --help | --version\n\nENGINE:\n  --engine NAME  Filter by engine: all|codex|claude|gemini|opencode (default: all)\n\nSESSIONS FLAGS:\n  --limit N      Max sessions to print (default: 10)\n  --offset N     Skip first N sessions (default: 0)\n  --size         Include file size bytes column\n\nHISTORY FLAGS:\n  --limit N      Max timeline items to print (default: 10)\n  --offset N     Skip first N timeline items (default: 0)\n  --id ID        Select a session id (positional session-id also supported)\n  --full         Include full details (tool outputs, long messages)\n  --size         Print stats to stderr (bytes + item counts)\n\nSKILLS FLAGS:\n  --id ID        Select a session id (positional session-id also supported)\n  --json         Output structured JSON\n  --full         Include per-span tool call summaries\n\nOUTPUT:\n  projects: project_name<TAB>project_path<TAB>session_count\n  sessions: started_at<TAB>session_id<TAB>title<TAB>log_path  (with --size adds file_size_bytes before log_path)\n\nENV:\n  CODEX_SESSIONS_DIR    Override Codex sessions dir (default: ~/.codex/sessions; Windows: %USERPROFILE%\\.codex\\sessions)\n  CLAUDE_PROJECTS_DIR   Override Claude projects dir (default: ~/.claude/projects)\n",
        name = env!("CARGO_PKG_NAME")
    );
    let mut out = io::stdout().lock();
    let _ = write!(out, "{text}");
}

fn run_tui(engine: Option<crate::domain::SessionEngine>) -> Result<(), crate::app::AppError> {
    let sessions_dir = resolve_sessions_dir()?;
    let scan = scan_all_sessions(&sessions_dir);
    let initial_data =
        app::build_index_from_sessions(sessions_dir.clone(), scan.sessions, scan.warnings);
    let mut model = AppModel::new(initial_data).with_notice(scan.notice);
    if let Some(engine) = engine {
        let filter = match engine {
            crate::domain::SessionEngine::Codex => crate::app::EngineFilter::Codex,
            crate::domain::SessionEngine::Claude => crate::app::EngineFilter::Claude,
            crate::domain::SessionEngine::Gemini => crate::app::EngineFilter::Gemini,
            crate::domain::SessionEngine::OpenCode => crate::app::EngineFilter::OpenCode,
        };
        model = model.with_engine_filter(filter);
    }
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
    let _ = stdout.execute(EnableMouseCapture);
    let keyboard_flags = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES;
    let _ = stdout.execute(PushKeyboardEnhancementFlags(keyboard_flags));
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
        DisableMouseCapture,
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

    let codex_watcher = match watch_sessions_dir(&model.data.sessions_dir) {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            *model = model.with_notice(Some(format!(
                "Auto-rescan disabled: {error} (Ctrl+R to rescan)"
            )));
            None
        }
    };

    let claude_watcher = match resolve_claude_projects_dir() {
        Ok(dir) => {
            if dir.exists() {
                match watch_sessions_dir(&dir) {
                    Ok(watcher) => Some(watcher),
                    Err(error) => {
                        *model = model.with_notice(Some(format!(
                            "Claude auto-rescan disabled: {error} (Ctrl+R to rescan)"
                        )));
                        None
                    }
                }
            } else {
                None
            }
        }
        Err(ResolveClaudeProjectsDirError::HomeDirNotFound) => None,
    };

    let gemini_watcher = match resolve_gemini_root_dir() {
        Ok(root) => {
            let tmp_dir = root.join("tmp");
            if tmp_dir.exists() {
                match watch_sessions_dir(&tmp_dir) {
                    Ok(watcher) => Some(watcher),
                    Err(error) => {
                        *model = model.with_notice(Some(format!(
                            "Gemini auto-rescan disabled: {error} (Ctrl+R to rescan)"
                        )));
                        None
                    }
                }
            } else {
                None
            }
        }
        Err(ResolveGeminiRootDirError::HomeDirNotFound) => None,
    };

    let opencode_watcher = match resolve_opencode_db_path() {
        Ok(db_path) => {
            if db_path.is_file() {
                match watch_sqlite_db_family(&db_path) {
                    Ok(watcher) => watcher,
                    Err(error) => {
                        *model = model.with_notice(Some(format!(
                            "OpenCode auto-rescan disabled: {error} (Ctrl+R to rescan)"
                        )));
                        None
                    }
                }
            } else {
                None
            }
        }
        Err(ResolveOpenCodeDbPathError::HomeDirNotFound) => None,
    };
    let (sessions_scan_tx, sessions_scan_rx) = channel::<SessionsDirScanSignal>();
    let mut sessions_scan_in_flight = false;
    let debounce = Duration::from_millis(900);
    let max_delay = Duration::from_secs(5);
    let mut pending_rescan = false;
    let mut first_change_at: Option<Instant> = None;
    let mut rescan_deadline: Option<Instant> = None;

    let (session_detail_tx, session_detail_rx) = channel::<SessionDetailTimelineSignal>();
    let session_detail_debounce = Duration::from_millis(450);
    let session_detail_max_delay = Duration::from_secs(3);
    let mut session_detail_watcher: Option<crate::infra::SessionFileWatcher> = None;
    let mut session_detail_watcher_path: Option<PathBuf> = None;
    let mut pending_session_detail_reload = false;
    let mut session_detail_first_change_at: Option<Instant> = None;
    let mut session_detail_reload_deadline: Option<Instant> = None;
    let mut session_detail_reload_in_flight_for: Option<PathBuf> = None;

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

        while let Ok(signal) = sessions_scan_rx.try_recv() {
            match signal {
                SessionsDirScanSignal::Scanned { data, notice } => {
                    sessions_scan_in_flight = false;
                    let prior_notice = model.notice.clone();
                    let updated = model.with_data(data);
                    let next_notice = prior_notice
                        .or(notice)
                        .or_else(|| Some("Auto-rescanned.".to_string()));
                    *model = updated.with_notice(next_notice);
                    refresh_open_project_stats_overlay(model);
                    request_session_index_refresh_optional(&session_index_req_tx, model);
                }
            }
        }

        if let Some(rx) = &session_index_rx {
            while let Ok(signal) = rx.try_recv() {
                match signal {
                    SessionIndexSignal::Updated { index } => {
                        *model = model.with_session_index(index);
                        refresh_open_project_stats_overlay(model);
                    }
                }
            }
        }

        while let Ok(signal) = session_detail_rx.try_recv() {
            match signal {
                SessionDetailTimelineSignal::Loaded { log_path, result } => {
                    if session_detail_reload_in_flight_for
                        .as_ref()
                        .is_some_and(|path| path == &log_path)
                    {
                        session_detail_reload_in_flight_for = None;
                    }

                    match result {
                        Ok(timeline) => {
                            refresh_open_session_detail(model, &log_path, timeline);
                        }
                        Err(error) => {
                            if is_session_detail_open_for_log_path(model, &log_path) {
                                *model = model.with_notice(Some(format!(
                                    "Failed to refresh session timeline: {error}"
                                )));
                            }
                        }
                    }
                }
            }
        }

        ensure_session_detail_watcher(
            model,
            &mut session_detail_watcher,
            &mut session_detail_watcher_path,
            &mut pending_session_detail_reload,
            &mut session_detail_first_change_at,
            &mut session_detail_reload_deadline,
            &mut session_detail_reload_in_flight_for,
        );

        if let Some(watcher) = &session_detail_watcher {
            while let Some(signal) = watcher.try_recv() {
                match signal {
                    WatchSignal::Changed => {
                        let now = Instant::now();
                        pending_session_detail_reload = true;
                        session_detail_reload_deadline = Some(now + session_detail_debounce);
                        if session_detail_first_change_at.is_none() {
                            session_detail_first_change_at = Some(now);
                        }
                    }
                    WatchSignal::Error(message) => {
                        if session_detail_watcher_path.is_some() {
                            *model = model
                                .with_notice(Some(format!("Session watcher error: {message}")));
                        }
                    }
                }
            }
        }

        if pending_session_detail_reload && session_detail_reload_in_flight_for.is_none() {
            let now = Instant::now();
            let due_by_debounce = session_detail_reload_deadline.is_some_and(|due| now >= due);
            let due_by_max_delay = session_detail_first_change_at
                .is_some_and(|first| now.duration_since(first) >= session_detail_max_delay);
            if due_by_debounce || due_by_max_delay {
                pending_session_detail_reload = false;
                session_detail_first_change_at = None;
                session_detail_reload_deadline = None;

                if let crate::app::View::SessionDetail(detail_view) = &model.view {
                    let session = detail_view.session.clone();
                    session_detail_reload_in_flight_for = Some(session.log_path.clone());
                    let tx = session_detail_tx.clone();
                    std::thread::spawn(move || {
                        let log_path = match crate::infra::prepare_session_log_path(&session) {
                            Ok(path) => path,
                            Err(error) => {
                                let _ = tx.send(SessionDetailTimelineSignal::Loaded {
                                    log_path: session.log_path.clone(),
                                    result: Err(error.to_string()),
                                });
                                return;
                            }
                        };

                        let result = match load_session_timeline(&log_path) {
                            Ok(timeline) => Ok(timeline),
                            Err(error) => Err(error.to_string()),
                        };
                        let _ = tx.send(SessionDetailTimelineSignal::Loaded { log_path, result });
                    });
                }
            }
        }

        if let Some(watcher) = &codex_watcher {
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

        if let Some(watcher) = &claude_watcher {
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
                        *model =
                            model.with_notice(Some(format!("Claude watcher error: {message}")));
                    }
                }
            }
        }

        if let Some(watcher) = &gemini_watcher {
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
                        *model =
                            model.with_notice(Some(format!("Gemini watcher error: {message}")));
                    }
                }
            }
        }

        if let Some(watcher) = &opencode_watcher {
            while let Some(signal) = watcher.try_recv() {
                match signal {
                    WatchSignal::Changed => {
                        let now = Instant::now();
                        pending_rescan = true;
                        rescan_deadline = Some(now + debounce);
                        if first_change_at.is_none() {
                            first_change_at = Some(now);
                        }

                        if matches!(
                            &model.view,
                            crate::app::View::SessionDetail(detail_view)
                                if detail_view.session.engine == crate::domain::SessionEngine::OpenCode
                        ) {
                            pending_session_detail_reload = true;
                            session_detail_reload_deadline = Some(now + session_detail_debounce);
                            if session_detail_first_change_at.is_none() {
                                session_detail_first_change_at = Some(now);
                            }
                        }
                    }
                    WatchSignal::Error(message) => {
                        *model =
                            model.with_notice(Some(format!("OpenCode watcher error: {message}")));
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

        if pending_rescan && !sessions_scan_in_flight {
            let now = Instant::now();
            let due_by_debounce = rescan_deadline.is_some_and(|due| now >= due);
            let due_by_max_delay =
                first_change_at.is_some_and(|first| now.duration_since(first) >= max_delay);
            if due_by_debounce || due_by_max_delay {
                pending_rescan = false;
                first_change_at = None;
                rescan_deadline = None;
                sessions_scan_in_flight = true;
                let sessions_dir = model.data.sessions_dir.clone();
                let tx = sessions_scan_tx.clone();
                std::thread::spawn(move || {
                    let output = scan_all_sessions(&sessions_dir);
                    let new_data = app::build_index_from_sessions(
                        sessions_dir.clone(),
                        output.sessions,
                        output.warnings,
                    );
                    let _ = tx.send(SessionsDirScanSignal::Scanned {
                        data: new_data,
                        notice: output.notice,
                    });
                });
            }
        }

        ui::clamp_scroll_state(model);
        terminal.draw(|frame| ui::render(frame, model))?;

        if event::poll(Duration::from_millis(200))? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind == KeyEventKind::Release {
                        continue;
                    }
                    let (next, command) = app::update(model.clone(), AppEvent::Key(key));
                    *model = next;
                    match command {
                        AppCommand::None => {}
                        AppCommand::Quit => return Ok(()),
                        AppCommand::Rescan => {
                            let sessions_dir = model.data.sessions_dir.clone();
                            let output = scan_all_sessions(&sessions_dir);
                            let new_data = app::build_index_from_sessions(
                                sessions_dir.clone(),
                                output.sessions,
                                output.warnings,
                            );
                            let notice = model.notice.clone().or(output.notice);
                            *model = model.with_data(new_data).with_notice(notice);
                            refresh_open_project_stats_overlay(model);
                            request_session_index_refresh_optional(&session_index_req_tx, model);
                        }
                        AppCommand::OpenTasks { return_to } => {
                            let store = match TaskStore::open_default() {
                                Ok(store) => store,
                                Err(error) => {
                                    *model = model.with_notice(Some(format!(
                                        "Failed to open tasks DB: {error}"
                                    )));
                                    continue;
                                }
                            };

                            let tasks = match store.list_tasks() {
                                Ok(tasks) => tasks,
                                Err(error) => {
                                    *model = model.with_notice(Some(format!(
                                        "Failed to load tasks: {error}"
                                    )));
                                    continue;
                                }
                            };

                            let summaries = tasks
                                .into_iter()
                                .map(|entry| crate::app::TaskSummaryRow {
                                    id: entry.task.id,
                                    title: derive_task_title(&entry.task.body),
                                    project_path: entry.task.project_path,
                                    updated_at: entry.task.updated_at,
                                    image_count: entry.image_count,
                                })
                                .collect::<Vec<_>>();

                            model.view = crate::app::View::Tasks(crate::app::TasksView::new(
                                return_to, summaries,
                            ));
                            model.help_open = false;
                            model.system_menu = None;
                        }
                        AppCommand::OpenTaskCreate {
                            return_to,
                            project_path,
                        } => {
                            let store = match TaskStore::open_default() {
                                Ok(store) => store,
                                Err(error) => {
                                    *model = model.with_notice(Some(format!(
                                        "Failed to open tasks DB: {error}"
                                    )));
                                    continue;
                                }
                            };

                            let tasks = match store.list_tasks() {
                                Ok(tasks) => tasks,
                                Err(error) => {
                                    *model = model.with_notice(Some(format!(
                                        "Failed to load tasks: {error}"
                                    )));
                                    continue;
                                }
                            };

                            let summaries = tasks
                                .into_iter()
                                .map(|entry| crate::app::TaskSummaryRow {
                                    id: entry.task.id,
                                    title: derive_task_title(&entry.task.body),
                                    project_path: entry.task.project_path,
                                    updated_at: entry.task.updated_at,
                                    image_count: entry.image_count,
                                })
                                .collect::<Vec<_>>();

                            let tasks_view = crate::app::TasksView::new(return_to, summaries);
                            let project_path = project_path
                                .or_else(|| std::env::current_dir().ok())
                                .unwrap_or_default();
                            model.view = crate::app::View::TaskCreate(
                                crate::app::TaskCreateView::new(tasks_view, project_path),
                            );
                            model.help_open = false;
                            model.system_menu = None;
                        }
                        AppCommand::OpenTaskDetail {
                            from_tasks,
                            task_id,
                        } => {
                            let store = match TaskStore::open_default() {
                                Ok(store) => store,
                                Err(error) => {
                                    *model = model.with_notice(Some(format!(
                                        "Failed to open tasks DB: {error}"
                                    )));
                                    continue;
                                }
                            };

                            match store.load_task(&task_id) {
                                Ok(Some((task, images))) => {
                                    let engine = from_tasks.engine;
                                    model.view =
                                        crate::app::View::TaskDetail(crate::app::TaskDetailView {
                                            from_tasks,
                                            task,
                                            images,
                                            engine,
                                            scroll: 0,
                                        });
                                    model.help_open = false;
                                    model.system_menu = None;
                                }
                                Ok(None) => {
                                    *model = model.with_notice(Some("Task not found.".to_string()));
                                }
                                Err(error) => {
                                    *model = model
                                        .with_notice(Some(format!("Failed to load task: {error}")));
                                }
                            }
                        }
                        AppCommand::CreateTask {
                            from_tasks,
                            project_path,
                            body,
                            image_paths,
                        } => {
                            let store = match TaskStore::open_default() {
                                Ok(store) => store,
                                Err(error) => {
                                    *model = model.with_notice(Some(format!(
                                        "Failed to open tasks DB: {error}"
                                    )));
                                    continue;
                                }
                            };

                            if let Err(error) =
                                store.create_task(&project_path, &body, &image_paths)
                            {
                                *model = model
                                    .with_notice(Some(format!("Failed to save task: {error}")));
                                continue;
                            }

                            let tasks = match store.list_tasks() {
                                Ok(tasks) => tasks,
                                Err(error) => {
                                    *model = model.with_notice(Some(format!(
                                        "Failed to load tasks: {error}"
                                    )));
                                    continue;
                                }
                            };
                            let summaries = tasks
                                .into_iter()
                                .map(|entry| crate::app::TaskSummaryRow {
                                    id: entry.task.id,
                                    title: derive_task_title(&entry.task.body),
                                    project_path: entry.task.project_path,
                                    updated_at: entry.task.updated_at,
                                    image_count: entry.image_count,
                                })
                                .collect::<Vec<_>>();

                            model.view =
                                crate::app::View::Tasks(from_tasks.with_reloaded_tasks(summaries));
                            *model = model.with_notice(Some("Saved task.".to_string()));
                        }
                        AppCommand::DeleteTask {
                            from_tasks,
                            task_id,
                        } => {
                            let store = match TaskStore::open_default() {
                                Ok(store) => store,
                                Err(error) => {
                                    *model = model.with_notice(Some(format!(
                                        "Failed to open tasks DB: {error}"
                                    )));
                                    continue;
                                }
                            };

                            match store.delete_task(&task_id) {
                                Ok(true) => {}
                                Ok(false) => {
                                    *model = model.with_notice(Some("Task not found.".to_string()));
                                    continue;
                                }
                                Err(error) => {
                                    *model = model.with_notice(Some(format!(
                                        "Failed to delete task: {error}"
                                    )));
                                    continue;
                                }
                            }

                            let tasks = match store.list_tasks() {
                                Ok(tasks) => tasks,
                                Err(error) => {
                                    *model = model.with_notice(Some(format!(
                                        "Failed to load tasks: {error}"
                                    )));
                                    continue;
                                }
                            };
                            let summaries = tasks
                                .into_iter()
                                .map(|entry| crate::app::TaskSummaryRow {
                                    id: entry.task.id,
                                    title: derive_task_title(&entry.task.body),
                                    project_path: entry.task.project_path,
                                    updated_at: entry.task.updated_at,
                                    image_count: entry.image_count,
                                })
                                .collect::<Vec<_>>();

                            model.view =
                                crate::app::View::Tasks(from_tasks.with_reloaded_tasks(summaries));
                            *model = model.with_notice(Some("Deleted task.".to_string()));
                        }
                        AppCommand::DeleteTasksBatch {
                            from_tasks,
                            task_ids,
                        } => {
                            let store = match TaskStore::open_default() {
                                Ok(store) => store,
                                Err(error) => {
                                    *model = model.with_notice(Some(format!(
                                        "Failed to open tasks DB: {error}"
                                    )));
                                    continue;
                                }
                            };

                            let mut deleted = 0usize;
                            let mut not_found = 0usize;
                            let mut failed = 0usize;
                            for task_id in &task_ids {
                                match store.delete_task(task_id) {
                                    Ok(true) => deleted = deleted.saturating_add(1),
                                    Ok(false) => not_found = not_found.saturating_add(1),
                                    Err(_) => failed = failed.saturating_add(1),
                                }
                            }

                            let tasks = match store.list_tasks() {
                                Ok(tasks) => tasks,
                                Err(error) => {
                                    *model = model.with_notice(Some(format!(
                                        "Failed to load tasks: {error}"
                                    )));
                                    continue;
                                }
                            };
                            let summaries = tasks
                                .into_iter()
                                .map(|entry| crate::app::TaskSummaryRow {
                                    id: entry.task.id,
                                    title: derive_task_title(&entry.task.body),
                                    project_path: entry.task.project_path,
                                    updated_at: entry.task.updated_at,
                                    image_count: entry.image_count,
                                })
                                .collect::<Vec<_>>();

                            model.view =
                                crate::app::View::Tasks(from_tasks.with_reloaded_tasks(summaries));

                            let mut message = format!("Deleted {deleted} task(s).");
                            if not_found > 0 {
                                message.push_str(&format!(" {not_found} not found."));
                            }
                            if failed > 0 {
                                message.push_str(&format!(" {failed} failed."));
                            }
                            *model = model.with_notice(Some(message));
                        }
                        AppCommand::SpawnTask { engine, task_id } => {
                            let Some(manager) = process_manager.as_mut() else {
                                *model = model
                                    .with_notice(Some("Process spawning is disabled.".to_string()));
                                continue;
                            };

                            let store = match TaskStore::open_default() {
                                Ok(store) => store,
                                Err(error) => {
                                    *model = model.with_notice(Some(format!(
                                        "Failed to open tasks DB: {error}"
                                    )));
                                    continue;
                                }
                            };

                            let Some((task, images)) = (match store.load_task(&task_id) {
                                Ok(task) => task,
                                Err(error) => {
                                    *model = model
                                        .with_notice(Some(format!("Failed to load task: {error}")));
                                    continue;
                                }
                            }) else {
                                *model = model.with_notice(Some("Task not found.".to_string()));
                                continue;
                            };

                            let prompt = format_task_spawn_prompt(&task, &images);

                            match manager.spawn_agent_process(
                                engine,
                                &task.project_path,
                                &prompt,
                                crate::domain::SpawnIoMode::Pipes,
                            ) {
                                Ok(spawned) => {
                                    let io_mode = match spawned.io {
                                        crate::infra::SpawnedAgentIo::Pipes {
                                            stdout_path,
                                            stderr_path,
                                            log_path,
                                        } => crate::app::ProcessIoMode::Pipes {
                                            stdout_path,
                                            stderr_path,
                                            log_path,
                                        },
                                        crate::infra::SpawnedAgentIo::Tty {
                                            transcript_path,
                                            log_path,
                                        } => crate::app::ProcessIoMode::Tty {
                                            transcript_path,
                                            log_path,
                                        },
                                    };

                                    model.processes.push(crate::app::ProcessInfo {
                                        id: spawned.id.clone(),
                                        pid: spawned.pid,
                                        engine: spawned.engine,
                                        project_path: spawned.project_path.clone(),
                                        prompt_preview: spawned.prompt_preview.clone(),
                                        started_at: spawned.started_at,
                                        status: crate::app::ProcessStatus::Running,
                                        io_mode,
                                        session_id: None,
                                        session_log_path: None,
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
                        AppCommand::TaskCreateInsertImage { path } => {
                            let Ok(metadata) = std::fs::metadata(&path) else {
                                *model = model.with_notice(Some(format!(
                                    "Image not found: {}",
                                    path.display()
                                )));
                                continue;
                            };
                            if !metadata.is_file() {
                                *model = model
                                    .with_notice(Some(format!("Not a file: {}", path.display())));
                                continue;
                            }
                            if !is_supported_image_path(&path) {
                                *model = model
                                    .with_notice(Some("Unsupported image type (v1).".to_string()));
                                continue;
                            }

                            let path = std::fs::canonicalize(&path).unwrap_or(path);

                            if let crate::app::View::TaskCreate(task_create) = &mut model.view {
                                let ordinal =
                                    u32::try_from(task_create.image_paths.len().saturating_add(1))
                                        .unwrap_or(u32::MAX);
                                task_create.image_paths.push(path);
                                task_create.editor.insert_str(&format!("[Image {ordinal}]"));
                                task_create.overlay = None;
                                model.notice = Some(format!("Inserted [Image {ordinal}]."));
                            }
                        }
                        AppCommand::TaskCreatePasteImageFromClipboard => {
                            if !matches!(&model.view, crate::app::View::TaskCreate(_)) {
                                *model = model.with_notice(Some(
                                    "Open New Task to paste an image from the clipboard."
                                        .to_string(),
                                ));
                                continue;
                            }

                            let path =
                                match crate::infra::paste_clipboard_image_to_task_images_dir() {
                                    Ok(path) => path,
                                    Err(crate::infra::PasteClipboardImageError::NoImage) => {
                                        *model = model.with_notice(Some(
                                            "Clipboard has no image to paste.".to_string(),
                                        ));
                                        continue;
                                    }
                                    Err(error) => {
                                        *model = model.with_notice(Some(format!(
                                            "Failed to paste clipboard image: {error}"
                                        )));
                                        continue;
                                    }
                                };

                            let path = std::fs::canonicalize(&path).unwrap_or(path);
                            if let crate::app::View::TaskCreate(task_create) = &mut model.view {
                                let ordinal =
                                    u32::try_from(task_create.image_paths.len().saturating_add(1))
                                        .unwrap_or(u32::MAX);
                                task_create.image_paths.push(path);
                                task_create.editor.insert_str(&format!("[Image {ordinal}]"));
                                task_create.overlay = None;
                                model.notice =
                                    Some(format!("Inserted [Image {ordinal}] from clipboard."));
                            }
                        }
                        AppCommand::OpenSessionDetail {
                            from_sessions,
                            session,
                        } => {
                            let mut session = session;
                            let log_path = match crate::infra::prepare_session_log_path(&session) {
                                Ok(path) => path,
                                Err(error) => {
                                    *model = model.with_notice(Some(format!(
                                        "Failed to prepare session log: {error}"
                                    )));
                                    continue;
                                }
                            };
                            session.log_path = log_path.clone();

                            match load_session_timeline(&log_path) {
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
                                    *model = model.with_notice(Some(format!(
                                        "Failed to load session: {error}"
                                    )));
                                }
                            }
                        }
                        AppCommand::OpenSessionStats { session } => {
                            let log_path = match crate::infra::prepare_session_log_path(&session) {
                                Ok(path) => path,
                                Err(error) => {
                                    *model = model.with_notice(Some(format!(
                                        "Failed to prepare stats: {error}"
                                    )));
                                    continue;
                                }
                            };

                            match load_session_timeline(&log_path) {
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
                            let log_path = match crate::infra::prepare_session_log_path(&session) {
                                Ok(path) => path,
                                Err(error) => {
                                    *model = model.with_notice(Some(format!(
                                        "Failed to prepare result: {error}"
                                    )));
                                    continue;
                                }
                            };

                            match load_last_assistant_output(&log_path) {
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
                        AppCommand::RenameSession { session, title } => {
                            let state_dir = match resolve_ccbox_state_dir() {
                                Ok(dir) => dir,
                                Err(error) => {
                                    *model = model.with_notice(Some(format!(
                                        "Rename disabled (state dir unavailable): {error}"
                                    )));
                                    continue;
                                }
                            };

                            if let Err(error) = set_session_alias(
                                &state_dir,
                                session.engine,
                                &session.meta.id,
                                &title,
                            ) {
                                *model = model.with_notice(Some(format!(
                                    "Failed to rename session: {error}"
                                )));
                                continue;
                            }

                            let fallback_title = title
                                .is_empty()
                                .then(|| infer_session_title_from_log(&session))
                                .flatten();
                            let cleared_needs_rescan = title.is_empty() && fallback_title.is_none();

                            let mut projects = model.data.projects.clone();
                            let mut updated = false;
                            for project in &mut projects {
                                for entry in &mut project.sessions {
                                    if entry.log_path != session.log_path {
                                        continue;
                                    }

                                    if title.is_empty() {
                                        if let Some(value) = fallback_title.as_ref() {
                                            entry.title = value.clone();
                                        }
                                    } else {
                                        entry.title = title.clone();
                                    }
                                    updated = true;
                                }
                            }

                            let new_data = crate::app::AppData {
                                sessions_dir: model.data.sessions_dir.clone(),
                                projects,
                                warnings: model.data.warnings,
                                load_error: model.data.load_error.clone(),
                            };
                            *model = model.with_data(new_data);

                            let notice = if !updated {
                                "Session not found.".to_string()
                            } else if title.is_empty() {
                                if cleared_needs_rescan {
                                    "Cleared session rename. Title will refresh on rescan."
                                        .to_string()
                                } else {
                                    "Cleared session rename.".to_string()
                                }
                            } else {
                                "Renamed session.".to_string()
                            };
                            *model = model.with_notice(Some(notice));
                        }
                        AppCommand::MoveSessionProject {
                            session,
                            project_path,
                        } => {
                            let state_dir = match resolve_ccbox_state_dir() {
                                Ok(dir) => dir,
                                Err(error) => {
                                    *model = model.with_notice(Some(format!(
                                        "Move disabled (state dir unavailable): {error}"
                                    )));
                                    continue;
                                }
                            };

                            let raw_path = project_path
                                .as_ref()
                                .map(|path| path.display().to_string())
                                .unwrap_or_default();
                            if let Err(error) = set_session_project(
                                &state_dir,
                                session.engine,
                                &session.meta.id,
                                &raw_path,
                            ) {
                                *model = model
                                    .with_notice(Some(format!("Failed to move session: {error}")));
                                continue;
                            }

                            let fallback_project = project_path
                                .is_none()
                                .then(|| infer_session_project_from_log(&session))
                                .flatten();
                            let cleared_needs_rescan =
                                project_path.is_none() && fallback_project.is_none();

                            if let crate::app::View::SessionDetail(detail_view) = &mut model.view {
                                if detail_view.session.log_path == session.log_path {
                                    if let Some(cwd) =
                                        project_path.clone().or_else(|| fallback_project.clone())
                                    {
                                        detail_view.session.meta.cwd = cwd.clone();
                                        detail_view.from_sessions.project_path = cwd;
                                    }
                                }
                            }

                            let mut sessions = model
                                .data
                                .projects
                                .iter()
                                .flat_map(|project| project.sessions.iter().cloned())
                                .collect::<Vec<_>>();

                            let next_cwd =
                                project_path.clone().or_else(|| fallback_project.clone());
                            let mut updated = false;
                            for entry in &mut sessions {
                                if entry.log_path != session.log_path {
                                    continue;
                                }
                                if let Some(cwd) = next_cwd.clone() {
                                    entry.meta.cwd = cwd;
                                }
                                updated = true;
                            }

                            let new_data = crate::app::build_index_from_sessions(
                                model.data.sessions_dir.clone(),
                                sessions,
                                model.data.warnings,
                            );
                            *model = model.with_data(new_data);

                            let notice = if !updated {
                                "Session not found.".to_string()
                            } else if project_path.is_none() {
                                if cleared_needs_rescan {
                                    "Cleared project override. Project will refresh on rescan."
                                        .to_string()
                                } else {
                                    "Cleared project override.".to_string()
                                }
                            } else {
                                "Moved session.".to_string()
                            };
                            *model = model.with_notice(Some(notice));
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
                            let output = scan_all_sessions(&sessions_dir);
                            let new_data = app::build_index_from_sessions(
                                sessions_dir.clone(),
                                output.sessions,
                                output.warnings,
                            );
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
                        AppCommand::DeleteProjectLogsBatch { project_paths } => {
                            let mut log_paths = Vec::new();
                            for project_path in &project_paths {
                                let Some(project) = model
                                    .data
                                    .projects
                                    .iter()
                                    .find(|project| &project.project_path == project_path)
                                else {
                                    continue;
                                };
                                log_paths.extend(
                                    project
                                        .sessions
                                        .iter()
                                        .map(|session| session.log_path.clone()),
                                );
                            }

                            let outcome = delete_session_logs(&model.data.sessions_dir, &log_paths);

                            pending_rescan = false;
                            first_change_at = None;
                            rescan_deadline = None;

                            let sessions_dir = model.data.sessions_dir.clone();
                            let output = scan_all_sessions(&sessions_dir);
                            let new_data = app::build_index_from_sessions(
                                sessions_dir.clone(),
                                output.sessions,
                                output.warnings,
                            );
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
                            let output = scan_all_sessions(&sessions_dir);
                            let new_data = app::build_index_from_sessions(
                                sessions_dir.clone(),
                                output.sessions,
                                output.warnings,
                            );
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
                        AppCommand::DeleteSessionLogsBatch { log_paths } => {
                            let outcome = delete_session_logs(&model.data.sessions_dir, &log_paths);

                            pending_rescan = false;
                            first_change_at = None;
                            rescan_deadline = None;

                            let sessions_dir = model.data.sessions_dir.clone();
                            let output = scan_all_sessions(&sessions_dir);
                            let new_data = app::build_index_from_sessions(
                                sessions_dir.clone(),
                                output.sessions,
                                output.warnings,
                            );
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
                            io_mode,
                        } => {
                            let Some(manager) = process_manager.as_mut() else {
                                *model = model
                                    .with_notice(Some("Process spawning is disabled.".to_string()));
                                continue;
                            };

                            match manager.spawn_agent_process(
                                engine,
                                &project_path,
                                &prompt,
                                io_mode,
                            ) {
                                Ok(spawned) => {
                                    let io_mode = match spawned.io {
                                        crate::infra::SpawnedAgentIo::Pipes {
                                            stdout_path,
                                            stderr_path,
                                            log_path,
                                        } => crate::app::ProcessIoMode::Pipes {
                                            stdout_path,
                                            stderr_path,
                                            log_path,
                                        },
                                        crate::infra::SpawnedAgentIo::Tty {
                                            transcript_path,
                                            log_path,
                                        } => crate::app::ProcessIoMode::Tty {
                                            transcript_path,
                                            log_path,
                                        },
                                    };

                                    model.processes.push(crate::app::ProcessInfo {
                                        id: spawned.id.clone(),
                                        pid: spawned.pid,
                                        engine: spawned.engine,
                                        project_path: spawned.project_path.clone(),
                                        prompt_preview: spawned.prompt_preview.clone(),
                                        started_at: spawned.started_at,
                                        status: crate::app::ProcessStatus::Running,
                                        io_mode,
                                        session_id: None,
                                        session_log_path: None,
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
                        AppCommand::ForkResumeCodexFromTimeline { fork, prompt } => {
                            let Some(manager) = process_manager.as_mut() else {
                                *model = model
                                    .with_notice(Some("Process spawning is disabled.".to_string()));
                                continue;
                            };

                            let forked = match fork_codex_session_log_at_cut(
                                &model.data.sessions_dir,
                                &fork.parent_log_path,
                                fork.cut,
                            ) {
                                Ok(forked) => forked,
                                Err(error) => {
                                    *model = model.with_notice(Some(format!(
                                        "Failed to fork session log: {error}"
                                    )));
                                    continue;
                                }
                            };

                            match manager.spawn_codex_resume_process(
                                &fork.project_path,
                                &forked.session_id,
                                &prompt,
                            ) {
                                Ok(spawned) => {
                                    let io_mode = match spawned.io {
                                        crate::infra::SpawnedAgentIo::Pipes {
                                            stdout_path,
                                            stderr_path,
                                            log_path,
                                        } => crate::app::ProcessIoMode::Pipes {
                                            stdout_path,
                                            stderr_path,
                                            log_path,
                                        },
                                        crate::infra::SpawnedAgentIo::Tty {
                                            transcript_path,
                                            log_path,
                                        } => crate::app::ProcessIoMode::Tty {
                                            transcript_path,
                                            log_path,
                                        },
                                    };

                                    model.processes.push(crate::app::ProcessInfo {
                                        id: spawned.id.clone(),
                                        pid: spawned.pid,
                                        engine: spawned.engine,
                                        project_path: spawned.project_path.clone(),
                                        prompt_preview: spawned.prompt_preview.clone(),
                                        started_at: spawned.started_at,
                                        status: crate::app::ProcessStatus::Running,
                                        io_mode,
                                        session_id: Some(forked.session_id.clone()),
                                        session_log_path: Some(forked.log_path.clone()),
                                    });

                                    *model = model.with_notice(Some(format!(
                                        "Forked and resumed Codex ({})",
                                        spawned.id
                                    )));
                                }
                                Err(error) => {
                                    *model = model.with_notice(Some(format!(
                                        "Failed to spawn resume process: {error}. Forked session: {} ({})",
                                        forked.session_id,
                                        forked.log_path.display()
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
                        AppCommand::AttachProcessTty { process_id } => {
                            let Some(manager) = process_manager.as_mut() else {
                                *model = model
                                    .with_notice(Some("Process manager disabled.".to_string()));
                                continue;
                            };

                            if let Err(error) =
                                attach_tty_process(terminal, model, manager, &process_id)
                            {
                                *model =
                                    model.with_notice(Some(format!("Failed to attach: {error}")));
                            }
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
                Event::Mouse(mouse) => {
                    let (next, _command) = app::update(model.clone(), AppEvent::Mouse(mouse));
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

#[derive(Debug, Error)]
enum AttachTtyProcessError {
    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    Attach(#[from] AttachTtyError),

    #[error(transparent)]
    Write(#[from] WriteTtyError),

    #[error(transparent)]
    Resize(#[from] ResizeTtyError),

    #[cfg(not(unix))]
    #[error("TTY attach is not supported on this OS yet")]
    Unsupported,
}

struct SuspendTuiGuard<'a> {
    terminal: &'a mut Terminal<CrosstermBackend<Stdout>>,
}

impl<'a> SuspendTuiGuard<'a> {
    fn suspend(terminal: &'a mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<Self> {
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;
        Ok(Self { terminal })
    }
}

impl Drop for SuspendTuiGuard<'_> {
    fn drop(&mut self) {
        let _ = execute!(self.terminal.backend_mut(), EnterAlternateScreen);
        let _ = self.terminal.hide_cursor();
        let _ = self.terminal.clear();
    }
}

fn attach_tty_process(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    model: &mut AppModel,
    manager: &mut ProcessManager,
    process_id: &str,
) -> Result<(), AttachTtyProcessError> {
    let Some(process) = model
        .processes
        .iter()
        .find(|process| process.id == process_id)
        .cloned()
    else {
        return Err(io::Error::other("process not found").into());
    };

    let rx = manager.attach_tty_output(process_id)?;
    let _suspended = match SuspendTuiGuard::suspend(terminal) {
        Ok(guard) => guard,
        Err(error) => {
            manager.detach_tty_output(process_id);
            return Err(error.into());
        }
    };

    {
        let mut out = io::stdout().lock();
        let _ = writeln!(
            out,
            "Attached to {} ({}). Press Ctrl-] to detach.",
            process.id,
            process.engine.label()
        );
        let _ = out.flush();
    }

    let outcome = attach_tty_loop(model, manager, process_id, rx);
    manager.detach_tty_output(process_id);
    let outcome = outcome?;

    match outcome {
        AttachTtyOutcome::Detached => {
            model.notice = Some(format!("Detached from {process_id}."));
        }
        AttachTtyOutcome::Exited(code) => {
            model.notice = Some(format!(
                "Process {} exited{}.",
                process_id,
                code.map(|code| format!(" (code {code})"))
                    .unwrap_or_default()
            ));
        }
    }

    Ok(())
}

enum AttachTtyOutcome {
    Detached,
    Exited(Option<i32>),
}

#[cfg(unix)]
fn attach_tty_loop(
    model: &mut AppModel,
    manager: &mut ProcessManager,
    process_id: &str,
    rx: std::sync::mpsc::Receiver<Vec<u8>>,
) -> Result<AttachTtyOutcome, AttachTtyProcessError> {
    use std::os::unix::io::AsRawFd;

    const DETACH_BYTE: u8 = 0x1d; // Ctrl-]

    let stdin_fd = io::stdin().as_raw_fd();
    let mut stdout = io::stdout().lock();

    let mut last_size: Option<(u16, u16)> = None;

    loop {
        while let Ok(chunk) = rx.try_recv() {
            stdout.write_all(&chunk)?;
            stdout.flush()?;
        }

        if let Ok((cols, rows)) = terminal_size() {
            let next_size = (rows, cols);
            if last_size != Some(next_size) {
                last_size = Some(next_size);
                let _ = manager.resize_tty(process_id, rows, cols);
            }
        }

        for exit in manager.poll_exits() {
            apply_process_exit(model, exit);
        }

        let is_running = model
            .processes
            .iter()
            .find(|process| process.id == process_id)
            .is_some_and(|process| process.status.is_running());
        if !is_running {
            let exit_code = model
                .processes
                .iter()
                .find(|process| process.id == process_id)
                .and_then(|process| match process.status {
                    crate::app::ProcessStatus::Exited(code) => code,
                    _ => None,
                });
            return Ok(AttachTtyOutcome::Exited(exit_code));
        }

        let mut poll_fd = libc::pollfd {
            fd: stdin_fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut poll_fd as *mut libc::pollfd, 1, 50) };
        if ready < 0 {
            return Err(io::Error::last_os_error().into());
        }
        if ready == 0 {
            continue;
        }

        if poll_fd.revents & libc::POLLIN == 0 {
            continue;
        }

        let mut buf = [0u8; 4096];
        let n = unsafe { libc::read(stdin_fd, buf.as_mut_ptr() as *mut _, buf.len()) };
        if n < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error.into());
        }
        if n == 0 {
            return Ok(AttachTtyOutcome::Detached);
        }

        let bytes = &buf[..(n as usize)];
        let mut detach = false;
        let mut to_write: Vec<u8> = Vec::new();
        for &byte in bytes {
            if byte == DETACH_BYTE {
                detach = true;
                break;
            }
            to_write.push(byte);
        }

        if !to_write.is_empty() {
            let _ = manager.write_tty(process_id, &to_write);
        }

        if detach {
            return Ok(AttachTtyOutcome::Detached);
        }
    }
}

#[cfg(not(unix))]
fn attach_tty_loop(
    _model: &mut AppModel,
    _manager: &mut ProcessManager,
    _process_id: &str,
    _rx: std::sync::mpsc::Receiver<Vec<u8>>,
) -> Result<AttachTtyOutcome, AttachTtyProcessError> {
    Err(AttachTtyProcessError::Unsupported)
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
        ProcessOutputKind::Stdout => process
            .io_mode
            .stdout_path()
            .cloned()
            .unwrap_or_else(|| process.io_mode.log_path().clone()),
        ProcessOutputKind::Stderr => {
            let Some(path) = process.io_mode.stderr_path() else {
                *model = model.with_notice(Some(format!(
                    "No stderr for {} process {}.",
                    process.io_mode.label(),
                    process.id
                )));
                return;
            };
            path.clone()
        }
        ProcessOutputKind::Log => process.io_mode.log_path().clone(),
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
        crate::domain::SessionEngine::Codex,
    ))
}

fn infer_session_title_from_log(session: &crate::domain::SessionSummary) -> Option<String> {
    match session.engine {
        crate::domain::SessionEngine::Codex => infer_codex_session_title(&session.log_path),
        crate::domain::SessionEngine::Claude => infer_claude_session_title(&session.log_path),
        crate::domain::SessionEngine::Gemini => infer_gemini_session_title(&session.log_path),
        crate::domain::SessionEngine::OpenCode => None,
    }
}

fn infer_codex_session_title(log_path: &std::path::Path) -> Option<String> {
    use std::io::BufRead;

    const MAX_TITLE_SCAN_LINES: usize = 250;

    let file = std::fs::File::open(log_path).ok()?;
    let mut reader = std::io::BufReader::new(file);

    let mut first_line = String::new();
    let bytes = reader.read_line(&mut first_line).ok()?;
    if bytes == 0 {
        return None;
    }

    for _ in 0..MAX_TITLE_SCAN_LINES {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line).ok()?;
        if bytes == 0 {
            break;
        }
        let Ok(Some(text)) = crate::domain::parse_user_message_text(line.trim_end()) else {
            continue;
        };
        if crate::domain::is_metadata_prompt(&text) {
            continue;
        }
        if let Some(title) = crate::domain::derive_title_from_user_text(&text) {
            return Some(title);
        }
    }

    None
}

fn infer_claude_session_title(log_path: &std::path::Path) -> Option<String> {
    use std::io::BufRead;

    const MAX_TITLE_SCAN_LINES: usize = 250;
    const MAX_TITLE_SCAN_BYTES: usize = 200_000;

    let file = std::fs::File::open(log_path).ok()?;
    let mut reader = std::io::BufReader::new(file);

    let mut bytes_read = 0usize;
    for _ in 0..MAX_TITLE_SCAN_LINES {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line).ok()?;
        if bytes == 0 {
            break;
        }
        bytes_read = bytes_read.saturating_add(bytes);
        if bytes_read > MAX_TITLE_SCAN_BYTES {
            break;
        }

        let value: serde_json::Value = match serde_json::from_str(line.trim_end()) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let Some(text) = crate::domain::parse_claude_user_message_text(&value) else {
            continue;
        };
        if crate::domain::is_metadata_prompt(&text) {
            continue;
        }
        if let Some(title) = crate::domain::derive_title_from_user_text(&text) {
            return Some(title);
        }
    }

    None
}

fn infer_gemini_session_title(log_path: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(log_path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    crate::domain::infer_gemini_title_from_session(&value)
}

fn infer_session_project_from_log(session: &crate::domain::SessionSummary) -> Option<PathBuf> {
    match session.engine {
        crate::domain::SessionEngine::Codex => infer_codex_session_project(&session.log_path),
        crate::domain::SessionEngine::Claude => infer_claude_session_project(&session.log_path),
        crate::domain::SessionEngine::Gemini => infer_gemini_session_project(&session.log_path),
        crate::domain::SessionEngine::OpenCode => None,
    }
}

fn infer_codex_session_project(log_path: &std::path::Path) -> Option<PathBuf> {
    use std::io::BufRead;

    let file = std::fs::File::open(log_path).ok()?;
    let mut reader = std::io::BufReader::new(file);

    let mut first_line = String::new();
    let bytes = reader.read_line(&mut first_line).ok()?;
    if bytes == 0 {
        return None;
    }

    let meta = parse_session_meta_line(first_line.trim_end()).ok()?;
    Some(meta.cwd)
}

fn infer_claude_session_project(log_path: &std::path::Path) -> Option<PathBuf> {
    use std::io::BufRead;

    const MAX_SCAN_LINES: usize = 250;
    const MAX_SCAN_BYTES: usize = 200_000;

    let file = std::fs::File::open(log_path).ok()?;
    let mut reader = std::io::BufReader::new(file);

    let mut bytes_read = 0usize;
    for _ in 0..MAX_SCAN_LINES {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line).ok()?;
        if bytes == 0 {
            break;
        }
        bytes_read = bytes_read.saturating_add(bytes);
        if bytes_read > MAX_SCAN_BYTES {
            break;
        }

        let value: serde_json::Value = match serde_json::from_str(line.trim_end()) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let hint = crate::domain::extract_claude_session_meta_hint(&value);
        if let Some(cwd) = hint.cwd {
            return Some(cwd);
        }
    }

    None
}

fn infer_gemini_session_project(log_path: &std::path::Path) -> Option<PathBuf> {
    log_path.parent()?.parent().map(|path| path.to_path_buf())
}

fn ensure_session_detail_watcher(
    model: &mut AppModel,
    watcher: &mut Option<crate::infra::SessionFileWatcher>,
    watcher_path: &mut Option<PathBuf>,
    pending_reload: &mut bool,
    first_change_at: &mut Option<Instant>,
    reload_deadline: &mut Option<Instant>,
    in_flight_for: &mut Option<PathBuf>,
) {
    let desired_path = match &model.view {
        crate::app::View::SessionDetail(detail_view) => Some(detail_view.session.log_path.clone()),
        _ => None,
    };

    if desired_path.as_ref() == watcher_path.as_ref() {
        return;
    }

    *watcher = None;
    *watcher_path = desired_path.clone();
    *pending_reload = false;
    *first_change_at = None;
    *reload_deadline = None;
    *in_flight_for = None;

    let Some(path) = desired_path else {
        return;
    };

    match watch_session_file(&path) {
        Ok(next) => {
            *watcher = Some(next);
        }
        Err(error) => {
            *watcher = None;
            *model = model.with_notice(Some(format!("Live session updates disabled: {error}")));
        }
    }
}

fn is_session_detail_open_for_log_path(model: &AppModel, log_path: &std::path::Path) -> bool {
    match &model.view {
        crate::app::View::SessionDetail(detail_view) => detail_view.session.log_path == log_path,
        _ => false,
    }
}

fn refresh_open_session_detail(
    model: &mut AppModel,
    log_path: &std::path::Path,
    timeline: crate::domain::SessionTimeline,
) {
    let crate::app::View::SessionDetail(detail_view) = &mut model.view else {
        return;
    };
    if detail_view.session.log_path != log_path {
        return;
    }

    detail_view.items = timeline.items;
    detail_view.turn_contexts = timeline.turn_contexts;
    detail_view.warnings = timeline.warnings;
    detail_view.truncated = timeline.truncated;

    detail_view.last_output = detail_view
        .items
        .iter()
        .rev()
        .find(|item| item.kind == crate::domain::TimelineItemKind::Assistant)
        .map(|item| item.detail.clone());

    if detail_view.items.is_empty() {
        detail_view.selected = 0;
    } else {
        detail_view.selected = detail_view
            .selected
            .min(detail_view.items.len().saturating_sub(1));
    }
}

fn is_supported_image_path(path: &std::path::Path) -> bool {
    let Some(ext) = path.extension().and_then(|ext| ext.to_str()) else {
        return false;
    };
    let ext = ext.to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff"
    )
}
