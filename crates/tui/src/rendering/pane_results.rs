use super::super::*;

pub(super) fn body_lines(app: &TuiApp, body_area: Rect) -> Vec<Line<'static>> {
    let visible_limit = usize::from(body_area.height.saturating_sub(8)).max(1);
    let window_start = app.results_cursor.saturating_sub(visible_limit / 2);
    let rows = app.results.visible_rows(window_start, visible_limit);
    let no_rows = rows.is_empty();

    let mut lines = vec![
        Line::from("Results View (virtualized)"),
        Line::from("Use arrows / hjkl to move cursor."),
    ];
    if app.results_search_mode {
        let query = if app.results_search_query.is_empty() {
            "(type to search)".to_string()
        } else {
            app.results_search_query.clone()
        };
        lines.push(Line::from(format!(
            "Search: {query} (Enter next, Esc cancel)"
        )));
    } else {
        lines.push(Line::from("Press 7 to search buffered rows."));
    }

    if let Some(state) = &app.pagination_state {
        let strategy = match &state.plan {
            PaginationPlan::Keyset { key_column, .. } => {
                format!("keyset by `{key_column}`")
            }
            PaginationPlan::Offset => "offset fallback".to_string(),
        };
        lines.push(Line::from(format!(
            "Page {} | Strategy: {} | Page size {}",
            state.page_index + 1,
            strategy,
            state.page_size
        )));
    }
    lines.push(Line::from(""));

    if no_rows {
        lines.push(Line::from(if app.query_running {
            "Query running... waiting for rows"
        } else {
            "No rows buffered. Tab to Query Editor + Enter, or use 1 in Schema Explorer."
        }));
    } else {
        let table_width = usize::from(body_area.width.saturating_sub(3));
        let table_lines = build_aligned_results_rows(
            &app.result_columns,
            &rows,
            app.results_cursor,
            window_start,
            table_width,
        );
        lines.extend(table_lines);
    }

    lines
}

fn build_aligned_results_rows(
    headers: &[String],
    rows: &[&QueryRow],
    selected_row: usize,
    window_start: usize,
    table_width: usize,
) -> Vec<Line<'static>> {
    let column_count = headers
        .len()
        .max(rows.iter().map(|row| row.values.len()).max().unwrap_or(0));
    if column_count == 0 {
        return vec![Line::from("Rows have no visible columns.")];
    }

    let prefix_width = 6_usize;
    let available_width = table_width.saturating_sub(prefix_width).max(8);

    let mut visible_columns = column_count;
    while visible_columns > 1 && min_table_width(visible_columns) > available_width {
        visible_columns = visible_columns.saturating_sub(1);
    }

    let mut widths = (0..visible_columns)
        .map(|column_index| {
            let header_len = char_len(&column_label(headers, column_index));
            let value_len = rows
                .iter()
                .filter_map(|row| row.values.get(column_index))
                .map(|value| char_len(value))
                .max()
                .unwrap_or(0);
            header_len.max(value_len).clamp(3, 28)
        })
        .collect::<Vec<_>>();

    let mut total_width =
        widths.iter().copied().sum::<usize>() + visible_columns.saturating_sub(1) * 3;
    while total_width > available_width {
        let mut reduced = false;
        if let Some((index, _)) = widths.iter().enumerate().max_by_key(|(_, width)| **width) {
            if widths[index] > 3 {
                widths[index] -= 1;
                total_width = total_width.saturating_sub(1);
                reduced = true;
            }
        }
        if !reduced {
            break;
        }
    }

    let header_cells = (0..visible_columns)
        .map(|column_index| column_label(headers, column_index))
        .collect::<Vec<_>>();
    let header_row = format_aligned_cells(&header_cells, &widths);
    let mut lines = vec![Line::from(Span::styled(
        format!("      {header_row}"),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    ))];
    lines.push(Line::from(format!(
        "      {}",
        "-".repeat(char_len(&header_row))
    )));

    for (offset, row) in rows.iter().enumerate() {
        let absolute_index = window_start + offset;
        let marker = if absolute_index == selected_row {
            ">"
        } else {
            " "
        };
        let row_cells = (0..visible_columns)
            .map(|column_index| {
                row.values
                    .get(column_index)
                    .cloned()
                    .unwrap_or_else(String::new)
            })
            .collect::<Vec<_>>();
        let row_text = format_aligned_cells(&row_cells, &widths);
        lines.push(Line::from(format!(
            "{marker}{:04} {row_text}",
            absolute_index + 1
        )));
    }

    if visible_columns < column_count {
        lines.push(Line::from(format!(
            "Showing {} of {} columns. Narrow terminal; widen window to see all.",
            visible_columns, column_count
        )));
    }

    lines
}

fn min_table_width(columns: usize) -> usize {
    if columns == 0 {
        return 0;
    }
    columns * 3 + columns.saturating_sub(1) * 3
}

fn column_label(headers: &[String], index: usize) -> String {
    headers
        .get(index)
        .cloned()
        .unwrap_or_else(|| format!("col{}", index + 1))
}

fn format_aligned_cells(cells: &[String], widths: &[usize]) -> String {
    widths
        .iter()
        .enumerate()
        .map(|(index, width)| {
            let value = cells.get(index).map_or("", std::string::String::as_str);
            pad_cell(value, *width)
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn pad_cell(value: &str, width: usize) -> String {
    let mut cell = truncate_cell(value, width);
    let padding = width.saturating_sub(char_len(&cell));
    if padding > 0 {
        cell.push_str(&" ".repeat(padding));
    }
    cell
}

fn truncate_cell(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if char_len(value) <= width {
        return value.to_string();
    }
    if width <= 3 {
        return ".".repeat(width);
    }
    let mut truncated = value.chars().take(width - 3).collect::<String>();
    truncated.push_str("...");
    truncated
}

fn char_len(value: &str) -> usize {
    value.chars().count()
}
