use crate::app::{AppModel, DeleteConfirmSelection, SessionDetailFocus, View};
use crate::domain::{TimelineItem, TimelineItemKind, TurnContextSummary};
use humansize::{DECIMAL, format_size};
use ratatui::prelude::*;
use ratatui::widgets::block::Title;
use ratatui::widgets::*;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::time::{Duration, SystemTime};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use unicode_width::UnicodeWidthChar;
use unicode_width::UnicodeWidthStr;

fn render_json_pretty_highlight_lines(text: &str) -> Vec<Line<'static>> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return vec![Line::from("")];
    }

    const JSON_PRETTY_PARSE_LIMIT: usize = 200_000;
    if trimmed.len() > JSON_PRETTY_PARSE_LIMIT {
        return render_json_highlight_lines(text);
    }

    match serde_json::from_str::<serde_json::Value>(trimmed)
        .ok()
        .and_then(|value| serde_json::to_string_pretty(&value).ok())
    {
        Some(pretty) => render_json_highlight_lines(&pretty),
        None => render_json_highlight_lines(text),
    }
}

pub fn render(frame: &mut Frame, model: &AppModel) {
    let full_area = frame.area();
    if full_area.width == 0 || full_area.height == 0 {
        return;
    }

    render_menu_bar(frame, full_area, model);

    let content_area = if full_area.height > 1 {
        Rect {
            x: full_area.x,
            y: full_area.y.saturating_add(1),
            width: full_area.width,
            height: full_area.height.saturating_sub(1),
        }
    } else {
        full_area
    };

    match &model.view {
        View::Projects(projects_view) => render_projects(frame, content_area, model, projects_view),
        View::Sessions(sessions_view) => render_sessions(frame, content_area, model, sessions_view),
        View::NewSession(new_session_view) => {
            render_new_session(frame, content_area, model, new_session_view)
        }
        View::SessionDetail(detail_view) => {
            render_session_detail(frame, content_area, model, detail_view)
        }
        View::Processes(processes_view) => {
            render_processes(frame, content_area, model, processes_view)
        }
        View::ProcessOutput(output_view) => {
            render_process_output(frame, content_area, model, output_view)
        }
        View::Error => render_error(frame, content_area, model),
    }

    if let Some(menu) = &model.system_menu {
        render_main_menu_overlay(frame, content_area, model, menu);
    }

    if model.help_open {
        render_help_overlay(frame, content_area);
    }

    if let Some(confirm) = &model.delete_confirm {
        render_delete_confirm_overlay(frame, content_area, model, confirm);
    }

    if let Some(confirm) = &model.delete_session_confirm {
        render_delete_session_confirm_overlay(frame, content_area, model, confirm);
    }

    if let Some(preview) = &model.session_result_preview {
        render_session_result_preview_overlay(frame, content_area, preview);
    }

    if let Some(overlay) = &model.session_stats_overlay {
        render_session_stats_overlay(frame, content_area, overlay);
    }

    if let Some(overlay) = &model.project_stats_overlay {
        render_project_stats_overlay(frame, content_area, overlay);
    }
}

fn render_menu_bar(frame: &mut Frame, area: Rect, model: &AppModel) {
    let bar_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };

    let bg = Color::DarkGray;
    let base_style = Style::default().fg(Color::White).bg(bg);
    let hint_label_style = Style::default().fg(Color::Gray).bg(bg);
    let hint_key_style = Style::default()
        .fg(Color::LightBlue)
        .bg(bg)
        .add_modifier(Modifier::BOLD);
    let menu_open_index = model.system_menu.as_ref().map(|menu| menu.menu_index);
    let menus = crate::app::main_menus_for_view(&model.view);
    let active_style = Style::default()
        .fg(Color::Black)
        .bg(Color::White)
        .add_modifier(Modifier::BOLD);
    let inactive_style = Style::default()
        .fg(Color::White)
        .bg(bg)
        .add_modifier(Modifier::BOLD);

    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(Span::styled("  ".to_string(), base_style));

    for (idx, menu) in menus.iter().enumerate() {
        let label = format!(" {} ", menu.label());
        let style = if menu_open_index == Some(idx) {
            active_style
        } else {
            inactive_style
        };
        spans.push(Span::styled(label, style));
        spans.push(Span::styled(" ".to_string(), base_style));
    }

    let left_width: usize = spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum();
    let hint_spans = vec![
        Span::styled("F1", hint_key_style),
        Span::styled(" Help", hint_label_style),
        Span::styled("  ", base_style),
        Span::styled("F2", hint_key_style),
        Span::styled(" System", hint_label_style),
        Span::styled("  ", base_style),
        Span::styled("F3", hint_key_style),
        Span::styled(" Statistics", hint_label_style),
    ];
    let hint_width: usize = hint_spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum();
    let width = bar_area.width as usize;

    if width > left_width + hint_width {
        spans.push(Span::styled(
            " ".repeat(width - left_width - hint_width),
            base_style,
        ));
        spans.extend(hint_spans);
    } else if width > left_width {
        spans.push(Span::styled(" ".repeat(width - left_width), base_style));
    }

    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(base_style),
        bar_area,
    );
}

fn render_main_menu_overlay(
    frame: &mut Frame,
    area: Rect,
    model: &AppModel,
    menu: &crate::app::SystemMenuOverlay,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let menus = crate::app::main_menus_for_view(&model.view);
    if menus.is_empty() {
        return;
    }

    let menu_index = menu.menu_index.min(menus.len().saturating_sub(1));
    let active = menus[menu_index];
    let items = crate::app::main_menu_items(active);
    let title = active.label();

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
    let popup_height = (items.len() as u16).saturating_add(2).min(area.height);

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
    let popup = Rect {
        x: popup_x,
        y: area.y,
        width: popup_width,
        height: popup_height,
    };

    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .padding(Padding::horizontal(1))
        .title(title);

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let mut list_items: Vec<ListItem> = Vec::new();
    for item in items {
        let label = item.label;
        let hotkey = item.hotkey;

        let label_width = UnicodeWidthStr::width(label);
        let hotkey_width = UnicodeWidthStr::width(hotkey);
        let gap = 2usize;

        let inner_width = inner.width as usize;
        if inner_width == 0 {
            list_items.push(ListItem::new(Line::from("")));
            continue;
        }

        let min_needed = label_width.saturating_add(gap).saturating_add(hotkey_width);
        if inner_width <= min_needed {
            let text = truncate_end(&format!("{label} {hotkey}"), inner_width);
            list_items.push(ListItem::new(Line::from(text)));
            continue;
        }

        let padding = inner_width.saturating_sub(label_width + gap + hotkey_width);
        list_items.push(ListItem::new(Line::from(vec![
            Span::raw(label.to_string()),
            Span::raw(" ".repeat(gap + padding)),
            Span::styled(
                hotkey.to_string(),
                Style::default()
                    .fg(Color::LightBlue)
                    .add_modifier(Modifier::BOLD),
            ),
        ])));
    }

    let list = List::new(list_items)
        .highlight_style(
            Style::default()
                .add_modifier(Modifier::REVERSED)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("");

    let mut state = ListState::default();
    state.select(Some(menu.item_index.min(items.len().saturating_sub(1))));
    frame.render_stateful_widget(list, inner, &mut state);
}

fn render_error(frame: &mut Frame, area: Rect, model: &AppModel) {
    let area = inner_area(area);
    let title = "ccbox";
    let error_text = model
        .data
        .load_error
        .clone()
        .unwrap_or_else(|| "unknown error".to_string());

    let paragraph = Paragraph::new(vec![
        Line::from("Failed to load sessions."),
        Line::from(""),
        Line::from(format!(
            "Resolved sessions dir: {}",
            model.data.sessions_dir.display()
        )),
        Line::from(""),
        Line::from(format!("Error: {error_text}")),
        Line::from(""),
        Line::from("Keys: Esc=back  Ctrl+R=rescan  Ctrl+Q/Ctrl+C=quit  F1/?=help"),
    ])
    .block(
        Block::default()
            .title(title)
            .borders(Borders::ALL)
            .padding(Padding::horizontal(1)),
    );

    frame.render_widget(paragraph, area);
}

fn render_projects(
    frame: &mut Frame,
    area: Rect,
    model: &AppModel,
    projects_view: &crate::app::ProjectsView,
) {
    let area = inner_area(area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    let search_text = if projects_view.query.is_empty() {
        Text::from(Line::from(Span::styled(
            "Type to filter projects…",
            Style::default().fg(Color::DarkGray),
        )))
    } else {
        Text::from(projects_view.query.as_str())
    };
    let search = Paragraph::new(search_text).block(
        Block::default()
            .borders(Borders::ALL)
            .padding(Padding::horizontal(1))
            .title("Find Projects"),
    );
    frame.render_widget(search, chunks[0]);

    let projects = &model.data.projects;
    let filtered_indices = &projects_view.filtered_indices;

    if filtered_indices.is_empty() {
        let message = if projects_view.query.trim().is_empty() {
            "No projects found."
        } else {
            "No matching projects. Press Esc to clear the filter."
        };
        let empty = Paragraph::new(message).block(
            Block::default()
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1))
                .title("Recent Projects"),
        );
        frame.render_widget(empty, chunks[1]);
    } else {
        let list_area = chunks[1];
        let max_width = (list_area.width as usize).saturating_sub(6);
        let (sessions_col_width, modified_col_width) =
            project_right_columns_width(projects, filtered_indices);
        let list_items: Vec<ListItem> = filtered_indices
            .iter()
            .copied()
            .filter_map(|project_index| {
                projects.get(project_index).map(|project| {
                    project_list_item(
                        project,
                        max_width,
                        sessions_col_width,
                        modified_col_width,
                        &projects_view.query,
                    )
                })
            })
            .collect();

        let list = List::new(list_items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .padding(Padding::horizontal(1))
                    .title("Recent Projects"),
            )
            .highlight_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▸ ");

        let mut state = ListState::default();
        state.select(Some(
            projects_view
                .selected
                .min(filtered_indices.len().saturating_sub(1)),
        ));
        frame.render_stateful_widget(list, list_area, &mut state);
    }

    let footer = projects_footer_line(
        model.data.warnings.get(),
        model.notice.as_deref(),
        model.update_hint.as_deref(),
        processes_running(model),
    );
    frame.render_widget(footer, chunks[2]);
}

fn render_sessions(
    frame: &mut Frame,
    area: Rect,
    model: &AppModel,
    sessions_view: &crate::app::SessionsView,
) {
    let area = inner_area(area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    let current_project = sessions_view.current_project(&model.data.projects);
    let search_title = current_project
        .map(|project| {
            format!(
                "Find Sessions · {} ({})",
                project.name,
                project.project_path.display()
            )
        })
        .unwrap_or_else(|| "Find Sessions".to_string());
    let title_budget = (chunks[0].width as usize).saturating_sub(4);
    let search_title = truncate_end(&search_title, title_budget);
    let search_text = if sessions_view.query.is_empty() {
        Text::from(Line::from(Span::styled(
            "Type to filter sessions…",
            Style::default().fg(Color::DarkGray),
        )))
    } else {
        Text::from(sessions_view.query.as_str())
    };
    let search = Paragraph::new(search_text).block(
        Block::default()
            .borders(Borders::ALL)
            .padding(Padding::horizontal(1))
            .title(search_title),
    );
    frame.render_widget(search, chunks[0]);

    let Some(project) = current_project else {
        let paragraph = Paragraph::new("Project no longer exists in index. Press Esc to go back.")
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .padding(Padding::horizontal(1)),
            );
        frame.render_widget(paragraph, chunks[1]);
        frame.render_widget(
            sessions_footer_line(
                model.data.warnings.get(),
                model.notice.as_deref(),
                model.update_hint.as_deref(),
                processes_running(model),
            ),
            chunks[2],
        );
        return;
    };

    let filtered_indices = &sessions_view.filtered_indices;
    let list_title = if sessions_view.query.trim().is_empty() {
        format!("Sessions · {} total · newest first", project.sessions.len())
    } else {
        format!(
            "Sessions · {}/{} shown · newest first",
            filtered_indices.len(),
            project.sessions.len()
        )
    };
    let list_title_budget = (chunks[1].width as usize).saturating_sub(4);
    let list_title = truncate_end(&list_title, list_title_budget);

    if filtered_indices.is_empty() {
        let message = if sessions_view.query.trim().is_empty() {
            "No sessions found."
        } else {
            "No matching sessions. Press Esc to clear the filter."
        };
        let empty = Paragraph::new(message).block(
            Block::default()
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1))
                .title(list_title),
        );
        frame.render_widget(empty, chunks[1]);
    } else {
        let list_area = chunks[1];
        let max_width = (list_area.width as usize).saturating_sub(6);
        let (size_col_width, modified_col_width) =
            session_right_columns_width_filtered(&project.sessions, filtered_indices);
        let items: Vec<ListItem> = filtered_indices
            .iter()
            .copied()
            .filter_map(|index| {
                project.sessions.get(index).map(|session| {
                    session_list_item(
                        session,
                        max_width,
                        size_col_width,
                        modified_col_width,
                        &sessions_view.query,
                    )
                })
            })
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .padding(Padding::horizontal(1))
                    .title(list_title),
            )
            .highlight_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▸ ");

        let mut state = ListState::default();
        state.select(Some(
            sessions_view
                .session_selected
                .min(filtered_indices.len().saturating_sub(1)),
        ));
        frame.render_stateful_widget(list, list_area, &mut state);
    }

    frame.render_widget(
        sessions_footer_line(
            model.data.warnings.get(),
            model.notice.as_deref(),
            model.update_hint.as_deref(),
            processes_running(model),
        ),
        chunks[2],
    );
}

fn render_new_session(
    frame: &mut Frame,
    area: Rect,
    model: &AppModel,
    new_session_view: &crate::app::NewSessionView,
) {
    let area = inner_area(area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    let project = model
        .data
        .projects
        .iter()
        .find(|project| project.project_path == new_session_view.from_sessions.project_path);
    let title = match project {
        Some(project) => format!(
            "New Session · {} ({})",
            project.name,
            project.project_path.display()
        ),
        None => format!(
            "New Session · {}",
            new_session_view.from_sessions.project_path.display()
        ),
    };
    let header_hint = "Write a prompt, then press Ctrl+Enter (or Cmd+Enter if supported) to send.";
    let header = Paragraph::new(truncate_end(
        header_hint,
        (chunks[0].width as usize).saturating_sub(4),
    ))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .padding(Padding::horizontal(1))
            .title(title),
    );
    frame.render_widget(header, chunks[0]);

    let editor_area = chunks[1];
    let editor_block = Block::default()
        .borders(Borders::ALL)
        .padding(Padding::horizontal(1))
        .title("Prompt");
    let editor_inner = editor_block.inner(editor_area);
    frame.render_widget(editor_block, editor_area);

    if editor_inner.width > 0 && editor_inner.height > 0 {
        let visible_height = editor_inner.height as usize;
        let cursor_row = new_session_view.editor.cursor_row;
        let scroll_row = cursor_row.saturating_sub(visible_height.saturating_sub(1));

        let mut lines = Vec::new();
        for offset in 0..visible_height {
            let index = scroll_row + offset;
            match new_session_view.editor.lines.get(index) {
                Some(line) => lines.push(Line::from(line.clone())),
                None => lines.push(Line::from("")),
            }
        }

        if new_session_view.editor.lines.len() == 1 && new_session_view.editor.lines[0].is_empty() {
            lines.clear();
            lines.push(Line::from(Span::styled(
                "Type or paste a prompt…",
                Style::default().fg(Color::DarkGray),
            )));
        }

        let paragraph = Paragraph::new(lines);
        frame.render_widget(paragraph, editor_inner);

        let cursor_line = new_session_view
            .editor
            .lines
            .get(cursor_row)
            .map(|line| line.as_str())
            .unwrap_or("");
        let cursor_col = new_session_view.editor.cursor_col;
        let cursor_y = cursor_row.saturating_sub(scroll_row);
        if cursor_y < visible_height {
            let mut x_offset = 0u16;
            for (idx, ch) in cursor_line.chars().enumerate() {
                if idx >= cursor_col {
                    break;
                }
                x_offset = x_offset.saturating_add(UnicodeWidthChar::width(ch).unwrap_or(0) as u16);
            }

            let x = editor_inner.x.saturating_add(x_offset).min(
                editor_inner
                    .x
                    .saturating_add(editor_inner.width.saturating_sub(1)),
            );
            let y = editor_inner.y.saturating_add(cursor_y as u16);
            frame.set_cursor_position(Position { x, y });
        }
    }

    let footer_text = "Keys: edit text  Ctrl+Enter/Cmd+Enter=send  Esc=cancel  Ctrl+R=rescan  Ctrl+Q/Ctrl+C=quit  F1/?=help";
    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(Span::raw(footer_text.to_string()));
    if let Some(notice) = model.notice.as_deref() {
        if !notice.trim().is_empty() {
            spans.push(Span::raw(format!("  ·  {notice}")));
        }
    }
    if let Some(hint) = model.update_hint.as_deref() {
        if !hint.trim().is_empty() {
            spans.push(Span::raw("  ·  "));
            spans.push(Span::styled(
                hint.to_string(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ));
        }
    }
    spans.push(Span::raw("  ·  "));
    spans.push(Span::styled(
        format!("Engine: {}", new_session_view.engine.label()),
        Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(
        " (Shift+Tab)".to_string(),
        Style::default().fg(Color::Blue),
    ));
    if processes_running(model) {
        spans.push(Span::raw("  ·  "));
        spans.push(Span::styled(
            "P●".to_string(),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ));
    }
    let footer = Paragraph::new(Line::from(spans)).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(footer, chunks[2]);
}

fn render_processes(
    frame: &mut Frame,
    area: Rect,
    model: &AppModel,
    processes_view: &crate::app::ProcessesView,
) {
    let area = inner_area(area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    let running_count = model
        .processes
        .iter()
        .filter(|process| process.status.is_running())
        .count();
    let header_hint = format!(
        "{} process(es)  ·  running: {}",
        model.processes.len(),
        running_count
    );
    let header = Paragraph::new(truncate_end(
        &header_hint,
        (chunks[0].width as usize).saturating_sub(4),
    ))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .padding(Padding::horizontal(1))
            .title("Processes"),
    );
    frame.render_widget(header, chunks[0]);

    let list_area = chunks[1];
    let max_width = (list_area.width as usize).saturating_sub(6);
    let (status_col_width, started_col_width) = process_right_columns_width(&model.processes);
    let items: Vec<ListItem> = model
        .processes
        .iter()
        .map(|process| process_list_item(process, max_width, status_col_width, started_col_width))
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1))
                .title("Spawned"),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    let mut state = ListState::default();
    if !model.processes.is_empty() {
        state.select(Some(
            processes_view
                .selected
                .min(model.processes.len().saturating_sub(1)),
        ));
    }
    frame.render_stateful_widget(list, list_area, &mut state);

    let footer_text = "Keys: arrows=move  Enter=session  s=stdout  e=stderr  l=log  k=kill  Esc/Backspace=back  Ctrl+Q/Ctrl+C=quit  F1/?=help";
    frame.render_widget(
        footer_paragraph(
            footer_text.to_string(),
            model.notice.as_deref(),
            model.update_hint.as_deref(),
            processes_running(model),
        ),
        chunks[2],
    );
}

fn render_process_output(
    frame: &mut Frame,
    area: Rect,
    model: &AppModel,
    output_view: &crate::app::ProcessOutputView,
) {
    let area = inner_area(area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    let process = model
        .processes
        .iter()
        .find(|process| process.id == output_view.process_id);
    let title = match process {
        Some(process) => format!(
            "Process · {} · {} · pid {} · {}",
            process.id,
            process.engine.label(),
            process.pid,
            output_view.kind.label()
        ),
        None => format!(
            "Process · {} · {}",
            output_view.process_id,
            output_view.kind.label()
        ),
    };
    let file_name = output_view
        .file_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| output_view.file_path.display().to_string());
    let header_hint = format!("file: {file_name}");
    let header = Paragraph::new(truncate_end(
        &header_hint,
        (chunks[0].width as usize).saturating_sub(4),
    ))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .padding(Padding::horizontal(1))
            .title(title),
    );
    frame.render_widget(header, chunks[0]);

    let body = Paragraph::new(output_view.buffer.as_str())
        .wrap(Wrap { trim: false })
        .scroll((output_view.scroll, 0))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1))
                .title("Output"),
        );
    frame.render_widget(body, chunks[1]);

    let footer_text = "Keys: arrows=scroll  s=stdout  e=stderr  l=log  k=kill  Esc/Backspace=back  Ctrl+Q/Ctrl+C=quit  F1/?=help";
    frame.render_widget(
        footer_paragraph(
            footer_text.to_string(),
            model.notice.as_deref(),
            model.update_hint.as_deref(),
            processes_running(model),
        ),
        chunks[2],
    );
}

fn projects_footer_line(
    warnings: usize,
    notice: Option<&str>,
    update_hint: Option<&str>,
    processes_running: bool,
) -> Paragraph<'static> {
    let text = if warnings == 0 {
        "Keys: arrows=move  PgUp/PgDn=page  Enter=open  Space=result  Del=delete  Esc=clear  Ctrl+R=rescan  Ctrl+Q/Ctrl+C=quit  F1/?=help"
            .to_string()
    } else {
        format!(
            "Keys: arrows=move  PgUp/PgDn=page  Enter=open  Space=result  Del=delete  Esc=clear  Ctrl+R=rescan  Ctrl+Q/Ctrl+C=quit  F1/?=help  ·  warnings: {warnings}"
        )
    };
    footer_paragraph(text, notice, update_hint, processes_running)
}

fn sessions_footer_line(
    warnings: usize,
    notice: Option<&str>,
    update_hint: Option<&str>,
    processes_running: bool,
) -> Paragraph<'static> {
    let text = if warnings == 0 {
        "Keys: type=filter  arrows=move  PgUp/PgDn=page  Enter=open  Space=result  F3=stats  Ctrl+N/Cmd+N=new  Del=delete  Esc=clear/back  Ctrl+R=rescan  Ctrl+Q/Ctrl+C=quit  F1/?=help"
            .to_string()
    } else {
        format!(
            "Keys: type=filter  arrows=move  PgUp/PgDn=page  Enter=open  Space=result  F3=stats  Ctrl+N/Cmd+N=new  Del=delete  Esc=clear/back  Ctrl+R=rescan  Ctrl+Q/Ctrl+C=quit  F1/?=help  ·  warnings: {warnings}"
        )
    };
    footer_paragraph(text, notice, update_hint, processes_running)
}

fn footer_paragraph(
    base: String,
    notice: Option<&str>,
    update_hint: Option<&str>,
    processes_running: bool,
) -> Paragraph<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    spans.push(Span::raw(base));

    if let Some(message) = notice {
        if !message.trim().is_empty() {
            spans.push(Span::raw("  ·  "));
            spans.push(Span::raw(message.to_string()));
        }
    }

    if let Some(hint) = update_hint {
        if !hint.trim().is_empty() {
            spans.push(Span::raw("  ·  "));
            spans.push(Span::styled(
                hint.to_string(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::raw("  ·  "));
            spans.push(Span::raw(format!("v{}", env!("CARGO_PKG_VERSION"))));
        }
    } else {
        spans.push(Span::raw("  ·  "));
        spans.push(Span::raw(format!("v{}", env!("CARGO_PKG_VERSION"))));
    }
    if processes_running {
        spans.push(Span::raw("  ·  "));
        spans.push(Span::styled(
            "P●".to_string(),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ));
    }

    Paragraph::new(Line::from(spans)).style(Style::default().fg(Color::DarkGray))
}

fn processes_running(model: &AppModel) -> bool {
    model
        .processes
        .iter()
        .any(|process| process.status.is_running())
}

fn project_right_columns_width(
    projects: &[crate::domain::ProjectSummary],
    indices: &[usize],
) -> (usize, usize) {
    let mut sessions_col_width = 0usize;
    let mut modified_col_width = 0usize;

    for project_index in indices {
        let Some(project) = projects.get(*project_index) else {
            continue;
        };

        let sessions_count = project.sessions.len();
        let session_word = if sessions_count == 1 {
            "session"
        } else {
            "sessions"
        };
        let sessions_col = format!("{sessions_count} {session_word}");
        sessions_col_width = sessions_col_width.max(UnicodeWidthStr::width(sessions_col.as_str()));

        let modified = if project.last_modified.is_some() {
            relative_time_ago(project.last_modified)
        } else {
            "-".to_string()
        };
        modified_col_width = modified_col_width.max(UnicodeWidthStr::width(modified.as_str()));
    }

    (sessions_col_width, modified_col_width)
}

fn session_right_columns_width_filtered(
    sessions: &[crate::domain::SessionSummary],
    indices: &[usize],
) -> (usize, usize) {
    let mut size_col_width = 0usize;
    let mut modified_col_width = 0usize;

    for session_index in indices {
        let Some(session) = sessions.get(*session_index) else {
            continue;
        };

        let size = format_size(session.file_size_bytes, DECIMAL);
        size_col_width = size_col_width.max(UnicodeWidthStr::width(size.as_str()));

        let modified = relative_time_ago(session.file_modified);
        modified_col_width = modified_col_width.max(UnicodeWidthStr::width(modified.as_str()));
    }

    (size_col_width, modified_col_width)
}

fn process_right_columns_width(processes: &[crate::app::ProcessInfo]) -> (usize, usize) {
    let mut status_col_width = 0usize;
    let mut started_col_width = 0usize;

    for process in processes {
        let status = process.status.label();
        status_col_width = status_col_width.max(UnicodeWidthStr::width(status.as_str()));

        let started = relative_time_ago(Some(process.started_at));
        started_col_width = started_col_width.max(UnicodeWidthStr::width(started.as_str()));
    }

    (status_col_width, started_col_width)
}

fn highlight_query_spans(text: &str, query: &str, base_style: Style) -> Vec<Span<'static>> {
    let query = query.trim();
    if query.is_empty() || text.is_empty() {
        return vec![Span::styled(text.to_string(), base_style)];
    }

    let query_lower = query.to_ascii_lowercase();
    if query_lower.is_empty() {
        return vec![Span::styled(text.to_string(), base_style)];
    }

    let highlighted_style = base_style
        .bg(Color::Blue)
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);
    let text_lower = text.to_ascii_lowercase();
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut cursor = 0usize;

    while cursor < text_lower.len() {
        let Some(relative) = text_lower[cursor..].find(&query_lower) else {
            break;
        };
        let start = cursor + relative;
        let end = start + query_lower.len();

        if start > cursor {
            spans.push(Span::styled(text[cursor..start].to_string(), base_style));
        }
        if end <= text.len() {
            spans.push(Span::styled(
                text[start..end].to_string(),
                highlighted_style,
            ));
        }

        cursor = end;
    }

    if cursor < text.len() {
        spans.push(Span::styled(text[cursor..].to_string(), base_style));
    }

    if spans.is_empty() {
        spans.push(Span::styled(text.to_string(), base_style));
    }

    spans
}

fn pad_left(text: &str, width: usize) -> String {
    let current = UnicodeWidthStr::width(text);
    if current >= width {
        return text.to_string();
    }
    format!("{}{}", " ".repeat(width.saturating_sub(current)), text)
}

fn pad_right(text: &str, width: usize) -> String {
    let current = UnicodeWidthStr::width(text);
    if current >= width {
        return text.to_string();
    }
    format!("{}{}", text, " ".repeat(width.saturating_sub(current)))
}

fn process_list_item(
    process: &crate::app::ProcessInfo,
    max_width: usize,
    status_col_width: usize,
    started_col_width: usize,
) -> ListItem<'static> {
    if max_width == 0 {
        return ListItem::new(Line::from(""));
    }

    let running = process.status.is_running();
    let online_dot_width = UnicodeWidthStr::width("● ");
    let dot = if running {
        Span::styled("● ", Style::default().fg(Color::Green))
    } else {
        Span::raw("  ")
    };

    let content_width = max_width.saturating_sub(online_dot_width);
    if content_width == 0 {
        return ListItem::new(Line::from(vec![dot]));
    }

    let status = pad_left(&process.status.label(), status_col_width);
    let started = pad_left(
        &relative_time_ago(Some(process.started_at)),
        started_col_width,
    );

    let column_sep = "  ·  ";
    let right_width = status_col_width + UnicodeWidthStr::width(column_sep) + started_col_width;

    let left = format!(
        "{}  {}  pid {}  {}",
        process.id,
        process.engine.label(),
        process.pid,
        process.prompt_preview
    );

    let min_left = 8usize;
    let gap = 2usize;
    if right_width + gap + min_left >= content_width {
        return ListItem::new(Line::from(vec![
            dot,
            Span::raw(truncate_end(&left, content_width)),
        ]));
    }

    let left_available = content_width.saturating_sub(right_width + gap);
    let left = truncate_end(&left, left_available);
    let left_width = UnicodeWidthStr::width(left.as_str());
    let padding_width = content_width.saturating_sub(left_width + right_width);

    ListItem::new(Line::from(vec![
        dot,
        Span::raw(left),
        Span::raw(" ".repeat(padding_width)),
        Span::styled(status, Style::default().fg(Color::DarkGray)),
        Span::styled(column_sep, Style::default().fg(Color::DarkGray)),
        Span::styled(started, Style::default().fg(Color::DarkGray)),
    ]))
}

fn project_list_item(
    project: &crate::domain::ProjectSummary,
    max_width: usize,
    sessions_col_width: usize,
    modified_col_width: usize,
    query: &str,
) -> ListItem<'static> {
    if max_width == 0 {
        return ListItem::new(Line::from(""));
    }

    let online = is_online(project.last_modified);
    let online_dot_width = UnicodeWidthStr::width("● ");
    let dot = if online {
        Span::styled("● ", Style::default().fg(Color::Green))
    } else {
        Span::raw("  ")
    };

    let content_width = max_width.saturating_sub(online_dot_width);
    if content_width == 0 {
        return ListItem::new(Line::from(vec![dot]));
    }

    let name = project.name.as_str();
    let path = project.project_path.display().to_string();

    let sessions_count = project.sessions.len();
    let session_word = if sessions_count == 1 {
        "session"
    } else {
        "sessions"
    };
    let sessions_col = format!("{sessions_count} {session_word}");
    let sessions_col = pad_left(&sessions_col, sessions_col_width);

    let modified = if project.last_modified.is_some() {
        relative_time_ago(project.last_modified)
    } else {
        "-".to_string()
    };
    let modified = pad_left(&modified, modified_col_width);

    let column_sep = "  ·  ";
    let right_width = sessions_col_width + UnicodeWidthStr::width(column_sep) + modified_col_width;

    let min_left = 8usize;
    let gap = 2usize;
    if right_width + gap + min_left >= content_width {
        let name = truncate_end(name, content_width);
        let mut spans = Vec::new();
        spans.push(dot);
        spans.extend(highlight_query_spans(
            &name,
            query,
            Style::default().add_modifier(Modifier::BOLD),
        ));
        return ListItem::new(Line::from(spans));
    }

    let left_available = content_width.saturating_sub(right_width + gap);

    let separator = "  ·  ";
    let separator_width = UnicodeWidthStr::width(separator);

    let min_path = 8usize;
    let mut left_width = 0usize;
    let mut spans = Vec::new();
    spans.push(dot);

    if left_available >= min_left + separator_width + min_path {
        let name_budget = left_available.saturating_sub(separator_width + min_path);
        let name = truncate_end(name, name_budget.max(min_left));
        let name_width = UnicodeWidthStr::width(name.as_str());
        spans.extend(highlight_query_spans(
            &name,
            query,
            Style::default().add_modifier(Modifier::BOLD),
        ));
        left_width += name_width;

        let path_budget = left_available
            .saturating_sub(left_width)
            .saturating_sub(separator_width);
        let path = truncate_middle(path.as_str(), path_budget);
        let path_width = UnicodeWidthStr::width(path.as_str());
        if !path.is_empty() {
            spans.push(Span::raw(separator));
            spans.extend(highlight_query_spans(
                &path,
                query,
                Style::default().fg(Color::DarkGray),
            ));
            left_width += separator_width + path_width;
        }
    } else {
        let name = truncate_end(name, left_available);
        let name_width = UnicodeWidthStr::width(name.as_str());
        spans.extend(highlight_query_spans(
            &name,
            query,
            Style::default().add_modifier(Modifier::BOLD),
        ));
        left_width += name_width;
    }

    let padding_width = content_width.saturating_sub(left_width + right_width);
    spans.push(Span::raw(" ".repeat(padding_width)));
    spans.push(Span::styled(
        sessions_col,
        Style::default().fg(Color::DarkGray),
    ));
    spans.push(Span::styled(
        column_sep,
        Style::default().fg(Color::DarkGray),
    ));
    spans.push(Span::styled(modified, Style::default().fg(Color::DarkGray)));

    ListItem::new(Line::from(spans))
}

fn is_online(modified: Option<SystemTime>) -> bool {
    const ONLINE_WINDOW: Duration = Duration::from_secs(60);

    let Some(modified) = modified else {
        return false;
    };
    let Ok(diff) = SystemTime::now().duration_since(modified) else {
        return false;
    };

    diff < ONLINE_WINDOW
}

fn session_list_item(
    session: &crate::domain::SessionSummary,
    max_width: usize,
    size_col_width: usize,
    modified_col_width: usize,
    query: &str,
) -> ListItem<'static> {
    if max_width == 0 {
        return ListItem::new(Line::from(""));
    }

    let online = is_online(session.file_modified);
    let online_dot_width = UnicodeWidthStr::width("● ");
    let dot = if online {
        Span::styled("● ", Style::default().fg(Color::Green))
    } else {
        Span::raw("  ")
    };

    let content_width = max_width.saturating_sub(online_dot_width);
    if content_width == 0 {
        return ListItem::new(Line::from(vec![dot]));
    }

    let size = format_size(session.file_size_bytes, DECIMAL);
    let size = pad_left(&size, size_col_width);

    let modified = relative_time_ago(session.file_modified);
    let modified = pad_left(&modified, modified_col_width);

    let column_sep = "  ·  ";
    let right_width = size_col_width + UnicodeWidthStr::width(column_sep) + modified_col_width;

    let min_left = 8usize;
    let gap = 2usize;
    if right_width + gap + min_left >= content_width {
        let title = truncate_end(&session.title, content_width);
        let mut spans = Vec::new();
        spans.push(dot);
        spans.extend(highlight_query_spans(&title, query, Style::default()));
        return ListItem::new(Line::from(spans));
    }

    let left_available = content_width.saturating_sub(right_width + gap);
    let title = truncate_end(&session.title, left_available);
    let title_width = UnicodeWidthStr::width(title.as_str());
    let padding_width = content_width.saturating_sub(title_width + right_width);

    let mut spans = Vec::new();
    spans.push(dot);
    spans.extend(highlight_query_spans(&title, query, Style::default()));
    spans.push(Span::raw(" ".repeat(padding_width)));
    spans.push(Span::styled(size, Style::default().fg(Color::DarkGray)));
    spans.push(Span::styled(
        column_sep,
        Style::default().fg(Color::DarkGray),
    ));
    spans.push(Span::styled(modified, Style::default().fg(Color::DarkGray)));

    ListItem::new(Line::from(spans))
}

fn truncate_end(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_string();
    }
    let ellipsis = "…";
    let available = max_width.saturating_sub(UnicodeWidthStr::width(ellipsis));
    let mut out = String::new();
    for ch in text.chars() {
        let next = format!("{out}{ch}");
        if UnicodeWidthStr::width(next.as_str()) > available {
            break;
        }
        out.push(ch);
    }
    out.push_str(ellipsis);
    out
}

fn truncate_middle(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(text) <= max_width {
        return text.to_string();
    }

    let ellipsis = "…";
    let available = max_width.saturating_sub(UnicodeWidthStr::width(ellipsis));
    if available <= 4 {
        return truncate_end(text, max_width);
    }

    let left_width = available / 2;
    let right_width = available - left_width;

    let left = take_prefix_width(text, left_width);
    let right = take_suffix_width(text, right_width);

    format!("{left}{ellipsis}{right}")
}

fn take_prefix_width(text: &str, width: usize) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        let next = format!("{out}{ch}");
        if UnicodeWidthStr::width(next.as_str()) > width {
            break;
        }
        out.push(ch);
    }
    out
}

fn take_suffix_width(text: &str, width: usize) -> String {
    let mut out = String::new();
    for ch in text.chars().rev() {
        let next = format!("{ch}{out}");
        if UnicodeWidthStr::width(next.as_str()) > width {
            break;
        }
        out.insert(0, ch);
    }
    out
}

fn relative_time_ago(time: Option<SystemTime>) -> String {
    let now = SystemTime::now();
    let moment = time.unwrap_or(SystemTime::UNIX_EPOCH);
    let diff = match now.duration_since(moment) {
        Ok(duration) => duration,
        Err(_) => Duration::from_secs(0),
    };
    match diff.cmp(&Duration::from_secs(60)) {
        Ordering::Less => "just now".to_string(),
        Ordering::Equal | Ordering::Greater => humanize_duration(diff),
    }
}

fn humanize_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < 60 {
        return format!("{seconds}s ago");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m ago");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours}h ago");
    }
    let days = hours / 24;
    format!("{days}d ago")
}

fn render_session_detail(
    frame: &mut Frame,
    area: Rect,
    model: &AppModel,
    detail_view: &crate::app::SessionDetailView,
) {
    let full_area = area;
    let area = inner_area(area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    let title = format!(
        "Session · {} · {}",
        short_id(&detail_view.session.meta.id),
        detail_view.session.meta.started_at_rfc3339
    );
    let cwd = detail_view.session.meta.cwd.display().to_string();
    let file_name = detail_view
        .session
        .log_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| detail_view.session.log_path.display().to_string());
    let size = format_size(detail_view.session.file_size_bytes, DECIMAL);
    let header_line = format!("cwd: {cwd}  ·  log: {file_name}  ·  {size}");
    let header = Paragraph::new(truncate_end(
        &header_line,
        (chunks[0].width as usize).saturating_sub(4),
    ))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .padding(Padding::horizontal(1))
            .title(title),
    );
    frame.render_widget(header, chunks[0]);

    let body = chunks[1];
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

    let list_area = panels[0];
    let detail_area = panels[1];

    let focused_border_style = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);
    let unfocused_border_style = Style::default().fg(Color::DarkGray);
    let focused = detail_view.focus;

    let timeline_border_style = if focused == SessionDetailFocus::Timeline {
        focused_border_style
    } else {
        unfocused_border_style
    };
    let details_border_style = if focused == SessionDetailFocus::Details {
        focused_border_style
    } else {
        unfocused_border_style
    };
    let timeline_title_style = if focused == SessionDetailFocus::Timeline {
        focused_border_style
    } else {
        unfocused_border_style.add_modifier(Modifier::BOLD)
    };
    let details_title_style = if focused == SessionDetailFocus::Details {
        focused_border_style
    } else {
        unfocused_border_style.add_modifier(Modifier::BOLD)
    };

    let max_width = (list_area.width as usize).saturating_sub(6);
    let TimelineRenderColumns {
        offset_col_width,
        duration_col_width,
        rows,
    } = build_timeline_render_columns(
        &detail_view.items,
        &detail_view.session.meta.started_at_rfc3339,
    );
    let list_items = detail_view
        .items
        .iter()
        .zip(rows)
        .map(|(item, cols)| {
            timeline_list_item(item, cols, max_width, offset_col_width, duration_col_width)
        })
        .collect::<Vec<_>>();

    let list_block = Block::default()
        .borders(Borders::ALL)
        .padding(Padding::horizontal(1))
        .border_style(timeline_border_style)
        .title(Title::from(Span::styled("Timeline", timeline_title_style)));
    let list_inner = list_block.inner(list_area);
    let list = List::new(list_items)
        .block(list_block)
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    let mut state = ListState::default();
    if !detail_view.items.is_empty() {
        state.select(Some(
            detail_view
                .selected
                .min(detail_view.items.len().saturating_sub(1)),
        ));
    }
    frame.render_stateful_widget(list, list_area, &mut state);

    let list_viewport = list_inner.height as usize;
    let list_total = detail_view.items.len();
    if list_total > list_viewport && list_viewport > 0 {
        let scrollbar_style = if focused == SessionDetailFocus::Timeline {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .thumb_style(scrollbar_style)
            .track_style(Style::default().fg(Color::DarkGray))
            .begin_style(scrollbar_style)
            .end_style(scrollbar_style);
        let mut scrollbar_state = ScrollbarState::new(list_total)
            .position(state.offset())
            .viewport_content_length(list_viewport);
        frame.render_stateful_widget(
            scrollbar,
            list_area.inner(Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut scrollbar_state,
        );
    }

    let detail_text = build_item_detail_text(detail_view);
    let detail_block = Block::default()
        .borders(Borders::ALL)
        .padding(Padding::horizontal(1))
        .border_style(details_border_style)
        .title(Title::from(Span::styled("Details", details_title_style)));
    let detail_inner = detail_block.inner(detail_area);
    let detail_viewport = detail_inner.height as usize;
    let detail_total = estimate_wrapped_text_height(&detail_text, detail_inner.width);
    let max_scroll = detail_total.saturating_sub(detail_viewport);
    let scroll = (detail_view.details_scroll as usize).min(max_scroll);
    let scroll_u16 = u16::try_from(scroll).unwrap_or(u16::MAX);
    let detail_paragraph = Paragraph::new(detail_text)
        .scroll((scroll_u16, 0))
        .wrap(Wrap { trim: false })
        .block(detail_block);
    frame.render_widget(Clear, detail_area);
    frame.render_widget(detail_paragraph, detail_area);

    if detail_total > detail_viewport && detail_viewport > 0 {
        let scrollbar_style = if focused == SessionDetailFocus::Details {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .thumb_style(scrollbar_style)
            .track_style(Style::default().fg(Color::DarkGray))
            .begin_style(scrollbar_style)
            .end_style(scrollbar_style);
        let mut scrollbar_state = ScrollbarState::new(detail_total)
            .position(scroll)
            .viewport_content_length(detail_viewport);
        frame.render_stateful_widget(
            scrollbar,
            detail_area.inner(Margin {
                vertical: 1,
                horizontal: 0,
            }),
            &mut scrollbar_state,
        );
    }

    let footer = session_detail_footer_line(
        model.data.warnings.get(),
        detail_view.warnings,
        detail_view.items.len(),
        detail_view.truncated,
        model.notice.as_deref(),
        model.update_hint.as_deref(),
        processes_running(model),
    );
    frame.render_widget(footer, chunks[2]);

    if detail_view.context_overlay_open {
        render_context_overlay(frame, full_area, detail_view);
    }

    if detail_view.output_overlay_open {
        render_last_output_overlay(frame, full_area, detail_view);
    }
}

fn estimate_wrapped_text_height(text: &Text<'static>, wrap_width: u16) -> usize {
    let wrap_width = wrap_width as usize;
    if wrap_width == 0 {
        return 0;
    }

    if text.lines.is_empty() {
        return 1;
    }

    text.lines
        .iter()
        .map(|line| estimate_wrapped_line_height(line, wrap_width))
        .sum()
}

fn estimate_wrapped_line_height(line: &Line<'static>, wrap_width: usize) -> usize {
    if wrap_width == 0 {
        return 0;
    }
    if line.spans.is_empty() {
        return 1;
    }

    let mut rows = 1usize;
    let mut col = 0usize;
    let mut token_is_whitespace: Option<bool> = None;
    let mut token_width = 0usize;
    let mut token_chars: Vec<char> = Vec::new();

    for span in &line.spans {
        for ch in span.content.chars() {
            let is_whitespace = ch.is_whitespace();

            if token_is_whitespace.is_some_and(|ws| ws != is_whitespace) {
                apply_wrap_token(
                    wrap_width,
                    token_is_whitespace.unwrap_or(false),
                    &token_chars,
                    token_width,
                    &mut rows,
                    &mut col,
                );
                token_chars.clear();
                token_width = 0;
            }

            token_is_whitespace = Some(is_whitespace);
            token_chars.push(ch);
            token_width = token_width.saturating_add(display_char_width(ch));
        }
    }

    if token_is_whitespace.is_some() {
        apply_wrap_token(
            wrap_width,
            token_is_whitespace.unwrap_or(false),
            &token_chars,
            token_width,
            &mut rows,
            &mut col,
        );
    }

    rows
}

fn apply_wrap_token(
    wrap_width: usize,
    _is_whitespace: bool,
    token_chars: &[char],
    token_width: usize,
    rows: &mut usize,
    col: &mut usize,
) {
    if wrap_width == 0 || token_chars.is_empty() || token_width == 0 {
        return;
    }

    if token_width > wrap_width {
        if *col > 0 {
            *rows = rows.saturating_add(1);
            *col = 0;
        }
        for ch in token_chars {
            let w = display_char_width(*ch);
            if w == 0 || w > wrap_width {
                continue;
            }
            if *col + w > wrap_width {
                *rows = rows.saturating_add(1);
                *col = 0;
            }
            *col = col.saturating_add(w);
        }
        return;
    }

    if *col > 0 && *col + token_width > wrap_width {
        *rows = rows.saturating_add(1);
        *col = 0;
    }
    *col = col.saturating_add(token_width);
}

fn display_char_width(ch: char) -> usize {
    if ch == '\t' {
        return 4;
    }
    UnicodeWidthChar::width(ch).unwrap_or(0)
}

fn session_detail_footer_line(
    scan_warnings: usize,
    detail_warnings: usize,
    item_count: usize,
    truncated: bool,
    notice: Option<&str>,
    update_hint: Option<&str>,
    processes_running: bool,
) -> Paragraph<'static> {
    let mut parts = vec![
        "Keys: Tab=focus  arrows=move/scroll  PgUp/PgDn=page  Enter=ToolOut (Tool)  o=result  F3=stats  c=context  Esc/Backspace=back  Ctrl+R=rescan  Ctrl+Q/Ctrl+C=quit  F1/?=help"
            .to_string(),
        format!("items: {item_count}"),
    ];
    if truncated {
        parts.push("truncated".to_string());
    }
    if scan_warnings > 0 {
        parts.push(format!("scan warnings: {scan_warnings}"));
    }
    if detail_warnings > 0 {
        parts.push(format!("parse warnings: {detail_warnings}"));
    }
    let base = parts.join("  ·  ");
    footer_paragraph(base, notice, update_hint, processes_running)
}

fn kind_label(kind: TimelineItemKind) -> &'static str {
    match kind {
        TimelineItemKind::Turn => "Turn",
        TimelineItemKind::User => "User",
        TimelineItemKind::Assistant => "Out",
        TimelineItemKind::Thinking => "Think",
        TimelineItemKind::ToolCall => "Tool",
        TimelineItemKind::ToolOutput => "ToolOut",
        TimelineItemKind::TokenCount => "Tokens",
        TimelineItemKind::Note => "Note",
    }
}

fn kind_style(kind: TimelineItemKind) -> Style {
    match kind {
        TimelineItemKind::Turn => Style::default().fg(Color::DarkGray),
        TimelineItemKind::User => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        TimelineItemKind::Assistant => Style::default().fg(Color::Green),
        TimelineItemKind::Thinking => Style::default().fg(Color::Magenta),
        TimelineItemKind::ToolCall => Style::default().fg(Color::LightBlue),
        TimelineItemKind::ToolOutput => Style::default().fg(Color::LightBlue),
        TimelineItemKind::TokenCount => Style::default().fg(Color::Yellow),
        TimelineItemKind::Note => Style::default().fg(Color::DarkGray),
    }
}

#[derive(Clone, Debug)]
struct TimelineRowColumns {
    offset: String,
    duration: String,
}

#[derive(Clone, Debug)]
struct TimelineRenderColumns {
    offset_col_width: usize,
    duration_col_width: usize,
    rows: Vec<TimelineRowColumns>,
}

fn build_timeline_render_columns(
    items: &[TimelineItem],
    session_start_rfc3339: &str,
) -> TimelineRenderColumns {
    let session_start_ms =
        parse_rfc3339_to_unix_ms(session_start_rfc3339).or_else(|| earliest_timestamp_ms(items));

    let mut tool_out_by_call_id: HashMap<String, i64> = HashMap::new();
    for item in items {
        if item.kind != TimelineItemKind::ToolOutput {
            continue;
        }
        let Some(call_id) = item.call_id.as_deref() else {
            continue;
        };
        let Some(ts) = item.timestamp_ms else {
            continue;
        };
        tool_out_by_call_id.entry(call_id.to_string()).or_insert(ts);
    }

    let mut rows = Vec::with_capacity(items.len());
    let mut offset_col_width = 0usize;
    let mut duration_col_width = 0usize;

    let mut prev_ts_ms: Option<i64> = None;
    for item in items {
        let offset = match (item.timestamp_ms, session_start_ms) {
            (Some(ts), Some(start)) if ts >= start => {
                format_offset(Duration::from_millis((ts - start) as u64))
            }
            _ => "-".to_string(),
        };

        let duration_ms = if item.kind == TimelineItemKind::ToolCall {
            match (item.call_id.as_deref(), item.timestamp_ms) {
                (Some(call_id), Some(call_ts)) => tool_out_by_call_id
                    .get(call_id)
                    .and_then(|out_ts| out_ts.checked_sub(call_ts)),
                _ => None,
            }
        } else {
            match (item.timestamp_ms, prev_ts_ms) {
                (Some(ts), Some(prev)) => ts.checked_sub(prev),
                _ => None,
            }
        };

        let duration = match duration_ms {
            Some(ms) => format_duration(Duration::from_millis(ms as u64)),
            None => "-".to_string(),
        };

        if let Some(ts) = item.timestamp_ms {
            prev_ts_ms = Some(ts);
        }

        offset_col_width = offset_col_width.max(UnicodeWidthStr::width(offset.as_str()));
        duration_col_width = duration_col_width.max(UnicodeWidthStr::width(duration.as_str()));
        rows.push(TimelineRowColumns { offset, duration });
    }

    for row in &mut rows {
        row.offset = pad_left(&row.offset, offset_col_width);
        row.duration = pad_left(&row.duration, duration_col_width);
    }

    TimelineRenderColumns {
        offset_col_width,
        duration_col_width,
        rows,
    }
}

fn earliest_timestamp_ms(items: &[TimelineItem]) -> Option<i64> {
    items.iter().filter_map(|item| item.timestamp_ms).min()
}

fn parse_rfc3339_to_unix_ms(value: &str) -> Option<i64> {
    let timestamp = OffsetDateTime::parse(value, &Rfc3339).ok()?;
    let ms: i128 = timestamp.unix_timestamp_nanos() / 1_000_000;
    i64::try_from(ms).ok()
}

fn format_offset(duration: Duration) -> String {
    let total_ms = duration.as_millis() as u64;
    let total_s = total_ms / 1000;
    let ms = total_ms % 1000;

    let s = total_s % 60;
    let total_m = total_s / 60;
    let m = total_m % 60;
    let h = total_m / 60;

    if h > 0 {
        format!("{h:02}:{m:02}:{s:02}.{ms:03}")
    } else {
        format!("{m:02}:{s:02}.{ms:03}")
    }
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

fn format_commas_u64(value: u64) -> String {
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

fn format_commas_usize(value: usize) -> String {
    format_commas_u64(value as u64)
}

fn timeline_list_item(
    item: &TimelineItem,
    cols: TimelineRowColumns,
    max_width: usize,
    offset_col_width: usize,
    duration_col_width: usize,
) -> ListItem<'static> {
    if max_width == 0 {
        return ListItem::new(Line::from(""));
    }

    const KIND_WIDTH: usize = 6;
    let label_raw = kind_label(item.kind);
    let label = pad_right(label_raw, KIND_WIDTH);
    let label_width = UnicodeWidthStr::width(label.as_str());

    let column_sep = "  ·  ";
    let right_width = offset_col_width + UnicodeWidthStr::width(column_sep) + duration_col_width;

    let left_prefix_width = label_width + UnicodeWidthStr::width("  ");
    let min_left = left_prefix_width.saturating_add(4);
    let gap = 2usize;
    if right_width + gap + min_left >= max_width {
        let content = format!("{label_raw}  {}", item.summary);
        return ListItem::new(Line::from(truncate_end(&content, max_width)));
    }

    let left_available = max_width.saturating_sub(right_width + gap);
    if left_available <= left_prefix_width {
        return ListItem::new(Line::from(vec![Span::styled(
            truncate_end(&label, max_width),
            kind_style(item.kind),
        )]));
    }

    let summary_budget = left_available.saturating_sub(left_prefix_width);
    let summary = truncate_end(&item.summary, summary_budget);
    let summary_width = UnicodeWidthStr::width(summary.as_str());
    let left_width = left_prefix_width + summary_width;
    let padding_width = max_width.saturating_sub(left_width + right_width);

    ListItem::new(Line::from(vec![
        Span::styled(label, kind_style(item.kind)),
        Span::raw("  "),
        Span::raw(summary),
        Span::raw(" ".repeat(padding_width)),
        Span::styled(cols.offset, Style::default().fg(Color::DarkGray)),
        Span::styled(column_sep, Style::default().fg(Color::DarkGray)),
        Span::styled(cols.duration, Style::default().fg(Color::DarkGray)),
    ]))
}

fn build_item_detail_text(detail_view: &crate::app::SessionDetailView) -> Text<'static> {
    let selected = detail_view
        .selected
        .min(detail_view.items.len().saturating_sub(1));
    let Some(item) = detail_view.items.get(selected) else {
        return Text::from("No timeline items.");
    };

    let key_style = Style::default().fg(Color::DarkGray);
    let value_style = Style::default();
    let summary_style = Style::default().add_modifier(Modifier::BOLD);

    let mut text = Text::default();
    text.lines.push(Line::from(vec![
        Span::styled("Kind: ", key_style),
        Span::styled(kind_label(item.kind), kind_style(item.kind)),
    ]));
    text.lines.push(Line::from(vec![
        Span::styled("Turn: ", key_style),
        Span::styled(
            item.turn_id.as_deref().unwrap_or("-").to_string(),
            value_style,
        ),
    ]));
    text.lines.push(Line::from(vec![
        Span::styled("Timestamp: ", key_style),
        Span::styled(
            item.timestamp.as_deref().unwrap_or("-").to_string(),
            value_style,
        ),
    ]));
    text.lines.push(Line::from(vec![
        Span::styled("Summary: ", key_style),
        Span::styled(item.summary.clone(), summary_style),
    ]));

    if let Some(call_id) = item.call_id.as_deref() {
        text.lines.push(Line::from(vec![
            Span::styled("Call ID: ", key_style),
            Span::styled(call_id.to_string(), value_style),
        ]));
    }
    text.lines.push(Line::from(""));

    let max = 12_000;

    if item.kind == TimelineItemKind::ToolCall {
        if let Some(call_id) = item.call_id.as_deref() {
            if let Some(tool_out) = find_tool_output_for_call(&detail_view.items, selected, call_id)
            {
                text.lines.push(Line::from(Span::styled(
                    "Output:",
                    Style::default()
                        .fg(Color::LightBlue)
                        .add_modifier(Modifier::BOLD),
                )));
                text.lines.extend(render_plain_highlight_lines(
                    truncate_chars(&tool_out.detail, max).as_str(),
                ));
                text.lines.push(Line::from(""));
                text.lines.push(Line::from(Span::styled(
                    "Input:",
                    Style::default()
                        .fg(Color::LightBlue)
                        .add_modifier(Modifier::BOLD),
                )));
                text.lines.extend(render_plain_highlight_lines(
                    truncate_chars(&item.detail, max).as_str(),
                ));
                return text;
            }
        }
    }

    let truncated = truncate_chars(&item.detail, max);
    text.lines
        .extend(render_detail_lines_for_kind(item.kind, &truncated));
    text
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MarkdownishToken {
    InlineCode,
    Bold,
    Link,
}

fn render_detail_lines_for_kind(kind: TimelineItemKind, text: &str) -> Vec<Line<'static>> {
    match kind {
        TimelineItemKind::Assistant
        | TimelineItemKind::User
        | TimelineItemKind::Thinking
        | TimelineItemKind::Note => render_markdownish_lines(text),
        TimelineItemKind::Turn
        | TimelineItemKind::ToolCall
        | TimelineItemKind::ToolOutput
        | TimelineItemKind::TokenCount => render_plain_highlight_lines(text),
    }
}

fn render_plain_highlight_lines(text: &str) -> Vec<Line<'static>> {
    if is_jsonish(text) {
        return render_json_pretty_highlight_lines(text);
    }

    text.split('\n')
        .flat_map(|raw_line| {
            if is_jsonish(raw_line) {
                return render_json_pretty_highlight_lines(raw_line);
            }

            let trimmed = raw_line.trim_start();
            let style = match trimmed.chars().next() {
                Some('+') => Style::default().fg(Color::Green),
                Some('-') => Style::default().fg(Color::Red),
                _ if trimmed.starts_with("@@") => Style::default().fg(Color::Cyan),
                _ => Style::default(),
            };
            vec![Line::from(Span::styled(raw_line.to_string(), style))]
        })
        .collect()
}

fn render_markdownish_lines(text: &str) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut in_code_block = false;

    for raw_line in text.split('\n') {
        let trimmed = raw_line.trim_start();
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }

        if in_code_block {
            if is_jsonish(trimmed) {
                lines.extend(render_json_pretty_highlight_lines(raw_line));
                continue;
            }

            let style = match trimmed.chars().next() {
                Some('+') => Style::default().fg(Color::Green),
                Some('-') => Style::default().fg(Color::Red),
                _ if trimmed.starts_with("@@") => Style::default().fg(Color::Cyan),
                _ => Style::default().fg(Color::LightBlue),
            };
            lines.push(Line::from(Span::styled(raw_line.to_string(), style)));
            continue;
        }

        if is_jsonish(trimmed) {
            lines.extend(render_json_pretty_highlight_lines(raw_line));
            continue;
        }

        let indent_len = raw_line.len().saturating_sub(trimmed.len());
        let indent = raw_line.get(0..indent_len).unwrap_or("").to_string();

        if let Some((level, heading_text)) = parse_markdown_heading(trimmed) {
            let style = match level {
                1 => Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
                    .add_modifier(Modifier::UNDERLINED),
                2 => Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
                _ => Style::default()
                    .fg(Color::LightYellow)
                    .add_modifier(Modifier::BOLD),
            };
            let mut spans = Vec::new();
            if !indent.is_empty() {
                spans.push(Span::raw(indent));
            }
            spans.extend(markdownish_inline_spans(heading_text, style));
            lines.push(Line::from(spans));
            continue;
        }

        if let Some(quote_text) = trimmed.strip_prefix("> ") {
            let quote_style = Style::default().fg(Color::DarkGray);
            let mut spans = Vec::new();
            if !indent.is_empty() {
                spans.push(Span::raw(indent));
            }
            spans.push(Span::styled("│ ", quote_style));
            spans.extend(markdownish_inline_spans(quote_text, quote_style));
            lines.push(Line::from(spans));
            continue;
        }

        if let Some(list_text) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
            .or_else(|| trimmed.strip_prefix("+ "))
        {
            let mut spans = Vec::new();
            if !indent.is_empty() {
                spans.push(Span::raw(indent));
            }
            spans.push(Span::styled("• ", Style::default().fg(Color::Yellow)));
            spans.extend(markdownish_inline_spans(list_text, Style::default()));
            lines.push(Line::from(spans));
            continue;
        }

        lines.push(Line::from(markdownish_inline_spans(
            raw_line,
            Style::default(),
        )));
    }

    lines
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum JsonStyleKind {
    Default,
    Punctuation,
    Key,
    String,
    Number,
    Boolean,
    Null,
}

#[derive(Clone, Debug)]
struct JsonSegment {
    kind: JsonStyleKind,
    text: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum JsonContext {
    Object(ObjectState),
    Array(ArrayState),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ObjectState {
    KeyOrEnd,
    Colon,
    Value,
    CommaOrEnd,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ArrayState {
    ValueOrEnd,
    CommaOrEnd,
}

fn is_jsonish(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with('{') || trimmed.starts_with('[')
}

fn push_json_segment(segments: &mut Vec<JsonSegment>, kind: JsonStyleKind, text: &str) {
    if text.is_empty() {
        return;
    }

    if let Some(last) = segments.last_mut() {
        if last.kind == kind {
            last.text.push_str(text);
            return;
        }
    }

    segments.push(JsonSegment {
        kind,
        text: text.to_string(),
    });
}

fn json_style(kind: JsonStyleKind) -> Style {
    match kind {
        JsonStyleKind::Default => Style::default(),
        JsonStyleKind::Punctuation => Style::default().fg(Color::DarkGray),
        JsonStyleKind::Key => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        JsonStyleKind::String => Style::default().fg(Color::Green),
        JsonStyleKind::Number => Style::default().fg(Color::Magenta),
        JsonStyleKind::Boolean => Style::default().fg(Color::Cyan),
        JsonStyleKind::Null => Style::default().fg(Color::DarkGray),
    }
}

fn json_segments_to_line(segments: Vec<JsonSegment>) -> Line<'static> {
    if segments.is_empty() {
        return Line::from("");
    }

    Line::from(
        segments
            .into_iter()
            .map(|segment| Span::styled(segment.text, json_style(segment.kind)))
            .collect::<Vec<_>>(),
    )
}

fn consume_value(stack: &mut [JsonContext]) {
    let Some(top) = stack.last_mut() else {
        return;
    };

    match top {
        JsonContext::Object(state) => {
            if *state == ObjectState::Value {
                *state = ObjectState::CommaOrEnd;
            }
        }
        JsonContext::Array(state) => {
            if *state == ArrayState::ValueOrEnd {
                *state = ArrayState::CommaOrEnd;
            }
        }
    }
}

fn render_json_highlight_lines(text: &str) -> Vec<Line<'static>> {
    let mut stack: Vec<JsonContext> = Vec::new();
    let mut current_segments: Vec<JsonSegment> = Vec::new();
    let mut lines: Vec<Line<'static>> = Vec::new();

    let mut iter = text.chars().peekable();
    while let Some(ch) = iter.next() {
        match ch {
            '\n' => {
                lines.push(json_segments_to_line(std::mem::take(&mut current_segments)));
            }
            '"' => {
                let mut token = String::new();
                token.push(ch);
                let mut escaped = false;
                for next in iter.by_ref() {
                    token.push(next);
                    if escaped {
                        escaped = false;
                        continue;
                    }
                    if next == '\\' {
                        escaped = true;
                        continue;
                    }
                    if next == '"' {
                        break;
                    }
                }

                let is_key = matches!(
                    stack.last(),
                    Some(JsonContext::Object(ObjectState::KeyOrEnd))
                );
                push_json_segment(
                    &mut current_segments,
                    if is_key {
                        JsonStyleKind::Key
                    } else {
                        JsonStyleKind::String
                    },
                    &token,
                );

                if is_key {
                    if let Some(JsonContext::Object(state)) = stack.last_mut() {
                        *state = ObjectState::Colon;
                    }
                } else {
                    consume_value(&mut stack);
                }
            }
            '{' => {
                push_json_segment(&mut current_segments, JsonStyleKind::Punctuation, "{");
                stack.push(JsonContext::Object(ObjectState::KeyOrEnd));
            }
            '}' => {
                push_json_segment(&mut current_segments, JsonStyleKind::Punctuation, "}");
                if matches!(stack.last(), Some(JsonContext::Object(_))) {
                    stack.pop();
                    consume_value(&mut stack);
                }
            }
            '[' => {
                push_json_segment(&mut current_segments, JsonStyleKind::Punctuation, "[");
                stack.push(JsonContext::Array(ArrayState::ValueOrEnd));
            }
            ']' => {
                push_json_segment(&mut current_segments, JsonStyleKind::Punctuation, "]");
                if matches!(stack.last(), Some(JsonContext::Array(_))) {
                    stack.pop();
                    consume_value(&mut stack);
                }
            }
            ':' => {
                push_json_segment(&mut current_segments, JsonStyleKind::Punctuation, ":");
                if let Some(JsonContext::Object(state)) = stack.last_mut() {
                    if *state == ObjectState::Colon {
                        *state = ObjectState::Value;
                    }
                }
            }
            ',' => {
                push_json_segment(&mut current_segments, JsonStyleKind::Punctuation, ",");
                if let Some(top) = stack.last_mut() {
                    match top {
                        JsonContext::Object(state) => {
                            if *state == ObjectState::CommaOrEnd {
                                *state = ObjectState::KeyOrEnd;
                            }
                        }
                        JsonContext::Array(state) => {
                            if *state == ArrayState::CommaOrEnd {
                                *state = ArrayState::ValueOrEnd;
                            }
                        }
                    }
                }
            }
            '-' | '0'..='9' => {
                let mut token = String::new();
                token.push(ch);
                while let Some(next) = iter.peek().copied() {
                    if next.is_ascii_digit() || matches!(next, '.' | 'e' | 'E' | '+' | '-') {
                        token.push(next);
                        let _ = iter.next();
                        continue;
                    }
                    break;
                }
                push_json_segment(&mut current_segments, JsonStyleKind::Number, &token);
                consume_value(&mut stack);
            }
            't' => {
                if iter.peek().copied() == Some('r') {
                    let snapshot = iter.clone().take(3).collect::<String>();
                    if snapshot == "rue" {
                        let _ = iter.next();
                        let _ = iter.next();
                        let _ = iter.next();
                        push_json_segment(&mut current_segments, JsonStyleKind::Boolean, "true");
                        consume_value(&mut stack);
                        continue;
                    }
                }
                push_json_segment(&mut current_segments, JsonStyleKind::Default, "t");
            }
            'f' => {
                if iter.peek().copied() == Some('a') {
                    let snapshot = iter.clone().take(4).collect::<String>();
                    if snapshot == "alse" {
                        let _ = iter.next();
                        let _ = iter.next();
                        let _ = iter.next();
                        let _ = iter.next();
                        push_json_segment(&mut current_segments, JsonStyleKind::Boolean, "false");
                        consume_value(&mut stack);
                        continue;
                    }
                }
                push_json_segment(&mut current_segments, JsonStyleKind::Default, "f");
            }
            'n' => {
                if iter.peek().copied() == Some('u') {
                    let snapshot = iter.clone().take(3).collect::<String>();
                    if snapshot == "ull" {
                        let _ = iter.next();
                        let _ = iter.next();
                        let _ = iter.next();
                        push_json_segment(&mut current_segments, JsonStyleKind::Null, "null");
                        consume_value(&mut stack);
                        continue;
                    }
                }
                push_json_segment(&mut current_segments, JsonStyleKind::Default, "n");
            }
            other if other.is_whitespace() => {
                push_json_segment(
                    &mut current_segments,
                    JsonStyleKind::Default,
                    &other.to_string(),
                );
            }
            other => {
                push_json_segment(
                    &mut current_segments,
                    JsonStyleKind::Default,
                    &other.to_string(),
                );
            }
        }
    }

    lines.push(json_segments_to_line(current_segments));
    lines
}

fn parse_markdown_heading(line: &str) -> Option<(usize, &str)> {
    if !line.starts_with('#') {
        return None;
    }

    let mut level = 0usize;
    for ch in line.chars() {
        if ch == '#' {
            level += 1;
            continue;
        }
        break;
    }

    if level == 0 || level > 6 {
        return None;
    }

    let rest = line.get(level..).unwrap_or("");
    if !rest.starts_with(' ') {
        return None;
    }

    Some((level, rest.trim()))
}

fn markdownish_inline_spans(text: &str, base_style: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut remaining = text;

    loop {
        let mut best_pos = None;
        let mut best_token = None;

        if let Some(pos) = remaining.find('`') {
            best_pos = Some(pos);
            best_token = Some(MarkdownishToken::InlineCode);
        }
        if let Some(pos) = remaining.find("**") {
            let replace = match best_pos {
                None => true,
                Some(current) => pos < current,
            };
            if replace {
                best_pos = Some(pos);
                best_token = Some(MarkdownishToken::Bold);
            }
        }
        if let Some(pos) = remaining.find('[') {
            let replace = match best_pos {
                None => true,
                Some(current) => pos < current,
            };
            if replace {
                best_pos = Some(pos);
                best_token = Some(MarkdownishToken::Link);
            }
        }

        let Some(pos) = best_pos else {
            if !remaining.is_empty() {
                spans.push(Span::styled(remaining.to_string(), base_style));
            }
            break;
        };
        let Some(token) = best_token else {
            spans.push(Span::styled(remaining.to_string(), base_style));
            break;
        };

        if pos > 0 {
            if let Some(prefix) = remaining.get(0..pos) {
                if !prefix.is_empty() {
                    spans.push(Span::styled(prefix.to_string(), base_style));
                }
            }
        }

        match token {
            MarkdownishToken::InlineCode => {
                let start = pos;
                let after = remaining.get(start + 1..).unwrap_or("");
                if let Some(end_rel) = after.find('`') {
                    let code = after.get(0..end_rel).unwrap_or("");
                    spans.push(Span::styled(
                        code.to_string(),
                        Style::default().fg(Color::LightBlue),
                    ));
                    remaining = after.get(end_rel + 1..).unwrap_or("");
                } else {
                    spans.push(Span::styled("`".to_string(), base_style));
                    remaining = after;
                }
            }
            MarkdownishToken::Bold => {
                let start = pos;
                let after = remaining.get(start + 2..).unwrap_or("");
                if let Some(end_rel) = after.find("**") {
                    let content = after.get(0..end_rel).unwrap_or("");
                    spans.extend(markdownish_inline_spans(
                        content,
                        base_style.add_modifier(Modifier::BOLD),
                    ));
                    remaining = after.get(end_rel + 2..).unwrap_or("");
                } else {
                    spans.push(Span::styled("**".to_string(), base_style));
                    remaining = after;
                }
            }
            MarkdownishToken::Link => {
                let start = pos;
                let after = remaining.get(start + 1..).unwrap_or("");
                let Some(end_bracket_rel) = after.find(']') else {
                    spans.push(Span::styled("[".to_string(), base_style));
                    remaining = after;
                    continue;
                };
                let link_text = after.get(0..end_bracket_rel).unwrap_or("");
                let rest_after = after.get(end_bracket_rel + 1..).unwrap_or("");
                spans.extend(markdownish_inline_spans(
                    link_text,
                    Style::default()
                        .fg(Color::LightBlue)
                        .add_modifier(Modifier::UNDERLINED),
                ));
                if let Some(rest_after_paren) = rest_after.strip_prefix('(') {
                    if let Some(close_paren) = rest_after_paren.find(')') {
                        remaining = rest_after_paren.get(close_paren + 1..).unwrap_or("");
                        continue;
                    }
                }
                remaining = rest_after;
            }
        }
    }

    spans
}

fn find_tool_output_for_call<'a>(
    items: &'a [TimelineItem],
    selected_index: usize,
    call_id: &str,
) -> Option<&'a TimelineItem> {
    if selected_index + 1 < items.len() {
        if let Some(hit) = items.iter().skip(selected_index + 1).find(|item| {
            item.kind == TimelineItemKind::ToolOutput && item.call_id.as_deref() == Some(call_id)
        }) {
            return Some(hit);
        }
    }

    items.iter().find(|item| {
        item.kind == TimelineItemKind::ToolOutput && item.call_id.as_deref() == Some(call_id)
    })
}

fn render_context_overlay(
    frame: &mut Frame,
    area: Rect,
    detail_view: &crate::app::SessionDetailView,
) {
    let popup = centered_rect(72, 60, area);
    frame.render_widget(Clear, popup);

    let ctx = selected_turn_context(detail_view);
    let paragraph = Paragraph::new(ctx).wrap(Wrap { trim: false }).block(
        Block::default()
            .borders(Borders::ALL)
            .padding(Padding::horizontal(1))
            .title("Visible Context (c or Esc to close)"),
    );
    frame.render_widget(paragraph, popup);
}

fn render_last_output_overlay(
    frame: &mut Frame,
    area: Rect,
    detail_view: &crate::app::SessionDetailView,
) {
    let popup = centered_rect(82, 72, area);
    frame.render_widget(Clear, popup);

    let output = detail_view
        .last_output
        .as_deref()
        .unwrap_or("(No assistant output found.)");
    let display = truncate_chars(output, 50_000);
    let text = Text::from(render_markdownish_lines(&display));

    let paragraph = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .scroll((detail_view.output_overlay_scroll, 0))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1))
                .title("Result (last Out) · Esc/Enter=close · arrows/PgUp/PgDn=scroll"),
        );
    frame.render_widget(paragraph, popup);
}

fn render_session_result_preview_overlay(
    frame: &mut Frame,
    area: Rect,
    preview: &crate::app::SessionResultPreviewOverlay,
) {
    let popup = centered_rect(82, 72, area);
    frame.render_widget(Clear, popup);

    let display = truncate_chars(&preview.output, 50_000);
    let text = Text::from(render_markdownish_lines(&display));

    let title_budget = (popup.width as usize).saturating_sub(4);
    let session_title = truncate_end(&preview.session_title, title_budget.saturating_sub(60));
    let title = format!(
        "Result (last Out) · {session_title} · Esc/Enter/Space=close · arrows/PgUp/PgDn=scroll"
    );
    let title = truncate_end(&title, title_budget);

    let paragraph = Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .scroll((preview.scroll, 0))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .padding(Padding::horizontal(1))
                .title(title),
        );
    frame.render_widget(paragraph, popup);
}

fn render_project_stats_overlay(
    frame: &mut Frame,
    area: Rect,
    overlay: &crate::app::ProjectStatsOverlay,
) {
    let popup = centered_rect(82, 58, area);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .padding(Padding::horizontal(1))
        .title("Project Stats");
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);

    let max_line_width = (chunks[0].width as usize).saturating_sub(1);

    let label_style = Style::default().fg(Color::Gray);
    let value_style = Style::default().fg(Color::White);
    let dim_style = Style::default().fg(Color::DarkGray);
    let path_style = Style::default().fg(Color::LightBlue);
    let section_style = Style::default()
        .fg(Color::LightBlue)
        .add_modifier(Modifier::BOLD);
    let token_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let warning_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);

    let mut lines: Vec<Line<'static>> = Vec::new();

    let project_prefix = "Project: ";
    let project_budget = max_line_width.saturating_sub(UnicodeWidthStr::width(project_prefix));
    let project_value = truncate_end(&overlay.project_name, project_budget);
    lines.push(Line::from(vec![
        Span::styled(project_prefix, label_style),
        Span::styled(project_value, value_style.add_modifier(Modifier::BOLD)),
    ]));

    let path_prefix = "Path: ";
    let path_budget = max_line_width.saturating_sub(UnicodeWidthStr::width(path_prefix));
    let path_value = truncate_middle(&overlay.project_path.display().to_string(), path_budget);
    lines.push(Line::from(vec![
        Span::styled(path_prefix, label_style),
        Span::styled(path_value, path_style),
    ]));

    lines.push(Line::from(""));

    lines.push(Line::from(vec![Span::styled("Sessions", section_style)]));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("Total:   ", label_style),
        Span::styled(
            format_commas_usize(overlay.session_count),
            value_style.add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("Indexed: ", label_style),
        Span::styled(
            format_commas_usize(overlay.indexed_sessions),
            value_style.add_modifier(Modifier::BOLD),
        ),
        Span::styled(" / ", dim_style),
        Span::styled(format_commas_usize(overlay.session_count), dim_style),
    ]));

    let missing_style = if overlay.missing_tokens_sessions > 0 {
        warning_style
    } else {
        dim_style
    };
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("Missing tokens: ", label_style),
        Span::styled(
            format_commas_usize(overlay.missing_tokens_sessions),
            missing_style,
        ),
    ]));

    lines.push(Line::from(""));

    lines.push(Line::from(vec![Span::styled("Tokens", section_style)]));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("Total (indexed): ", label_style),
        Span::styled(format_commas_u64(overlay.total_tokens_indexed), token_style),
    ]));
    if overlay.indexed_sessions > 0 {
        let avg = overlay.total_tokens_indexed / overlay.indexed_sessions as u64;
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("Avg / session:   ", label_style),
            Span::styled(format_commas_u64(avg), value_style),
        ]));
    }

    if overlay.missing_tokens_sessions > 0 {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("Note: totals update in background.", dim_style),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled("Cache", section_style)]));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("Index: ", label_style),
        Span::styled("~/.ccbox/session_index.json", dim_style),
    ]));

    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((overlay.scroll, 0));
    frame.render_widget(paragraph, chunks[0]);

    let hint = Paragraph::new("Keys: arrows/PgUp/PgDn=scroll  Esc/Backspace=close")
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    frame.render_widget(hint, chunks[1]);
}

fn render_session_stats_overlay(
    frame: &mut Frame,
    area: Rect,
    overlay: &crate::app::SessionStatsOverlay,
) {
    let popup = centered_rect(82, 78, area);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .padding(Padding::horizontal(1))
        .title("Session Stats");
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);

    let max_line_width = (chunks[0].width as usize).saturating_sub(1);

    let label_style = Style::default().fg(Color::Gray);
    let value_style = Style::default().fg(Color::White);
    let dim_style = Style::default().fg(Color::DarkGray);
    let path_style = Style::default().fg(Color::LightBlue);
    let section_style = Style::default()
        .fg(Color::LightBlue)
        .add_modifier(Modifier::BOLD);
    let token_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let success_style = Style::default()
        .fg(Color::Green)
        .add_modifier(Modifier::BOLD);
    let invalid_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let error_style = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
    let unknown_style = Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::BOLD);
    let added_style = Style::default()
        .fg(Color::Green)
        .add_modifier(Modifier::BOLD);
    let removed_style = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);

    let duration = overlay.stats.duration_ms.and_then(|ms| {
        if ms >= 0 {
            Some(format_duration(Duration::from_millis(ms as u64)))
        } else {
            None
        }
    });

    let token_total_span = match overlay.stats.total_tokens {
        Some(v) => Span::styled(format_commas_u64(v), token_style),
        None => Span::styled("-", dim_style),
    };
    let token_last_span = match overlay.stats.last_tokens {
        Some(v) => Span::styled(format_commas_u64(v), token_style),
        None => Span::styled("-", dim_style),
    };
    let duration_span = match duration {
        Some(value) => Span::styled(value, value_style.add_modifier(Modifier::BOLD)),
        None => Span::styled("-", dim_style),
    };
    let invalid_count_style = if overlay.stats.tool_calls_invalid > 0 {
        invalid_style
    } else {
        dim_style
    };
    let error_count_style = if overlay.stats.tool_calls_error > 0 {
        error_style
    } else {
        dim_style
    };
    let unknown_count_style = if overlay.stats.tool_calls_unknown > 0 {
        unknown_style
    } else {
        dim_style
    };
    let lines_added_style = if overlay.stats.lines_added > 0 {
        added_style
    } else {
        dim_style
    };
    let lines_removed_style = if overlay.stats.lines_removed > 0 {
        removed_style
    } else {
        dim_style
    };

    let mut lines: Vec<Line<'static>> = Vec::new();
    let title_prefix = "Title: ";
    let title_budget = max_line_width.saturating_sub(UnicodeWidthStr::width(title_prefix));
    let title_value = truncate_end(&overlay.session.title, title_budget);
    lines.push(Line::from(vec![
        Span::styled(title_prefix, label_style),
        Span::styled(title_value, value_style.add_modifier(Modifier::BOLD)),
    ]));

    let id_prefix = "Session ID: ";
    let id_budget = max_line_width.saturating_sub(UnicodeWidthStr::width(id_prefix));
    let id_value = truncate_middle(&overlay.session.meta.id, id_budget);
    lines.push(Line::from(vec![
        Span::styled(id_prefix, label_style),
        Span::styled(id_value, dim_style),
    ]));

    let project_prefix = "Project: ";
    let project_budget = max_line_width.saturating_sub(UnicodeWidthStr::width(project_prefix));
    let project_value = truncate_middle(
        &overlay.session.meta.cwd.display().to_string(),
        project_budget,
    );
    lines.push(Line::from(vec![
        Span::styled(project_prefix, label_style),
        Span::styled(project_value, path_style),
    ]));

    let started_prefix = "Started: ";
    let started_budget = max_line_width.saturating_sub(UnicodeWidthStr::width(started_prefix));
    let started_value = truncate_end(&overlay.session.meta.started_at_rfc3339, started_budget);
    lines.push(Line::from(vec![
        Span::styled(started_prefix, label_style),
        Span::styled(started_value, value_style),
    ]));
    if let Some(modified) = overlay.session.file_modified {
        let modified_prefix = "Modified: ";
        let modified_budget =
            max_line_width.saturating_sub(UnicodeWidthStr::width(modified_prefix));
        let modified_value = truncate_end(&relative_time_ago(Some(modified)), modified_budget);
        lines.push(Line::from(vec![
            Span::styled(modified_prefix, label_style),
            Span::styled(modified_value, value_style),
        ]));
    }

    let log_size_prefix = "Log size: ";
    let log_size_budget = max_line_width.saturating_sub(UnicodeWidthStr::width(log_size_prefix));
    let log_size_human = format_size(overlay.session.file_size_bytes, DECIMAL);
    let log_size_bytes = format_commas_u64(overlay.session.file_size_bytes);
    let log_size_value = format!("{log_size_human} ({log_size_bytes})");
    if UnicodeWidthStr::width(log_size_value.as_str()) <= log_size_budget {
        lines.push(Line::from(vec![
            Span::styled(log_size_prefix, label_style),
            Span::styled(log_size_human, value_style.add_modifier(Modifier::BOLD)),
            Span::styled(" (", dim_style),
            Span::styled(log_size_bytes, dim_style),
            Span::styled(")", dim_style),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled(log_size_prefix, label_style),
            Span::styled(truncate_end(&log_size_value, log_size_budget), value_style),
        ]));
    }

    let log_prefix = "Log: ";
    let log_budget = max_line_width.saturating_sub(UnicodeWidthStr::width(log_prefix));
    let log_value = truncate_middle(&overlay.session.log_path.display().to_string(), log_budget);
    lines.push(Line::from(vec![
        Span::styled(log_prefix, label_style),
        Span::styled(log_value, path_style),
    ]));
    lines.push(Line::from(""));

    lines.push(Line::from(vec![Span::styled("Time", section_style)]));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("Duration: ", label_style),
        duration_span,
    ]));
    lines.push(Line::from(""));

    lines.push(Line::from(vec![Span::styled("Tokens", section_style)]));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("Total: ", label_style),
        token_total_span,
    ]));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("Last:  ", label_style),
        token_last_span,
    ]));
    lines.push(Line::from(""));

    lines.push(Line::from(vec![Span::styled("Tools", section_style)]));
    lines.push(Line::from(vec![
        Span::styled("Tool calls: ", label_style),
        Span::styled("total ", label_style),
        Span::styled(
            format_commas_usize(overlay.stats.tool_calls_total),
            value_style.add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("success ", label_style),
        Span::styled(
            format_commas_usize(overlay.stats.tool_calls_success),
            success_style,
        ),
        Span::raw("  "),
        Span::styled("invalid ", label_style),
        Span::styled(
            format_commas_usize(overlay.stats.tool_calls_invalid),
            invalid_count_style,
        ),
        Span::raw("  "),
        Span::styled("error ", label_style),
        Span::styled(
            format_commas_usize(overlay.stats.tool_calls_error),
            error_count_style,
        ),
        Span::raw("  "),
        Span::styled("unknown ", label_style),
        Span::styled(
            format_commas_usize(overlay.stats.tool_calls_unknown),
            unknown_count_style,
        ),
    ]));
    if overlay.stats.tools_used.is_empty() {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("(none)", dim_style),
        ]));
    } else {
        for tool in &overlay.stats.tools_used {
            let calls = format_commas_usize(tool.calls);
            let prefix = "  - ";
            let suffix = format!(": {calls}");
            let budget = max_line_width
                .saturating_sub(UnicodeWidthStr::width(prefix))
                .saturating_sub(UnicodeWidthStr::width(suffix.as_str()));
            let tool_name = truncate_end(&tool.name, budget);
            lines.push(Line::from(vec![
                Span::styled(prefix, dim_style),
                Span::styled(tool_name, path_style),
                Span::styled(": ", dim_style),
                Span::styled(calls, value_style.add_modifier(Modifier::BOLD)),
            ]));
        }
    }
    lines.push(Line::from(""));

    lines.push(Line::from(vec![Span::styled(
        "Changes (apply_patch)",
        section_style,
    )]));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled("Calls: ", label_style),
        Span::styled(
            format_commas_usize(overlay.stats.apply_patch_calls),
            value_style.add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("Ops: ", label_style),
        Span::styled(
            format_commas_usize(overlay.stats.apply_patch_operations),
            value_style.add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("Lines: ", label_style),
        Span::styled(
            format!("+{}", format_commas_usize(overlay.stats.lines_added)),
            lines_added_style,
        ),
        Span::raw(" "),
        Span::styled(
            format!("-{}", format_commas_usize(overlay.stats.lines_removed)),
            lines_removed_style,
        ),
        Span::raw("  "),
        Span::styled("Files: ", label_style),
        Span::styled(
            format_commas_usize(overlay.stats.files_changed.len()),
            value_style.add_modifier(Modifier::BOLD),
        ),
    ]));
    if overlay.stats.files_changed.is_empty() {
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("(none)", dim_style),
        ]));
    } else {
        for file in &overlay.stats.files_changed {
            let ops = format_commas_usize(file.operations);
            let prefix = "  - ";
            let suffix = format!(" (ops: {ops})");
            let budget = max_line_width
                .saturating_sub(UnicodeWidthStr::width(prefix))
                .saturating_sub(UnicodeWidthStr::width(suffix.as_str()));
            let file_path = truncate_middle(&file.path, budget);
            lines.push(Line::from(vec![
                Span::styled(prefix, dim_style),
                Span::styled(file_path, path_style),
                Span::styled(" (ops: ", dim_style),
                Span::styled(ops, value_style.add_modifier(Modifier::BOLD)),
                Span::styled(")", dim_style),
            ]));
        }
    }

    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((overlay.scroll, 0));
    frame.render_widget(paragraph, chunks[0]);

    let hint = Paragraph::new("Keys: arrows/PgUp/PgDn=scroll  Esc/Backspace=close")
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    frame.render_widget(hint, chunks[1]);
}

fn selected_turn_context(detail_view: &crate::app::SessionDetailView) -> String {
    let Some(item) = detail_view.items.get(
        detail_view
            .selected
            .min(detail_view.items.len().saturating_sub(1)),
    ) else {
        return "No selection.".to_string();
    };
    let Some(turn_id) = item.turn_id.as_deref() else {
        return "No turn id for this item.".to_string();
    };
    let Some(ctx) = detail_view.turn_contexts.get(turn_id) else {
        return "No turn_context found for this item.".to_string();
    };

    format_turn_context(ctx)
}

fn format_turn_context(ctx: &TurnContextSummary) -> String {
    let mut lines = Vec::new();
    lines.push(format!("turn_id: {}", ctx.turn_id));
    if let Some(cwd) = &ctx.cwd {
        lines.push(format!("cwd: {cwd}"));
    }
    if let Some(model) = &ctx.model {
        lines.push(format!("model: {model}"));
    }
    if let Some(sandbox) = &ctx.sandbox_policy {
        lines.push(format!("sandbox: {sandbox}"));
    }
    if let Some(approval) = &ctx.approval_policy {
        lines.push(format!("approval: {approval}"));
    }
    if let Some(personality) = &ctx.personality {
        lines.push(format!("personality: {personality}"));
    }
    if let Some(len) = ctx.user_instructions_len {
        lines.push(format!("user_instructions: {len} chars"));
    }
    if let Some(len) = ctx.developer_instructions_len {
        lines.push(format!("developer_instructions: {len} chars"));
    }
    lines.join("\n")
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let mut out = String::new();
    for (idx, ch) in text.chars().enumerate() {
        if idx >= max_chars {
            out.push_str("\n…(truncated)");
            break;
        }
        out.push(ch);
    }
    out
}

fn short_id(value: &str) -> String {
    value.chars().take(8).collect()
}

fn inner_area(area: Rect) -> Rect {
    if area.width < 40 || area.height < 12 {
        return area;
    }
    area.inner(Margin {
        vertical: 0,
        horizontal: 2,
    })
}

fn render_help_overlay(frame: &mut Frame, area: Rect) {
    let popup = centered_rect(74, 70, area);
    frame.render_widget(Clear, popup);

    let title = format!("{} v{}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"));
    let text = vec![
        Line::from(vec![Span::styled(
            title,
            Style::default().add_modifier(Modifier::BOLD),
        )]),
        Line::from("Manage coding-agent sessions (Codex + Claude)."),
        Line::from(
            "Browse projects/sessions, view timelines, spawn sessions, and keep ccbox updated.",
        ),
        Line::from(""),
        Line::from("Navigation"),
        Line::from("  - Arrows: move selection"),
        Line::from("  - PgUp/PgDn: page up/down"),
        Line::from("  - Enter: open"),
        Line::from("  - Esc: back / close windows"),
        Line::from("  - Delete confirm: ←/→ choose, Enter confirms (Esc cancels)"),
        Line::from(""),
        Line::from("Global"),
        Line::from("  - Ctrl+R: rescan sessions"),
        Line::from("  - F2: system menu"),
        Line::from("  - P: processes"),
        Line::from("  - Auto-rescan: watches sessions dir"),
        Line::from("  - Ctrl+Q or Ctrl+C: quit"),
        Line::from(""),
        Line::from("View-specific"),
        Line::from("  - Projects: type to filter, Esc clears filter"),
        Line::from("  - Projects: Del deletes project logs"),
        Line::from("  - Projects: Space shows Result (newest session Out)"),
        Line::from("  - Projects: F3 shows Statistics"),
        Line::from("  - Sessions: type to filter, Esc clears filter"),
        Line::from("  - Sessions: Del deletes session log (Backspace edits filter)"),
        Line::from("  - Sessions: Space shows Result (last Out)"),
        Line::from("  - Sessions: Ctrl+N/Cmd+N opens New Session"),
        Line::from("  - Sessions: F3 shows Stats"),
        Line::from("  - New Session: Ctrl+Enter/Cmd+Enter sends, Shift+Tab switches engine"),
        Line::from("  - Projects/Sessions: ● indicates online"),
        Line::from("  - Session Detail: Tab switches focus (Timeline / Details)"),
        Line::from("  - Session Detail: o shows Result (last Out)"),
        Line::from("  - Session Detail: F3 shows Stats"),
        Line::from("  - Session Detail: Enter jumps to ToolOut for Tool calls"),
        Line::from("  - Session Detail: c toggles Visible Context"),
        Line::from("  - Processes: s/e/l=open output, k=kill, Enter=open session"),
        Line::from(""),
        Line::from("Help"),
        Line::from("  - F1 or ?: toggle this help"),
    ];

    let paragraph = Paragraph::new(text).wrap(Wrap { trim: false }).block(
        Block::default()
            .borders(Borders::ALL)
            .padding(Padding::horizontal(1))
            .title("Help (F1 or ? to close)"),
    );
    frame.render_widget(paragraph, popup);
}

fn render_delete_confirm_overlay(
    frame: &mut Frame,
    area: Rect,
    model: &AppModel,
    confirm: &crate::app::DeleteConfirmDialog,
) {
    let popup = centered_rect(72, 44, area);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .padding(Padding::horizontal(1))
        .title("Delete Project Logs");
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(inner);

    let max_line_width = (chunks[0].width as usize).saturating_sub(1);
    let sessions_dir = model.data.sessions_dir.display().to_string();
    let sessions_dir = truncate_middle(&sessions_dir, max_line_width);
    let project_path = confirm.project_path.display().to_string();
    let project_path = truncate_middle(&project_path, max_line_width);

    let size = format_size(confirm.total_size_bytes, DECIMAL);
    let session_word = if confirm.session_count == 1 {
        "session"
    } else {
        "sessions"
    };

    let mut message = Vec::new();
    message.push(Line::from(vec![
        Span::raw("Delete session logs for "),
        Span::styled(
            format!("\"{}\"", confirm.project_name),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw("?"),
    ]));
    message.push(Line::from(""));
    message.push(Line::from(format!(
        "Sessions: {} {session_word}  ·  Total: {size}",
        confirm.session_count
    )));
    message.push(Line::from(format!("Project path: {project_path}")));
    message.push(Line::from(""));
    message.push(Line::from(vec![Span::styled(
        "This deletes log files under the Codex sessions directory only.",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )]));
    message.push(Line::from(format!("Sessions dir: {sessions_dir}")));
    message.push(Line::from("Your project folder is not modified."));

    let paragraph = Paragraph::new(message).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, chunks[0]);

    let cancel_style = if confirm.selection == DeleteConfirmSelection::Cancel {
        Style::default()
            .add_modifier(Modifier::REVERSED)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let delete_base = Style::default().fg(Color::Red);
    let delete_style = if confirm.selection == DeleteConfirmSelection::Delete {
        delete_base
            .add_modifier(Modifier::REVERSED)
            .add_modifier(Modifier::BOLD)
    } else {
        delete_base.add_modifier(Modifier::BOLD)
    };

    let buttons = Paragraph::new(Line::from(vec![
        Span::styled("[ Cancel ]", cancel_style),
        Span::raw("   "),
        Span::styled("[ Delete ]", delete_style),
    ]))
    .alignment(Alignment::Center);
    frame.render_widget(buttons, chunks[1]);

    let hint = Paragraph::new("Keys: ←/→ choose  Enter confirm  Esc/Backspace cancel  y/n")
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    frame.render_widget(hint, chunks[2]);
}

fn render_delete_session_confirm_overlay(
    frame: &mut Frame,
    area: Rect,
    model: &AppModel,
    confirm: &crate::app::DeleteSessionConfirmDialog,
) {
    let popup = centered_rect(72, 44, area);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .padding(Padding::horizontal(1))
        .title("Delete Session Log");
    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(inner);

    let max_line_width = (chunks[0].width as usize).saturating_sub(1);
    let sessions_dir = model.data.sessions_dir.display().to_string();
    let sessions_dir = truncate_middle(&sessions_dir, max_line_width);
    let project_path = confirm.project_path.display().to_string();
    let project_path = truncate_middle(&project_path, max_line_width);
    let log_file = confirm
        .log_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| confirm.log_path.display().to_string());
    let log_file = truncate_middle(&log_file, max_line_width);

    let size = format_size(confirm.size_bytes, DECIMAL);
    let modified = if confirm.file_modified.is_some() {
        relative_time_ago(confirm.file_modified)
    } else {
        "-".to_string()
    };

    let mut message = Vec::new();
    message.push(Line::from(vec![
        Span::raw("Delete this session log from "),
        Span::styled(
            format!("\"{}\"", confirm.project_name),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw("?"),
    ]));
    message.push(Line::from(""));
    message.push(Line::from(vec![
        Span::raw("Session: "),
        Span::styled(
            truncate_end(&confirm.session_title, max_line_width.saturating_sub(9)),
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ]));
    message.push(Line::from(format!("Size: {size}  ·  Modified: {modified}")));
    message.push(Line::from(format!("Log file: {log_file}")));
    message.push(Line::from(format!("Project path: {project_path}")));
    message.push(Line::from(""));
    message.push(Line::from(vec![Span::styled(
        "This deletes 1 log file under the Codex sessions directory only.",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )]));
    message.push(Line::from(format!("Sessions dir: {sessions_dir}")));
    message.push(Line::from("Your project folder is not modified."));

    let paragraph = Paragraph::new(message).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, chunks[0]);

    let cancel_style = if confirm.selection == DeleteConfirmSelection::Cancel {
        Style::default()
            .add_modifier(Modifier::REVERSED)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let delete_base = Style::default().fg(Color::Red);
    let delete_style = if confirm.selection == DeleteConfirmSelection::Delete {
        delete_base
            .add_modifier(Modifier::REVERSED)
            .add_modifier(Modifier::BOLD)
    } else {
        delete_base.add_modifier(Modifier::BOLD)
    };

    let buttons = Paragraph::new(Line::from(vec![
        Span::styled("[ Cancel ]", cancel_style),
        Span::raw("   "),
        Span::styled("[ Delete ]", delete_style),
    ]))
    .alignment(Alignment::Center);
    frame.render_widget(buttons, chunks[1]);

    let hint = Paragraph::new("Keys: ←/→ choose  Enter confirm  Esc/Backspace cancel  y/n")
        .style(Style::default().fg(Color::DarkGray))
        .alignment(Alignment::Center);
    frame.render_widget(hint, chunks[2]);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
