mod fork;
mod line_editor;
mod mouse;
mod text_editor;

use crate::app::fork::{default_fork_prompt, fork_context_from_timeline_item};
use crate::domain::{
    AgentEngine, ForkContext, ProjectIndex, ProjectSummary, SessionStats, SessionSummary,
    SpawnIoMode, Task, TaskId, TaskImage, TimelineItem, TimelineItemKind, TurnContextSummary,
    index_projects,
};
use crate::infra::{ScanWarningCount, SessionIndex};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;
use thiserror::Error;

pub use line_editor::LineEditor;
pub use text_editor::TextEditor;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("terminal I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    ResolveSessionsDir(#[from] crate::infra::ResolveSessionsDirError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EngineFilter {
    All,
    Codex,
    Claude,
    Gemini,
}

impl EngineFilter {
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Codex => "Codex",
            Self::Claude => "Claude",
            Self::Gemini => "Gemini",
        }
    }
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
}

#[derive(Clone, Debug)]
pub struct AppModel {
    pub data: AppData,
    pub session_index: Arc<SessionIndex>,
    pub view: View,
    pub terminal_size: (u16, u16),
    pub notice: Option<String>,
    pub update_hint: Option<String>,
    pub engine_filter: EngineFilter,
    pub help_open: bool,
    pub system_menu: Option<SystemMenuOverlay>,
    pub delete_confirm: Option<DeleteConfirmDialog>,
    pub delete_projects_confirm: Option<DeleteProjectsConfirmDialog>,
    pub delete_session_confirm: Option<DeleteSessionConfirmDialog>,
    pub delete_sessions_confirm: Option<DeleteSessionsConfirmDialog>,
    pub delete_task_confirm: Option<DeleteTaskConfirmDialog>,
    pub delete_tasks_confirm: Option<DeleteTasksConfirmDialog>,
    pub session_result_preview: Option<SessionResultPreviewOverlay>,
    pub session_stats_overlay: Option<SessionStatsOverlay>,
    pub project_stats_overlay: Option<ProjectStatsOverlay>,
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
            session_index: Arc::new(SessionIndex::default()),
            view,
            terminal_size: (0, 0),
            notice: None,
            update_hint: None,
            engine_filter: EngineFilter::All,
            help_open: false,
            system_menu: None,
            delete_confirm: None,
            delete_projects_confirm: None,
            delete_session_confirm: None,
            delete_sessions_confirm: None,
            delete_task_confirm: None,
            delete_tasks_confirm: None,
            session_result_preview: None,
            session_stats_overlay: None,
            project_stats_overlay: None,
            processes: Vec::new(),
        }
    }

    pub fn with_data(&self, data: AppData) -> Self {
        if data.load_error.is_some() {
            return Self {
                data,
                session_index: self.session_index.clone(),
                view: View::Error,
                terminal_size: self.terminal_size,
                notice: None,
                update_hint: self.update_hint.clone(),
                engine_filter: self.engine_filter,
                help_open: self.help_open,
                system_menu: self.system_menu.clone(),
                delete_confirm: self.delete_confirm.clone(),
                delete_projects_confirm: self.delete_projects_confirm.clone(),
                delete_session_confirm: self.delete_session_confirm.clone(),
                delete_sessions_confirm: self.delete_sessions_confirm.clone(),
                delete_task_confirm: self.delete_task_confirm.clone(),
                delete_tasks_confirm: self.delete_tasks_confirm.clone(),
                session_result_preview: self.session_result_preview.clone(),
                session_stats_overlay: self.session_stats_overlay.clone(),
                project_stats_overlay: self.project_stats_overlay.clone(),
                processes: self.processes.clone(),
            };
        }

        let view = match &self.view {
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
                    selection_anchor: projects_view.selection_anchor.clone(),
                    selected_project_paths: projects_view.selected_project_paths.clone(),
                };
                apply_project_filter(&data.projects, &mut next_view, self.engine_filter);
                prune_project_selection(&data.projects, &mut next_view);

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
                        apply_session_filter(&project.sessions, &mut next_view, self.engine_filter);
                        prune_sessions_selection(&project.sessions, &mut next_view);
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
                    None => {
                        let mut projects_view = ProjectsView::new(&data.projects);
                        apply_project_filter(
                            &data.projects,
                            &mut projects_view,
                            self.engine_filter,
                        );
                        View::Projects(projects_view)
                    }
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
                        apply_session_filter(
                            &project.sessions,
                            &mut next_view.from_sessions,
                            self.engine_filter,
                        );
                        prune_sessions_selection(&project.sessions, &mut next_view.from_sessions);

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
                    None => {
                        let mut projects_view = ProjectsView::new(&data.projects);
                        apply_project_filter(
                            &data.projects,
                            &mut projects_view,
                            self.engine_filter,
                        );
                        View::Projects(projects_view)
                    }
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
                        apply_session_filter(
                            &project.sessions,
                            &mut next_view.from_sessions,
                            self.engine_filter,
                        );
                        prune_sessions_selection(&project.sessions, &mut next_view.from_sessions);
                        if let Some(pos) = project
                            .sessions
                            .iter()
                            .position(|session| session.log_path == detail_view.session.log_path)
                        {
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
                    None => {
                        let mut projects_view = ProjectsView::new(&data.projects);
                        apply_project_filter(
                            &data.projects,
                            &mut projects_view,
                            self.engine_filter,
                        );
                        View::Projects(projects_view)
                    }
                }
            }
            View::Tasks(tasks_view) => View::Tasks(tasks_view.clone()),
            View::TaskCreate(task_create_view) => View::TaskCreate(task_create_view.clone()),
            View::TaskDetail(task_detail_view) => View::TaskDetail(task_detail_view.clone()),
            View::Processes(processes_view) => View::Processes(processes_view.clone()),
            View::ProcessOutput(output_view) => View::ProcessOutput(output_view.clone()),
            View::Error => {
                let mut projects_view = ProjectsView::new(&data.projects);
                apply_project_filter(&data.projects, &mut projects_view, self.engine_filter);
                View::Projects(projects_view)
            }
        };

        Self {
            data,
            session_index: self.session_index.clone(),
            view,
            terminal_size: self.terminal_size,
            notice: None,
            update_hint: self.update_hint.clone(),
            engine_filter: self.engine_filter,
            help_open: self.help_open,
            system_menu: self.system_menu.clone(),
            delete_confirm: self.delete_confirm.clone(),
            delete_projects_confirm: self.delete_projects_confirm.clone(),
            delete_session_confirm: self.delete_session_confirm.clone(),
            delete_sessions_confirm: self.delete_sessions_confirm.clone(),
            delete_task_confirm: self.delete_task_confirm.clone(),
            delete_tasks_confirm: self.delete_tasks_confirm.clone(),
            session_result_preview: self.session_result_preview.clone(),
            session_stats_overlay: self.session_stats_overlay.clone(),
            project_stats_overlay: self.project_stats_overlay.clone(),
            processes: self.processes.clone(),
        }
    }

    pub fn with_terminal_size(&self, width: u16, height: u16) -> Self {
        Self {
            data: self.data.clone(),
            session_index: self.session_index.clone(),
            view: self.view.clone(),
            terminal_size: (width, height),
            notice: self.notice.clone(),
            update_hint: self.update_hint.clone(),
            engine_filter: self.engine_filter,
            help_open: self.help_open,
            system_menu: self.system_menu.clone(),
            delete_confirm: self.delete_confirm.clone(),
            delete_projects_confirm: self.delete_projects_confirm.clone(),
            delete_session_confirm: self.delete_session_confirm.clone(),
            delete_sessions_confirm: self.delete_sessions_confirm.clone(),
            delete_task_confirm: self.delete_task_confirm.clone(),
            delete_tasks_confirm: self.delete_tasks_confirm.clone(),
            session_result_preview: self.session_result_preview.clone(),
            session_stats_overlay: self.session_stats_overlay.clone(),
            project_stats_overlay: self.project_stats_overlay.clone(),
            processes: self.processes.clone(),
        }
    }

    pub fn with_notice(&self, notice: Option<String>) -> Self {
        Self {
            data: self.data.clone(),
            session_index: self.session_index.clone(),
            view: self.view.clone(),
            terminal_size: self.terminal_size,
            notice,
            update_hint: self.update_hint.clone(),
            engine_filter: self.engine_filter,
            help_open: self.help_open,
            system_menu: self.system_menu.clone(),
            delete_confirm: self.delete_confirm.clone(),
            delete_projects_confirm: self.delete_projects_confirm.clone(),
            delete_session_confirm: self.delete_session_confirm.clone(),
            delete_sessions_confirm: self.delete_sessions_confirm.clone(),
            delete_task_confirm: self.delete_task_confirm.clone(),
            delete_tasks_confirm: self.delete_tasks_confirm.clone(),
            session_result_preview: self.session_result_preview.clone(),
            session_stats_overlay: self.session_stats_overlay.clone(),
            project_stats_overlay: self.project_stats_overlay.clone(),
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
            session_index: self.session_index.clone(),
            terminal_size: self.terminal_size,
            notice: None,
            update_hint: self.update_hint.clone(),
            engine_filter: self.engine_filter,
            help_open: self.help_open,
            system_menu: self.system_menu.clone(),
            delete_confirm: self.delete_confirm.clone(),
            delete_projects_confirm: self.delete_projects_confirm.clone(),
            delete_session_confirm: self.delete_session_confirm.clone(),
            delete_sessions_confirm: self.delete_sessions_confirm.clone(),
            delete_task_confirm: self.delete_task_confirm.clone(),
            delete_tasks_confirm: self.delete_tasks_confirm.clone(),
            session_result_preview: self.session_result_preview.clone(),
            session_stats_overlay: self.session_stats_overlay.clone(),
            project_stats_overlay: self.project_stats_overlay.clone(),
            processes: self.processes.clone(),
            view: View::SessionDetail(SessionDetailView {
                from_sessions,
                session,
                items,
                turn_contexts,
                warnings,
                truncated,
                selected: 0,
                focus: SessionDetailFocus::Timeline,
                details_scroll: 0,
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

#[derive(Clone, Debug)]
pub struct ProjectStatsOverlay {
    pub project_name: String,
    pub project_path: PathBuf,
    pub session_count: usize,
    pub indexed_sessions: usize,
    pub total_tokens_indexed: u64,
    pub missing_tokens_sessions: usize,
    pub scroll: u16,
}

impl ProjectStatsOverlay {
    pub fn from_project(project: &ProjectSummary, index: &SessionIndex) -> Self {
        let mut total_tokens_indexed = 0u64;
        let mut indexed_sessions = 0usize;
        let mut missing_tokens_sessions = 0usize;
        for session in &project.sessions {
            match index.total_tokens(&session.log_path) {
                Some(tokens) => {
                    total_tokens_indexed = total_tokens_indexed.saturating_add(tokens);
                    indexed_sessions = indexed_sessions.saturating_add(1);
                }
                None => {
                    missing_tokens_sessions = missing_tokens_sessions.saturating_add(1);
                }
            }
        }

        Self {
            project_name: project.name.clone(),
            project_path: project.project_path.clone(),
            session_count: project.sessions.len(),
            indexed_sessions,
            total_tokens_indexed,
            missing_tokens_sessions,
            scroll: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MainMenu {
    System,
    /// Requirement: every distinct screen/window should be reachable from the Window menu.
    /// When adding a new `View` variant or a new popup window, add a Window menu entry.
    Window,
    Engine,
    Projects,
    Sessions,
    NewSession,
    Session,
    Tasks,
    TaskCreate,
    TaskDetail,
    Processes,
    ProcessOutput,
    Error,
}

impl MainMenu {
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Window => "Window",
            Self::Engine => "Engine",
            Self::Projects => "Projects",
            Self::Sessions => "Sessions",
            Self::NewSession => "New Session",
            Self::Session => "Session",
            Self::Tasks => "Tasks",
            Self::TaskCreate => "New Task",
            Self::TaskDetail => "Task",
            Self::Processes => "Processes",
            Self::ProcessOutput => "Output",
            Self::Error => "Error",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MainMenuKey {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MainMenuEntry {
    pub label: &'static str,
    pub hotkey: &'static str,
    pub key: MainMenuKey,
}

pub const MAIN_MENU_SYSTEM_ITEMS: [MainMenuEntry; 8] = [
    MainMenuEntry {
        label: "Help",
        hotkey: "F1 or ?",
        key: MainMenuKey {
            code: KeyCode::F(1),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Close menu",
        hotkey: "F2",
        key: MainMenuKey {
            code: KeyCode::F(2),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Statistics",
        hotkey: "F3",
        key: MainMenuKey {
            code: KeyCode::F(3),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Tasks",
        hotkey: "Ctrl+4 or Cmd+4",
        key: MainMenuKey {
            code: KeyCode::Char('4'),
            modifiers: KeyModifiers::CONTROL,
        },
    },
    MainMenuEntry {
        label: "New Task",
        hotkey: "Ctrl+T or Cmd+T",
        key: MainMenuKey {
            code: KeyCode::Char('t'),
            modifiers: KeyModifiers::CONTROL,
        },
    },
    MainMenuEntry {
        label: "Rescan sessions",
        hotkey: "Ctrl+R",
        key: MainMenuKey {
            code: KeyCode::Char('r'),
            modifiers: KeyModifiers::CONTROL,
        },
    },
    MainMenuEntry {
        label: "Processes",
        hotkey: "P",
        key: MainMenuKey {
            code: KeyCode::Char('P'),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Quit",
        hotkey: "Ctrl+Q or Ctrl+C",
        key: MainMenuKey {
            code: KeyCode::Char('q'),
            modifiers: KeyModifiers::CONTROL,
        },
    },
];

pub const MAIN_MENU_WINDOW_ITEMS: [MainMenuEntry; 13] = [
    MainMenuEntry {
        label: "Projects",
        hotkey: "Ctrl+1 or Cmd+1",
        key: MainMenuKey {
            code: KeyCode::Char('1'),
            modifiers: KeyModifiers::CONTROL,
        },
    },
    MainMenuEntry {
        label: "Sessions",
        hotkey: "Ctrl+2 or Cmd+2",
        key: MainMenuKey {
            code: KeyCode::Char('2'),
            modifiers: KeyModifiers::CONTROL,
        },
    },
    MainMenuEntry {
        label: "Tasks",
        hotkey: "Ctrl+4 or Cmd+4",
        key: MainMenuKey {
            code: KeyCode::Char('4'),
            modifiers: KeyModifiers::CONTROL,
        },
    },
    MainMenuEntry {
        label: "Session Detail",
        hotkey: "Enter",
        key: MainMenuKey {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Task Detail",
        hotkey: "Ctrl+D or Cmd+D",
        key: MainMenuKey {
            code: KeyCode::Char('d'),
            modifiers: KeyModifiers::CONTROL,
        },
    },
    MainMenuEntry {
        label: "New Session",
        hotkey: "Ctrl+N or Cmd+N",
        key: MainMenuKey {
            code: KeyCode::Char('n'),
            modifiers: KeyModifiers::CONTROL,
        },
    },
    MainMenuEntry {
        label: "New Task",
        hotkey: "Ctrl+T or Cmd+T",
        key: MainMenuKey {
            code: KeyCode::Char('t'),
            modifiers: KeyModifiers::CONTROL,
        },
    },
    MainMenuEntry {
        label: "Processes",
        hotkey: "Ctrl+3 or Cmd+3",
        key: MainMenuKey {
            code: KeyCode::Char('3'),
            modifiers: KeyModifiers::CONTROL,
        },
    },
    MainMenuEntry {
        label: "Output: stdout",
        hotkey: "s",
        key: MainMenuKey {
            code: KeyCode::Char('s'),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Output: stderr",
        hotkey: "e",
        key: MainMenuKey {
            code: KeyCode::Char('e'),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Output: log",
        hotkey: "l",
        key: MainMenuKey {
            code: KeyCode::Char('l'),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Statistics",
        hotkey: "F3",
        key: MainMenuKey {
            code: KeyCode::F(3),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Help",
        hotkey: "F1 or ?",
        key: MainMenuKey {
            code: KeyCode::F(1),
            modifiers: KeyModifiers::NONE,
        },
    },
];

pub const MAIN_MENU_ENGINE_ITEMS: [MainMenuEntry; 4] = [
    MainMenuEntry {
        label: "All",
        hotkey: "",
        key: MainMenuKey {
            code: KeyCode::F(2),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Codex",
        hotkey: "",
        key: MainMenuKey {
            code: KeyCode::F(2),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Claude",
        hotkey: "",
        key: MainMenuKey {
            code: KeyCode::F(2),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Gemini",
        hotkey: "",
        key: MainMenuKey {
            code: KeyCode::F(2),
            modifiers: KeyModifiers::NONE,
        },
    },
];

pub const MAIN_MENU_PROJECTS_ITEMS: [MainMenuEntry; 4] = [
    MainMenuEntry {
        label: "Open",
        hotkey: "Enter",
        key: MainMenuKey {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Result (newest Out)",
        hotkey: "Space",
        key: MainMenuKey {
            code: KeyCode::Char(' '),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Delete project logs",
        hotkey: "Del",
        key: MainMenuKey {
            code: KeyCode::Delete,
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Clear filter",
        hotkey: "Esc",
        key: MainMenuKey {
            code: KeyCode::Esc,
            modifiers: KeyModifiers::NONE,
        },
    },
];

pub const MAIN_MENU_SESSIONS_ITEMS: [MainMenuEntry; 6] = [
    MainMenuEntry {
        label: "Open",
        hotkey: "Enter",
        key: MainMenuKey {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Result (last Out)",
        hotkey: "Space",
        key: MainMenuKey {
            code: KeyCode::Char(' '),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Stats",
        hotkey: "F3",
        key: MainMenuKey {
            code: KeyCode::F(3),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "New Session",
        hotkey: "Ctrl+N or Cmd+N",
        key: MainMenuKey {
            code: KeyCode::Char('n'),
            modifiers: KeyModifiers::CONTROL,
        },
    },
    MainMenuEntry {
        label: "Delete session log",
        hotkey: "Del",
        key: MainMenuKey {
            code: KeyCode::Delete,
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Back / Clear filter",
        hotkey: "Esc",
        key: MainMenuKey {
            code: KeyCode::Esc,
            modifiers: KeyModifiers::NONE,
        },
    },
];

pub const MAIN_MENU_NEW_SESSION_ITEMS: [MainMenuEntry; 4] = [
    MainMenuEntry {
        label: "Send",
        hotkey: "Ctrl+Enter or Cmd+Enter",
        key: MainMenuKey {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::CONTROL,
        },
    },
    MainMenuEntry {
        label: "Switch engine",
        hotkey: "Shift+Tab",
        key: MainMenuKey {
            code: KeyCode::BackTab,
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Switch I/O mode",
        hotkey: "F4",
        key: MainMenuKey {
            code: KeyCode::F(4),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Cancel",
        hotkey: "Esc",
        key: MainMenuKey {
            code: KeyCode::Esc,
            modifiers: KeyModifiers::NONE,
        },
    },
];

pub const MAIN_MENU_SESSION_ITEMS: [MainMenuEntry; 7] = [
    MainMenuEntry {
        label: "Jump Tool -> ToolOut",
        hotkey: "Enter",
        key: MainMenuKey {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Fork/Resume from here",
        hotkey: "f",
        key: MainMenuKey {
            code: KeyCode::Char('f'),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Switch pane focus",
        hotkey: "Tab",
        key: MainMenuKey {
            code: KeyCode::Tab,
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Result (last Out)",
        hotkey: "o",
        key: MainMenuKey {
            code: KeyCode::Char('o'),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Stats",
        hotkey: "F3",
        key: MainMenuKey {
            code: KeyCode::F(3),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Visible Context",
        hotkey: "c",
        key: MainMenuKey {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Back",
        hotkey: "Esc or Backspace",
        key: MainMenuKey {
            code: KeyCode::Esc,
            modifiers: KeyModifiers::NONE,
        },
    },
];

pub const MAIN_MENU_TASKS_ITEMS: [MainMenuEntry; 6] = [
    MainMenuEntry {
        label: "Open",
        hotkey: "Enter",
        key: MainMenuKey {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Spawn",
        hotkey: "Ctrl+Enter or Cmd+Enter",
        key: MainMenuKey {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::CONTROL,
        },
    },
    MainMenuEntry {
        label: "New Task",
        hotkey: "n",
        key: MainMenuKey {
            code: KeyCode::Char('n'),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Switch engine",
        hotkey: "Shift+Tab",
        key: MainMenuKey {
            code: KeyCode::BackTab,
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Delete task",
        hotkey: "Del",
        key: MainMenuKey {
            code: KeyCode::Delete,
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Back / Clear filter",
        hotkey: "Esc or Backspace",
        key: MainMenuKey {
            code: KeyCode::Esc,
            modifiers: KeyModifiers::NONE,
        },
    },
];

pub const MAIN_MENU_TASK_CREATE_ITEMS: [MainMenuEntry; 5] = [
    MainMenuEntry {
        label: "Save",
        hotkey: "Ctrl+S or Cmd+S",
        key: MainMenuKey {
            code: KeyCode::Char('s'),
            modifiers: KeyModifiers::CONTROL,
        },
    },
    MainMenuEntry {
        label: "Insert image",
        hotkey: "Ctrl+I or Cmd+I",
        key: MainMenuKey {
            code: KeyCode::Char('i'),
            modifiers: KeyModifiers::CONTROL,
        },
    },
    MainMenuEntry {
        label: "Paste image",
        hotkey: "Ctrl+V",
        key: MainMenuKey {
            code: KeyCode::Char('v'),
            modifiers: KeyModifiers::CONTROL,
        },
    },
    MainMenuEntry {
        label: "Edit project path",
        hotkey: "Ctrl+P or Cmd+P",
        key: MainMenuKey {
            code: KeyCode::Char('p'),
            modifiers: KeyModifiers::CONTROL,
        },
    },
    MainMenuEntry {
        label: "Cancel",
        hotkey: "Esc",
        key: MainMenuKey {
            code: KeyCode::Esc,
            modifiers: KeyModifiers::NONE,
        },
    },
];

pub const MAIN_MENU_TASK_DETAIL_ITEMS: [MainMenuEntry; 4] = [
    MainMenuEntry {
        label: "Spawn",
        hotkey: "Ctrl+Enter or Cmd+Enter",
        key: MainMenuKey {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::CONTROL,
        },
    },
    MainMenuEntry {
        label: "Switch engine",
        hotkey: "Shift+Tab",
        key: MainMenuKey {
            code: KeyCode::BackTab,
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Delete task",
        hotkey: "Del",
        key: MainMenuKey {
            code: KeyCode::Delete,
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Back",
        hotkey: "Esc or Backspace",
        key: MainMenuKey {
            code: KeyCode::Esc,
            modifiers: KeyModifiers::NONE,
        },
    },
];

pub const MAIN_MENU_PROCESSES_ITEMS: [MainMenuEntry; 7] = [
    MainMenuEntry {
        label: "Attach (TTY)",
        hotkey: "a",
        key: MainMenuKey {
            code: KeyCode::Char('a'),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Open stdout",
        hotkey: "s",
        key: MainMenuKey {
            code: KeyCode::Char('s'),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Open stderr",
        hotkey: "e",
        key: MainMenuKey {
            code: KeyCode::Char('e'),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Open log",
        hotkey: "l",
        key: MainMenuKey {
            code: KeyCode::Char('l'),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Kill process",
        hotkey: "k",
        key: MainMenuKey {
            code: KeyCode::Char('k'),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Open session",
        hotkey: "Enter",
        key: MainMenuKey {
            code: KeyCode::Enter,
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Back",
        hotkey: "Esc or Backspace",
        key: MainMenuKey {
            code: KeyCode::Esc,
            modifiers: KeyModifiers::NONE,
        },
    },
];

pub const MAIN_MENU_PROCESS_OUTPUT_ITEMS: [MainMenuEntry; 5] = [
    MainMenuEntry {
        label: "stdout",
        hotkey: "s",
        key: MainMenuKey {
            code: KeyCode::Char('s'),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "stderr",
        hotkey: "e",
        key: MainMenuKey {
            code: KeyCode::Char('e'),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "log",
        hotkey: "l",
        key: MainMenuKey {
            code: KeyCode::Char('l'),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Kill process",
        hotkey: "k",
        key: MainMenuKey {
            code: KeyCode::Char('k'),
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Back",
        hotkey: "Esc or Backspace",
        key: MainMenuKey {
            code: KeyCode::Esc,
            modifiers: KeyModifiers::NONE,
        },
    },
];

pub const MAIN_MENU_ERROR_ITEMS: [MainMenuEntry; 2] = [
    MainMenuEntry {
        label: "Back",
        hotkey: "Esc or Backspace",
        key: MainMenuKey {
            code: KeyCode::Esc,
            modifiers: KeyModifiers::NONE,
        },
    },
    MainMenuEntry {
        label: "Rescan sessions",
        hotkey: "Ctrl+R",
        key: MainMenuKey {
            code: KeyCode::Char('r'),
            modifiers: KeyModifiers::CONTROL,
        },
    },
];

pub const MAIN_MENUS_PROJECTS: [MainMenu; 4] = [
    MainMenu::System,
    MainMenu::Window,
    MainMenu::Engine,
    MainMenu::Projects,
];
pub const MAIN_MENUS_SESSIONS: [MainMenu; 4] = [
    MainMenu::System,
    MainMenu::Window,
    MainMenu::Engine,
    MainMenu::Sessions,
];
pub const MAIN_MENUS_NEW_SESSION: [MainMenu; 3] =
    [MainMenu::System, MainMenu::Window, MainMenu::NewSession];
pub const MAIN_MENUS_SESSION_DETAIL: [MainMenu; 3] =
    [MainMenu::System, MainMenu::Window, MainMenu::Session];
pub const MAIN_MENUS_TASKS: [MainMenu; 3] = [MainMenu::System, MainMenu::Window, MainMenu::Tasks];
pub const MAIN_MENUS_TASK_CREATE: [MainMenu; 3] =
    [MainMenu::System, MainMenu::Window, MainMenu::TaskCreate];
pub const MAIN_MENUS_TASK_DETAIL: [MainMenu; 3] =
    [MainMenu::System, MainMenu::Window, MainMenu::TaskDetail];
pub const MAIN_MENUS_PROCESSES: [MainMenu; 3] =
    [MainMenu::System, MainMenu::Window, MainMenu::Processes];
pub const MAIN_MENUS_PROCESS_OUTPUT: [MainMenu; 3] =
    [MainMenu::System, MainMenu::Window, MainMenu::ProcessOutput];
pub const MAIN_MENUS_ERROR: [MainMenu; 3] = [MainMenu::System, MainMenu::Window, MainMenu::Error];

pub fn main_menus_for_view(view: &View) -> &'static [MainMenu] {
    match view {
        View::Projects(_) => &MAIN_MENUS_PROJECTS,
        View::Sessions(_) => &MAIN_MENUS_SESSIONS,
        View::NewSession(_) => &MAIN_MENUS_NEW_SESSION,
        View::SessionDetail(_) => &MAIN_MENUS_SESSION_DETAIL,
        View::Tasks(_) => &MAIN_MENUS_TASKS,
        View::TaskCreate(_) => &MAIN_MENUS_TASK_CREATE,
        View::TaskDetail(_) => &MAIN_MENUS_TASK_DETAIL,
        View::Processes(_) => &MAIN_MENUS_PROCESSES,
        View::ProcessOutput(_) => &MAIN_MENUS_PROCESS_OUTPUT,
        View::Error => &MAIN_MENUS_ERROR,
    }
}

pub fn main_menu_items(menu: MainMenu) -> &'static [MainMenuEntry] {
    match menu {
        MainMenu::System => &MAIN_MENU_SYSTEM_ITEMS,
        MainMenu::Window => &MAIN_MENU_WINDOW_ITEMS,
        MainMenu::Engine => &MAIN_MENU_ENGINE_ITEMS,
        MainMenu::Projects => &MAIN_MENU_PROJECTS_ITEMS,
        MainMenu::Sessions => &MAIN_MENU_SESSIONS_ITEMS,
        MainMenu::NewSession => &MAIN_MENU_NEW_SESSION_ITEMS,
        MainMenu::Session => &MAIN_MENU_SESSION_ITEMS,
        MainMenu::Tasks => &MAIN_MENU_TASKS_ITEMS,
        MainMenu::TaskCreate => &MAIN_MENU_TASK_CREATE_ITEMS,
        MainMenu::TaskDetail => &MAIN_MENU_TASK_DETAIL_ITEMS,
        MainMenu::Processes => &MAIN_MENU_PROCESSES_ITEMS,
        MainMenu::ProcessOutput => &MAIN_MENU_PROCESS_OUTPUT_ITEMS,
        MainMenu::Error => &MAIN_MENU_ERROR_ITEMS,
    }
}

#[derive(Clone, Debug)]
pub struct SystemMenuOverlay {
    pub menu_index: usize,
    pub item_index: usize,
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
pub struct DeleteProjectsConfirmDialog {
    pub project_paths: Vec<PathBuf>,
    pub project_count: usize,
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

#[derive(Clone, Debug)]
pub struct DeleteSessionsConfirmDialog {
    pub project_name: String,
    pub project_path: PathBuf,
    pub log_paths: Vec<PathBuf>,
    pub session_count: usize,
    pub total_size_bytes: u64,
    pub selection: DeleteConfirmSelection,
}

#[derive(Clone, Debug)]
pub struct DeleteTaskConfirmDialog {
    pub task_id: TaskId,
    pub task_title: String,
    pub project_path: PathBuf,
    pub selection: DeleteConfirmSelection,
    pub return_to_tasks: TasksView,
}

#[derive(Clone, Debug)]
pub struct DeleteTasksConfirmDialog {
    pub task_ids: Vec<TaskId>,
    pub task_count: usize,
    pub selection: DeleteConfirmSelection,
    pub return_to_tasks: TasksView,
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
pub enum ProcessIoMode {
    Pipes {
        stdout_path: PathBuf,
        stderr_path: PathBuf,
        log_path: PathBuf,
    },
    Tty {
        transcript_path: PathBuf,
        log_path: PathBuf,
    },
}

impl ProcessIoMode {
    pub fn is_tty(&self) -> bool {
        matches!(self, Self::Tty { .. })
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Pipes { .. } => "pipes",
            Self::Tty { .. } => "tty",
        }
    }

    pub fn stdout_path(&self) -> Option<&PathBuf> {
        match self {
            Self::Pipes { stdout_path, .. } => Some(stdout_path),
            Self::Tty {
                transcript_path, ..
            } => Some(transcript_path),
        }
    }

    pub fn stderr_path(&self) -> Option<&PathBuf> {
        match self {
            Self::Pipes { stderr_path, .. } => Some(stderr_path),
            Self::Tty { .. } => None,
        }
    }

    pub fn log_path(&self) -> &PathBuf {
        match self {
            Self::Pipes { log_path, .. } => log_path,
            Self::Tty { log_path, .. } => log_path,
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
    pub io_mode: ProcessIoMode,
    pub session_id: Option<String>,
    pub session_log_path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub enum View {
    Projects(ProjectsView),
    Sessions(SessionsView),
    NewSession(NewSessionView),
    SessionDetail(SessionDetailView),
    Tasks(TasksView),
    TaskCreate(TaskCreateView),
    TaskDetail(TaskDetailView),
    Processes(ProcessesView),
    ProcessOutput(ProcessOutputView),
    Error,
}

#[derive(Clone, Debug)]
pub struct ProjectsView {
    pub query: String,
    pub filtered_indices: Vec<usize>,
    pub selected: usize,
    pub selection_anchor: Option<PathBuf>,
    pub selected_project_paths: BTreeSet<PathBuf>,
}

impl ProjectsView {
    pub fn new(projects: &[ProjectSummary]) -> Self {
        let filtered_indices = (0..projects.len()).collect();
        Self {
            query: String::new(),
            filtered_indices,
            selected: 0,
            selection_anchor: None,
            selected_project_paths: BTreeSet::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SessionsView {
    pub project_path: PathBuf,
    pub query: String,
    pub filtered_indices: Vec<usize>,
    pub session_selected: usize,
    pub selection_anchor: Option<PathBuf>,
    pub selected_log_paths: BTreeSet<PathBuf>,
}

impl SessionsView {
    pub fn new(project_path: PathBuf, session_count: usize) -> Self {
        Self {
            project_path,
            query: String::new(),
            filtered_indices: (0..session_count).collect(),
            session_selected: 0,
            selection_anchor: None,
            selected_log_paths: BTreeSet::new(),
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
    pub io_mode: SpawnIoMode,
    pub fork: Option<ForkContext>,
}

impl NewSessionView {
    pub fn new(from_sessions: SessionsView) -> Self {
        Self {
            from_sessions,
            editor: TextEditor::new(),
            engine: AgentEngine::Codex,
            io_mode: SpawnIoMode::Pipes,
            fork: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionDetailFocus {
    Timeline,
    Details,
}

impl SessionDetailFocus {
    fn toggle(self) -> Self {
        match self {
            Self::Timeline => Self::Details,
            Self::Details => Self::Timeline,
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
    pub focus: SessionDetailFocus,
    pub details_scroll: u16,
    pub context_overlay_open: bool,
    pub last_output: Option<String>,
    pub output_overlay_open: bool,
    pub output_overlay_scroll: u16,
}

#[derive(Clone, Debug)]
pub struct TaskSummaryRow {
    pub id: TaskId,
    pub title: String,
    pub project_path: PathBuf,
    pub updated_at: SystemTime,
    pub image_count: u32,
}

#[derive(Clone, Debug)]
pub struct TasksView {
    pub return_to: Box<View>,
    pub tasks: Vec<TaskSummaryRow>,
    pub query: String,
    pub filtered_indices: Vec<usize>,
    pub selected: usize,
    pub selection_anchor: Option<TaskId>,
    pub selected_task_ids: BTreeSet<TaskId>,
    pub engine: AgentEngine,
}

impl TasksView {
    pub fn new(return_to: Box<View>, tasks: Vec<TaskSummaryRow>) -> Self {
        let filtered_indices = (0..tasks.len()).collect();
        Self {
            return_to,
            tasks,
            query: String::new(),
            filtered_indices,
            selected: 0,
            selection_anchor: None,
            selected_task_ids: BTreeSet::new(),
            engine: AgentEngine::Codex,
        }
    }

    pub fn with_reloaded_tasks(mut self, tasks: Vec<TaskSummaryRow>) -> Self {
        let selected_id = selected_task_id(&self);
        self.tasks = tasks;
        apply_task_filter(&mut self);
        prune_task_selection(&mut self);

        if let Some(task_id) = selected_id {
            if let Some(pos) = self.filtered_indices.iter().position(|index| {
                self.tasks
                    .get(*index)
                    .is_some_and(|task| task.id == task_id)
            }) {
                self.selected = pos;
            }
        }

        self
    }
}

#[derive(Clone, Debug)]
pub enum TaskCreateOverlay {
    ImagePath(LineEditor),
    ProjectPath(LineEditor),
}

#[derive(Clone, Debug)]
pub struct TaskCreateView {
    pub from_tasks: TasksView,
    pub editor: TextEditor,
    pub project_path: PathBuf,
    pub image_paths: Vec<PathBuf>,
    pub overlay: Option<TaskCreateOverlay>,
}

impl TaskCreateView {
    pub fn new(from_tasks: TasksView, project_path: PathBuf) -> Self {
        Self {
            from_tasks,
            editor: TextEditor::new(),
            project_path,
            image_paths: Vec::new(),
            overlay: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TaskDetailView {
    pub from_tasks: TasksView,
    pub task: Task,
    pub images: Vec<TaskImage>,
    pub engine: AgentEngine,
    pub scroll: u16,
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
    Mouse(MouseEvent),
}

#[derive(Clone, Debug)]
pub enum AppCommand {
    None,
    Quit,
    Rescan,
    OpenTasks {
        return_to: Box<View>,
    },
    OpenTaskCreate {
        return_to: Box<View>,
        project_path: Option<PathBuf>,
    },
    OpenTaskDetail {
        from_tasks: TasksView,
        task_id: TaskId,
    },
    CreateTask {
        from_tasks: TasksView,
        project_path: PathBuf,
        body: String,
        image_paths: Vec<PathBuf>,
    },
    DeleteTask {
        from_tasks: TasksView,
        task_id: TaskId,
    },
    DeleteTasksBatch {
        from_tasks: TasksView,
        task_ids: Vec<TaskId>,
    },
    SpawnTask {
        engine: AgentEngine,
        task_id: TaskId,
    },
    TaskCreateInsertImage {
        path: PathBuf,
    },
    TaskCreatePasteImageFromClipboard,
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
    DeleteProjectLogsBatch {
        project_paths: Vec<PathBuf>,
    },
    DeleteSessionLog {
        log_path: PathBuf,
    },
    DeleteSessionLogsBatch {
        log_paths: Vec<PathBuf>,
    },
    SpawnAgentSession {
        engine: AgentEngine,
        project_path: PathBuf,
        prompt: String,
        io_mode: SpawnIoMode,
    },
    ForkResumeCodexFromTimeline {
        fork: ForkContext,
        prompt: String,
    },
    KillProcess {
        process_id: String,
    },
    OpenProcessOutput {
        process_id: String,
        kind: ProcessOutputKind,
    },
    AttachProcessTty {
        process_id: String,
    },
}

pub fn update(model: AppModel, event: AppEvent) -> (AppModel, AppCommand) {
    match event {
        AppEvent::Key(key) => update_on_key(model, key),
        AppEvent::Paste(text) => update_on_paste(model, text),
        AppEvent::Mouse(mouse) => mouse::update_on_mouse(model, mouse),
    }
}

fn update_on_key(model: AppModel, key: KeyEvent) -> (AppModel, AppCommand) {
    let mut model = model;
    model.notice = None;

    let command_modifier = key.modifiers.contains(KeyModifiers::CONTROL)
        || key.modifiers.contains(KeyModifiers::SUPER)
        || key.modifiers.contains(KeyModifiers::META);

    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return (model, AppCommand::Quit);
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
        return (model, AppCommand::Quit);
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('r') {
        return (model, AppCommand::Rescan);
    }

    if command_modifier && matches!(key.code, KeyCode::Char('1')) {
        if model.help_open
            || model.system_menu.is_some()
            || model.delete_confirm.is_some()
            || model.delete_projects_confirm.is_some()
            || model.delete_session_confirm.is_some()
            || model.delete_sessions_confirm.is_some()
            || model.delete_task_confirm.is_some()
            || model.delete_tasks_confirm.is_some()
            || model.session_result_preview.is_some()
            || model.session_stats_overlay.is_some()
            || model.project_stats_overlay.is_some()
        {
            return (model, AppCommand::None);
        }

        let mut view = ProjectsView::new(&model.data.projects);
        apply_project_filter(&model.data.projects, &mut view, model.engine_filter);
        model.view = View::Projects(view);
        model.help_open = false;
        model.system_menu = None;
        return (model, AppCommand::None);
    }

    if command_modifier && matches!(key.code, KeyCode::Char('2')) {
        if model.help_open
            || model.system_menu.is_some()
            || model.delete_confirm.is_some()
            || model.delete_projects_confirm.is_some()
            || model.delete_session_confirm.is_some()
            || model.delete_sessions_confirm.is_some()
            || model.delete_task_confirm.is_some()
            || model.delete_tasks_confirm.is_some()
            || model.session_result_preview.is_some()
            || model.session_stats_overlay.is_some()
            || model.project_stats_overlay.is_some()
        {
            return (model, AppCommand::None);
        }

        let Some(mut sessions_view) = infer_sessions_view_for_window_menu(&model) else {
            model.notice = Some("No project selected.".to_string());
            return (model, AppCommand::None);
        };

        if let Some(project) = sessions_view.current_project(&model.data.projects) {
            apply_session_filter(&project.sessions, &mut sessions_view, model.engine_filter);
        }

        model.view = View::Sessions(sessions_view);
        model.help_open = false;
        model.system_menu = None;
        return (model, AppCommand::None);
    }

    if command_modifier && matches!(key.code, KeyCode::Char('3')) {
        if model.help_open
            || model.system_menu.is_some()
            || model.delete_confirm.is_some()
            || model.delete_projects_confirm.is_some()
            || model.delete_session_confirm.is_some()
            || model.delete_sessions_confirm.is_some()
            || model.delete_task_confirm.is_some()
            || model.delete_tasks_confirm.is_some()
            || model.session_result_preview.is_some()
            || model.session_stats_overlay.is_some()
            || model.project_stats_overlay.is_some()
        {
            return (model, AppCommand::None);
        }

        if !matches!(&model.view, View::Processes(_)) {
            open_processes_view(&mut model);
        }
        model.help_open = false;
        model.system_menu = None;
        return (model, AppCommand::None);
    }

    if command_modifier && matches!(key.code, KeyCode::Char('4')) {
        if model.help_open
            || model.system_menu.is_some()
            || model.delete_confirm.is_some()
            || model.delete_projects_confirm.is_some()
            || model.delete_session_confirm.is_some()
            || model.delete_sessions_confirm.is_some()
            || model.delete_task_confirm.is_some()
            || model.delete_tasks_confirm.is_some()
            || model.session_result_preview.is_some()
            || model.session_stats_overlay.is_some()
            || model.project_stats_overlay.is_some()
        {
            return (model, AppCommand::None);
        }

        match model.view.clone() {
            View::Tasks(_) => {}
            View::TaskCreate(task_create) => {
                model.view = View::Tasks(task_create.from_tasks);
            }
            View::TaskDetail(task_detail) => {
                model.view = View::Tasks(task_detail.from_tasks);
            }
            _ => {
                let return_to = Box::new(model.view.clone());
                model.help_open = false;
                model.system_menu = None;
                return (model, AppCommand::OpenTasks { return_to });
            }
        }

        model.help_open = false;
        model.system_menu = None;
        return (model, AppCommand::None);
    }

    if command_modifier && matches!(key.code, KeyCode::Char('d') | KeyCode::Char('D')) {
        if model.help_open
            || model.system_menu.is_some()
            || model.delete_confirm.is_some()
            || model.delete_projects_confirm.is_some()
            || model.delete_session_confirm.is_some()
            || model.delete_sessions_confirm.is_some()
            || model.delete_task_confirm.is_some()
            || model.delete_tasks_confirm.is_some()
            || model.session_result_preview.is_some()
            || model.session_stats_overlay.is_some()
            || model.project_stats_overlay.is_some()
        {
            return (model, AppCommand::None);
        }

        if matches!(&model.view, View::TaskDetail(_)) {
            return (model, AppCommand::None);
        }

        let Some((from_tasks, task_id)) = infer_task_detail_target(&model) else {
            model.notice = Some("No task selected.".to_string());
            return (model, AppCommand::None);
        };

        model.view = View::Tasks(from_tasks.clone());
        model.help_open = false;
        model.system_menu = None;
        return (
            model,
            AppCommand::OpenTaskDetail {
                from_tasks,
                task_id,
            },
        );
    }

    if command_modifier && matches!(key.code, KeyCode::Char('n') | KeyCode::Char('N')) {
        if model.help_open
            || model.system_menu.is_some()
            || model.delete_confirm.is_some()
            || model.delete_projects_confirm.is_some()
            || model.delete_session_confirm.is_some()
            || model.delete_sessions_confirm.is_some()
            || model.delete_task_confirm.is_some()
            || model.delete_tasks_confirm.is_some()
            || model.session_result_preview.is_some()
            || model.session_stats_overlay.is_some()
            || model.project_stats_overlay.is_some()
        {
            return (model, AppCommand::None);
        }

        match model.view.clone() {
            View::Projects(projects_view) => {
                let Some(project_index) = projects_view
                    .filtered_indices
                    .get(projects_view.selected)
                    .copied()
                else {
                    model.notice = Some("No project selected.".to_string());
                    return (model, AppCommand::None);
                };
                let Some(project) = model.data.projects.get(project_index) else {
                    model.notice = Some("No project selected.".to_string());
                    return (model, AppCommand::None);
                };

                let mut sessions_view =
                    SessionsView::new(project.project_path.clone(), project.sessions.len());
                apply_session_filter(&project.sessions, &mut sessions_view, model.engine_filter);
                model.view = View::NewSession(NewSessionView::new(sessions_view));
            }
            View::Sessions(sessions_view) => {
                model.view = View::NewSession(NewSessionView::new(sessions_view));
            }
            View::SessionDetail(detail_view) => {
                model.view = View::NewSession(NewSessionView::new(detail_view.from_sessions));
            }
            View::Tasks(tasks_view) => {
                let Some(mut sessions_view) =
                    infer_sessions_view_for_window_menu_view(tasks_view.return_to.as_ref(), &model)
                else {
                    model.notice = Some("No project selected.".to_string());
                    return (model, AppCommand::None);
                };
                if let Some(project) = sessions_view.current_project(&model.data.projects) {
                    apply_session_filter(
                        &project.sessions,
                        &mut sessions_view,
                        model.engine_filter,
                    );
                }
                model.view = View::NewSession(NewSessionView::new(sessions_view));
            }
            View::TaskCreate(task_create) => {
                let Some(mut sessions_view) = infer_sessions_view_for_window_menu_view(
                    task_create.from_tasks.return_to.as_ref(),
                    &model,
                ) else {
                    model.notice = Some("No project selected.".to_string());
                    return (model, AppCommand::None);
                };
                if let Some(project) = sessions_view.current_project(&model.data.projects) {
                    apply_session_filter(
                        &project.sessions,
                        &mut sessions_view,
                        model.engine_filter,
                    );
                }
                model.view = View::NewSession(NewSessionView::new(sessions_view));
            }
            View::TaskDetail(task_detail) => {
                let Some(mut sessions_view) = infer_sessions_view_for_window_menu_view(
                    task_detail.from_tasks.return_to.as_ref(),
                    &model,
                ) else {
                    model.notice = Some("No project selected.".to_string());
                    return (model, AppCommand::None);
                };
                if let Some(project) = sessions_view.current_project(&model.data.projects) {
                    apply_session_filter(
                        &project.sessions,
                        &mut sessions_view,
                        model.engine_filter,
                    );
                }
                model.view = View::NewSession(NewSessionView::new(sessions_view));
            }
            View::NewSession(_) => {}
            View::Processes(_) | View::ProcessOutput(_) | View::Error => {
                model.notice = Some("Open a project to start a new session.".to_string());
                return (model, AppCommand::None);
            }
        }

        model.help_open = false;
        model.system_menu = None;
        return (model, AppCommand::None);
    }

    if command_modifier && matches!(key.code, KeyCode::Char('t') | KeyCode::Char('T')) {
        if model.help_open
            || model.system_menu.is_some()
            || model.delete_confirm.is_some()
            || model.delete_projects_confirm.is_some()
            || model.delete_session_confirm.is_some()
            || model.delete_sessions_confirm.is_some()
            || model.delete_task_confirm.is_some()
            || model.delete_tasks_confirm.is_some()
            || model.session_result_preview.is_some()
            || model.session_stats_overlay.is_some()
            || model.project_stats_overlay.is_some()
        {
            return (model, AppCommand::None);
        }

        match model.view.clone() {
            View::Tasks(tasks_view) => {
                let project_path = default_task_create_project_path(&model, &tasks_view);
                model.view = View::TaskCreate(TaskCreateView::new(tasks_view, project_path));
            }
            View::TaskDetail(detail_view) => {
                let project_path = detail_view.task.project_path.clone();
                model.view =
                    View::TaskCreate(TaskCreateView::new(detail_view.from_tasks, project_path));
            }
            View::TaskCreate(_) => {}
            _ => {
                let project_path = infer_project_path_for_new_task(&model);
                let return_to = Box::new(model.view.clone());
                model.help_open = false;
                model.system_menu = None;
                return (
                    model,
                    AppCommand::OpenTaskCreate {
                        return_to,
                        project_path,
                    },
                );
            }
        }

        model.help_open = false;
        model.system_menu = None;
        return (model, AppCommand::None);
    }

    if matches!(key.code, KeyCode::F(2)) {
        if model.delete_confirm.is_some()
            || model.delete_projects_confirm.is_some()
            || model.delete_session_confirm.is_some()
            || model.delete_sessions_confirm.is_some()
            || model.delete_task_confirm.is_some()
            || model.delete_tasks_confirm.is_some()
            || model.session_result_preview.is_some()
            || model.session_stats_overlay.is_some()
            || model.project_stats_overlay.is_some()
        {
            return (model, AppCommand::None);
        }

        if model.system_menu.is_some() {
            model.system_menu = None;
        } else {
            model.system_menu = Some(SystemMenuOverlay {
                menu_index: 0,
                item_index: 0,
            });
            model.help_open = false;
        }
        return (model, AppCommand::None);
    }

    if let Some(menu) = model.system_menu.take() {
        return update_system_menu_overlay(model, menu, key);
    }

    if let Some(overlay) = model.project_stats_overlay.take() {
        return update_project_stats_overlay(model, overlay, key);
    }

    if let Some(overlay) = model.session_stats_overlay.take() {
        return update_session_stats_overlay(model, overlay, key);
    }

    if let Some(preview) = model.session_result_preview.take() {
        return update_session_result_preview_overlay(model, preview, key);
    }

    if let Some(confirm) = model.delete_sessions_confirm.take() {
        return update_delete_sessions_confirm(model, confirm, key);
    }

    if let Some(confirm) = model.delete_session_confirm.take() {
        return update_delete_session_confirm(model, confirm, key);
    }

    if let Some(confirm) = model.delete_tasks_confirm.take() {
        return update_delete_tasks_confirm(model, confirm, key);
    }

    if let Some(confirm) = model.delete_task_confirm.take() {
        return update_delete_task_confirm(model, confirm, key);
    }

    if let Some(confirm) = model.delete_projects_confirm.take() {
        return update_delete_projects_confirm(model, confirm, key);
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
        View::Tasks(tasks_view) => update_tasks(model, tasks_view, key),
        View::TaskCreate(task_create_view) => update_task_create(model, task_create_view, key),
        View::TaskDetail(task_detail_view) => update_task_detail(model, task_detail_view, key),
        View::Processes(processes_view) => update_processes(model, processes_view, key),
        View::ProcessOutput(output_view) => update_process_output(model, output_view, key),
        View::Error => update_error(model, key),
    }
}

fn infer_sessions_view_for_window_menu(model: &AppModel) -> Option<SessionsView> {
    infer_sessions_view_for_window_menu_view(&model.view, model)
}

fn infer_sessions_view_for_window_menu_view(view: &View, model: &AppModel) -> Option<SessionsView> {
    match view {
        View::Projects(projects_view) => {
            let project_index = projects_view
                .filtered_indices
                .get(projects_view.selected)
                .copied()?;
            let project = model.data.projects.get(project_index)?;
            Some(SessionsView::new(
                project.project_path.clone(),
                project.sessions.len(),
            ))
        }
        View::Sessions(sessions_view) => Some(sessions_view.clone()),
        View::NewSession(new_session_view) => Some(new_session_view.from_sessions.clone()),
        View::SessionDetail(detail_view) => Some(detail_view.from_sessions.clone()),
        View::Tasks(tasks_view) => {
            infer_sessions_view_for_window_menu_view(tasks_view.return_to.as_ref(), model)
        }
        View::TaskCreate(task_create) => infer_sessions_view_for_window_menu_view(
            task_create.from_tasks.return_to.as_ref(),
            model,
        ),
        View::TaskDetail(task_detail) => infer_sessions_view_for_window_menu_view(
            task_detail.from_tasks.return_to.as_ref(),
            model,
        ),
        View::Processes(processes_view) => {
            infer_sessions_view_for_window_menu_view(processes_view.return_to.as_ref(), model)
        }
        View::ProcessOutput(output_view) => {
            infer_sessions_view_for_window_menu_view(output_view.return_to.as_ref(), model)
        }
        View::Error => None,
    }
}

fn infer_session_detail_target(model: &AppModel) -> Option<(SessionsView, SessionSummary)> {
    infer_session_detail_target_view(&model.view, model)
}

fn infer_session_detail_target_view(
    view: &View,
    model: &AppModel,
) -> Option<(SessionsView, SessionSummary)> {
    match view {
        View::Projects(projects_view) => {
            let project_index = projects_view
                .filtered_indices
                .get(projects_view.selected)
                .copied()?;
            let project = model.data.projects.get(project_index)?;
            let session = match model.engine_filter {
                EngineFilter::All => project.sessions.first().cloned()?,
                EngineFilter::Codex | EngineFilter::Claude | EngineFilter::Gemini => project
                    .sessions
                    .iter()
                    .find(|session| session_matches_engine_filter(session, model.engine_filter))
                    .cloned()?,
            };
            let mut from_sessions =
                SessionsView::new(project.project_path.clone(), project.sessions.len());
            apply_session_filter(&project.sessions, &mut from_sessions, model.engine_filter);
            if let Some(selected_index) = project
                .sessions
                .iter()
                .position(|candidate| candidate.log_path == session.log_path)
            {
                if let Some(pos) = from_sessions
                    .filtered_indices
                    .iter()
                    .position(|index| *index == selected_index)
                {
                    from_sessions.session_selected = pos;
                }
            }
            Some((from_sessions, session))
        }
        View::Sessions(sessions_view) => {
            let project = sessions_view.current_project(&model.data.projects)?;
            let selected_index = sessions_view
                .filtered_indices
                .get(sessions_view.session_selected)
                .copied()?;
            let session = project.sessions.get(selected_index).cloned()?;
            Some((sessions_view.clone(), session))
        }
        View::NewSession(new_session_view) => {
            let from_sessions = new_session_view.from_sessions.clone();
            let project = from_sessions.current_project(&model.data.projects)?;
            let selected_index = from_sessions
                .filtered_indices
                .get(from_sessions.session_selected)
                .copied()?;
            let session = project.sessions.get(selected_index).cloned()?;
            Some((from_sessions, session))
        }
        View::SessionDetail(detail_view) => Some((
            detail_view.from_sessions.clone(),
            detail_view.session.clone(),
        )),
        View::Tasks(tasks_view) => {
            infer_session_detail_target_view(tasks_view.return_to.as_ref(), model)
        }
        View::TaskCreate(task_create) => {
            infer_session_detail_target_view(task_create.from_tasks.return_to.as_ref(), model)
        }
        View::TaskDetail(task_detail) => {
            infer_session_detail_target_view(task_detail.from_tasks.return_to.as_ref(), model)
        }
        View::Processes(processes_view) => {
            infer_session_detail_target_view(processes_view.return_to.as_ref(), model)
        }
        View::ProcessOutput(output_view) => {
            infer_session_detail_target_view(output_view.return_to.as_ref(), model)
        }
        View::Error => None,
    }
}

fn infer_task_detail_target(model: &AppModel) -> Option<(TasksView, TaskId)> {
    infer_task_detail_target_view(&model.view)
}

fn infer_task_detail_target_view(view: &View) -> Option<(TasksView, TaskId)> {
    match view {
        View::Tasks(tasks_view) => {
            selected_task_id(tasks_view).map(|task_id| (tasks_view.clone(), task_id))
        }
        View::TaskCreate(task_create) => {
            let tasks_view = task_create.from_tasks.clone();
            selected_task_id(&tasks_view).map(|task_id| (tasks_view, task_id))
        }
        View::TaskDetail(task_detail) => {
            Some((task_detail.from_tasks.clone(), task_detail.task.id.clone()))
        }
        View::Processes(processes_view) => {
            infer_task_detail_target_view(processes_view.return_to.as_ref())
        }
        View::ProcessOutput(output_view) => {
            infer_task_detail_target_view(output_view.return_to.as_ref())
        }
        View::Projects(_)
        | View::Sessions(_)
        | View::NewSession(_)
        | View::SessionDetail(_)
        | View::Error => None,
    }
}

fn engine_filter_from_engine_menu_index(index: usize) -> Option<EngineFilter> {
    match index {
        0 => Some(EngineFilter::All),
        1 => Some(EngineFilter::Codex),
        2 => Some(EngineFilter::Claude),
        3 => Some(EngineFilter::Gemini),
        _ => None,
    }
}

fn apply_engine_filter(mut model: AppModel, filter: EngineFilter) -> AppModel {
    if model.engine_filter == filter {
        return model;
    }
    model.engine_filter = filter;

    match model.view.clone() {
        View::Projects(mut view) => {
            apply_project_filter(&model.data.projects, &mut view, model.engine_filter);
            clear_project_selection(&mut view);
            model.view = View::Projects(view);
        }
        View::Sessions(mut view) => {
            if let Some(project) = view.current_project(&model.data.projects) {
                apply_session_filter(&project.sessions, &mut view, model.engine_filter);
            } else {
                view.filtered_indices.clear();
                view.session_selected = 0;
            }
            clear_sessions_selection(&mut view);
            model.view = View::Sessions(view);
        }
        _ => {}
    }

    model
}

fn update_system_menu_overlay(
    mut model: AppModel,
    mut menu: SystemMenuOverlay,
    key: KeyEvent,
) -> (AppModel, AppCommand) {
    let menus = main_menus_for_view(&model.view);
    if menus.is_empty() {
        model.system_menu = None;
        return (model, AppCommand::None);
    }

    menu.menu_index = menu.menu_index.min(menus.len().saturating_sub(1));

    match key.code {
        KeyCode::Esc | KeyCode::Backspace => {
            model.system_menu = None;
            return (model, AppCommand::None);
        }
        KeyCode::Left => {
            menu.menu_index = if menu.menu_index == 0 {
                menus.len().saturating_sub(1)
            } else {
                menu.menu_index.saturating_sub(1)
            };
            menu.item_index = 0;
        }
        KeyCode::Right => {
            menu.menu_index = (menu.menu_index + 1) % menus.len();
            menu.item_index = 0;
        }
        KeyCode::Up => {
            menu.item_index = menu.item_index.saturating_sub(1);
        }
        KeyCode::Down => {
            let active = menus[menu.menu_index];
            let items = main_menu_items(active);
            menu.item_index = (menu.item_index + 1).min(items.len().saturating_sub(1));
        }
        KeyCode::Enter => {
            let active = menus[menu.menu_index];
            let items = main_menu_items(active);
            let Some(entry) = items.get(menu.item_index).copied() else {
                model.system_menu = None;
                return (model, AppCommand::None);
            };

            model.system_menu = None;

            if active == MainMenu::Engine {
                if let Some(filter) = engine_filter_from_engine_menu_index(menu.item_index) {
                    model = apply_engine_filter(model, filter);
                }
                return (model, AppCommand::None);
            }

            if active == MainMenu::Window {
                return apply_window_menu_entry(model, entry);
            }

            if entry.key.code == KeyCode::F(2) && entry.key.modifiers == KeyModifiers::NONE {
                return (model, AppCommand::None);
            }

            let key = KeyEvent::new(entry.key.code, entry.key.modifiers);
            return update_on_key(model, key);
        }
        _ => {}
    }

    model.system_menu = Some(menu);
    (model, AppCommand::None)
}

fn apply_window_menu_entry(model: AppModel, entry: MainMenuEntry) -> (AppModel, AppCommand) {
    match entry.label {
        "Session Detail" => {
            let mut model = model;

            if matches!(&model.view, View::SessionDetail(_)) {
                return (model, AppCommand::None);
            }

            let Some((from_sessions, session)) = infer_session_detail_target(&model) else {
                model.notice = Some("No session selected.".to_string());
                return (model, AppCommand::None);
            };

            model.view = View::Sessions(from_sessions.clone());
            model.help_open = false;
            model.system_menu = None;

            (
                model,
                AppCommand::OpenSessionDetail {
                    from_sessions,
                    session,
                },
            )
        }
        "Task Detail" => {
            let mut model = model;

            if matches!(&model.view, View::TaskDetail(_)) {
                return (model, AppCommand::None);
            }

            let Some((from_tasks, task_id)) = infer_task_detail_target(&model) else {
                model.notice = Some("No task selected.".to_string());
                return (model, AppCommand::None);
            };

            model.view = View::Tasks(from_tasks.clone());
            model.help_open = false;
            model.system_menu = None;

            (
                model,
                AppCommand::OpenTaskDetail {
                    from_tasks,
                    task_id,
                },
            )
        }
        "Output: stdout" => apply_window_menu_open_output(model, ProcessOutputKind::Stdout),
        "Output: stderr" => apply_window_menu_open_output(model, ProcessOutputKind::Stderr),
        "Output: log" => apply_window_menu_open_output(model, ProcessOutputKind::Log),
        _ => {
            let key = KeyEvent::new(entry.key.code, entry.key.modifiers);
            update_on_key(model, key)
        }
    }
}

fn apply_window_menu_open_output(
    model: AppModel,
    kind: ProcessOutputKind,
) -> (AppModel, AppCommand) {
    let mut model = model;

    let process_id = match &model.view {
        View::Processes(processes_view) => model
            .processes
            .get(processes_view.selected)
            .map(|process| process.id.clone()),
        View::ProcessOutput(output_view) => Some(output_view.process_id.clone()),
        _ => None,
    };

    let Some(process_id) = process_id else {
        model.notice = Some("Open Processes to select a process.".to_string());
        return (model, AppCommand::None);
    };

    (model, AppCommand::OpenProcessOutput { process_id, kind })
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
    if model.delete_confirm.is_some()
        || model.delete_session_confirm.is_some()
        || model.delete_task_confirm.is_some()
    {
        return (model, AppCommand::None);
    }
    if model.session_result_preview.is_some() {
        return (model, AppCommand::None);
    }
    if model.session_stats_overlay.is_some() {
        return (model, AppCommand::None);
    }
    if model.project_stats_overlay.is_some() {
        return (model, AppCommand::None);
    }

    let view = model.view.clone();
    if let View::NewSession(mut new_session_view) = view {
        new_session_view.editor.insert_str(&text);
        model.view = View::NewSession(new_session_view);
    } else if let View::TaskCreate(mut task_create_view) = view {
        match task_create_view.overlay.take() {
            Some(TaskCreateOverlay::ImagePath(mut editor)) => {
                editor.insert_str(&text);
                task_create_view.overlay = Some(TaskCreateOverlay::ImagePath(editor));
            }
            Some(TaskCreateOverlay::ProjectPath(mut editor)) => {
                editor.insert_str(&text);
                task_create_view.overlay = Some(TaskCreateOverlay::ProjectPath(editor));
            }
            None => {
                task_create_view.editor.insert_str(&text);
            }
        }
        model.view = View::TaskCreate(task_create_view);
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

fn update_project_stats_overlay(
    mut model: AppModel,
    mut overlay: ProjectStatsOverlay,
    key: KeyEvent,
) -> (AppModel, AppCommand) {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace => {
            model.project_stats_overlay = None;
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

    model.project_stats_overlay = Some(overlay);
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

fn update_delete_projects_confirm(
    mut model: AppModel,
    mut confirm: DeleteProjectsConfirmDialog,
    key: KeyEvent,
) -> (AppModel, AppCommand) {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace => {
            model.delete_projects_confirm = None;
            return (model, AppCommand::None);
        }
        KeyCode::Left | KeyCode::Right => {
            confirm.selection = confirm.selection.toggle();
        }
        KeyCode::Enter => {
            let command = if confirm.selection == DeleteConfirmSelection::Delete {
                AppCommand::DeleteProjectLogsBatch {
                    project_paths: confirm.project_paths.clone(),
                }
            } else {
                AppCommand::None
            };
            model.delete_projects_confirm = None;
            return (model, command);
        }
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            model.delete_projects_confirm = None;
            return (
                model,
                AppCommand::DeleteProjectLogsBatch {
                    project_paths: confirm.project_paths.clone(),
                },
            );
        }
        KeyCode::Char('n') | KeyCode::Char('N') => {
            model.delete_projects_confirm = None;
            return (model, AppCommand::None);
        }
        _ => {}
    }

    model.delete_projects_confirm = Some(confirm);
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

fn update_delete_sessions_confirm(
    mut model: AppModel,
    mut confirm: DeleteSessionsConfirmDialog,
    key: KeyEvent,
) -> (AppModel, AppCommand) {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace => {
            model.delete_sessions_confirm = None;
            return (model, AppCommand::None);
        }
        KeyCode::Left | KeyCode::Right => {
            confirm.selection = confirm.selection.toggle();
        }
        KeyCode::Enter => {
            let command = if confirm.selection == DeleteConfirmSelection::Delete {
                AppCommand::DeleteSessionLogsBatch {
                    log_paths: confirm.log_paths.clone(),
                }
            } else {
                AppCommand::None
            };
            model.delete_sessions_confirm = None;
            return (model, command);
        }
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            model.delete_sessions_confirm = None;
            return (
                model,
                AppCommand::DeleteSessionLogsBatch {
                    log_paths: confirm.log_paths.clone(),
                },
            );
        }
        KeyCode::Char('n') | KeyCode::Char('N') => {
            model.delete_sessions_confirm = None;
            return (model, AppCommand::None);
        }
        _ => {}
    }

    model.delete_sessions_confirm = Some(confirm);
    (model, AppCommand::None)
}

fn update_delete_task_confirm(
    mut model: AppModel,
    mut confirm: DeleteTaskConfirmDialog,
    key: KeyEvent,
) -> (AppModel, AppCommand) {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace => {
            model.delete_task_confirm = None;
            return (model, AppCommand::None);
        }
        KeyCode::Left | KeyCode::Right => {
            confirm.selection = confirm.selection.toggle();
        }
        KeyCode::Enter => {
            let command = if confirm.selection == DeleteConfirmSelection::Delete {
                AppCommand::DeleteTask {
                    from_tasks: confirm.return_to_tasks.clone(),
                    task_id: confirm.task_id.clone(),
                }
            } else {
                AppCommand::None
            };
            model.delete_task_confirm = None;
            return (model, command);
        }
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            model.delete_task_confirm = None;
            return (
                model,
                AppCommand::DeleteTask {
                    from_tasks: confirm.return_to_tasks.clone(),
                    task_id: confirm.task_id.clone(),
                },
            );
        }
        KeyCode::Char('n') | KeyCode::Char('N') => {
            model.delete_task_confirm = None;
            return (model, AppCommand::None);
        }
        _ => {}
    }

    model.delete_task_confirm = Some(confirm);
    (model, AppCommand::None)
}

fn update_delete_tasks_confirm(
    mut model: AppModel,
    mut confirm: DeleteTasksConfirmDialog,
    key: KeyEvent,
) -> (AppModel, AppCommand) {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace => {
            model.delete_tasks_confirm = None;
            return (model, AppCommand::None);
        }
        KeyCode::Left | KeyCode::Right => {
            confirm.selection = confirm.selection.toggle();
        }
        KeyCode::Enter => {
            let command = if confirm.selection == DeleteConfirmSelection::Delete {
                AppCommand::DeleteTasksBatch {
                    from_tasks: confirm.return_to_tasks.clone(),
                    task_ids: confirm.task_ids.clone(),
                }
            } else {
                AppCommand::None
            };
            model.delete_tasks_confirm = None;
            return (model, command);
        }
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            model.delete_tasks_confirm = None;
            return (
                model,
                AppCommand::DeleteTasksBatch {
                    from_tasks: confirm.return_to_tasks.clone(),
                    task_ids: confirm.task_ids.clone(),
                },
            );
        }
        KeyCode::Char('n') | KeyCode::Char('N') => {
            model.delete_tasks_confirm = None;
            return (model, AppCommand::None);
        }
        _ => {}
    }

    model.delete_tasks_confirm = Some(confirm);
    (model, AppCommand::None)
}

fn update_error(model: AppModel, key: KeyEvent) -> (AppModel, AppCommand) {
    match key.code {
        KeyCode::Esc | KeyCode::Backspace => {
            let mut view = ProjectsView::new(&model.data.projects);
            apply_project_filter(&model.data.projects, &mut view, model.engine_filter);
            (
                AppModel {
                    data: model.data.clone(),
                    session_index: model.session_index.clone(),
                    terminal_size: model.terminal_size,
                    notice: None,
                    update_hint: model.update_hint.clone(),
                    engine_filter: model.engine_filter,
                    help_open: model.help_open,
                    system_menu: model.system_menu.clone(),
                    delete_confirm: model.delete_confirm.clone(),
                    delete_projects_confirm: model.delete_projects_confirm.clone(),
                    delete_session_confirm: model.delete_session_confirm.clone(),
                    delete_sessions_confirm: model.delete_sessions_confirm.clone(),
                    delete_task_confirm: model.delete_task_confirm.clone(),
                    delete_tasks_confirm: model.delete_tasks_confirm.clone(),
                    session_result_preview: model.session_result_preview.clone(),
                    session_stats_overlay: model.session_stats_overlay.clone(),
                    project_stats_overlay: model.project_stats_overlay.clone(),
                    processes: model.processes.clone(),
                    view: View::Projects(view),
                },
                AppCommand::None,
            )
        }
        _ => (model, AppCommand::None),
    }
}

fn update_projects(
    mut model: AppModel,
    mut view: ProjectsView,
    key: KeyEvent,
) -> (AppModel, AppCommand) {
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    match key.code {
        KeyCode::F(3) => {
            let Some(project_index) = view.filtered_indices.get(view.selected).copied() else {
                return (model, AppCommand::None);
            };
            let Some(project) = model.data.projects.get(project_index) else {
                return (model, AppCommand::None);
            };

            model.project_stats_overlay = Some(ProjectStatsOverlay::from_project(
                project,
                &model.session_index,
            ));
            model.help_open = false;
            model.system_menu = None;
            return (model, AppCommand::None);
        }
        KeyCode::Enter => {
            let Some(project_index) = view.filtered_indices.get(view.selected).copied() else {
                return (model, AppCommand::None);
            };
            let Some(project) = model.data.projects.get(project_index) else {
                return (model, AppCommand::None);
            };
            let mut sessions_view =
                SessionsView::new(project.project_path.clone(), project.sessions.len());
            apply_session_filter(&project.sessions, &mut sessions_view, model.engine_filter);
            let next = AppModel {
                data: model.data.clone(),
                session_index: model.session_index.clone(),
                terminal_size: model.terminal_size,
                notice: None,
                update_hint: model.update_hint.clone(),
                engine_filter: model.engine_filter,
                help_open: model.help_open,
                system_menu: model.system_menu.clone(),
                delete_confirm: model.delete_confirm.clone(),
                delete_projects_confirm: model.delete_projects_confirm.clone(),
                delete_session_confirm: model.delete_session_confirm.clone(),
                delete_sessions_confirm: model.delete_sessions_confirm.clone(),
                delete_task_confirm: model.delete_task_confirm.clone(),
                delete_tasks_confirm: model.delete_tasks_confirm.clone(),
                session_result_preview: model.session_result_preview.clone(),
                session_stats_overlay: model.session_stats_overlay.clone(),
                project_stats_overlay: model.project_stats_overlay.clone(),
                processes: model.processes.clone(),
                view: View::Sessions(sessions_view),
            };
            return (next, AppCommand::None);
        }
        KeyCode::Esc => {
            if !view.query.is_empty() {
                view.query.clear();
                apply_project_filter(&model.data.projects, &mut view, model.engine_filter);
                clear_project_selection(&mut view);
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
            if shift {
                ensure_project_selection_anchor(&model.data.projects, &mut view);
                view.selected = view.selected.saturating_sub(1);
                update_project_range_selection(&model.data.projects, &mut view);
            } else {
                view.selected = view.selected.saturating_sub(1);
                clear_project_selection(&mut view);
            }
        }
        KeyCode::Down => {
            if !view.filtered_indices.is_empty() {
                if shift {
                    ensure_project_selection_anchor(&model.data.projects, &mut view);
                    view.selected =
                        (view.selected + 1).min(view.filtered_indices.len().saturating_sub(1));
                    update_project_range_selection(&model.data.projects, &mut view);
                } else {
                    view.selected =
                        (view.selected + 1).min(view.filtered_indices.len().saturating_sub(1));
                    clear_project_selection(&mut view);
                }
            }
        }
        KeyCode::PageUp => {
            let step = page_step_standard_list(model.terminal_size);
            if shift {
                ensure_project_selection_anchor(&model.data.projects, &mut view);
                view.selected = view.selected.saturating_sub(step);
                update_project_range_selection(&model.data.projects, &mut view);
            } else {
                view.selected = view.selected.saturating_sub(step);
                clear_project_selection(&mut view);
            }
        }
        KeyCode::PageDown => {
            if !view.filtered_indices.is_empty() {
                let step = page_step_standard_list(model.terminal_size);
                if shift {
                    ensure_project_selection_anchor(&model.data.projects, &mut view);
                    view.selected =
                        (view.selected + step).min(view.filtered_indices.len().saturating_sub(1));
                    update_project_range_selection(&model.data.projects, &mut view);
                } else {
                    view.selected =
                        (view.selected + step).min(view.filtered_indices.len().saturating_sub(1));
                    clear_project_selection(&mut view);
                }
            }
        }
        KeyCode::Backspace => {
            if !view.query.is_empty() {
                view.query.pop();
                apply_project_filter(&model.data.projects, &mut view, model.engine_filter);
                clear_project_selection(&mut view);
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
                apply_project_filter(&model.data.projects, &mut view, model.engine_filter);
                clear_project_selection(&mut view);
            }
        }
        _ => {}
    }

    (
        AppModel {
            data: model.data.clone(),
            session_index: model.session_index.clone(),
            terminal_size: model.terminal_size,
            notice: None,
            update_hint: model.update_hint.clone(),
            engine_filter: model.engine_filter,
            help_open: model.help_open,
            system_menu: model.system_menu.clone(),
            delete_confirm: model.delete_confirm.clone(),
            delete_projects_confirm: model.delete_projects_confirm.clone(),
            delete_session_confirm: model.delete_session_confirm.clone(),
            delete_sessions_confirm: model.delete_sessions_confirm.clone(),
            delete_task_confirm: model.delete_task_confirm.clone(),
            delete_tasks_confirm: model.delete_tasks_confirm.clone(),
            session_result_preview: model.session_result_preview.clone(),
            session_stats_overlay: model.session_stats_overlay.clone(),
            project_stats_overlay: model.project_stats_overlay.clone(),
            processes: model.processes.clone(),
            view: View::Projects(view),
        },
        AppCommand::None,
    )
}

fn open_delete_confirm(model: &mut AppModel, view: &ProjectsView) {
    if view.selected_project_paths.len() >= 2 {
        let project_paths = view
            .selected_project_paths
            .iter()
            .cloned()
            .collect::<Vec<_>>();

        let mut session_count = 0usize;
        let mut total_size_bytes = 0u64;
        for project_path in &project_paths {
            let Some(project) = model
                .data
                .projects
                .iter()
                .find(|project| &project.project_path == project_path)
            else {
                continue;
            };
            session_count = session_count.saturating_add(project.sessions.len());
            total_size_bytes = total_size_bytes.saturating_add(
                project
                    .sessions
                    .iter()
                    .map(|session| session.file_size_bytes)
                    .sum::<u64>(),
            );
        }

        model.delete_projects_confirm = Some(DeleteProjectsConfirmDialog {
            project_count: project_paths.len(),
            project_paths,
            session_count,
            total_size_bytes,
            selection: DeleteConfirmSelection::Cancel,
        });
        return;
    }

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

fn infer_engine_from_log_path(log_path: &Path) -> EngineFilter {
    if crate::domain::has_path_component(log_path, ".claude") {
        return EngineFilter::Claude;
    }
    if crate::domain::has_path_component(log_path, ".gemini") {
        return EngineFilter::Gemini;
    }
    EngineFilter::Codex
}

fn session_matches_engine_filter(session: &SessionSummary, filter: EngineFilter) -> bool {
    match filter {
        EngineFilter::All => true,
        EngineFilter::Codex | EngineFilter::Claude | EngineFilter::Gemini => {
            infer_engine_from_log_path(&session.log_path) == filter
        }
    }
}

fn project_matches_engine_filter(project: &ProjectSummary, filter: EngineFilter) -> bool {
    match filter {
        EngineFilter::All => true,
        EngineFilter::Codex | EngineFilter::Claude | EngineFilter::Gemini => project
            .sessions
            .iter()
            .any(|session| session_matches_engine_filter(session, filter)),
    }
}

fn apply_project_filter(
    projects: &[ProjectSummary],
    view: &mut ProjectsView,
    engine: EngineFilter,
) {
    let query = view.query.trim().to_lowercase();
    if query.is_empty() {
        view.filtered_indices = projects
            .iter()
            .enumerate()
            .filter_map(|(index, project)| {
                project_matches_engine_filter(project, engine).then_some(index)
            })
            .collect();
    } else {
        view.filtered_indices = projects
            .iter()
            .enumerate()
            .filter_map(|(index, project)| {
                if !project_matches_engine_filter(project, engine) {
                    return None;
                }
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

fn prune_project_selection(projects: &[ProjectSummary], view: &mut ProjectsView) {
    let mut available = BTreeSet::new();
    for project in projects {
        available.insert(project.project_path.clone());
    }

    view.selected_project_paths
        .retain(|path| available.contains(path));
    if let Some(anchor) = &view.selection_anchor {
        if !available.contains(anchor) {
            view.selection_anchor = None;
        }
    }
}

fn apply_session_filter(
    sessions: &[SessionSummary],
    view: &mut SessionsView,
    engine: EngineFilter,
) {
    let query = view.query.trim().to_lowercase();
    if query.is_empty() {
        view.filtered_indices = sessions
            .iter()
            .enumerate()
            .filter_map(|(index, session)| {
                session_matches_engine_filter(session, engine).then_some(index)
            })
            .collect();
    } else {
        view.filtered_indices = sessions
            .iter()
            .enumerate()
            .filter_map(|(index, session)| {
                if !session_matches_engine_filter(session, engine) {
                    return None;
                }
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

fn prune_sessions_selection(sessions: &[SessionSummary], view: &mut SessionsView) {
    let mut available = BTreeSet::new();
    for session in sessions {
        available.insert(session.log_path.clone());
    }

    view.selected_log_paths
        .retain(|path| available.contains(path));
    if let Some(anchor) = &view.selection_anchor {
        if !available.contains(anchor) {
            view.selection_anchor = None;
        }
    }
}

fn apply_task_filter(view: &mut TasksView) {
    let query = view.query.trim().to_lowercase();
    if query.is_empty() {
        view.filtered_indices = (0..view.tasks.len()).collect();
    } else {
        view.filtered_indices = view
            .tasks
            .iter()
            .enumerate()
            .filter_map(|(index, task)| {
                let haystack = format!(
                    "{}\n{}",
                    task.title.to_lowercase(),
                    task.project_path.display().to_string().to_lowercase()
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

fn prune_task_selection(view: &mut TasksView) {
    let mut available = BTreeSet::new();
    for task in &view.tasks {
        available.insert(task.id.clone());
    }

    view.selected_task_ids
        .retain(|task_id| available.contains(task_id));
    if let Some(anchor) = &view.selection_anchor {
        if !available.contains(anchor) {
            view.selection_anchor = None;
        }
    }
}

fn clear_project_selection(view: &mut ProjectsView) {
    view.selection_anchor = None;
    view.selected_project_paths.clear();
}

fn ensure_project_selection_anchor(projects: &[ProjectSummary], view: &mut ProjectsView) {
    if view.selection_anchor.is_some() {
        return;
    }
    let Some(anchor) = current_project_path(projects, view) else {
        return;
    };
    view.selection_anchor = Some(anchor);
}

fn update_project_range_selection(projects: &[ProjectSummary], view: &mut ProjectsView) {
    if view.filtered_indices.is_empty() {
        clear_project_selection(view);
        return;
    }

    let Some(cursor_path) = current_project_path(projects, view) else {
        clear_project_selection(view);
        return;
    };

    let Some(anchor_path) = view.selection_anchor.clone() else {
        view.selection_anchor = Some(cursor_path.clone());
        view.selected_project_paths.clear();
        view.selected_project_paths.insert(cursor_path);
        return;
    };

    let Some(anchor_pos) = view.filtered_indices.iter().position(|index| {
        projects
            .get(*index)
            .is_some_and(|project| project.project_path == anchor_path)
    }) else {
        view.selection_anchor = Some(cursor_path.clone());
        view.selected_project_paths.clear();
        view.selected_project_paths.insert(cursor_path);
        return;
    };

    let cursor_pos = view
        .selected
        .min(view.filtered_indices.len().saturating_sub(1));
    let start = anchor_pos.min(cursor_pos);
    let end = anchor_pos.max(cursor_pos);

    view.selected_project_paths.clear();
    for pos in start..=end {
        let Some(project_index) = view.filtered_indices.get(pos).copied() else {
            continue;
        };
        let Some(project) = projects.get(project_index) else {
            continue;
        };
        view.selected_project_paths
            .insert(project.project_path.clone());
    }
}

fn current_project_path(projects: &[ProjectSummary], view: &ProjectsView) -> Option<PathBuf> {
    let project_index = view.filtered_indices.get(view.selected).copied()?;
    let project = projects.get(project_index)?;
    Some(project.project_path.clone())
}

fn clear_sessions_selection(view: &mut SessionsView) {
    view.selection_anchor = None;
    view.selected_log_paths.clear();
}

fn ensure_sessions_selection_anchor(sessions: &[SessionSummary], view: &mut SessionsView) {
    if view.selection_anchor.is_some() {
        return;
    }
    let Some(anchor) = current_session_log_path(sessions, view) else {
        return;
    };
    view.selection_anchor = Some(anchor);
}

fn update_sessions_range_selection(sessions: &[SessionSummary], view: &mut SessionsView) {
    if view.filtered_indices.is_empty() {
        clear_sessions_selection(view);
        return;
    }

    let Some(cursor_path) = current_session_log_path(sessions, view) else {
        clear_sessions_selection(view);
        return;
    };

    let Some(anchor_path) = view.selection_anchor.clone() else {
        view.selection_anchor = Some(cursor_path.clone());
        view.selected_log_paths.clear();
        view.selected_log_paths.insert(cursor_path);
        return;
    };

    let Some(anchor_pos) = view.filtered_indices.iter().position(|index| {
        sessions
            .get(*index)
            .is_some_and(|session| session.log_path == anchor_path)
    }) else {
        view.selection_anchor = Some(cursor_path.clone());
        view.selected_log_paths.clear();
        view.selected_log_paths.insert(cursor_path);
        return;
    };

    let cursor_pos = view
        .session_selected
        .min(view.filtered_indices.len().saturating_sub(1));
    let start = anchor_pos.min(cursor_pos);
    let end = anchor_pos.max(cursor_pos);

    view.selected_log_paths.clear();
    for pos in start..=end {
        let Some(session_index) = view.filtered_indices.get(pos).copied() else {
            continue;
        };
        let Some(session) = sessions.get(session_index) else {
            continue;
        };
        view.selected_log_paths.insert(session.log_path.clone());
    }
}

fn current_session_log_path(sessions: &[SessionSummary], view: &SessionsView) -> Option<PathBuf> {
    let session_index = view.filtered_indices.get(view.session_selected).copied()?;
    let session = sessions.get(session_index)?;
    Some(session.log_path.clone())
}

fn clear_tasks_selection(view: &mut TasksView) {
    view.selection_anchor = None;
    view.selected_task_ids.clear();
}

fn ensure_tasks_selection_anchor(view: &mut TasksView) {
    if view.selection_anchor.is_some() {
        return;
    }
    let Some(anchor) = current_task_id(view) else {
        return;
    };
    view.selection_anchor = Some(anchor);
}

fn update_tasks_range_selection(view: &mut TasksView) {
    if view.filtered_indices.is_empty() {
        clear_tasks_selection(view);
        return;
    }

    let Some(cursor_id) = current_task_id(view) else {
        clear_tasks_selection(view);
        return;
    };

    let Some(anchor_id) = view.selection_anchor.clone() else {
        view.selection_anchor = Some(cursor_id.clone());
        view.selected_task_ids.clear();
        view.selected_task_ids.insert(cursor_id);
        return;
    };

    let Some(anchor_pos) = view.filtered_indices.iter().position(|index| {
        view.tasks
            .get(*index)
            .is_some_and(|task| task.id == anchor_id)
    }) else {
        view.selection_anchor = Some(cursor_id.clone());
        view.selected_task_ids.clear();
        view.selected_task_ids.insert(cursor_id);
        return;
    };

    let cursor_pos = view
        .selected
        .min(view.filtered_indices.len().saturating_sub(1));
    let start = anchor_pos.min(cursor_pos);
    let end = anchor_pos.max(cursor_pos);

    view.selected_task_ids.clear();
    for pos in start..=end {
        let Some(task_index) = view.filtered_indices.get(pos).copied() else {
            continue;
        };
        let Some(task) = view.tasks.get(task_index) else {
            continue;
        };
        view.selected_task_ids.insert(task.id.clone());
    }
}

fn current_task_id(view: &TasksView) -> Option<TaskId> {
    let task_index = view.filtered_indices.get(view.selected).copied()?;
    let task = view.tasks.get(task_index)?;
    Some(task.id.clone())
}

fn selected_task_id(view: &TasksView) -> Option<TaskId> {
    let index = view.filtered_indices.get(view.selected).copied()?;
    let task = view.tasks.get(index)?;
    Some(task.id.clone())
}

fn open_delete_task_confirm_from_tasks(model: &mut AppModel, view: &TasksView) -> bool {
    if view.selected_task_ids.len() >= 2 {
        let task_ids = view.selected_task_ids.iter().cloned().collect::<Vec<_>>();
        model.delete_tasks_confirm = Some(DeleteTasksConfirmDialog {
            task_count: task_ids.len(),
            task_ids,
            selection: DeleteConfirmSelection::Cancel,
            return_to_tasks: view.clone(),
        });
        return true;
    }

    let Some(task_index) = view.filtered_indices.get(view.selected).copied() else {
        return false;
    };
    let Some(task) = view.tasks.get(task_index) else {
        return false;
    };

    model.delete_task_confirm = Some(DeleteTaskConfirmDialog {
        task_id: task.id.clone(),
        task_title: task.title.clone(),
        project_path: task.project_path.clone(),
        selection: DeleteConfirmSelection::Cancel,
        return_to_tasks: view.clone(),
    });

    true
}

fn default_task_create_project_path(model: &AppModel, view: &TasksView) -> PathBuf {
    let selected_path = view
        .filtered_indices
        .get(view.selected)
        .copied()
        .and_then(|index| view.tasks.get(index).map(|task| task.project_path.clone()));
    if let Some(path) = selected_path {
        return path;
    }

    infer_project_path_for_new_task_view(view.return_to.as_ref(), model).unwrap_or_default()
}

fn infer_project_path_for_new_task(model: &AppModel) -> Option<PathBuf> {
    infer_project_path_for_new_task_view(&model.view, model)
}

fn infer_project_path_for_new_task_view(view: &View, model: &AppModel) -> Option<PathBuf> {
    match view {
        View::Projects(projects_view) => {
            let project_index = projects_view
                .filtered_indices
                .get(projects_view.selected)
                .copied()?;
            let project = model.data.projects.get(project_index)?;
            Some(project.project_path.clone())
        }
        View::Sessions(sessions_view) => Some(sessions_view.project_path.clone()),
        View::NewSession(new_session_view) => {
            Some(new_session_view.from_sessions.project_path.clone())
        }
        View::SessionDetail(detail_view) => Some(detail_view.from_sessions.project_path.clone()),
        View::Tasks(tasks_view) => {
            infer_project_path_for_new_task_view(tasks_view.return_to.as_ref(), model)
        }
        View::TaskCreate(task_create) => {
            if !task_create.project_path.as_os_str().is_empty() {
                Some(task_create.project_path.clone())
            } else {
                infer_project_path_for_new_task_view(
                    task_create.from_tasks.return_to.as_ref(),
                    model,
                )
            }
        }
        View::TaskDetail(task_detail) => Some(task_detail.task.project_path.clone()),
        View::Processes(processes_view) => {
            infer_project_path_for_new_task_view(processes_view.return_to.as_ref(), model)
        }
        View::ProcessOutput(output_view) => {
            infer_project_path_for_new_task_view(output_view.return_to.as_ref(), model)
        }
        View::Error => None,
    }
}

fn open_delete_session_confirm(model: &mut AppModel, view: &SessionsView) -> bool {
    let Some(project) = view.current_project(&model.data.projects) else {
        return false;
    };

    if view.selected_log_paths.len() >= 2 {
        let log_paths = view.selected_log_paths.iter().cloned().collect::<Vec<_>>();
        let mut total_size_bytes = 0u64;
        for session in &project.sessions {
            if view.selected_log_paths.contains(&session.log_path) {
                total_size_bytes = total_size_bytes.saturating_add(session.file_size_bytes);
            }
        }

        model.delete_sessions_confirm = Some(DeleteSessionsConfirmDialog {
            project_name: project.name.clone(),
            project_path: project.project_path.clone(),
            session_count: log_paths.len(),
            log_paths,
            total_size_bytes,
            selection: DeleteConfirmSelection::Cancel,
        });
        return true;
    }

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
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

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
                    apply_session_filter(&project.sessions, &mut view, model.engine_filter);
                } else {
                    view.filtered_indices.clear();
                    view.session_selected = 0;
                }
                clear_sessions_selection(&mut view);
                model.view = View::Sessions(view);
                return (model, AppCommand::None);
            }

            let mut projects_view = ProjectsView::new(&model.data.projects);
            apply_project_filter(
                &model.data.projects,
                &mut projects_view,
                model.engine_filter,
            );
            let next = AppModel {
                data: model.data.clone(),
                session_index: model.session_index.clone(),
                terminal_size: model.terminal_size,
                notice: None,
                update_hint: model.update_hint.clone(),
                engine_filter: model.engine_filter,
                help_open: model.help_open,
                system_menu: model.system_menu.clone(),
                delete_confirm: model.delete_confirm.clone(),
                delete_projects_confirm: model.delete_projects_confirm.clone(),
                delete_session_confirm: model.delete_session_confirm.clone(),
                delete_sessions_confirm: model.delete_sessions_confirm.clone(),
                delete_task_confirm: model.delete_task_confirm.clone(),
                delete_tasks_confirm: model.delete_tasks_confirm.clone(),
                session_result_preview: model.session_result_preview.clone(),
                session_stats_overlay: model.session_stats_overlay.clone(),
                project_stats_overlay: model.project_stats_overlay.clone(),
                processes: model.processes.clone(),
                view: View::Projects(projects_view),
            };
            return (next, AppCommand::None);
        }
        KeyCode::Up => {
            if shift {
                if let Some(project) = view.current_project(&model.data.projects) {
                    ensure_sessions_selection_anchor(&project.sessions, &mut view);
                    view.session_selected = view.session_selected.saturating_sub(1);
                    update_sessions_range_selection(&project.sessions, &mut view);
                } else {
                    clear_sessions_selection(&mut view);
                }
            } else {
                view.session_selected = view.session_selected.saturating_sub(1);
                clear_sessions_selection(&mut view);
            }
        }
        KeyCode::Down => {
            if !view.filtered_indices.is_empty() {
                if shift {
                    if let Some(project) = view.current_project(&model.data.projects) {
                        ensure_sessions_selection_anchor(&project.sessions, &mut view);
                        view.session_selected = (view.session_selected + 1)
                            .min(view.filtered_indices.len().saturating_sub(1));
                        update_sessions_range_selection(&project.sessions, &mut view);
                    } else {
                        clear_sessions_selection(&mut view);
                    }
                } else {
                    view.session_selected = (view.session_selected + 1)
                        .min(view.filtered_indices.len().saturating_sub(1));
                    clear_sessions_selection(&mut view);
                }
            }
        }
        KeyCode::PageUp => {
            let step = page_step_standard_list(model.terminal_size);
            if shift {
                if let Some(project) = view.current_project(&model.data.projects) {
                    ensure_sessions_selection_anchor(&project.sessions, &mut view);
                    view.session_selected = view.session_selected.saturating_sub(step);
                    update_sessions_range_selection(&project.sessions, &mut view);
                } else {
                    clear_sessions_selection(&mut view);
                }
            } else {
                view.session_selected = view.session_selected.saturating_sub(step);
                clear_sessions_selection(&mut view);
            }
        }
        KeyCode::PageDown => {
            if !view.filtered_indices.is_empty() {
                let step = page_step_standard_list(model.terminal_size);
                if shift {
                    if let Some(project) = view.current_project(&model.data.projects) {
                        ensure_sessions_selection_anchor(&project.sessions, &mut view);
                        view.session_selected = (view.session_selected + step)
                            .min(view.filtered_indices.len().saturating_sub(1));
                        update_sessions_range_selection(&project.sessions, &mut view);
                    } else {
                        clear_sessions_selection(&mut view);
                    }
                } else {
                    view.session_selected = (view.session_selected + step)
                        .min(view.filtered_indices.len().saturating_sub(1));
                    clear_sessions_selection(&mut view);
                }
            }
        }
        KeyCode::Backspace => {
            if !view.query.is_empty() {
                view.query.pop();
                if let Some(project) = view.current_project(&model.data.projects) {
                    apply_session_filter(&project.sessions, &mut view, model.engine_filter);
                } else {
                    view.filtered_indices.clear();
                    view.session_selected = 0;
                }
                clear_sessions_selection(&mut view);
                model.view = View::Sessions(view);
                return (model, AppCommand::None);
            }

            let opened = open_delete_session_confirm(&mut model, &view);
            if !opened {
                let mut projects_view = ProjectsView::new(&model.data.projects);
                apply_project_filter(
                    &model.data.projects,
                    &mut projects_view,
                    model.engine_filter,
                );
                let next = AppModel {
                    data: model.data.clone(),
                    session_index: model.session_index.clone(),
                    terminal_size: model.terminal_size,
                    notice: None,
                    update_hint: model.update_hint.clone(),
                    engine_filter: model.engine_filter,
                    help_open: model.help_open,
                    system_menu: model.system_menu.clone(),
                    delete_confirm: model.delete_confirm.clone(),
                    delete_projects_confirm: model.delete_projects_confirm.clone(),
                    delete_session_confirm: model.delete_session_confirm.clone(),
                    delete_sessions_confirm: model.delete_sessions_confirm.clone(),
                    delete_task_confirm: model.delete_task_confirm.clone(),
                    delete_tasks_confirm: model.delete_tasks_confirm.clone(),
                    session_result_preview: model.session_result_preview.clone(),
                    session_stats_overlay: model.session_stats_overlay.clone(),
                    project_stats_overlay: model.project_stats_overlay.clone(),
                    processes: model.processes.clone(),
                    view: View::Projects(projects_view),
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
                session_index: model.session_index.clone(),
                terminal_size: model.terminal_size,
                notice: None,
                update_hint: model.update_hint.clone(),
                engine_filter: model.engine_filter,
                help_open: model.help_open,
                system_menu: model.system_menu.clone(),
                delete_confirm: model.delete_confirm.clone(),
                delete_projects_confirm: model.delete_projects_confirm.clone(),
                delete_session_confirm: model.delete_session_confirm.clone(),
                delete_sessions_confirm: model.delete_sessions_confirm.clone(),
                delete_task_confirm: model.delete_task_confirm.clone(),
                delete_tasks_confirm: model.delete_tasks_confirm.clone(),
                session_result_preview: model.session_result_preview.clone(),
                session_stats_overlay: model.session_stats_overlay.clone(),
                project_stats_overlay: model.project_stats_overlay.clone(),
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
                    apply_session_filter(&project.sessions, &mut view, model.engine_filter);
                } else {
                    view.filtered_indices.clear();
                    view.session_selected = 0;
                }
                clear_sessions_selection(&mut view);
            }
        }
        _ => {}
    }

    (
        AppModel {
            data: model.data.clone(),
            session_index: model.session_index.clone(),
            terminal_size: model.terminal_size,
            notice: None,
            update_hint: model.update_hint.clone(),
            engine_filter: model.engine_filter,
            help_open: model.help_open,
            system_menu: model.system_menu.clone(),
            delete_confirm: model.delete_confirm.clone(),
            delete_projects_confirm: model.delete_projects_confirm.clone(),
            delete_session_confirm: model.delete_session_confirm.clone(),
            delete_sessions_confirm: model.delete_sessions_confirm.clone(),
            delete_task_confirm: model.delete_task_confirm.clone(),
            delete_tasks_confirm: model.delete_tasks_confirm.clone(),
            session_result_preview: model.session_result_preview.clone(),
            session_stats_overlay: model.session_stats_overlay.clone(),
            project_stats_overlay: model.project_stats_overlay.clone(),
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
        KeyCode::F(4) => {
            if view.fork.is_some() {
                model.notice = Some("I/O mode is locked for fork resume.".to_string());
            } else {
                view.io_mode = view.io_mode.toggle();
            }
        }
        KeyCode::BackTab => {
            if view.fork.is_some() {
                model.notice = Some("Engine is locked to Codex for fork resume.".to_string());
            } else {
                view.engine = view.engine.toggle();
            }
        }
        KeyCode::Enter if send_modifier => {
            let prompt = view.editor.text();
            if prompt.trim().is_empty() {
                model.notice = Some("Prompt is empty.".to_string());
                model.view = View::NewSession(view);
                return (model, AppCommand::None);
            }

            model.view = View::Sessions(view.from_sessions.clone());
            if let Some(fork) = view.fork.clone() {
                return (
                    model,
                    AppCommand::ForkResumeCodexFromTimeline { fork, prompt },
                );
            }

            let project_path = view.from_sessions.project_path.clone();
            let engine = view.engine;
            let io_mode = view.io_mode;
            return (
                model,
                AppCommand::SpawnAgentSession {
                    engine,
                    project_path,
                    prompt,
                    io_mode,
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
        KeyCode::Char('a') | KeyCode::Char('A') => {
            if let Some(process) = model.processes.get(view.selected) {
                if !process.io_mode.is_tty() {
                    model.notice = Some("Process is not a TTY session.".to_string());
                    model.view = View::Processes(view);
                    return (model, AppCommand::None);
                }
                if !process.status.is_running() {
                    model.notice = Some("Process is not running.".to_string());
                    model.view = View::Processes(view);
                    return (model, AppCommand::None);
                }
                let process_id = process.id.clone();
                model.view = View::Processes(view);
                return (model, AppCommand::AttachProcessTty { process_id });
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
        KeyCode::Tab | KeyCode::BackTab => {
            view.focus = view.focus.toggle();
        }
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
        KeyCode::Up => match view.focus {
            SessionDetailFocus::Timeline => {
                view.selected = view.selected.saturating_sub(1);
                view.details_scroll = 0;
            }
            SessionDetailFocus::Details => {
                view.details_scroll = view.details_scroll.saturating_sub(1);
            }
        },
        KeyCode::Down => match view.focus {
            SessionDetailFocus::Timeline => {
                if !view.items.is_empty() {
                    view.selected = (view.selected + 1).min(view.items.len().saturating_sub(1));
                }
                view.details_scroll = 0;
            }
            SessionDetailFocus::Details => {
                view.details_scroll = view.details_scroll.saturating_add(1);
            }
        },
        KeyCode::PageUp => match view.focus {
            SessionDetailFocus::Timeline => {
                let step = page_step_session_detail_list(model.terminal_size);
                view.selected = view.selected.saturating_sub(step);
                view.details_scroll = 0;
            }
            SessionDetailFocus::Details => {
                let step = page_step_session_detail_details(model.terminal_size) as u16;
                view.details_scroll = view.details_scroll.saturating_sub(step);
            }
        },
        KeyCode::PageDown => match view.focus {
            SessionDetailFocus::Timeline => {
                if !view.items.is_empty() {
                    let step = page_step_session_detail_list(model.terminal_size);
                    view.selected = (view.selected + step).min(view.items.len().saturating_sub(1));
                }
                view.details_scroll = 0;
            }
            SessionDetailFocus::Details => {
                let step = page_step_session_detail_details(model.terminal_size) as u16;
                view.details_scroll = view.details_scroll.saturating_add(step);
            }
        },
        KeyCode::Enter => {
            let selected = view.selected.min(view.items.len().saturating_sub(1));
            if let Some(item) = view.items.get(selected) {
                if item.kind == TimelineItemKind::ToolCall {
                    if let Some(call_id) = item.call_id.as_deref() {
                        if let Some(output_index) =
                            find_tool_output_index(&view.items, selected, call_id)
                        {
                            view.selected = output_index;
                            view.details_scroll = 0;
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
        KeyCode::Char('f') | KeyCode::Char('F') => {
            if infer_engine_from_log_path(&view.session.log_path) != EngineFilter::Codex {
                model.notice =
                    Some("Fork/resume is only available for Codex sessions.".to_string());
                model.view = View::SessionDetail(view);
                return (model, AppCommand::None);
            }

            if view.items.is_empty() {
                model.notice = Some("No timeline items.".to_string());
                model.view = View::SessionDetail(view);
                return (model, AppCommand::None);
            }

            let selected = view.selected.min(view.items.len().saturating_sub(1));
            let Some(item) = view.items.get(selected) else {
                model.view = View::SessionDetail(view);
                return (model, AppCommand::None);
            };

            match fork_context_from_timeline_item(&view.session, item) {
                Ok(fork) => {
                    let mut new_view = NewSessionView::new(view.from_sessions.clone());
                    new_view.engine = AgentEngine::Codex;
                    new_view.io_mode = SpawnIoMode::Pipes;
                    new_view.fork = Some(fork.clone());
                    new_view.editor.insert_str(&default_fork_prompt(&fork));

                    model.view = View::NewSession(new_view);
                    return (model, AppCommand::None);
                }
                Err(error) => {
                    model.notice = Some(error.to_string());
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

fn update_tasks(mut model: AppModel, mut view: TasksView, key: KeyEvent) -> (AppModel, AppCommand) {
    let send_modifier = key.modifiers.contains(KeyModifiers::CONTROL)
        || key.modifiers.contains(KeyModifiers::SUPER)
        || key.modifiers.contains(KeyModifiers::META);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    match key.code {
        KeyCode::F(3) => {
            let Some(task_index) = view.filtered_indices.get(view.selected).copied() else {
                return (model, AppCommand::None);
            };
            let Some(task) = view.tasks.get(task_index) else {
                return (model, AppCommand::None);
            };
            let Some(project) = model
                .data
                .projects
                .iter()
                .find(|project| project.project_path == task.project_path)
            else {
                model.notice = Some("Project not indexed.".to_string());
                model.view = View::Tasks(view);
                return (model, AppCommand::None);
            };

            model.project_stats_overlay = Some(ProjectStatsOverlay::from_project(
                project,
                &model.session_index,
            ));
            model.help_open = false;
            model.system_menu = None;
            return (model, AppCommand::None);
        }
        KeyCode::Esc => {
            if !view.query.is_empty() {
                view.query.clear();
                apply_task_filter(&mut view);
                clear_tasks_selection(&mut view);
                model.view = View::Tasks(view);
                return (model, AppCommand::None);
            }

            model.view = *view.return_to;
            return (model, AppCommand::None);
        }
        KeyCode::Backspace => {
            if !view.query.is_empty() {
                view.query.pop();
                apply_task_filter(&mut view);
                clear_tasks_selection(&mut view);
                model.view = View::Tasks(view);
                return (model, AppCommand::None);
            }

            model.view = *view.return_to;
            return (model, AppCommand::None);
        }
        KeyCode::Up => {
            if shift {
                ensure_tasks_selection_anchor(&mut view);
                view.selected = view.selected.saturating_sub(1);
                update_tasks_range_selection(&mut view);
            } else {
                view.selected = view.selected.saturating_sub(1);
                clear_tasks_selection(&mut view);
            }
        }
        KeyCode::Down => {
            if !view.filtered_indices.is_empty() {
                if shift {
                    ensure_tasks_selection_anchor(&mut view);
                    view.selected =
                        (view.selected + 1).min(view.filtered_indices.len().saturating_sub(1));
                    update_tasks_range_selection(&mut view);
                } else {
                    view.selected =
                        (view.selected + 1).min(view.filtered_indices.len().saturating_sub(1));
                    clear_tasks_selection(&mut view);
                }
            }
        }
        KeyCode::PageUp => {
            let step = page_step_standard_list(model.terminal_size);
            if shift {
                ensure_tasks_selection_anchor(&mut view);
                view.selected = view.selected.saturating_sub(step);
                update_tasks_range_selection(&mut view);
            } else {
                view.selected = view.selected.saturating_sub(step);
                clear_tasks_selection(&mut view);
            }
        }
        KeyCode::PageDown => {
            if !view.filtered_indices.is_empty() {
                let step = page_step_standard_list(model.terminal_size);
                if shift {
                    ensure_tasks_selection_anchor(&mut view);
                    view.selected =
                        (view.selected + step).min(view.filtered_indices.len().saturating_sub(1));
                    update_tasks_range_selection(&mut view);
                } else {
                    view.selected =
                        (view.selected + step).min(view.filtered_indices.len().saturating_sub(1));
                    clear_tasks_selection(&mut view);
                }
            }
        }
        KeyCode::BackTab => {
            view.engine = view.engine.toggle();
        }
        KeyCode::Delete => {
            open_delete_task_confirm_from_tasks(&mut model, &view);
        }
        KeyCode::Enter if send_modifier => {
            let Some(task_id) = selected_task_id(&view) else {
                model.notice = Some("No task selected.".to_string());
                model.view = View::Tasks(view);
                return (model, AppCommand::None);
            };
            return (
                model,
                AppCommand::SpawnTask {
                    engine: view.engine,
                    task_id,
                },
            );
        }
        KeyCode::Enter => {
            let Some(task_id) = selected_task_id(&view) else {
                return (model, AppCommand::None);
            };
            return (
                model,
                AppCommand::OpenTaskDetail {
                    from_tasks: view,
                    task_id,
                },
            );
        }
        KeyCode::Char('n') | KeyCode::Char('N') => {
            let project_path = default_task_create_project_path(&model, &view);
            model.view = View::TaskCreate(TaskCreateView::new(view, project_path));
            return (model, AppCommand::None);
        }
        KeyCode::Char(character) => {
            if is_text_input_char(character) {
                view.query.push(character);
                apply_task_filter(&mut view);
                clear_tasks_selection(&mut view);
            }
        }
        _ => {}
    }

    model.view = View::Tasks(view);
    (model, AppCommand::None)
}

fn update_task_create(
    mut model: AppModel,
    mut view: TaskCreateView,
    key: KeyEvent,
) -> (AppModel, AppCommand) {
    let command_modifier = key.modifiers.contains(KeyModifiers::CONTROL)
        || key.modifiers.contains(KeyModifiers::SUPER)
        || key.modifiers.contains(KeyModifiers::META);

    if let Some(overlay) = view.overlay.take() {
        match overlay {
            TaskCreateOverlay::ImagePath(mut editor) => {
                match key.code {
                    KeyCode::Esc => {
                        view.overlay = None;
                        model.view = View::TaskCreate(view);
                        return (model, AppCommand::None);
                    }
                    KeyCode::Char('v') | KeyCode::Char('V') if command_modifier => {
                        view.overlay = None;
                        model.view = View::TaskCreate(view);
                        return (model, AppCommand::TaskCreatePasteImageFromClipboard);
                    }
                    KeyCode::Backspace => editor.backspace(),
                    KeyCode::Enter => {
                        let path = editor.text.trim().to_string();
                        if path.is_empty() {
                            view.overlay = None;
                            model.view = View::TaskCreate(view);
                            return (model, AppCommand::TaskCreatePasteImageFromClipboard);
                        }
                        view.overlay = Some(TaskCreateOverlay::ImagePath(editor));
                        model.view = View::TaskCreate(view);
                        return (
                            model,
                            AppCommand::TaskCreateInsertImage {
                                path: PathBuf::from(path),
                            },
                        );
                    }
                    KeyCode::Left => editor.move_left(),
                    KeyCode::Right => editor.move_right(),
                    KeyCode::Home => editor.move_home(),
                    KeyCode::End => editor.move_end(),
                    KeyCode::Delete => editor.delete_forward(),
                    KeyCode::Char(character) => {
                        if is_text_input_char(character) {
                            editor.insert_char(character);
                        }
                    }
                    _ => {}
                }
                view.overlay = Some(TaskCreateOverlay::ImagePath(editor));
            }
            TaskCreateOverlay::ProjectPath(mut editor) => {
                match key.code {
                    KeyCode::Esc => {
                        view.overlay = None;
                        model.view = View::TaskCreate(view);
                        return (model, AppCommand::None);
                    }
                    KeyCode::Backspace => editor.backspace(),
                    KeyCode::Enter => {
                        let path = editor.text.trim().to_string();
                        view.project_path = PathBuf::from(path);
                        view.overlay = None;
                        model.view = View::TaskCreate(view);
                        return (model, AppCommand::None);
                    }
                    KeyCode::Left => editor.move_left(),
                    KeyCode::Right => editor.move_right(),
                    KeyCode::Home => editor.move_home(),
                    KeyCode::End => editor.move_end(),
                    KeyCode::Delete => editor.delete_forward(),
                    KeyCode::Char(character) => {
                        if is_text_input_char(character) {
                            editor.insert_char(character);
                        }
                    }
                    _ => {}
                }
                view.overlay = Some(TaskCreateOverlay::ProjectPath(editor));
            }
        }

        model.view = View::TaskCreate(view);
        return (model, AppCommand::None);
    }

    match key.code {
        KeyCode::Esc => {
            model.view = View::Tasks(view.from_tasks.clone());
            return (model, AppCommand::None);
        }
        KeyCode::Char('v') | KeyCode::Char('V') if command_modifier => {
            model.view = View::TaskCreate(view);
            return (model, AppCommand::TaskCreatePasteImageFromClipboard);
        }
        KeyCode::Char('s') | KeyCode::Char('S') if command_modifier => {
            let body = view.editor.text();
            if body.trim().is_empty() {
                model.notice = Some("Task is empty.".to_string());
                model.view = View::TaskCreate(view);
                return (model, AppCommand::None);
            }
            if view.project_path.as_os_str().is_empty() {
                model.notice = Some("Project path is not set (Ctrl+P).".to_string());
                model.view = View::TaskCreate(view);
                return (model, AppCommand::None);
            }

            let from_tasks = view.from_tasks.clone();
            return (
                model,
                AppCommand::CreateTask {
                    from_tasks,
                    project_path: view.project_path.clone(),
                    body,
                    image_paths: view.image_paths.clone(),
                },
            );
        }
        KeyCode::Char('i') | KeyCode::Char('I') if command_modifier => {
            view.overlay = Some(TaskCreateOverlay::ImagePath(LineEditor::new()));
        }
        KeyCode::Char('p') | KeyCode::Char('P') if command_modifier => {
            let current = view.project_path.display().to_string();
            view.overlay = Some(TaskCreateOverlay::ProjectPath(LineEditor::from_text(
                current,
            )));
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

    model.view = View::TaskCreate(view);
    (model, AppCommand::None)
}

fn update_task_detail(
    mut model: AppModel,
    mut view: TaskDetailView,
    key: KeyEvent,
) -> (AppModel, AppCommand) {
    let send_modifier = key.modifiers.contains(KeyModifiers::CONTROL)
        || key.modifiers.contains(KeyModifiers::SUPER)
        || key.modifiers.contains(KeyModifiers::META);

    match key.code {
        KeyCode::F(3) => {
            let Some(project) = model
                .data
                .projects
                .iter()
                .find(|project| project.project_path == view.task.project_path)
            else {
                model.notice = Some("Project not indexed.".to_string());
                model.view = View::TaskDetail(view);
                return (model, AppCommand::None);
            };

            model.project_stats_overlay = Some(ProjectStatsOverlay::from_project(
                project,
                &model.session_index,
            ));
            model.help_open = false;
            model.system_menu = None;
            return (model, AppCommand::None);
        }
        KeyCode::Esc | KeyCode::Backspace => {
            model.view = View::Tasks(view.from_tasks.clone());
            return (model, AppCommand::None);
        }
        KeyCode::BackTab => {
            view.engine = view.engine.toggle();
        }
        KeyCode::Delete => {
            model.delete_task_confirm = Some(DeleteTaskConfirmDialog {
                task_id: view.task.id.clone(),
                task_title: crate::domain::derive_task_title(&view.task.body),
                project_path: view.task.project_path.clone(),
                selection: DeleteConfirmSelection::Cancel,
                return_to_tasks: view.from_tasks.clone(),
            });
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
        KeyCode::Enter if send_modifier => {
            return (
                model,
                AppCommand::SpawnTask {
                    engine: view.engine,
                    task_id: view.task.id.clone(),
                },
            );
        }
        _ => {}
    }

    model.view = View::TaskDetail(view);
    (model, AppCommand::None)
}

#[cfg(test)]
mod task_create_clipboard_tests {
    use super::*;

    fn dummy_project() -> ProjectSummary {
        ProjectSummary {
            name: "proj".to_string(),
            project_path: PathBuf::from("/tmp/proj"),
            sessions: Vec::new(),
            last_modified: None,
        }
    }

    fn task_create_model() -> AppModel {
        let data = AppData::from_scan(
            PathBuf::from("/tmp/sessions"),
            vec![dummy_project()],
            ScanWarningCount::from(0usize),
        );
        let mut model = AppModel::new(data);
        let return_to = Box::new(model.view.clone());
        let tasks_view = TasksView::new(return_to, Vec::new());
        model.view = View::TaskCreate(TaskCreateView::new(tasks_view, PathBuf::from("/tmp/proj")));
        model
    }

    #[test]
    fn ctrl_v_pastes_image_from_clipboard_in_task_create() {
        let model = task_create_model();
        let key = KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL);
        let (_next, cmd) = update(model, AppEvent::Key(key));
        assert!(matches!(cmd, AppCommand::TaskCreatePasteImageFromClipboard));
    }

    #[test]
    fn enter_on_empty_image_path_uses_clipboard() {
        let mut model = task_create_model();
        let View::TaskCreate(mut view) = model.view.clone() else {
            panic!("expected TaskCreate view");
        };
        view.overlay = Some(TaskCreateOverlay::ImagePath(LineEditor::new()));
        model.view = View::TaskCreate(view);

        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let (next, cmd) = update(model, AppEvent::Key(key));
        assert!(matches!(cmd, AppCommand::TaskCreatePasteImageFromClipboard));
        let View::TaskCreate(next_view) = next.view else {
            panic!("expected TaskCreate view");
        };
        assert!(next_view.overlay.is_none());
    }
}

#[cfg(test)]
mod multi_select_tests {
    use super::*;

    fn make_session(project_path: &str, id: &str, log_file: &str) -> SessionSummary {
        SessionSummary {
            meta: crate::domain::SessionMeta {
                id: id.to_string(),
                cwd: PathBuf::from(project_path),
                started_at_rfc3339: "2026-02-01T00:00:00Z".to_string(),
            },
            log_path: PathBuf::from(log_file),
            title: format!("session {id}"),
            file_size_bytes: 123,
            file_modified: None,
        }
    }

    fn projects_model() -> AppModel {
        let p1 = ProjectSummary {
            name: "p1".to_string(),
            project_path: PathBuf::from("/tmp/p1"),
            sessions: vec![make_session("/tmp/p1", "s1", "/tmp/sessions/p1-s1.jsonl")],
            last_modified: None,
        };
        let p2 = ProjectSummary {
            name: "p2".to_string(),
            project_path: PathBuf::from("/tmp/p2"),
            sessions: vec![make_session("/tmp/p2", "s2", "/tmp/sessions/p2-s2.jsonl")],
            last_modified: None,
        };
        let p3 = ProjectSummary {
            name: "p3".to_string(),
            project_path: PathBuf::from("/tmp/p3"),
            sessions: vec![make_session("/tmp/p3", "s3", "/tmp/sessions/p3-s3.jsonl")],
            last_modified: None,
        };

        let data = AppData::from_scan(
            PathBuf::from("/tmp/sessions"),
            vec![p1, p2, p3],
            ScanWarningCount::from(0usize),
        );
        AppModel::new(data)
    }

    #[test]
    fn shift_down_selects_range_in_projects() {
        let model = projects_model();

        let key = KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT);
        let (next, cmd) = update(model, AppEvent::Key(key));
        assert!(matches!(cmd, AppCommand::None));

        let View::Projects(view) = next.view else {
            panic!("expected Projects view");
        };
        assert_eq!(view.selected, 1);
        assert_eq!(view.selected_project_paths.len(), 2);
        assert!(
            view.selected_project_paths
                .contains(&PathBuf::from("/tmp/p1"))
        );
        assert!(
            view.selected_project_paths
                .contains(&PathBuf::from("/tmp/p2"))
        );
    }

    #[test]
    fn delete_opens_batch_confirm_for_selected_projects() {
        let model = projects_model();
        let key = KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT);
        let (model, _cmd) = update(model, AppEvent::Key(key));

        let key = KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE);
        let (next, cmd) = update(model, AppEvent::Key(key));
        assert!(matches!(cmd, AppCommand::None));
        assert!(next.delete_projects_confirm.is_some());

        let key = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
        let (_next, cmd) = update(next, AppEvent::Key(key));
        match cmd {
            AppCommand::DeleteProjectLogsBatch { project_paths } => {
                assert_eq!(project_paths.len(), 2);
            }
            _ => panic!("expected DeleteProjectLogsBatch"),
        }
    }

    #[test]
    fn delete_opens_batch_confirm_for_selected_sessions() {
        let mut model = projects_model();
        let project = model.data.projects.first().expect("project");
        model.view = View::Sessions(SessionsView::new(
            project.project_path.clone(),
            project.sessions.len(),
        ));

        // Seed a sessions view with at least two sessions.
        if let View::Sessions(view) = &mut model.view {
            view.filtered_indices = vec![0, 0];
        }

        // Better: build a project with 2 sessions.
        let p = ProjectSummary {
            name: "p".to_string(),
            project_path: PathBuf::from("/tmp/p"),
            sessions: vec![
                make_session("/tmp/p", "a", "/tmp/sessions/p-a.jsonl"),
                make_session("/tmp/p", "b", "/tmp/sessions/p-b.jsonl"),
            ],
            last_modified: None,
        };
        model.data.projects = vec![p.clone()];
        model.view = View::Sessions(SessionsView::new(p.project_path.clone(), p.sessions.len()));

        let key = KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT);
        let (model, _cmd) = update(model, AppEvent::Key(key));

        let key = KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE);
        let (next, cmd) = update(model, AppEvent::Key(key));
        assert!(matches!(cmd, AppCommand::None));
        assert!(next.delete_sessions_confirm.is_some());

        let key = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
        let (_next, cmd) = update(next, AppEvent::Key(key));
        match cmd {
            AppCommand::DeleteSessionLogsBatch { log_paths } => {
                assert_eq!(log_paths.len(), 2);
            }
            _ => panic!("expected DeleteSessionLogsBatch"),
        }
    }

    #[test]
    fn delete_opens_batch_confirm_for_selected_tasks() {
        let mut model = projects_model();
        let return_to = Box::new(model.view.clone());
        let tasks = vec![
            TaskSummaryRow {
                id: TaskId::new("t1".to_string()),
                title: "one".to_string(),
                project_path: PathBuf::from("/tmp/p1"),
                updated_at: SystemTime::UNIX_EPOCH,
                image_count: 0,
            },
            TaskSummaryRow {
                id: TaskId::new("t2".to_string()),
                title: "two".to_string(),
                project_path: PathBuf::from("/tmp/p1"),
                updated_at: SystemTime::UNIX_EPOCH,
                image_count: 0,
            },
        ];
        model.view = View::Tasks(TasksView::new(return_to, tasks));

        let key = KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT);
        let (model, _cmd) = update(model, AppEvent::Key(key));

        let key = KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE);
        let (next, cmd) = update(model, AppEvent::Key(key));
        assert!(matches!(cmd, AppCommand::None));
        assert!(next.delete_tasks_confirm.is_some());

        let key = KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE);
        let (_next, cmd) = update(next, AppEvent::Key(key));
        match cmd {
            AppCommand::DeleteTasksBatch { task_ids, .. } => {
                assert_eq!(task_ids.len(), 2);
            }
            _ => panic!("expected DeleteTasksBatch"),
        }
    }
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

fn page_step_session_detail_details(terminal_size: (u16, u16)) -> usize {
    let (inner_width, inner_height) = inner_terminal_size(terminal_size);
    let body_height = inner_height.saturating_sub(4); // header 3 + footer 1

    let details_height = if inner_width >= 90 {
        body_height
    } else {
        let body_height = u32::from(body_height);
        u16::try_from((body_height * 40) / 100).unwrap_or(0)
    };

    let viewport_height = details_height.saturating_sub(2);
    page_step_for_height(viewport_height)
}

pub fn build_index_from_sessions(
    sessions_dir: PathBuf,
    sessions: Vec<crate::domain::SessionSummary>,
    warnings: ScanWarningCount,
) -> AppData {
    let projects = index_projects(&sessions);
    AppData::from_scan(sessions_dir, projects, warnings)
}
