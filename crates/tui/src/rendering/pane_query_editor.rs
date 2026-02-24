use super::super::*;

pub(super) fn body_lines(
    app: &TuiApp,
    body_area: Rect,
) -> (Vec<Line<'static>>, Option<(u16, u16)>) {
    let mut lines = vec![
        Line::from("Query Editor"),
        Line::from("SQL section is editable."),
        Line::from(
            "Enter: run query | Ctrl+Enter: newline | Left/Right: cursor | Up/Down: history",
        ),
        Line::from(""),
        Line::from(Span::styled(
            "SQL (editable):",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    let mut query_cursor_screen_position = None;
    let mut editor_lines = app.query_editor_text.lines().collect::<Vec<_>>();
    if app.query_editor_text.ends_with('\n') {
        editor_lines.push("");
    }

    if editor_lines.is_empty() {
        lines.push(Line::from("  (empty query)"));
    } else {
        let (cursor_line, cursor_col) = app.query_cursor_line_col();
        let line_number_width = editor_lines.len().max(1).to_string().len();
        let prefix_chars = line_number_width + 3;

        for (index, line) in editor_lines.iter().enumerate() {
            let mut rendered_line = (*line).to_string();
            if index + 1 == cursor_line {
                let cursor_char_offset = cursor_col.saturating_sub(1);
                let byte_index = byte_index_for_char_offset(&rendered_line, cursor_char_offset);
                rendered_line.insert(byte_index, '|');
            }
            let numbered = format!(
                "{:>width$} | {}",
                index + 1,
                rendered_line,
                width = line_number_width
            );

            if index + 1 == cursor_line {
                lines.push(Line::from(Span::styled(
                    numbered,
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )));
            } else {
                lines.push(Line::from(numbered));
            }
        }

        let body_left = body_area.x.saturating_add(1);
        let body_top = body_area.y.saturating_add(1);
        let cursor_x = body_left
            .saturating_add(prefix_chars as u16)
            .saturating_add(cursor_col.saturating_sub(1) as u16)
            .min(
                body_area
                    .x
                    .saturating_add(body_area.width.saturating_sub(2)),
            );
        let cursor_y = body_top
            .saturating_add(5_u16)
            .saturating_add(cursor_line.saturating_sub(1) as u16)
            .min(
                body_area
                    .y
                    .saturating_add(body_area.height.saturating_sub(2)),
            );
        query_cursor_screen_position = Some((cursor_x, cursor_y));
    }

    let (cursor_line, cursor_col) = app.query_cursor_line_col();
    let history_state = match app.query_history_index {
        Some(index) => format!("history {} / {}", index + 1, app.query_history.len()),
        None => format!("history {}", app.query_history.len()),
    };
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Metadata:",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(format!(
        "Cursor: line {cursor_line}, col {cursor_col} | {history_state}"
    )));

    (lines, query_cursor_screen_position)
}

fn byte_index_for_char_offset(value: &str, char_offset: usize) -> usize {
    if char_offset == 0 {
        return 0;
    }

    value
        .char_indices()
        .map(|(index, _)| index)
        .nth(char_offset)
        .unwrap_or(value.len())
}
