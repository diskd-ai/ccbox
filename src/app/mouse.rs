use super::{AppCommand, AppModel, SessionDetailFocus, SystemMenuOverlay, View};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use unicode_width::UnicodeWidthStr;

const SCROLL_STEP: usize = 3;

pub(super) fn update_on_mouse(model: AppModel, mouse: MouseEvent) -> (AppModel, AppCommand) {
    let mut model = model;
    if model.terminal_size.0 == 0 || model.terminal_size.1 == 0 {
        return (model, AppCommand::None);
    }

    match mouse.kind {
        MouseEventKind::ScrollUp => {
            model = apply_scroll(model, mouse.column, mouse.row, ScrollDirection::Up);
        }
        MouseEventKind::ScrollDown => {
            model = apply_scroll(model, mouse.column, mouse.row, ScrollDirection::Down);
        }
        MouseEventKind::Down(MouseButton::Left) => {
            model = apply_left_click(model, mouse.column, mouse.row);
        }
        _ => {}
    }

    (model, AppCommand::None)
}

#[derive(Clone, Copy, Debug)]
enum ScrollDirection {
    Up,
    Down,
}

fn apply_scroll(mut model: AppModel, col: u16, row: u16, direction: ScrollDirection) -> AppModel {
    if let Some(mut menu) = model.system_menu.take() {
        let menus = super::main_menus_for_view(&model.view);
        if menus.is_empty() {
            model.system_menu = None;
            return model;
        }
        menu.menu_index = menu.menu_index.min(menus.len().saturating_sub(1));
        let active = menus[menu.menu_index];
        let items = super::main_menu_items(active);
        if items.is_empty() {
            menu.item_index = 0;
        } else {
            match direction {
                ScrollDirection::Up => {
                    menu.item_index = menu.item_index.saturating_sub(1);
                }
                ScrollDirection::Down => {
                    menu.item_index = (menu.item_index + 1).min(items.len().saturating_sub(1));
                }
            }
        }
        model.system_menu = Some(menu);
        return model;
    }

    if let View::SessionDetail(mut view) = model.view.clone() {
        if view.output_overlay_open {
            let step = usize_to_u16(SCROLL_STEP);
            match direction {
                ScrollDirection::Up => {
                    view.output_overlay_scroll = view.output_overlay_scroll.saturating_sub(step);
                }
                ScrollDirection::Down => {
                    view.output_overlay_scroll = view.output_overlay_scroll.saturating_add(step);
                }
            }
            model.view = View::SessionDetail(view);
            return model;
        }
    }

    if let Some(mut preview) = model.session_result_preview.take() {
        let step = usize_to_u16(SCROLL_STEP);
        match direction {
            ScrollDirection::Up => {
                preview.scroll = preview.scroll.saturating_sub(step);
            }
            ScrollDirection::Down => {
                preview.scroll = preview.scroll.saturating_add(step);
            }
        }
        model.session_result_preview = Some(preview);
        return model;
    }

    if let Some(mut overlay) = model.session_stats_overlay.take() {
        let step = usize_to_u16(SCROLL_STEP);
        match direction {
            ScrollDirection::Up => {
                overlay.scroll = overlay.scroll.saturating_sub(step);
            }
            ScrollDirection::Down => {
                overlay.scroll = overlay.scroll.saturating_add(step);
            }
        }
        model.session_stats_overlay = Some(overlay);
        return model;
    }

    if let Some(mut overlay) = model.project_stats_overlay.take() {
        let step = usize_to_u16(SCROLL_STEP);
        match direction {
            ScrollDirection::Up => {
                overlay.scroll = overlay.scroll.saturating_sub(step);
            }
            ScrollDirection::Down => {
                overlay.scroll = overlay.scroll.saturating_add(step);
            }
        }
        model.project_stats_overlay = Some(overlay);
        return model;
    }

    match model.view.clone() {
        View::Projects(mut view) => {
            let total = view.filtered_indices.len();
            view.selected = scroll_index(view.selected, total, direction);
            model.view = View::Projects(view);
        }
        View::Sessions(mut view) => {
            let total = view.filtered_indices.len();
            view.session_selected = scroll_index(view.session_selected, total, direction);
            model.view = View::Sessions(view);
        }
        View::SessionDetail(mut view) => {
            let panels = session_detail_panels(model.terminal_size);
            let focus_from_mouse = panels.and_then(|panels| {
                if rect_contains(panels.timeline, col, row) {
                    Some(SessionDetailFocus::Timeline)
                } else if rect_contains(panels.details, col, row) {
                    Some(SessionDetailFocus::Details)
                } else {
                    None
                }
            });
            if let Some(focus) = focus_from_mouse {
                view.focus = focus;
            }

            match view.focus {
                SessionDetailFocus::Timeline => {
                    let total = view.items.len();
                    view.selected = scroll_index(view.selected, total, direction);
                    view.details_scroll = 0;
                }
                SessionDetailFocus::Details => {
                    let step = usize_to_u16(SCROLL_STEP);
                    match direction {
                        ScrollDirection::Up => {
                            view.details_scroll = view.details_scroll.saturating_sub(step);
                        }
                        ScrollDirection::Down => {
                            view.details_scroll = view.details_scroll.saturating_add(step);
                        }
                    }
                }
            }
            model.view = View::SessionDetail(view);
        }
        View::Tasks(mut view) => {
            let total = view.filtered_indices.len();
            view.selected = scroll_index(view.selected, total, direction);
            model.view = View::Tasks(view);
        }
        View::TaskDetail(mut view) => {
            let step = usize_to_u16(SCROLL_STEP);
            match direction {
                ScrollDirection::Up => {
                    view.scroll = view.scroll.saturating_sub(step);
                }
                ScrollDirection::Down => {
                    view.scroll = view.scroll.saturating_add(step);
                }
            }
            model.view = View::TaskDetail(view);
        }
        View::Processes(mut view) => {
            let total = model.processes.len();
            view.selected = scroll_index(view.selected, total, direction);
            model.view = View::Processes(view);
        }
        View::ProcessOutput(mut view) => {
            let step = usize_to_u16(SCROLL_STEP);
            match direction {
                ScrollDirection::Up => {
                    view.scroll = view.scroll.saturating_sub(step);
                }
                ScrollDirection::Down => {
                    view.scroll = view.scroll.saturating_add(step);
                }
            }
            model.view = View::ProcessOutput(view);
        }
        View::NewSession(_) | View::TaskCreate(_) | View::Error => {}
    }

    model
}

fn apply_left_click(mut model: AppModel, col: u16, row: u16) -> AppModel {
    if row == 0 {
        model = click_menu_bar(model, col);
        return model;
    }

    if let Some(menu) = model.system_menu.take() {
        model.system_menu =
            click_system_menu_popup(&model.view, model.terminal_size, menu, col, row);
        return model;
    }

    if model.help_open
        || model.delete_confirm.is_some()
        || model.delete_session_confirm.is_some()
        || model.delete_task_confirm.is_some()
        || model.session_result_preview.is_some()
        || model.session_stats_overlay.is_some()
        || model.project_stats_overlay.is_some()
    {
        return model;
    }

    match model.view.clone() {
        View::Projects(mut view) => {
            let list_area = standard_list_area(model.terminal_size);
            if let Some(selected) = hit_test_list_click(
                list_area,
                view.selected,
                view.filtered_indices.len(),
                col,
                row,
            ) {
                view.selected = selected;
                model.view = View::Projects(view);
            }
        }
        View::Sessions(mut view) => {
            let list_area = standard_list_area(model.terminal_size);
            if let Some(selected) = hit_test_list_click(
                list_area,
                view.session_selected,
                view.filtered_indices.len(),
                col,
                row,
            ) {
                view.session_selected = selected;
                model.view = View::Sessions(view);
            }
        }
        View::Tasks(mut view) => {
            let list_area = standard_list_area(model.terminal_size);
            if let Some(selected) = hit_test_list_click(
                list_area,
                view.selected,
                view.filtered_indices.len(),
                col,
                row,
            ) {
                view.selected = selected;
                model.view = View::Tasks(view);
            }
        }
        View::Processes(mut view) => {
            let list_area = standard_list_area(model.terminal_size);
            if let Some(selected) =
                hit_test_list_click(list_area, view.selected, model.processes.len(), col, row)
            {
                view.selected = selected;
                model.view = View::Processes(view);
            }
        }
        View::SessionDetail(mut view) => {
            if let Some(panels) = session_detail_panels(model.terminal_size) {
                if rect_contains(panels.timeline, col, row) {
                    view.focus = SessionDetailFocus::Timeline;
                    let timeline_list_area = panels.timeline;
                    if let Some(selected) = hit_test_list_click(
                        timeline_list_area,
                        view.selected,
                        view.items.len(),
                        col,
                        row,
                    ) {
                        view.selected = selected;
                        view.details_scroll = 0;
                    }
                    model.view = View::SessionDetail(view);
                } else if rect_contains(panels.details, col, row) {
                    view.focus = SessionDetailFocus::Details;
                    model.view = View::SessionDetail(view);
                }
            }
        }
        View::NewSession(_)
        | View::TaskCreate(_)
        | View::TaskDetail(_)
        | View::ProcessOutput(_)
        | View::Error => {}
    }

    model
}

fn click_menu_bar(mut model: AppModel, col: u16) -> AppModel {
    let menus = super::main_menus_for_view(&model.view);
    if menus.is_empty() {
        model.system_menu = None;
        return model;
    }

    let Some(menu_index) = hit_test_menu_bar(menus, col) else {
        model.system_menu = None;
        return model;
    };

    match model.system_menu.take() {
        Some(mut overlay) => {
            if overlay.menu_index == menu_index {
                model.system_menu = None;
            } else {
                overlay.menu_index = menu_index;
                overlay.item_index = 0;
                model.system_menu = Some(overlay);
            }
        }
        None => {
            model.system_menu = Some(SystemMenuOverlay {
                menu_index,
                item_index: 0,
            });
            model.help_open = false;
        }
    }

    model
}

fn click_system_menu_popup(
    view: &View,
    terminal_size: (u16, u16),
    mut menu: SystemMenuOverlay,
    col: u16,
    row: u16,
) -> Option<SystemMenuOverlay> {
    let menus = super::main_menus_for_view(view);
    if menus.is_empty() {
        return None;
    }
    menu.menu_index = menu.menu_index.min(menus.len().saturating_sub(1));

    let area = content_area(terminal_size);
    let active = menus[menu.menu_index];
    let items = super::main_menu_items(active);
    let popup = system_menu_popup_rect(area, menus, menu.menu_index, active.label(), items);

    if !rect_contains(popup, col, row) {
        return None;
    }

    let inner = popup_inner_rect(popup);
    if rect_contains(inner, col, row) {
        let index = (row - inner.y) as usize;
        if index < items.len() {
            menu.item_index = index;
        }
    }

    Some(menu)
}

fn hit_test_menu_bar(menus: &[super::MainMenu], col: u16) -> Option<usize> {
    let col = col as usize;
    let mut x_offset = UnicodeWidthStr::width("  ");
    for (idx, menu) in menus.iter().enumerate() {
        let label = format!(" {} ", menu.label());
        let width = UnicodeWidthStr::width(label.as_str());
        if col >= x_offset && col < x_offset.saturating_add(width) {
            return Some(idx);
        }
        x_offset = x_offset.saturating_add(width).saturating_add(1);
    }
    None
}

fn standard_list_area(terminal_size: (u16, u16)) -> Rect {
    let content = content_area(terminal_size);
    let area = inner_area(content);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);
    chunks[1]
}

fn session_detail_panels(terminal_size: (u16, u16)) -> Option<SessionDetailPanels> {
    let content = content_area(terminal_size);
    let area = inner_area(content);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    let body = chunks.get(1).copied()?;
    if body.width == 0 || body.height == 0 {
        return None;
    }

    let panels = if body.width >= 90 {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(body)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(body)
    };

    Some(SessionDetailPanels {
        timeline: panels[0],
        details: panels[1],
    })
}

#[derive(Clone, Copy, Debug)]
struct SessionDetailPanels {
    timeline: Rect,
    details: Rect,
}

fn hit_test_list_click(
    list_area: Rect,
    selected: usize,
    total: usize,
    col: u16,
    row: u16,
) -> Option<usize> {
    let _ = col;
    if total == 0 {
        return None;
    }
    if list_area.width == 0 || list_area.height < 3 {
        return None;
    }

    let inner_y = list_area.y.saturating_add(1);
    let inner_height = list_area.height.saturating_sub(2) as usize;
    if inner_height == 0 {
        return None;
    }
    if row < inner_y
        || row
            >= list_area
                .y
                .saturating_add(list_area.height)
                .saturating_sub(1)
    {
        return None;
    }
    let clicked_row = (row - inner_y) as usize;
    if clicked_row >= inner_height {
        return None;
    }

    let offset = list_offset(selected, inner_height, total);
    let index = offset.saturating_add(clicked_row);
    if index >= total { None } else { Some(index) }
}

fn list_offset(selected: usize, viewport_height: usize, total: usize) -> usize {
    if viewport_height == 0 || total <= viewport_height {
        return 0;
    }
    let max_offset = total.saturating_sub(viewport_height);
    let raw_offset = selected.saturating_add(1).saturating_sub(viewport_height);
    raw_offset.min(max_offset)
}

fn scroll_index(selected: usize, total: usize, direction: ScrollDirection) -> usize {
    if total == 0 {
        return 0;
    }

    match direction {
        ScrollDirection::Up => selected.saturating_sub(SCROLL_STEP),
        ScrollDirection::Down => selected
            .saturating_add(SCROLL_STEP)
            .min(total.saturating_sub(1)),
    }
}

fn content_area(terminal_size: (u16, u16)) -> Rect {
    let (width, height) = terminal_size;
    let full = Rect {
        x: 0,
        y: 0,
        width,
        height,
    };

    if full.height > 1 {
        Rect {
            x: full.x,
            y: full.y.saturating_add(1),
            width: full.width,
            height: full.height.saturating_sub(1),
        }
    } else {
        full
    }
}

fn inner_area(area: Rect) -> Rect {
    if area.width < 40 || area.height < 12 {
        return area;
    }
    area.inner(ratatui::layout::Margin {
        vertical: 0,
        horizontal: 2,
    })
}

fn system_menu_popup_rect(
    area: Rect,
    menus: &[super::MainMenu],
    menu_index: usize,
    title: &str,
    items: &[super::MainMenuEntry],
) -> Rect {
    let max_label_width = items
        .iter()
        .map(|item| UnicodeWidthStr::width(item.label))
        .max()
        .unwrap_or(0);
    let max_hotkey_width = items
        .iter()
        .map(|item| UnicodeWidthStr::width(item.hotkey))
        .max()
        .unwrap_or(0);
    let title_width = UnicodeWidthStr::width(title);

    let inner_width = max_label_width
        .saturating_add(2)
        .saturating_add(max_hotkey_width)
        .max(title_width)
        .max(18);
    let desired_width = inner_width.saturating_add(4);

    let popup_width = (desired_width as u16).min(area.width);
    let popup_height = (items.len() as u16).saturating_add(4).min(area.height);

    let mut x_offset = UnicodeWidthStr::width("  ");
    for menu in menus.iter().take(menu_index) {
        let label = format!(" {} ", menu.label());
        x_offset = x_offset.saturating_add(UnicodeWidthStr::width(label.as_str()));
        x_offset = x_offset.saturating_add(1);
    }

    let max_x = area
        .x
        .saturating_add(area.width.saturating_sub(popup_width));
    let popup_x = area.x.saturating_add(x_offset as u16).min(max_x);
    Rect {
        x: popup_x,
        y: area.y,
        width: popup_width,
        height: popup_height,
    }
}

fn popup_inner_rect(popup: Rect) -> Rect {
    if popup.width < 4 || popup.height < 4 {
        return Rect {
            x: popup.x,
            y: popup.y,
            width: 0,
            height: 0,
        };
    }

    Rect {
        x: popup.x.saturating_add(2),
        y: popup.y.saturating_add(2),
        width: popup.width.saturating_sub(4),
        height: popup.height.saturating_sub(4),
    }
}

fn rect_contains(area: Rect, col: u16, row: u16) -> bool {
    col >= area.x
        && col < area.x.saturating_add(area.width)
        && row >= area.y
        && row < area.y.saturating_add(area.height)
}

fn usize_to_u16(value: usize) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppData, AppEvent, AppModel, ProjectsView};
    use crate::domain::ProjectSummary;
    use crate::infra::ScanWarningCount;
    use crossterm::event::KeyModifiers;
    use std::path::PathBuf;

    fn dummy_projects(count: usize) -> Vec<ProjectSummary> {
        (0..count)
            .map(|idx| ProjectSummary {
                name: format!("project-{idx}"),
                project_path: PathBuf::from(format!("/tmp/project-{idx}")),
                sessions: Vec::new(),
                last_modified: None,
            })
            .collect()
    }

    #[test]
    fn click_menu_bar_opens_window_menu() {
        let data = AppData::from_scan(
            PathBuf::from("/tmp/sessions"),
            dummy_projects(3),
            ScanWarningCount::from(0usize),
        );
        let mut model = AppModel::new(data);
        model.terminal_size = (120, 30);
        model.view = View::Projects(ProjectsView::new(&model.data.projects));

        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 12,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };

        let (next, cmd) = crate::app::update(model, AppEvent::Mouse(mouse));
        assert!(matches!(cmd, AppCommand::None));
        let menu = next.system_menu.expect("menu should open");
        assert_eq!(menu.menu_index, 1);
    }

    #[test]
    fn click_projects_list_selects_row() {
        let data = AppData::from_scan(
            PathBuf::from("/tmp/sessions"),
            dummy_projects(10),
            ScanWarningCount::from(0usize),
        );
        let mut model = AppModel::new(data);
        model.terminal_size = (120, 30);
        model.view = View::Projects(ProjectsView::new(&model.data.projects));

        let list_area = standard_list_area(model.terminal_size);
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: list_area.x.saturating_add(2),
            row: list_area.y.saturating_add(1).saturating_add(2),
            modifiers: KeyModifiers::NONE,
        };

        let (next, _cmd) = crate::app::update(model, AppEvent::Mouse(mouse));
        let View::Projects(view) = next.view else {
            panic!("expected projects view");
        };
        assert_eq!(view.selected, 2);
    }

    #[test]
    fn scroll_session_result_preview_overlay() {
        let data = AppData::from_scan(
            PathBuf::from("/tmp/sessions"),
            dummy_projects(1),
            ScanWarningCount::from(0usize),
        );
        let mut model = AppModel::new(data);
        model.terminal_size = (120, 30);
        model.session_result_preview = Some(crate::app::SessionResultPreviewOverlay {
            session_title: "session".to_string(),
            output: "hello\nworld".to_string(),
            scroll: 0,
        });

        let mouse = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 10,
            row: 10,
            modifiers: KeyModifiers::NONE,
        };

        let (next, _cmd) = crate::app::update(model, AppEvent::Mouse(mouse));
        assert_eq!(next.session_result_preview.unwrap().scroll, 3);
    }
}
