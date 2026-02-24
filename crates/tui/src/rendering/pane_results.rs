use super::super::*;

pub(super) fn body_lines(app: &TuiApp, body_area: Rect) -> Vec<Line<'static>> {
    let visible_limit = usize::from(body_area.height.saturating_sub(8)).max(1);
    let window_start = app.results_cursor.saturating_sub(visible_limit / 2);
    let rows = app.results.visible_rows(window_start, visible_limit);
    let no_rows = rows.is_empty();

    let mut lines = vec![
        Line::from("Results View (virtualized)"),
        Line::from("Use Up/Down for rows and Left/Right for columns (hjkl also works)."),
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
            app.results_column_cursor,
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
    selected_column: usize,
    window_start: usize,
    table_width: usize,
) -> Vec<Line<'static>> {
    let prefix_width = 6_usize;
    let available_width = table_width.saturating_sub(prefix_width).max(8);
    let Some(layout) = compute_results_layout(headers, rows, selected_column, available_width)
    else {
        return vec![Line::from("Rows have no visible columns.")];
    };

    let active_column_label = column_label(headers, layout.selected_column);
    let mut lines = vec![Line::from(format!(
        "Columns {}-{} / {} | Active col {}: {}",
        layout.column_start + 1,
        layout.column_end,
        layout.column_count,
        layout.selected_column + 1,
        active_column_label
    ))];

    let header_cells = (layout.column_start..layout.column_end)
        .map(|column_index| column_label(headers, column_index))
        .collect::<Vec<_>>();
    let header_row = format_aligned_cells(
        &header_cells,
        &layout.widths,
        Some(layout.selected_relative_column),
    );
    lines.push(Line::from(Span::styled(
        format!("      {header_row}"),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )));
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
        let row_cells = (layout.column_start..layout.column_end)
            .map(|column_index| {
                row.values
                    .get(column_index)
                    .cloned()
                    .unwrap_or_else(String::new)
            })
            .collect::<Vec<_>>();
        let row_text = format_aligned_cells(
            &row_cells,
            &layout.widths,
            Some(layout.selected_relative_column),
        );
        let row_line = format!("{marker}{:04} {row_text}", absolute_index + 1);
        if absolute_index == selected_row {
            lines.push(Line::from(Span::styled(
                row_line,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
        } else {
            lines.push(Line::from(row_line));
        }
    }

    if layout.column_start > 0 || layout.column_end < layout.column_count {
        lines.push(Line::from(format!(
            "Horizontal viewport active: showing columns {}-{} of {}.",
            layout.column_start + 1,
            layout.column_end,
            layout.column_count
        )));
    }

    lines
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResultsLayout {
    column_count: usize,
    column_start: usize,
    column_end: usize,
    selected_column: usize,
    selected_relative_column: usize,
    widths: Vec<usize>,
}

fn compute_results_layout(
    headers: &[String],
    rows: &[&QueryRow],
    selected_column: usize,
    available_width: usize,
) -> Option<ResultsLayout> {
    let column_count = headers
        .len()
        .max(rows.iter().map(|row| row.values.len()).max().unwrap_or(0));
    if column_count == 0 {
        return None;
    }

    let all_widths = (0..column_count)
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

    let selected_column = selected_column.min(column_count.saturating_sub(1));
    let start_hint = selected_column.saturating_sub(2);
    let (column_start, column_end) = visible_column_window(
        &all_widths,
        start_hint,
        selected_column,
        available_width.max(8),
    );

    let mut widths = all_widths[column_start..column_end].to_vec();
    shrink_widths_to_fit(&mut widths, available_width.max(8));

    Some(ResultsLayout {
        column_count,
        column_start,
        column_end,
        selected_column,
        selected_relative_column: selected_column.saturating_sub(column_start),
        widths,
    })
}

fn visible_column_window(
    widths: &[usize],
    start_hint: usize,
    selected_column: usize,
    available_width: usize,
) -> (usize, usize) {
    let mut start = start_hint.min(widths.len().saturating_sub(1));
    loop {
        let end = fit_column_range_from(widths, start, available_width);
        if selected_column < start {
            start = selected_column;
            continue;
        }
        if selected_column >= end {
            start = (start + 1).min(selected_column);
            continue;
        }
        return (start, end);
    }
}

fn fit_column_range_from(widths: &[usize], start: usize, available_width: usize) -> usize {
    let mut total = 0_usize;
    let mut count = 0_usize;
    for width in widths.iter().skip(start).copied() {
        let projected = if count == 0 { width } else { total + 3 + width };
        if projected > available_width && count > 0 {
            break;
        }
        total = projected;
        count += 1;
    }

    (start + count.max(1)).min(widths.len())
}

fn shrink_widths_to_fit(widths: &mut [usize], available_width: usize) {
    let mut total_width =
        widths.iter().copied().sum::<usize>() + widths.len().saturating_sub(1) * 3;
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
}

fn column_label(headers: &[String], index: usize) -> String {
    headers
        .get(index)
        .cloned()
        .unwrap_or_else(|| format!("col{}", index + 1))
}

fn format_aligned_cells(
    cells: &[String],
    widths: &[usize],
    selected_column: Option<usize>,
) -> String {
    widths
        .iter()
        .enumerate()
        .map(|(index, width)| {
            let value = cells.get(index).map_or("", std::string::String::as_str);
            if Some(index) == selected_column {
                emphasize_cell(value, *width)
            } else {
                pad_cell(value, *width)
            }
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

fn emphasize_cell(value: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if width == 1 {
        return ">".to_string();
    }
    if width == 2 {
        return "[]".to_string();
    }

    let inner_width = width.saturating_sub(2);
    let inner = truncate_cell(value, inner_width);
    let mut cell = format!("[{inner}]");
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

#[cfg(test)]
mod tests {
    use super::{compute_results_layout, format_aligned_cells};
    use crate::QueryRow;

    #[test]
    fn narrow_layout_keeps_selected_column_visible() {
        let headers = vec![
            "id".to_string(),
            "user_id".to_string(),
            "category".to_string(),
            "payload".to_string(),
            "created_at".to_string(),
        ];
        let rows = vec![QueryRow::new(vec![
            "1".to_string(),
            "22".to_string(),
            "search".to_string(),
            "long-payload-value".to_string(),
            "2026-02-24 12:00:00".to_string(),
        ])];
        let row_refs = rows.iter().collect::<Vec<_>>();

        let layout =
            compute_results_layout(&headers, &row_refs, 4, 26).expect("layout should be present");

        assert!(layout.column_start > 0);
        assert!(layout.column_end <= layout.column_count);
        assert!(layout.selected_column >= layout.column_start);
        assert!(layout.selected_column < layout.column_end);
    }

    #[test]
    fn selected_column_is_emphasized_in_aligned_output() {
        let cells = vec!["id".to_string(), "value".to_string(), "created".to_string()];
        let widths = vec![4, 7, 7];
        let rendered = format_aligned_cells(&cells, &widths, Some(1));
        assert!(rendered.contains("[value]"));
    }
}
