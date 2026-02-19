mod text_editor;

use crate::domain::{
    AgentEngine, ProjectIndex, ProjectSummary, SessionStats, SessionSummary, TimelineItem,
    TimelineItemKind, TurnContextSummary, index_projects,
};
use crate::infra::ScanWarningCount;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;
use thiserror::Error;

pub use text_editor::TextEditor;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("terminal I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    ResolveSessionsDir(#[from] crate::infra::ResolveSessionsDirError),
}

#[derive(Clone, Debug)]
pub struct AppData {
    pub sessions_dir: PathBuf,
    pub projects: Vec<ProjectSummary>,
    pub warnings: ScanWarningCount,
    pub load_error: Option<String>,
}

impl AppData {
    pub fn from_scan(
        sessions_dir: PathBuf,
        sessions: ProjectIndex,
        warnings: ScanWarningCount,
    ) -> Self {
        Self {
            sessions_dir,
            projects: sessions,
            warnings,
            load_error: None,
        }
    }

    pub fn from_load_error(sessions_dir: PathBuf, error: String) -> Self {
        Self {
            sessions_dir,
            projects: Vec::new(),
            warnings: ScanWarningCount::from(0usize),
            load_error: Some(error),
        }
    }
}

#[derive(Clone, Debug)]
pub struct AppModel {
    pub data: AppData,
    pub view: View,
    pub terminal_size: (u16, u16),
    pub notice: Option<String>,
    pub update_hint: Option<String>,
    pub help_open: bool,
    pub system_menu: Option<SystemMenuOverlay>,
    pub delete_confirm: Option<DeleteConfirmDialog>,
    pub delete_session_confirm: Option<DeleteSessionConfirmDialog>,
    pub session_result_preview: Option<SessionResultPreviewOverlay>,
    pub session_stats_overlay: Option<SessionStatsOverlay>,
    pub processes: Vec<ProcessInfo>,
}

impl AppModel {
    pub fn new(data: AppData) -> Self {
        let view = if data.load_error.is_some() {
            View::Error
        } else {
            View::Projects(ProjectsView::new(&data.projects))
        };
        Self {
            data,
            view,
            terminal_size: (0, 0),
            notice: None,
            update_hint: None,
            help_open: false,
            system_menu: None,
            delete_confirm: None,
            delete_session_confirm: None,
            session_result_preview: None,
            session_stats_overlay: None,
            processes: Vec::new(),
        }
    }

    pub fn with_data(&self, data: AppData) -> Self {
        if data.load_error.is_some() {
            return Self {
                data,
                view: View::Error,
                terminal_size: self.terminal_size,
                notice: None,
                update_hint: self.update_hint.clone(),
                help_open: self.help_open,
                system_menu: self.system_menu.clone(),
                delete_confirm: self.delete_confirm.clone(),
                delete_session_confirm: self.delete_session_confirm.clone(),
                session_result_preview: self.session_result_preview.clone(),
                session_stats_overlay: self.session_stats_overlay.clone(),
                processes: self.processes.clone(),
            };
        }

        let view =
            match &self.view {
                View::Projects(projects_view) => {
                    let selected_project_path = projects_view
                        .filtered_indices
                        .get(projects_view.selected)
                        .copied()
                        .and_then(|index| self.data.projects.get(index))
                        .map(|project| project.project_path.clone());

                    let mut next_view = ProjectsView {
                        query: projects_view.query.clone(),
                        filtered_indices: Vec::new(),
                        selected: projects_view.selected,
                    };
                    apply_project_filter(&data.projects, &mut next_view);

                    if let Some(path) = selected_project_path {
                        if let Some(pos) = next_view.filtered_indices.iter().position(|index| {
                            data.projects
                                .get(*index)
                                .is_some_and(|project| project.project_path == path)
                        }) {
                            next_view.selected = pos;
                        }
                    }

                    View::Projects(next_view)
                }
                View::Sessions(sessions_view) => {
                    let selected_log_path = self
                        .data
                        .projects
                        .iter()
                        .find(|project| project.project_path == sessions_view.project_path)
                        .and_then(|project| {
                            sessions_view
                                .filtered_indices
                                .get(sessions_view.session_selected)
                                .copied()
                                .and_then(|index| project.sessions.get(index))
                        })
                        .map(|session| session.log_path.clone());

                    match data
                        .projects
                        .iter()
                        .find(|project| project.project_path == sessions_view.project_path)
                    {
                        Some(project) => {
                            let mut next_view = sessions_view.clone();
                            apply_session_filter(&project.sessions, &mut next_view);
                            if let Some(log_path) = selected_log_path {
                                if let Some(session_index) = project
                                    .sessions
                                    .iter()
                                    .position(|session| session.log_path == log_path)
                                {
                                    if let Some(pos) = next_view
                                        .filtered_indices
                                        .iter()
                                        .position(|index| *index == session_index)
                                    {
                                        next_view.session_selected = pos;
                                    }
                                }
                            }

                            View::Sessions(next_view)
                        }
                        None => View::Projects(ProjectsView::new(&data.projects)),
                    }
                }
                View::NewSession(new_session_view) => {
                    let selected_log_path = self
                        .data
                        .projects
                        .iter()
                        .find(|project| {
                            project.project_path == new_session_view.from_sessions.project_path
                        })
                        .and_then(|project| {
                            let from = &new_session_view.from_sessions;
                            from.filtered_indices
                                .get(from.session_selected)
                                .copied()
                                .and_then(|index| project.sessions.get(index))
                        })
                        .map(|session| session.log_path.clone());

                    match data.projects.iter().find(|project| {
                        project.project_path == new_session_view.from_sessions.project_path
                    }) {
                        Some(project) => {
                            let mut next_view = new_session_view.clone();
                            apply_session_filter(&project.sessions, &mut next_view.from_sessions);

                            if let Some(log_path) = selected_log_path {
                                if let Some(session_index) = project
                                    .sessions
                                    .iter()
                                    .position(|session| session.log_path == log_path)
                                {
                                    if let Some(pos) = next_view
                                        .from_sessions
                                        .filtered_indices
                                        .iter()
                                        .position(|index| *index == session_index)
                                    {
                                        next_view.from_sessions.session_selected = pos;
                                    }
                                }
                            }
                            View::NewSession(next_view)
                        }
                        None => View::Projects(ProjectsView::new(&data.projects)),
                    }
                }
                View::SessionDetail(detail_view) => {
                    match data
                        .projects
                        .iter()
                        .find(|project| project.project_path == detail_view.session.meta.cwd)
                    {
                        Some(project) => {
                            let mut next_view = detail_view.clone();
                            apply_session_filter(&project.sessions, &mut next_view.from_sessions);
                            if let Some(pos) = project.sessions.iter().position(|session| {
                                session.log_path == detail_view.session.log_path
                            }) {
                                next_view.from_sessions.session_selected = next_view
                                    .from_sessions
                                    .filtered_indices
                                    .iter()
                                    .position(|index| *index == pos)
                                    .unwrap_or(0);
                                if let Some(session) = project.sessions.get(pos).cloned() {
                                    next_view.session = session;
                                }
                            }

                            View::SessionDetail(next_view)
                        }
                        None => View::Projects(ProjectsView::new(&data.projects)),
                    }
                }
                View::Processes(processes_view) => View::Processes(processes_view.clone()),
                View::ProcessOutput(output_view) => View::ProcessOutput(output_view.clone()),
                View::Error => View::Projects(ProjectsView::new(&data.projects)),
            };

        Self {
            data,
            view,
            terminal_size: self.terminal_size,
            notice: None,
            update_hint: self.update_hint.clone(),
            help_open: self.help_open,
            system_menu: self.system_menu.clone(),
            delete_confirm: self.delete_confirm.clone(),
            delete_session_confirm: self.delete_session_confirm.clone(),
            session_result_preview: self.session_result_preview.clone(),
            session_stats_overlay: self.session_stats_overlay.clone(),
            processes: self.processes.clone(),
        }
    }

    pub fn with_terminal_size(&self, width: u16, height: u16) -> Self {
        Self {
            data: self.data.clone(),
            view: self.view.clone(),
            terminal_size: (width, height),
            notice: self.notice.clone(),
            update_hint: self.update_hint.clone(),
            help_open: self.help_open,
            system_menu: self.system_menu.clone(),
            delete_confirm: self.delete_confirm.clone(),
            delete_session_confirm: self.delete_session_confirm.clone(),
            session_result_preview: self.session_result_preview.clone(),
            session_stats_overlay: self.session_stats_overlay.clone(),
            processes: self.processes.clone(),
        }
    }

    pub fn with_notice(&self, notice: Option<String>) -> Self {
        Self {
            data: self.data.clone(),
            view: self.view.clone(),
            terminal_size: self.terminal_size,
            notice,
            update_hint: self.update_hint.clone(),
            help_open: self.help_open,
            system_menu: self.system_menu.clone(),
            delete_confirm: self.delete_confirm.clone(),
            delete_session_confirm: self.delete_session_confirm.clone(),
            session_result_preview: self.session_result_preview.clone(),
            session_stats_overlay: self.session_stats_overlay.clone(),
            processes: self.processes.clone(),
        }
    }

    pub fn open_session_detail(
        &self,
        from_sessions: SessionsView,
        session: SessionSummary,
        items: Vec<TimelineItem>,
        turn_contexts: BTreeMap<String, TurnContextSummary>,
        warnings: usize,
        truncated: bool,
    ) -> Self {
        let last_output = items
            .iter()
            .rev()
            .find(|item| item.kind == TimelineItemKind::Assistant)
            .map(|item| item.detail.clone());

        Self {
            data: self.data.clone(),
            terminal_size: self.terminal_size,
            notice: None,
            update_hint: self.update_hint.clone(),
            help_open: self.help_open,
            system_menu: self.system_menu.clone(),
            delete_confirm: self.delete_confirm.clone(),
            delete_session_confirm: self.delete_session_confirm.clone(),
            session_result_preview: self.session_result_preview.clone(),
            session_stats_overlay: self.session_stats_overlay.clone(),
            processes: self.processes.clone(),
            view: View::SessionDetail(SessionDetailView {
                from_sessions,
                session,
                items,
                turn_contexts,
                warnings,
                truncated,
                selected: 0,
                context_overlay_open: false,
                last_output: last_output.clone(),
                output_overlay_open: false,
                output_overlay_scroll: 0,
            }),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SessionResultPreviewOverlay {
    pub session_title: String,
    pub output: String,
    pub scroll: u16,
}

#[derive(Clone, Debug)]
pub struct SessionStatsOverlay {
    pub session: SessionSummary,
    pub stats: SessionStats,
    pub scroll: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SystemMenuItem {
    Rescan,
    Processes,
    Help,
    Quit,
}

impl SystemMenuItem {
    pub fn label(self) -> &'static str {
        match self {
            Self::Rescan => "Rescan sessions",
            Self::Processes => "Processes",
            Self::Help => "Help",
            Self::Quit => "Quit",
        }
    }

    pub fn hotkey(self) -> &'static str {
        match self {
            Self::Rescan => "Ctrl+R",
            Self::Processes => "P",
            Self::Help => "F1 or ?",
            Self::Quit => "Ctrl+Q or Ctrl+C",
        }
    }
}

pub const SYSTEM_MENU_ITEMS: [SystemMenuItem; 4] = [
    SystemMenuItem::Rescan,
    SystemMenuItem::Processes,
    SystemMenuItem::Help,
    SystemMenuItem::Quit,
];

#[derive(Clone, Debug)]
pub struct SystemMenuOverlay {
    pub selected: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeleteConfirmSelection {
    Cancel,
    Delete,
}

impl DeleteConfirmSelection {
    fn toggle(self) -> Self {
        match self {
            Self::Cancel => Self::Delete,
            Self::Delete => Self::Cancel,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DeleteConfirmDialog {
    pub project_name: String,
    pub project_path: PathBuf,
    pub session_count: usize,
    pub total_size_bytes: u64,
    pub selection: DeleteConfirmSelection,
}

#[derive(Clone, Debug)]
pub struct DeleteSessionConfirmDialog {
    pub project_name: String,
    pub project_path: PathBuf,
    pub session_title: String,
    pub log_path: PathBuf,
    pub size_bytes: u64,
    pub file_modified: Option<SystemTime>,
    pub selection: DeleteConfirmSelection,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProcessStatus {
    Running,
    Exited(Option<i32>),
    Killed,
}

impl ProcessStatus {
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running)
    }

    pub fn label(&self) -> String {
        match self {
            Self::Running => "running".to_string(),
            Self::Exited(Some(code)) => format!("exit {code}"),
            Self::Exited(None) => "exited".to_string(),
            Self::Killed => "killed".to_string(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ProcessInfo {
    pub id: String,
    pub pid: u32,
    pub engine: AgentEngine,
    pub project_path: PathBuf,
    pub prompt_preview: String,
    pub started_at: SystemTime,
    pub status: ProcessStatus,
    pub session_id: Option<String>,
    pub session_log_path: Option<PathBuf>,
    pub stdout_path: PathBuf,
    pub stderr_path: PathBuf,
    pub log_path: PathBuf,
}

#[derive(Clone, Debug)]
pub enum View {
    Projects(ProjectsView),
    Sessions(SessionsView),
    NewSession(NewSessionView),
    SessionDetail(SessionDetailView),
    Processes(ProcessesView),
    ProcessOutput(ProcessOutputView),
    Error,
}

#[derive(Clone, Debug)]
pub struct ProjectsView {
    pub query: String,
    pub filtered_indices: Vec<usize>,
    pub selected: usize,
}

impl ProjectsView {
    pub fn new(projects: &[ProjectSummary]) -> Self {
        let filtered_indices = (0..projects.len()).collect();
        Self {
            query: String::new(),
            filtered_indices,
            selected: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SessionsView {
    pub project_path: PathBuf,
    pub query: String,
    pub filtered_indices: Vec<usize>,
    pub session_selected: usize,
}

impl SessionsView {
    pub fn new(project_path: PathBuf, session_count: usize) -> Self {
        Self {
            project_path,
            query: String::new(),
            filtered_indices: (0..session_count).collect(),
            session_selected: 0,
        }
    }

    pub fn current_project<'a>(
        &self,
        projects: &'a [ProjectSummary],
    ) -> Option<&'a ProjectSummary> {
        projects
            .iter()
            .find(|project| project.project_path == self.project_path)
    }
}

#[derive(Clone, Debug)]
pub struct NewSessionView {
    pub from_sessions: SessionsView,
    pub editor: TextEditor,
    pub engine: AgentEngine,
}

impl NewSessionView {
    pub fn new(from_sessions: SessionsView) -> Self {
        Self {
            from_sessions,
            editor: TextEditor::new(),
            engine: AgentEngine::Codex,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SessionDetailView {
    pub from_sessions: SessionsView,
    pub session: SessionSummary,
    pub items: Vec<TimelineItem>,
    pub turn_contexts: BTreeMap<String, TurnContextSummary>,
    pub warnings: usize,
    pub truncated: bool,
    pub selected: usize,
    pub context_overlay_open: bool,
    pub last_output: Option<String>,
    pub output_overlay_open: bool,
    pub output_overlay_scroll: u16,
}

#[derive(Clone, Debug)]
pub struct ProcessesView {
    pub return_to: Box<View>,
    pub selected: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessOutputKind {
    Stdout,
    Stderr,
    Log,
}

impl ProcessOutputKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
            Self::Log => "log",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ProcessOutputView {
    pub return_to: Box<View>,
    pub process_id: String,
    pub kind: ProcessOutputKind,
    pub file_path: PathBuf,
    pub buffer: Arc<String>,
    pub file_offset: u64,
    pub scroll: u16,
}

#[derive(Clone, Debug)]
pub enum AppEvent {
    Key(KeyEvent),
    Paste(String),
}

#[derive(Clone, Debug)]
pub enum AppCommand {
    None,
    Quit,
    Rescan,
    OpenSessionDetail {
        from_sessions: SessionsView,
        session: SessionSummary,
    },
    OpenSessionStats {
        session: SessionSummary,
    },
    OpenSessionDetailByLogPath {
        project_path: PathBuf,
        log_path: PathBuf,
    },
    OpenSessionResultPreview {
        session: SessionSummary,
    },
    DeleteProjectLogs {
        project_path: PathBuf,
    },
    DeleteSessionLog {
        log_path: PathBuf,
    },
    SpawnAgentSession {
        engine: AgentEngine,
        project_path: PathBuf,
        prompt: String,
    },
    KillProcess {
        process_id: String,
    },
    OpenProcessOutput {
        process_id: String,
        kind: ProcessOutputKind,
    },
}

pub fn update(model: AppModel, event: AppEvent) -> (AppModel, AppCommand) {
    match event {
        AppEvent::Key(key) => update_on_key(model, key),
        AppEvent::Paste(text) => update_on_paste(model, text),
    }
}

fn update_on_key(model: AppModel, key: KeyEvent) -> (AppModel, AppCommand) {
    let mut model = model;
    model.notice = None;

    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return (model, AppCommand::Quit);
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
        return (model, AppCommand::Quit);
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('r') {
        return (model, AppCommand::Rescan);
    }

    if matches!(key.code, KeyCode::F(2)) {
        if model.delete_confirm.is_some()
            || model.delete_session_confirm.is_some()
            || model.session_result_preview.is_some()
            || model.session_stats_overlay.is_some()
        {
            return (model, AppCommand::None);
        }

        if model.system_menu.is_some() {
            model.system_menu = None;
        } else {
            model.system_menu = Some(SystemMenuOverlay { selected: 0 });
            model.help_open = false;
        }
        return (model, AppCommand::None);
    }

    if let Some(menu) = model.system_menu.take() {
        return update_system_menu_overlay(model, menu, key);
    }

    if let Some(overlay) = model.session_stats_overlay.take() {
        return update_session_stats_overlay(model, overlay, key);
    }

    if let Some(preview) = model.session_result_preview.take() {
        return update_session_result_preview_overlay(model, preview, key);
    }

    if let Some(confirm) = model.delete_session_confirm.take() {
        return update_delete_session_confirm(model, confirm, key);
    }

    if let Some(confirm) = model.delete_confirm.take() {
        return update_delete_confirm(model, confirm, key);
    }

    if matches!(key.code, KeyCode::F(1) | KeyCode::Char('?')) {
        model.help_open = !model.help_open;
        return (model, AppCommand::None);
    }

    if model.help_open {
        match key.code {
            KeyCode::Esc | KeyCode::Backspace => {
                model.help_open = false;
                return (model, AppCommand::None);
            }
            _ => return (model, AppCommand::None),
        }
    }

    if key.code == KeyCode::Char('P')
        && !matches!(&model.view, View::Processes(_) | View::ProcessOutput(_))
    {
        open_processes_view(&mut model);
        return (model, AppCommand::None);
    }

    let view = model.view.clone();
    match view {
        View::Projects(projects_view) => update_projects(model, projects_view, key),
        View::Sessions(sessions_view) => update_sessions(model, sessions_view, key),
        View::NewSession(new_session_view) => update_new_session(model, new_session_view, key),
        View::SessionDetail(detail_view) => update_session_detail(model, detail_view, key),
        View::Processes(processes_view) => update_processes(model, processes_view, key),
        View::ProcessOutput(output_view) => update_process_output(model, output_view, key),
        View::Error => update_error(model, key),
    }
}

fn update_system_menu_overlay(
    mut model: AppModel,
    mut menu: SystemMenuOverlay,
    key: KeyEvent,
) -> (AppModel, AppCommand) {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace => {
            model.system_menu = None;
            return (model, AppCommand::None);
        }
        KeyCode::Up => {
            menu.selected = menu.selected.saturating_sub(1);
        }
        KeyCode::Down => {
            menu.selected = (menu.selected + 1).min(SYSTEM_MENU_ITEMS.len().saturating_sub(1));
        }
        KeyCode::Enter => {
            let Some(item) = SYSTEM_MENU_ITEMS.get(menu.selected).copied() else {
                model.system_menu = None;
                return (model, AppCommand::None);
            };

            model.system_menu = None;

            match item {
                SystemMenuItem::Rescan => return (model, AppCommand::Rescan),
                SystemMenuItem::Processes => {
                    if !matches!(&model.view, View::Processes(_) | View::ProcessOutput(_)) {
                        open_processes_view(&mut model);
                    }
                    return (model, AppCommand::None);
                }
                SystemMenuItem::Help => {
                    model.help_open = true;
                    return (model, AppCommand::None);
                }
                SystemMenuItem::Quit => return (model, AppCommand::Quit),
            }
        }
        _ => {}
    }

    model.system_menu = Some(menu);
    (model, AppCommand::None)
}

fn open_processes_view(model: &mut AppModel) {
    let return_to = model.view.clone();
    model.view = View::Processes(ProcessesView {
        return_to: Box::new(return_to),
        selected: 0,
    });
}

fn update_on_paste(model: AppModel, text: String) -> (AppModel, AppCommand) {
    let mut model = model;
    model.notice = None;

    if model.system_menu.is_some() {
        return (model, AppCommand::None);
    }
    if model.help_open {
        return (model, AppCommand::None);
    }
    if model.delete_confirm.is_some() || model.delete_session_confirm.is_some() {
        return (model, AppCommand::None);
    }
    if model.session_result_preview.is_some() {
        return (model, AppCommand::None);
    }
    if model.session_stats_overlay.is_some() {
        return (model, AppCommand::None);
    }

    let view = model.view.clone();
    if let View::NewSession(mut new_session_view) = view {
        new_session_view.editor.insert_str(&text);
        model.view = View::NewSession(new_session_view);
    }

    (model, AppCommand::None)
}

fn update_session_result_preview_overlay(
    mut model: AppModel,
    mut preview: SessionResultPreviewOverlay,
    key: KeyEvent,
) -> (AppModel, AppCommand) {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace | KeyCode::Enter | KeyCode::Char(' ') => {
            model.session_result_preview = None;
            return (model, AppCommand::None);
        }
        KeyCode::Up => {
            preview.scroll = preview.scroll.saturating_sub(1);
        }
        KeyCode::Down => {
            preview.scroll = preview.scroll.saturating_add(1);
        }
        KeyCode::PageUp => {
            let step = page_step_standard_list(model.terminal_size) as u16;
            preview.scroll = preview.scroll.saturating_sub(step);
        }
        KeyCode::PageDown => {
            let step = page_step_standard_list(model.terminal_size) as u16;
            preview.scroll = preview.scroll.saturating_add(step);
        }
        _ => {}
    }

    model.session_result_preview = Some(preview);
    (model, AppCommand::None)
}

fn update_session_stats_overlay(
    mut model: AppModel,
    mut overlay: SessionStatsOverlay,
    key: KeyEvent,
) -> (AppModel, AppCommand) {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace => {
            model.session_stats_overlay = None;
            return (model, AppCommand::None);
        }
        KeyCode::Up => {
            overlay.scroll = overlay.scroll.saturating_sub(1);
        }
        KeyCode::Down => {
            overlay.scroll = overlay.scroll.saturating_add(1);
        }
        KeyCode::PageUp => {
            let step = page_step_standard_list(model.terminal_size) as u16;
            overlay.scroll = overlay.scroll.saturating_sub(step);
        }
        KeyCode::PageDown => {
            let step = page_step_standard_list(model.terminal_size) as u16;
            overlay.scroll = overlay.scroll.saturating_add(step);
        }
        _ => {}
    }

    model.session_stats_overlay = Some(overlay);
    (model, AppCommand::None)
}

fn update_delete_confirm(
    mut model: AppModel,
    mut confirm: DeleteConfirmDialog,
    key: KeyEvent,
) -> (AppModel, AppCommand) {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace => {
            model.delete_confirm = None;
            return (model, AppCommand::None);
        }
        KeyCode::Left | KeyCode::Right => {
            confirm.selection = confirm.selection.toggle();
        }
        KeyCode::Enter => {
            let command = if confirm.selection == DeleteConfirmSelection::Delete {
                AppCommand::DeleteProjectLogs {
                    project_path: confirm.project_path.clone(),
                }
            } else {
                AppCommand::None
            };
            model.delete_confirm = None;
            return (model, command);
        }
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            model.delete_confirm = None;
            return (
                model,
                AppCommand::DeleteProjectLogs {
                    project_path: confirm.project_path.clone(),
                },
            );
        }
        KeyCode::Char('n') | KeyCode::Char('N') => {
            model.delete_confirm = None;
            return (model, AppCommand::None);
        }
        _ => {}
    }

    model.delete_confirm = Some(confirm);
    (model, AppCommand::None)
}

fn update_delete_session_confirm(
    mut model: AppModel,
    mut confirm: DeleteSessionConfirmDialog,
    key: KeyEvent,
) -> (AppModel, AppCommand) {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace => {
            model.delete_session_confirm = None;
            return (model, AppCommand::None);
        }
        KeyCode::Left | KeyCode::Right => {
            confirm.selection = confirm.selection.toggle();
        }
        KeyCode::Enter => {
            let command = if confirm.selection == DeleteConfirmSelection::Delete {
                AppCommand::DeleteSessionLog {
                    log_path: confirm.log_path.clone(),
                }
            } else {
                AppCommand::None
            };
            model.delete_session_confirm = None;
            return (model, command);
        }
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            model.delete_session_confirm = None;
            return (
                model,
                AppCommand::DeleteSessionLog {
                    log_path: confirm.log_path.clone(),
                },
            );
        }
        KeyCode::Char('n') | KeyCode::Char('N') => {
            model.delete_session_confirm = None;
            return (model, AppCommand::None);
        }
        _ => {}
    }

    model.delete_session_confirm = Some(confirm);
    (model, AppCommand::None)
}

fn update_error(model: AppModel, key: KeyEvent) -> (AppModel, AppCommand) {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace => (
            AppModel {
                data: model.data.clone(),
                terminal_size: model.terminal_size,
                notice: None,
                update_hint: model.update_hint.clone(),
                help_open: model.help_open,
                system_menu: model.system_menu.clone(),
                delete_confirm: model.delete_confirm.clone(),
                delete_session_confirm: model.delete_session_confirm.clone(),
                session_result_preview: model.session_result_preview.clone(),
                session_stats_overlay: model.session_stats_overlay.clone(),
                processes: model.processes.clone(),
                view: View::Projects(ProjectsView::new(&model.data.projects)),
            },
            AppCommand::None,
        ),
        _ => (model, AppCommand::None),
    }
}

fn update_projects(
    mut model: AppModel,
    mut view: ProjectsView,
    key: KeyEvent,
) -> (AppModel, AppCommand) {
    match key.code {
        KeyCode::Enter => {
            let Some(project_index) = view.filtered_indices.get(view.selected).copied() else {
                return (model, AppCommand::None);
            };
            let Some(project) = model.data.projects.get(project_index) else {
                return (model, AppCommand::None);
            };
            let next = AppModel {
                data: model.data.clone(),
                terminal_size: model.terminal_size,
                notice: None,
                update_hint: model.update_hint.clone(),
                help_open: model.help_open,
                system_menu: model.system_menu.clone(),
                delete_confirm: model.delete_confirm.clone(),
                delete_session_confirm: model.delete_session_confirm.clone(),
                session_result_preview: model.session_result_preview.clone(),
                session_stats_overlay: model.session_stats_overlay.clone(),
                processes: model.processes.clone(),
                view: View::Sessions(SessionsView::new(
                    project.project_path.clone(),
                    project.sessions.len(),
                )),
            };
            return (next, AppCommand::None);
        }
        KeyCode::Esc => {
            if !view.query.is_empty() {
                view.query.clear();
                apply_project_filter(&model.data.projects, &mut view);
            }
        }
        KeyCode::Char(' ') => {
            let Some(project_index) = view.filtered_indices.get(view.selected).copied() else {
                return (model, AppCommand::None);
            };
            let Some(project) = model.data.projects.get(project_index) else {
                return (model, AppCommand::None);
            };
            let Some(session) = project.sessions.first().cloned() else {
                model.notice = Some("No sessions in this project.".to_string());
                return (model, AppCommand::None);
            };

            return (model, AppCommand::OpenSessionResultPreview { session });
        }
        KeyCode::Up => {
            view.selected = view.selected.saturating_sub(1);
        }
        KeyCode::Down => {
            if !view.filtered_indices.is_empty() {
                view.selected =
                    (view.selected + 1).min(view.filtered_indices.len().saturating_sub(1));
            }
        }
        KeyCode::PageUp => {
            let step = page_step_standard_list(model.terminal_size);
            view.selected = view.selected.saturating_sub(step);
        }
        KeyCode::PageDown => {
            if !view.filtered_indices.is_empty() {
                let step = page_step_standard_list(model.terminal_size);
                view.selected =
                    (view.selected + step).min(view.filtered_indices.len().saturating_sub(1));
            }
        }
        KeyCode::Backspace => {
            if !view.query.is_empty() {
                view.query.pop();
                apply_project_filter(&model.data.projects, &mut view);
            } else {
                open_delete_confirm(&mut model, &view);
            }
        }
        KeyCode::Delete => {
            open_delete_confirm(&mut model, &view);
        }
        KeyCode::Char(character) => {
            if is_text_input_char(character) {
                view.query.push(character);
                apply_project_filter(&model.data.projects, &mut view);
            }
        }
        _ => {}
    }

    (
        AppModel {
            data: model.data.clone(),
            terminal_size: model.terminal_size,
            notice: None,
            update_hint: model.update_hint.clone(),
            help_open: model.help_open,
            system_menu: model.system_menu.clone(),
            delete_confirm: model.delete_confirm.clone(),
            delete_session_confirm: model.delete_session_confirm.clone(),
            session_result_preview: model.session_result_preview.clone(),
            session_stats_overlay: model.session_stats_overlay.clone(),
            processes: model.processes.clone(),
            view: View::Projects(view),
        },
        AppCommand::None,
    )
}

fn open_delete_confirm(model: &mut AppModel, view: &ProjectsView) {
    let Some(project_index) = view.filtered_indices.get(view.selected).copied() else {
        return;
    };
    let Some(project) = model.data.projects.get(project_index) else {
        return;
    };

    let total_size_bytes = project
        .sessions
        .iter()
        .map(|session| session.file_size_bytes)
        .sum();

    model.delete_confirm = Some(DeleteConfirmDialog {
        project_name: project.name.clone(),
        project_path: project.project_path.clone(),
        session_count: project.sessions.len(),
        total_size_bytes,
        selection: DeleteConfirmSelection::Cancel,
    });
}

fn apply_project_filter(projects: &[ProjectSummary], view: &mut ProjectsView) {
    let query = view.query.trim().to_lowercase();
    if query.is_empty() {
        view.filtered_indices = (0..projects.len()).collect();
    } else {
        view.filtered_indices = projects
            .iter()
            .enumerate()
            .filter_map(|(index, project)| {
                let haystack = format!(
                    "{}\n{}",
                    project.name.to_lowercase(),
                    project.project_path.display().to_string().to_lowercase()
                );
                if haystack.contains(&query) {
                    Some(index)
                } else {
                    None
                }
            })
            .collect();
    }

    if view.filtered_indices.is_empty() {
        view.selected = 0;
    } else {
        view.selected = view
            .selected
            .min(view.filtered_indices.len().saturating_sub(1));
    }
}

fn apply_session_filter(sessions: &[SessionSummary], view: &mut SessionsView) {
    let query = view.query.trim().to_lowercase();
    if query.is_empty() {
        view.filtered_indices = (0..sessions.len()).collect();
    } else {
        view.filtered_indices = sessions
            .iter()
            .enumerate()
            .filter_map(|(index, session)| {
                let haystack = format!(
                    "{}\n{}\n{}\n{}",
                    session.title.to_lowercase(),
                    session.meta.id.to_lowercase(),
                    session.meta.started_at_rfc3339.to_lowercase(),
                    session.log_path.display().to_string().to_lowercase()
                );
                if haystack.contains(&query) {
                    Some(index)
                } else {
                    None
                }
            })
            .collect();
    }

    if view.filtered_indices.is_empty() {
        view.session_selected = 0;
    } else {
        view.session_selected = view
            .session_selected
            .min(view.filtered_indices.len().saturating_sub(1));
    }
}

fn open_delete_session_confirm(model: &mut AppModel, view: &SessionsView) -> bool {
    let Some(project) = view.current_project(&model.data.projects) else {
        return false;
    };
    let Some(selected_index) = view.filtered_indices.get(view.session_selected).copied() else {
        return false;
    };
    let Some(session) = project.sessions.get(selected_index) else {
        return false;
    };

    model.delete_session_confirm = Some(DeleteSessionConfirmDialog {
        project_name: project.name.clone(),
        project_path: project.project_path.clone(),
        session_title: session.title.clone(),
        log_path: session.log_path.clone(),
        size_bytes: session.file_size_bytes,
        file_modified: session.file_modified,
        selection: DeleteConfirmSelection::Cancel,
    });

    true
}

fn update_sessions(
    mut model: AppModel,
    mut view: SessionsView,
    key: KeyEvent,
) -> (AppModel, AppCommand) {
    let new_modifier = key.modifiers.contains(KeyModifiers::CONTROL)
        || key.modifiers.contains(KeyModifiers::SUPER)
        || key.modifiers.contains(KeyModifiers::META);

    match key.code {
        KeyCode::F(3) => {
            let Some(project) = view.current_project(&model.data.projects) else {
                return (model, AppCommand::None);
            };
            let Some(selected_index) = view.filtered_indices.get(view.session_selected).copied()
            else {
                model.notice = Some("No session selected.".to_string());
                return (model, AppCommand::None);
            };
            let Some(session) = project.sessions.get(selected_index).cloned() else {
                model.notice = Some("No session selected.".to_string());
                return (model, AppCommand::None);
            };
            return (model, AppCommand::OpenSessionStats { session });
        }
        KeyCode::Enter => {
            let Some(project) = view.current_project(&model.data.projects) else {
                return (model, AppCommand::None);
            };
            let Some(selected_index) = view.filtered_indices.get(view.session_selected).copied()
            else {
                return (model, AppCommand::None);
            };
            let Some(session) = project.sessions.get(selected_index).cloned() else {
                return (model, AppCommand::None);
            };
            return (
                model,
                AppCommand::OpenSessionDetail {
                    from_sessions: view,
                    session,
                },
            );
        }
        KeyCode::Esc => {
            if !view.query.is_empty() {
                view.query.clear();
                if let Some(project) = view.current_project(&model.data.projects) {
                    apply_session_filter(&project.sessions, &mut view);
                } else {
                    view.filtered_indices.clear();
                    view.session_selected = 0;
                }
                model.view = View::Sessions(view);
                return (model, AppCommand::None);
            }

            let next = AppModel {
                data: model.data.clone(),
                terminal_size: model.terminal_size,
                notice: None,
                update_hint: model.update_hint.clone(),
                help_open: model.help_open,
                system_menu: model.system_menu.clone(),
                delete_confirm: model.delete_confirm.clone(),
                delete_session_confirm: model.delete_session_confirm.clone(),
                session_result_preview: model.session_result_preview.clone(),
                session_stats_overlay: model.session_stats_overlay.clone(),
                processes: model.processes.clone(),
                view: View::Projects(ProjectsView::new(&model.data.projects)),
            };
            return (next, AppCommand::None);
        }
        KeyCode::Up => {
            view.session_selected = view.session_selected.saturating_sub(1);
        }
        KeyCode::Down => {
            if !view.filtered_indices.is_empty() {
                view.session_selected =
                    (view.session_selected + 1).min(view.filtered_indices.len().saturating_sub(1));
            }
        }
        KeyCode::PageUp => {
            let step = page_step_standard_list(model.terminal_size);
            view.session_selected = view.session_selected.saturating_sub(step);
        }
        KeyCode::PageDown => {
            if !view.filtered_indices.is_empty() {
                let step = page_step_standard_list(model.terminal_size);
                view.session_selected = (view.session_selected + step)
                    .min(view.filtered_indices.len().saturating_sub(1));
            }
        }
        KeyCode::Backspace => {
            if !view.query.is_empty() {
                view.query.pop();
                if let Some(project) = view.current_project(&model.data.projects) {
                    apply_session_filter(&project.sessions, &mut view);
                } else {
                    view.filtered_indices.clear();
                    view.session_selected = 0;
                }
                model.view = View::Sessions(view);
                return (model, AppCommand::None);
            }

            let opened = open_delete_session_confirm(&mut model, &view);
            if !opened {
                let next = AppModel {
                    data: model.data.clone(),
                    terminal_size: model.terminal_size,
                    notice: None,
                    update_hint: model.update_hint.clone(),
                    help_open: model.help_open,
                    system_menu: model.system_menu.clone(),
                    delete_confirm: model.delete_confirm.clone(),
                    delete_session_confirm: model.delete_session_confirm.clone(),
                    session_result_preview: model.session_result_preview.clone(),
                    session_stats_overlay: model.session_stats_overlay.clone(),
                    processes: model.processes.clone(),
                    view: View::Projects(ProjectsView::new(&model.data.projects)),
                };
                return (next, AppCommand::None);
            }
        }
        KeyCode::Delete => {
            open_delete_session_confirm(&mut model, &view);
        }
        KeyCode::Char('n') | KeyCode::Char('N') if new_modifier => {
            let next = AppModel {
                data: model.data.clone(),
                terminal_size: model.terminal_size,
                notice: None,
                update_hint: model.update_hint.clone(),
                help_open: model.help_open,
                system_menu: model.system_menu.clone(),
                delete_confirm: model.delete_confirm.clone(),
                delete_session_confirm: model.delete_session_confirm.clone(),
                session_result_preview: model.session_result_preview.clone(),
                session_stats_overlay: model.session_stats_overlay.clone(),
                processes: model.processes.clone(),
                view: View::NewSession(NewSessionView::new(view.clone())),
            };
            return (next, AppCommand::None);
        }
        KeyCode::Char(' ') => {
            let Some(project) = view.current_project(&model.data.projects) else {
                return (model, AppCommand::None);
            };
            let Some(selected_index) = view.filtered_indices.get(view.session_selected).copied()
            else {
                return (model, AppCommand::None);
            };
            let Some(session) = project.sessions.get(selected_index).cloned() else {
                return (model, AppCommand::None);
            };
            return (model, AppCommand::OpenSessionResultPreview { session });
        }
        KeyCode::Char(character) => {
            if is_text_input_char(character) {
                view.query.push(character);
                if let Some(project) = view.current_project(&model.data.projects) {
                    apply_session_filter(&project.sessions, &mut view);
                } else {
                    view.filtered_indices.clear();
                    view.session_selected = 0;
                }
            }
        }
        _ => {}
    }

    (
        AppModel {
            data: model.data.clone(),
            terminal_size: model.terminal_size,
            notice: None,
            update_hint: model.update_hint.clone(),
            help_open: model.help_open,
            system_menu: model.system_menu.clone(),
            delete_confirm: model.delete_confirm.clone(),
            delete_session_confirm: model.delete_session_confirm.clone(),
            session_result_preview: model.session_result_preview.clone(),
            session_stats_overlay: model.session_stats_overlay.clone(),
            processes: model.processes.clone(),
            view: View::Sessions(view),
        },
        AppCommand::None,
    )
}

fn update_new_session(
    mut model: AppModel,
    mut view: NewSessionView,
    key: KeyEvent,
) -> (AppModel, AppCommand) {
    let send_modifier = key.modifiers.contains(KeyModifiers::CONTROL)
        || key.modifiers.contains(KeyModifiers::SUPER)
        || key.modifiers.contains(KeyModifiers::META);

    match key.code {
        KeyCode::Esc => {
            model.view = View::Sessions(view.from_sessions.clone());
            return (model, AppCommand::None);
        }
        KeyCode::BackTab => {
            view.engine = view.engine.toggle();
        }
        KeyCode::Enter if send_modifier => {
            let prompt = view.editor.text();
            if prompt.trim().is_empty() {
                model.notice = Some("Prompt is empty.".to_string());
                model.view = View::NewSession(view);
                return (model, AppCommand::None);
            }

            let project_path = view.from_sessions.project_path.clone();
            let engine = view.engine;
            model.view = View::Sessions(view.from_sessions.clone());
            return (
                model,
                AppCommand::SpawnAgentSession {
                    engine,
                    project_path,
                    prompt,
                },
            );
        }
        KeyCode::Enter => {
            view.editor.insert_newline();
        }
        KeyCode::Backspace => {
            view.editor.backspace();
        }
        KeyCode::Delete => {
            view.editor.delete_forward();
        }
        KeyCode::Left => {
            view.editor.move_left();
        }
        KeyCode::Right => {
            view.editor.move_right();
        }
        KeyCode::Up => {
            view.editor.move_up();
        }
        KeyCode::Down => {
            view.editor.move_down();
        }
        KeyCode::Home => {
            view.editor.move_home();
        }
        KeyCode::End => {
            view.editor.move_end();
        }
        KeyCode::Tab => {
            view.editor.insert_str("    ");
        }
        KeyCode::Char(character) => {
            if is_text_input_char(character) {
                view.editor.insert_char(character);
            }
        }
        _ => {}
    }

    model.view = View::NewSession(view);
    (model, AppCommand::None)
}

fn update_processes(
    mut model: AppModel,
    mut view: ProcessesView,
    key: KeyEvent,
) -> (AppModel, AppCommand) {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace => {
            model.view = *view.return_to;
            return (model, AppCommand::None);
        }
        KeyCode::Up => {
            view.selected = view.selected.saturating_sub(1);
        }
        KeyCode::Down => {
            if !model.processes.is_empty() {
                view.selected = (view.selected + 1).min(model.processes.len().saturating_sub(1));
            }
        }
        KeyCode::PageUp => {
            let step = page_step_standard_list(model.terminal_size);
            view.selected = view.selected.saturating_sub(step);
        }
        KeyCode::PageDown => {
            if !model.processes.is_empty() {
                let step = page_step_standard_list(model.terminal_size);
                view.selected = (view.selected + step).min(model.processes.len().saturating_sub(1));
            }
        }
        KeyCode::Char('s') | KeyCode::Char('S') => {
            if let Some(process_id) = model
                .processes
                .get(view.selected)
                .map(|process| process.id.clone())
            {
                model.view = View::Processes(view);
                return (
                    model,
                    AppCommand::OpenProcessOutput {
                        process_id,
                        kind: ProcessOutputKind::Stdout,
                    },
                );
            }
        }
        KeyCode::Char('e') | KeyCode::Char('E') => {
            if let Some(process_id) = model
                .processes
                .get(view.selected)
                .map(|process| process.id.clone())
            {
                model.view = View::Processes(view);
                return (
                    model,
                    AppCommand::OpenProcessOutput {
                        process_id,
                        kind: ProcessOutputKind::Stderr,
                    },
                );
            }
        }
        KeyCode::Char('l') | KeyCode::Char('L') => {
            if let Some(process_id) = model
                .processes
                .get(view.selected)
                .map(|process| process.id.clone())
            {
                model.view = View::Processes(view);
                return (
                    model,
                    AppCommand::OpenProcessOutput {
                        process_id,
                        kind: ProcessOutputKind::Log,
                    },
                );
            }
        }
        KeyCode::Char('k') | KeyCode::Char('K') => {
            if let Some(process_id) = model
                .processes
                .get(view.selected)
                .map(|process| process.id.clone())
            {
                model.view = View::Processes(view);
                return (model, AppCommand::KillProcess { process_id });
            }
        }
        KeyCode::Enter => {
            if let Some((project_path, session_log_path)) =
                model.processes.get(view.selected).map(|process| {
                    (
                        process.project_path.clone(),
                        process.session_log_path.clone(),
                    )
                })
            {
                if let Some(log_path) = session_log_path {
                    model.view = View::Processes(view);
                    return (
                        model,
                        AppCommand::OpenSessionDetailByLogPath {
                            project_path,
                            log_path,
                        },
                    );
                }

                model.notice = Some("No session log path yet.".to_string());
            }
        }
        _ => {}
    }

    view.selected = view.selected.min(model.processes.len().saturating_sub(1));
    model.view = View::Processes(view);
    (model, AppCommand::None)
}

fn update_process_output(
    mut model: AppModel,
    mut view: ProcessOutputView,
    key: KeyEvent,
) -> (AppModel, AppCommand) {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace => {
            model.view = *view.return_to;
            return (model, AppCommand::None);
        }
        KeyCode::Up => {
            view.scroll = view.scroll.saturating_sub(1);
        }
        KeyCode::Down => {
            view.scroll = view.scroll.saturating_add(1);
        }
        KeyCode::PageUp => {
            let step = page_step_standard_list(model.terminal_size) as u16;
            view.scroll = view.scroll.saturating_sub(step);
        }
        KeyCode::PageDown => {
            let step = page_step_standard_list(model.terminal_size) as u16;
            view.scroll = view.scroll.saturating_add(step);
        }
        KeyCode::Char('s') | KeyCode::Char('S') => {
            let process_id = view.process_id.clone();
            model.view = View::ProcessOutput(view);
            return (
                model,
                AppCommand::OpenProcessOutput {
                    process_id,
                    kind: ProcessOutputKind::Stdout,
                },
            );
        }
        KeyCode::Char('e') | KeyCode::Char('E') => {
            let process_id = view.process_id.clone();
            model.view = View::ProcessOutput(view);
            return (
                model,
                AppCommand::OpenProcessOutput {
                    process_id,
                    kind: ProcessOutputKind::Stderr,
                },
            );
        }
        KeyCode::Char('l') | KeyCode::Char('L') => {
            let process_id = view.process_id.clone();
            model.view = View::ProcessOutput(view);
            return (
                model,
                AppCommand::OpenProcessOutput {
                    process_id,
                    kind: ProcessOutputKind::Log,
                },
            );
        }
        KeyCode::Char('k') | KeyCode::Char('K') => {
            let process_id = view.process_id.clone();
            model.view = View::ProcessOutput(view);
            return (model, AppCommand::KillProcess { process_id });
        }
        _ => {}
    }

    model.view = View::ProcessOutput(view);
    (model, AppCommand::None)
}

fn update_session_detail(
    mut model: AppModel,
    mut view: SessionDetailView,
    key: KeyEvent,
) -> (AppModel, AppCommand) {
    if view.output_overlay_open {
        match key.code {
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Enter => {
                view.output_overlay_open = false;
            }
            KeyCode::Up => {
                view.output_overlay_scroll = view.output_overlay_scroll.saturating_sub(1);
            }
            KeyCode::Down => {
                view.output_overlay_scroll = view.output_overlay_scroll.saturating_add(1);
            }
            KeyCode::PageUp => {
                let step = page_step_standard_list(model.terminal_size) as u16;
                view.output_overlay_scroll = view.output_overlay_scroll.saturating_sub(step);
            }
            KeyCode::PageDown => {
                let step = page_step_standard_list(model.terminal_size) as u16;
                view.output_overlay_scroll = view.output_overlay_scroll.saturating_add(step);
            }
            KeyCode::Char('o') | KeyCode::Char('O') => {
                view.output_overlay_open = false;
            }
            _ => {}
        }

        model.view = View::SessionDetail(view);
        return (model, AppCommand::None);
    }

    match key.code {
        KeyCode::Esc | KeyCode::Backspace => {
            if view.context_overlay_open {
                view.context_overlay_open = false;
            } else {
                model.view = View::Sessions(view.from_sessions.clone());
                return (model, AppCommand::None);
            }
        }
        KeyCode::F(3) => {
            return (
                model,
                AppCommand::OpenSessionStats {
                    session: view.session.clone(),
                },
            );
        }
        KeyCode::Up => {
            view.selected = view.selected.saturating_sub(1);
        }
        KeyCode::Down => {
            if !view.items.is_empty() {
                view.selected = (view.selected + 1).min(view.items.len().saturating_sub(1));
            }
        }
        KeyCode::PageUp => {
            let step = page_step_session_detail_list(model.terminal_size);
            view.selected = view.selected.saturating_sub(step);
        }
        KeyCode::PageDown => {
            if !view.items.is_empty() {
                let step = page_step_session_detail_list(model.terminal_size);
                view.selected = (view.selected + step).min(view.items.len().saturating_sub(1));
            }
        }
        KeyCode::Enter => {
            let selected = view.selected.min(view.items.len().saturating_sub(1));
            if let Some(item) = view.items.get(selected) {
                if item.kind == TimelineItemKind::ToolCall {
                    if let Some(call_id) = item.call_id.as_deref() {
                        if let Some(output_index) =
                            find_tool_output_index(&view.items, selected, call_id)
                        {
                            view.selected = output_index;
                        } else {
                            model.notice =
                                Some("No ToolOut found for the selected Tool call.".to_string());
                        }

                        model.view = View::SessionDetail(view);
                        return (model, AppCommand::None);
                    }
                }
            }
        }
        KeyCode::Char('o') | KeyCode::Char('O') => {
            if view.last_output.is_some() {
                view.output_overlay_open = true;
            } else {
                model.notice = Some("No assistant output found.".to_string());
            }
        }
        KeyCode::Char('c') => {
            view.context_overlay_open = !view.context_overlay_open;
        }
        _ => {}
    }

    model.view = View::SessionDetail(view);
    (model, AppCommand::None)
}

fn is_text_input_char(character: char) -> bool {
    !character.is_control()
}

fn find_tool_output_index(
    items: &[TimelineItem],
    selected_index: usize,
    call_id: &str,
) -> Option<usize> {
    if selected_index + 1 < items.len() {
        if let Some((index, _)) =
            items
                .iter()
                .enumerate()
                .skip(selected_index + 1)
                .find(|(_, item)| {
                    item.kind == TimelineItemKind::ToolOutput
                        && item.call_id.as_deref() == Some(call_id)
                })
        {
            return Some(index);
        }
    }

    items
        .iter()
        .enumerate()
        .find(|(_, item)| {
            item.kind == TimelineItemKind::ToolOutput && item.call_id.as_deref() == Some(call_id)
        })
        .map(|(index, _)| index)
}

fn inner_terminal_size(terminal_size: (u16, u16)) -> (u16, u16) {
    let (width, height) = terminal_size;
    if width < 40 || height < 12 {
        return (width, height);
    }

    (width.saturating_sub(4), height.saturating_sub(2))
}

fn page_step_for_height(height: u16) -> usize {
    height.saturating_sub(1).max(1) as usize
}

fn page_step_standard_list(terminal_size: (u16, u16)) -> usize {
    let (_inner_width, inner_height) = inner_terminal_size(terminal_size);
    let list_height = inner_height.saturating_sub(4); // header 3 + footer 1
    page_step_for_height(list_height)
}

fn page_step_session_detail_list(terminal_size: (u16, u16)) -> usize {
    let (inner_width, inner_height) = inner_terminal_size(terminal_size);
    let body_height = inner_height.saturating_sub(4); // header 3 + footer 1

    let list_height = if inner_width >= 90 {
        body_height
    } else {
        let body_height = u32::from(body_height);
        u16::try_from((body_height * 60) / 100).unwrap_or(0)
    };

    page_step_for_height(list_height)
}

pub fn build_index_from_sessions(
    sessions_dir: PathBuf,
    sessions: Vec<crate::domain::SessionSummary>,
    warnings: ScanWarningCount,
) -> AppData {
    let projects = index_projects(&sessions);
    AppData::from_scan(sessions_dir, projects, warnings)
}
