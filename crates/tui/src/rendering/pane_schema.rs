use super::super::*;
use super::support::relationship_direction_label;

pub(super) fn body_lines(app: &TuiApp, body_area: Rect) -> Vec<Line<'static>> {
    let section_window = usize::from(body_area.height.saturating_sub(10)).clamp(3, 8);
    let mut lines = vec![
        Line::from("Schema Explorer"),
        Line::from("Left/Right: lane focus | Up/Down: selection | 1: preview table"),
        Line::from(format!(
            "Focus lane: {} | Active DB: {}",
            app.schema_lane.label(),
            app.active_database.as_deref().unwrap_or("-")
        )),
        Line::from(""),
    ];

    let database_position = if app.schema_databases.is_empty() {
        "0/0".to_string()
    } else {
        format!(
            "{}/{}",
            app.selected_database_index.saturating_add(1),
            app.schema_databases.len()
        )
    };
    lines.push(Line::from(Span::styled(
        format!("Databases ({database_position})"),
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
    );

    lines.push(Line::from(""));
    let table_position = if app.schema_tables.is_empty() {
        "0/0".to_string()
    } else {
        format!(
            "{}/{}",
            app.selected_table_index.saturating_add(1),
            app.schema_tables.len()
        )
    };
    lines.push(Line::from(Span::styled(
        format!("Tables ({table_position})"),
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
    );

    lines.push(Line::from(""));
    let column_position = if app.schema_columns.is_empty() {
        "0/0".to_string()
    } else {
        format!(
            "{}/{}",
            app.selected_column_index.saturating_add(1),
            app.schema_columns.len()
        )
    };
    lines.push(Line::from(Span::styled(
        format!("Columns ({column_position})"),
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    )));
    append_windowed_schema_items(
        &mut lines,
        &app.schema_columns,
        app.selected_column_index,
        app.schema_lane == SchemaLane::Columns,
        section_window,
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
) {
    if items.is_empty() {
        lines.push(Line::from("  (none)"));
        return;
    }

    let clamped_selected = selected_index.min(items.len().saturating_sub(1));
    let visible = max_visible.max(1).min(items.len());
    let mut start = clamped_selected.saturating_sub(visible / 2);
    if start + visible > items.len() {
        start = items.len().saturating_sub(visible);
    }
    let end = (start + visible).min(items.len());

    if start > 0 {
        lines.push(Line::from(format!("  ... {} above", start)));
    }

    for (offset, item) in items[start..end].iter().enumerate() {
        let index = start + offset;
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

    if end < items.len() {
        lines.push(Line::from(format!("  ... {} more", items.len() - end)));
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
