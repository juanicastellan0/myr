impl Default for TuiApp {
    fn default() -> Self {
        let demo_columns = demo_column_schemas();
        Self {
            actions: ActionsEngine::new(),
            pane: Pane::ConnectionWizard,
            wizard_form: startup_wizard_form(),
            connected_profile: None,
            last_connection_latency: None,
            data_backend: None,
            schema_cache: None,
            schema_databases: vec!["app".to_string()],
            selected_database_index: 0,
            active_database: Some("app".to_string()),
            schema_tables: DEMO_SCHEMA_TABLES
                .iter()
                .map(|table| (*table).to_string())
                .collect(),
            selected_table_index: 0,
            schema_columns: demo_columns
                .iter()
                .map(|column| column.name.clone())
                .collect(),
            schema_column_schemas: demo_columns,
            selected_column_index: 0,
            schema_relationships: demo_relationships(Some("app"), Some("users")),
            selected_relationship_index: 0,
            schema_lane: SchemaLane::Tables,
            schema_column_view_mode: SchemaColumnViewMode::Compact,
            schema_database_filter: String::new(),
            schema_table_filter: String::new(),
            schema_column_filter: String::new(),
            show_help: false,
            show_palette: false,
            palette_query: String::new(),
            palette_selection: 0,
            show_perf_overlay: false,
            last_render_ms: 0.0,
            recent_render_total_ms: 0.0,
            recent_render_count: 0,
            fps: 0.0,
            fps_window_started_at: Instant::now(),
            should_quit: false,
            query_running: false,
            query_ticks_remaining: 0,
            safe_mode_guard: SafeModeGuard::new(true),
            pending_confirmation: None,
            has_results: false,
            result_columns: vec![
                "id".to_string(),
                "value".to_string(),
                "observed_at".to_string(),
            ],
            results_cursor: 0,
            results_column_cursor: 1,
            results_search_mode: false,
            results_search_query: String::new(),
            results: ResultsRingBuffer::new(RESULT_BUFFER_CAPACITY),
            pagination_state: None,
            pending_page_transition: None,
            cancel_requested: false,
            connect_requested: false,
            connect_intent: ConnectIntent::Manual,
            connect_result_rx: None,
            query_result_rx: None,
            query_cancellation: None,
            active_connection_profile: None,
            last_connect_profile: None,
            pending_retry_query: None,
            reconnect_attempts: 0,
            query_retry_attempts: 0,
            inflight_query_sql: None,
            last_failed_query: None,
            error_panel: None,
            loading_tick: 0,
            pane_flash_ticks: 0,
            exit_confirmation: false,
            status_line: "Select a field with Up/Down, press E to edit, F5 to connect".to_string(),
            audit_trail: default_audit_trail(),
            bookmark_store: default_bookmark_store(),
            profile_store: default_profile_store(),
            bookmark_cycle_index: 0,
            manager_lane: ManagerLane::Profiles,
            manager_profile_cursor: 0,
            manager_bookmark_cursor: 0,
            manager_rename_mode: false,
            manager_rename_buffer: String::new(),
            query_editor_text: "SELECT * FROM `users`".to_string(),
            query_cursor: "SELECT * FROM `users`".len(),
            query_history: Vec::new(),
            query_history_index: None,
            query_history_draft: None,
            selection: SchemaSelection {
                database: Some("app".to_string()),
                table: Some("users".to_string()),
                column: Some("id".to_string()),
            },
        }
    }
}

pub(super) fn wizard_form_from_profile(profile: &ConnectionProfile) -> ConnectionWizardForm {
    ConnectionWizardForm {
        profile_name: profile.name.clone(),
        host: profile.host.clone(),
        port: profile.port.to_string(),
        user: profile.user.clone(),
        password_source: match profile.password_source {
            PasswordSource::EnvVar => "env".to_string(),
            PasswordSource::Keyring => "keyring".to_string(),
        },
        database: profile.database.clone().unwrap_or_default(),
        tls_mode: match profile.tls_mode {
            TlsMode::Disabled => "disabled".to_string(),
            TlsMode::Prefer => "prefer".to_string(),
            TlsMode::Require => "require".to_string(),
            TlsMode::VerifyIdentity => "verify_identity".to_string(),
        },
        read_only: if profile.read_only {
            "yes".to_string()
        } else {
            "no".to_string()
        },
        active_field: WizardField::ProfileName,
        editing: false,
        edit_buffer: String::new(),
    }
}

#[cfg(test)]
fn startup_wizard_form() -> ConnectionWizardForm {
    ConnectionWizardForm::default()
}

#[cfg(not(test))]
fn startup_wizard_form() -> ConnectionWizardForm {
    let default_form = ConnectionWizardForm::default();
    let Ok(store) = FileProfilesStore::load_default() else {
        return default_form;
    };

    let profile = store
        .default_profile()
        .or_else(|| store.profile(default_form.profile_name.as_str()))
        .or_else(|| store.quick_reconnect_profile())
        .or_else(|| store.profiles().first());

    profile.map_or(default_form, wizard_form_from_profile)
}
