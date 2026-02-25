use super::super::*;

pub(super) fn body_lines(
    app: &TuiApp,
    body_area: Rect,
) -> (Vec<Line<'static>>, Option<(u16, u16)>) {
    let mut lines = vec![
        Line::from("Query Editor"),
        Line::from("SQL block is editable; metadata below is read-only."),
        Line::from(
            "Enter: run query | Ctrl+Enter: newline | Left/Right: cursor | Up/Down: history",
        ),
        Line::from(""),
        Line::from(Span::styled(
            "SQL (active region):",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )),
    ];

    let mut query_cursor_screen_position = None;
    let mut cursor_row_in_lines = None;
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
        let content_width = query_content_width(body_area, prefix_chars);
        let query_budget = query_line_budget(body_area.height);
        let window = query_window(
            editor_lines.len(),
            cursor_line.saturating_sub(1),
            query_budget,
        );

        lines.push(Line::from(Span::styled(
            format!(
                "{:>width$} | {}",
                "",
                column_ruler(content_width),
                width = line_number_width
            ),
            Style::default().fg(Color::DarkGray),
        )));

        if window.start > 0 {
            lines.push(Line::from(format!("  ... {} lines above", window.start)));
        }

        for (offset, line) in editor_lines[window.start..window.end].iter().enumerate() {
            let index = window.start + offset;
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
                cursor_row_in_lines = Some(lines.len());
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

        if window.end < editor_lines.len() {
            lines.push(Line::from(format!(
                "  ... {} lines below",
                editor_lines.len() - window.end
            )));
        }

        lines.push(Line::from(Span::styled(
            "---- End SQL ----",
            Style::default().fg(Color::Green),
        )));

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
        if let Some(cursor_row) = cursor_row_in_lines {
            let cursor_y = body_top.saturating_add(cursor_row as u16).min(
                body_area
                    .y
                    .saturating_add(body_area.height.saturating_sub(2)),
            );
            query_cursor_screen_position = Some((cursor_x, cursor_y));
        }
    }

    let (cursor_line, cursor_col) = app.query_cursor_line_col();
    let history_state = match app.query_history_index {
        Some(index) => format!("history {} / {}", index + 1, app.query_history.len()),
        None => format!("history {}", app.query_history.len()),
    };
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Metadata (read-only):",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(format!(
        "Cursor: line {cursor_line}, col {cursor_col} | {history_state}"
    )));
    lines.push(Line::from(format!(
        "SQL lines: {} | cursor offset {} bytes",
        editor_lines.len().max(1),
        app.query_cursor.min(app.query_editor_text.len())
    )));

    (lines, query_cursor_screen_position)
}

fn query_content_width(body_area: Rect, prefix_chars: usize) -> usize {
    let editor_width = usize::from(body_area.width.saturating_sub(2));
    editor_width.saturating_sub(prefix_chars).max(1)
}

fn query_line_budget(body_height: u16) -> usize {
    const HEADER_LINES: usize = 5;
    const FOOTER_LINES: usize = 4;
    let max_visible = usize::from(body_height.saturating_sub(2));
    max_visible
        .saturating_sub(HEADER_LINES + FOOTER_LINES)
        .max(3)
}

fn query_window(total_lines: usize, cursor_index: usize, query_line_budget: usize) -> QueryWindow {
    if total_lines == 0 {
        return QueryWindow { start: 0, end: 0 };
    }

    let query_line_budget = query_line_budget.max(1);
    let mut content_budget = query_line_budget.saturating_sub(2).max(1);
    if total_lines > content_budget {
        content_budget = content_budget.saturating_sub(2).max(1);
    }

    let clamped_cursor = cursor_index.min(total_lines.saturating_sub(1));
    let mut start = clamped_cursor.saturating_sub(content_budget / 2);
    if start + content_budget > total_lines {
        start = total_lines.saturating_sub(content_budget);
    }
    let end = (start + content_budget).min(total_lines);

    QueryWindow { start, end }
}

fn column_ruler(width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let mut ruler = String::with_capacity(width);
    for column in 1..=width {
        if column % 10 == 0 {
            ruler.push('|');
        } else if column % 5 == 0 {
            ruler.push('+');
        } else {
            ruler.push('.');
        }
    }
    ruler
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QueryWindow {
    start: usize,
    end: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_editor_includes_ruler_and_sql_boundaries() {
        let app = TuiApp::default();
        let (lines, cursor) = body_lines(&app, Rect::new(0, 0, 100, 24));
        let rendered = lines
            .iter()
            .map(line_to_text)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("SQL (active region):"));
        assert!(rendered.contains("---- End SQL ----"));
        assert!(rendered.contains("Metadata (read-only):"));
        let ruler_fragment = column_ruler(20);
        assert!(rendered.contains(ruler_fragment.as_str()));
        assert!(cursor.is_some());
    }

    #[test]
    fn query_editor_uses_window_indicators_for_long_multiline_sql() {
        let query = (1..=40)
            .map(|index| format!("SELECT {index} AS value"))
            .collect::<Vec<_>>()
            .join("\n");
        let app = TuiApp {
            query_editor_text: query.clone(),
            query_cursor: query.len(),
            ..TuiApp::default()
        };

        let (lines, cursor) = body_lines(&app, Rect::new(0, 0, 80, 14));
        let rendered = lines
            .iter()
            .map(line_to_text)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("lines above"));
        assert!(rendered.contains("Cursor: line 40, col"));
        assert!(cursor.is_some());
    }

    #[test]
    fn query_window_keeps_cursor_in_view() {
        let window = query_window(30, 18, 8);
        assert!(window.start <= 18);
        assert!(18 < window.end);
    }

    fn line_to_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }
}
