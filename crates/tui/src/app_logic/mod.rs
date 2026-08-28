use super::*;

include!("default.rs");
include!("runtime/handle.rs");
include!("runtime/application.rs");
include!("runtime/connect.rs");
include!("runtime/query.rs");
include!("navigation.rs");
include!("input.rs");
include!("query_actions/action_dispatch.rs");
include!("query_actions/query_execution.rs");
include!("query_actions/pagination.rs");
include!("query_actions/error_panel.rs");
include!("query_actions/pane_state.rs");

#[cfg(test)]
mod application_sync_tests {
    use myr_application::{AppError, AppSnapshot, ConfirmationSnapshot};
    use myr_core::query_runner::ColumnMeta;

    use super::*;

    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn application_snapshot_maps_domain_state_without_ui_state_leaking_back() {
        let mut app = TuiApp::default();
        app.query_running = true;
        app.inflight_query_sql = Some("SELECT id FROM users".to_string());

        let mut profile = ConnectionProfile::new("local", "localhost", "root");
        profile.read_only = true;
        let mut snapshot = AppSnapshot::default();
        snapshot.profiles = vec![profile];
        snapshot.connection.status = ConnectionStatus::Connected;
        snapshot.connection.profile_name = Some("local".to_string());
        snapshot.schema.databases = vec!["app".to_string(), "analytics".to_string()];
        snapshot.schema.selected_database = Some("analytics".to_string());
        snapshot.schema.tables = vec!["events".to_string()];
        snapshot.schema.selected_table = Some("events".to_string());
        snapshot.schema.columns = vec![ColumnSchema {
            name: "id".to_string(),
            data_type: "bigint".to_string(),
            nullable: false,
            default_value: None,
        }];
        snapshot.schema.relationships = vec![TableRelationship {
            direction: RelationshipDirection::Outbound,
            constraint_name: "fk_account".to_string(),
            source_column: "account_id".to_string(),
            related_database: "app".to_string(),
            related_table: "accounts".to_string(),
            related_column: "id".to_string(),
        }];
        snapshot.results.columns = vec![ColumnMeta::new("id")];
        snapshot.results.rows = vec![QueryRow::from_values(vec![QueryValue::UInt(7)])];
        snapshot.results.rows_seen = 2_005;
        snapshot.results.rows_buffered = 2_000;
        snapshot.results.truncated = true;
        snapshot.last_error = Some(AppError::new(AppErrorKind::Query, "query failed"));

        app.apply_application_snapshot(snapshot);

        assert_eq!(app.connected_profile.as_deref(), Some("local"));
        assert!(app
            .active_connection_profile
            .as_ref()
            .is_some_and(|p| p.read_only));
        assert_eq!(app.selected_database_index, 1);
        assert_eq!(app.selection.table.as_deref(), Some("events"));
        assert_eq!(app.selection.column.as_deref(), Some("id"));
        assert_eq!(app.result_columns, ["id"]);
        assert!(app.has_results);
        assert!(app.status_line.contains("truncated"));
        assert!(app.error_panel.is_some());
        assert!(app.inflight_query_sql.is_none());

        let mut confirmation = AppSnapshot::default();
        confirmation.connection.status = ConnectionStatus::Reconnecting;
        confirmation.query.confirmation = Some(ConfirmationSnapshot {
            operation_id: OperationId(9),
            sql: "DELETE FROM users".to_string(),
            reasons: vec!["DestructiveStatement".to_string()],
        });
        app.apply_application_snapshot(confirmation);
        assert_eq!(app.application_confirmation, Some(OperationId(9)));
        assert_eq!(app.pane, Pane::QueryEditor);
        assert!(app.status_line.contains("confirmation required"));

        app.connected_profile = Some("local".to_string());
        app.apply_application_snapshot(AppSnapshot::default());
        assert!(app.connected_profile.is_none());
        assert_eq!(app.status_line, "Disconnected");
    }
}
