mod chrome;
mod overlays;
mod pane_connection;
mod pane_query_editor;
mod pane_results;
mod pane_schema;
mod support;

use super::*;

pub(super) fn centered_rect(width_percent: u16, height_percent: u16, area: Rect) -> Rect {
    support::centered_rect(width_percent, height_percent, area)
}

pub(super) fn spinner_char(tick: usize) -> char {
    support::spinner_char(tick)
}

pub(super) fn connection_badge_and_marker(
    connection_state: &str,
    tick: usize,
) -> (&'static str, char) {
    support::connection_badge_and_marker(connection_state, tick)
}

pub(super) fn relationship_direction_label(direction: RelationshipDirection) -> &'static str {
    support::relationship_direction_label(direction)
}

pub(super) fn demo_relationships(
    database: Option<&str>,
    table: Option<&str>,
) -> Vec<TableRelationship> {
    support::demo_relationships(database, table)
}

pub(super) fn render(frame: &mut Frame<'_>, app: &TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Min(8),
            Constraint::Length(4),
        ])
        .split(frame.area());
    let top_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(3)])
        .split(chunks[0]);

    chrome::render_runtime_bar(frame, app, top_chunks[0]);
    chrome::render_tabs_bar(frame, app, top_chunks[1]);

    let body_area = chunks[1];
    let (body_text, query_cursor_screen_position) = match app.pane {
        Pane::ConnectionWizard => (pane_connection::body_lines(app), None),
        Pane::SchemaExplorer => (pane_schema::body_lines(app, body_area), None),
        Pane::Results => (pane_results::body_lines(app, body_area), None),
        Pane::QueryEditor => pane_query_editor::body_lines(app, body_area),
    };

    let body = Paragraph::new(body_text)
        .block(Block::default().borders(Borders::ALL).title("Workspace"))
        .alignment(Alignment::Left);
    frame.render_widget(body, body_area);

    let overlays_visible =
        app.show_palette || app.show_help || app.exit_confirmation || app.error_panel.is_some();
    if !overlays_visible {
        if let Some((x, y)) = query_cursor_screen_position {
            frame.set_cursor_position((x, y));
        }
    }

    let footer = Paragraph::new(vec![
        Line::from(chrome::footer_line(app)),
        Line::from(format!("Status: {}", app.status_line)),
    ])
    .block(Block::default().borders(Borders::ALL).title("Next Actions"));
    frame.render_widget(footer, chunks[2]);

    if app.show_palette {
        overlays::render_palette_popup(frame, app);
    }
    if app.show_help {
        overlays::render_help_popup(frame);
    }
    if app.exit_confirmation {
        overlays::render_exit_popup(frame);
    }
    if app.error_panel.is_some() {
        overlays::render_error_popup(frame, app);
    }
}
