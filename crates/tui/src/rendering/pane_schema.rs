use super::super::*;
use super::support::relationship_direction_label;

pub(super) fn body_lines(app: &TuiApp, body_area: Rect) -> Vec<Line<'static>> {
    let section_window = usize::from(body_area.height.saturating_sub(10)).clamp(3, 8);
    let mut lines = vec![
        Line::from("Schema Explorer"),
        Line::from(
            "Left/Right: lane focus | Up/Down: selection | Type to filter lane | F4: column view | 1: preview table",
        ),
        Line::from(format!(
            "Focus lane: {} | Active DB: {}",
            app.schema_lane.label(),
            app.active_database.as_deref().unwrap_or("-")
        )),
        Line::from(""),
    ];

    let database_matches =
        filtered_item_indices(&app.schema_databases, app.schema_database_filter.as_str());
    lines.push(Line::from(Span::styled(
        format!(
            "Databases ({}/{}) | filter `{}`",
            database_matches.len(),
            app.schema_databases.len(),
            display_filter_value(app.schema_database_filter.as_str())
        ),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    append_windowed_schema_items(
        &mut lines,
        &app.schema_databases,
        app.selected_database_index,
        app.schema_lane == SchemaLane::Databases,
        section_window,
        app.schema_database_filter.as_str(),
        None,
    );

    lines.push(Line::from(""));
    let table_matches = filtered_item_indices(&app.schema_tables, app.schema_table_filter.as_str());
    lines.push(Line::from(Span::styled(
        format!(
            "Tables ({}/{}) | filter `{}`",
            table_matches.len(),
            app.schema_tables.len(),
            display_filter_value(app.schema_table_filter.as_str())
        ),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )));
    append_windowed_schema_items(
        &mut lines,
        &app.schema_tables,
        app.selected_table_index,
        app.schema_lane == SchemaLane::Tables,
        section_window,
        app.schema_table_filter.as_str(),
        None,
    );

    lines.push(Line::from(""));
    let column_matches =
        filtered_item_indices(&app.schema_columns, app.schema_column_filter.as_str());
    let column_items = schema_column_items(app);
    lines.push(Line::from(Span::styled(
        format!(
            "Columns ({}/{}) | view {} | filter `{}`",
            column_matches.len(),
            app.schema_columns.len(),
            app.schema_column_view_mode.label(),
            display_filter_value(app.schema_column_filter.as_str())
        ),
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    )));
    append_windowed_schema_items(
        &mut lines,
        &column_items,
        app.selected_column_index,
        app.schema_lane == SchemaLane::Columns,
        section_window,
        app.schema_column_filter.as_str(),
        Some(&app.schema_columns),
    );

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Relationships (Jump to related table action):",
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    )));
    append_windowed_relationship_items(
        &mut lines,
        &app.schema_relationships,
        app.selected_relationship_index,
        section_window.saturating_sub(1).max(2),
    );

    lines
}

fn append_windowed_schema_items(
    lines: &mut Vec<Line<'static>>,
    items: &[String],
    selected_index: usize,
    active: bool,
    max_visible: usize,
    filter: &str,
    filter_source: Option<&[String]>,
) {
    if items.is_empty() {
        lines.push(Line::from("  (none)"));
        return;
    }

    let filtered = filtered_item_indices(filter_source.unwrap_or(items), filter);
    if filtered.is_empty() {
        lines.push(Line::from("  (no matches)"));
        return;
    }
    let clamped_selected = if filtered.contains(&selected_index) {
        selected_index
    } else {
        filtered[0]
    };
    let selected_position = filtered
        .iter()
        .position(|index| *index == clamped_selected)
        .unwrap_or(0);

    let visible = max_visible.max(1).min(filtered.len());
    let mut start = selected_position.saturating_sub(visible / 2);
    if start + visible > filtered.len() {
        start = filtered.len().saturating_sub(visible);
    }
    let end = (start + visible).min(filtered.len());

    if start > 0 {
        lines.push(Line::from(format!("  ... {} above", start)));
    }

    for index in filtered.iter().take(end).skip(start).copied() {
        let item = items.get(index).map_or("", String::as_str);
        let marker = if index == clamped_selected {
            if active {
                ">"
            } else {
                "*"
            }
        } else {
            " "
        };
        let rendered = format!("{marker} {item}");
        if index == clamped_selected {
            lines.push(Line::from(Span::styled(
                rendered,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
        } else {
            lines.push(Line::from(rendered));
        }
    }

    if end < filtered.len() {
        lines.push(Line::from(format!("  ... {} more", filtered.len() - end)));
    }
}

fn schema_column_items(app: &TuiApp) -> Vec<String> {
    if app.schema_column_view_mode == SchemaColumnViewMode::Compact {
        return app.schema_columns.clone();
    }

    if app.schema_column_schemas.len() == app.schema_columns.len() {
        return app
            .schema_column_schemas
            .iter()
            .map(format_column_metadata)
            .collect();
    }

    app.schema_columns.clone()
}

fn format_column_metadata(column: &ColumnSchema) -> String {
    let default_value = column.default_value.as_deref().unwrap_or("-");
    let nullability = if column.nullable { "NULL" } else { "NOT NULL" };
    format!(
        "{} | {} | {} | default {default_value}",
        column.name, column.data_type, nullability
    )
}

fn filtered_item_indices(items: &[String], filter: &str) -> Vec<usize> {
    let needle = filter.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return (0..items.len()).collect();
    }

    items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            item.to_ascii_lowercase()
                .contains(needle.as_str())
                .then_some(index)
        })
        .collect()
}

fn display_filter_value(filter: &str) -> &str {
    let trimmed = filter.trim();
    if trimmed.is_empty() {
        "-"
    } else {
        trimmed
    }
}

fn append_windowed_relationship_items(
    lines: &mut Vec<Line<'static>>,
    relationships: &[TableRelationship],
    selected_index: usize,
    max_visible: usize,
) {
    if relationships.is_empty() {
        lines.push(Line::from("  (none detected)"));
        return;
    }

    let clamped_selected = selected_index.min(relationships.len().saturating_sub(1));
    let visible = max_visible.max(1).min(relationships.len());
    let mut start = clamped_selected.saturating_sub(visible / 2);
    if start + visible > relationships.len() {
        start = relationships.len().saturating_sub(visible);
    }
    let end = (start + visible).min(relationships.len());

    if start > 0 {
        lines.push(Line::from(format!("  ... {} above", start)));
    }

    for (offset, relationship) in relationships[start..end].iter().enumerate() {
        let index = start + offset;
        let marker = if index == clamped_selected { "*" } else { " " };
        let direction = relationship_direction_label(relationship.direction);
        let rendered = format!(
            "{marker} {direction} {}.{} ({}, {} -> {})",
            relationship.related_database,
            relationship.related_table,
            relationship.constraint_name,
            relationship.source_column,
            relationship.related_column
        );
        if index == clamped_selected {
            lines.push(Line::from(Span::styled(
                rendered,
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            )));
        } else {
            lines.push(Line::from(rendered));
        }
    }

    if end < relationships.len() {
        lines.push(Line::from(format!(
            "  ... {} more",
            relationships.len() - end
        )));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_column_view_renders_metadata_lines() {
        let mut app = TuiApp {
            schema_lane: SchemaLane::Columns,
            schema_column_view_mode: SchemaColumnViewMode::Full,
            ..TuiApp::default()
        };
        app.schema_column_filter.clear();
        let lines = body_lines(&app, Rect::new(0, 0, 120, 40));
        let rendered = lines
            .iter()
            .map(line_to_text)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("view full"));
        assert!(rendered.contains("id | bigint unsigned | NOT NULL"));
        assert!(rendered.contains("created_at | timestamp | NOT NULL | default CURRENT_TIMESTAMP"));
    }

    #[test]
    fn format_column_metadata_handles_nullable_defaults() {
        let column = ColumnSchema {
            name: "deleted_at".to_string(),
            data_type: "timestamp".to_string(),
            nullable: true,
            default_value: None,
        };

        assert_eq!(
            format_column_metadata(&column),
            "deleted_at | timestamp | NULL | default -"
        );
    }

    fn line_to_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }
}
