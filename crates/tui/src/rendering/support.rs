use super::super::*;

pub(super) fn centered_rect(width_percent: u16, height_percent: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100_u16 - height_percent) / 2),
            Constraint::Percentage(height_percent),
            Constraint::Percentage((100_u16 - height_percent) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100_u16 - width_percent) / 2),
            Constraint::Percentage(width_percent),
            Constraint::Percentage((100_u16 - width_percent) / 2),
        ])
        .split(vertical[1])[1]
}

pub(super) fn spinner_char(tick: usize) -> char {
    const FRAMES: [char; 4] = ['|', '/', '-', '\\'];
    FRAMES[tick % FRAMES.len()]
}

pub(super) fn pulse_char(tick: usize) -> char {
    const FRAMES: [char; 4] = ['.', 'o', 'O', 'o'];
    FRAMES[tick % FRAMES.len()]
}

pub(super) fn connection_badge_and_marker(
    connection_state: &str,
    tick: usize,
) -> (&'static str, char) {
    match connection_state {
        "CONNECTED" => ("[+]", pulse_char(tick)),
        "CONNECTING" | "RECONNECTING" => ("[~]", spinner_char(tick)),
        _ => ("[x]", if tick.is_multiple_of(2) { '-' } else { ' ' }),
    }
}

pub(super) fn relationship_direction_label(direction: RelationshipDirection) -> &'static str {
    match direction {
        RelationshipDirection::Outbound => "->",
        RelationshipDirection::Inbound => "<-",
    }
}

pub(super) fn demo_relationships(
    database: Option<&str>,
    table: Option<&str>,
) -> Vec<TableRelationship> {
    let db = database.unwrap_or("app");
    match table.unwrap_or_default() {
        "users" => vec![
            TableRelationship {
                direction: RelationshipDirection::Inbound,
                constraint_name: "fk_sessions_users".to_string(),
                source_column: "id".to_string(),
                related_database: db.to_string(),
                related_table: "sessions".to_string(),
                related_column: "user_id".to_string(),
            },
            TableRelationship {
                direction: RelationshipDirection::Inbound,
                constraint_name: "fk_playlists_users".to_string(),
                source_column: "id".to_string(),
                related_database: db.to_string(),
                related_table: "playlists".to_string(),
                related_column: "user_id".to_string(),
            },
            TableRelationship {
                direction: RelationshipDirection::Inbound,
                constraint_name: "fk_events_users".to_string(),
                source_column: "id".to_string(),
                related_database: db.to_string(),
                related_table: "events".to_string(),
                related_column: "user_id".to_string(),
            },
        ],
        "sessions" => vec![TableRelationship {
            direction: RelationshipDirection::Outbound,
            constraint_name: "fk_sessions_users".to_string(),
            source_column: "user_id".to_string(),
            related_database: db.to_string(),
            related_table: "users".to_string(),
            related_column: "id".to_string(),
        }],
        "playlists" => vec![TableRelationship {
            direction: RelationshipDirection::Outbound,
            constraint_name: "fk_playlists_users".to_string(),
            source_column: "user_id".to_string(),
            related_database: db.to_string(),
            related_table: "users".to_string(),
            related_column: "id".to_string(),
        }],
        "events" => vec![TableRelationship {
            direction: RelationshipDirection::Outbound,
            constraint_name: "fk_events_users".to_string(),
            source_column: "user_id".to_string(),
            related_database: db.to_string(),
            related_table: "users".to_string(),
            related_column: "id".to_string(),
        }],
        _ => Vec::new(),
    }
}
