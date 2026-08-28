impl TuiApp {
    fn sync_application_snapshot(&mut self) {
        let Some(application) = &self.application_handle else {
            return;
        };
        self.apply_application_snapshot(application.snapshot());
    }

    fn apply_application_snapshot(&mut self, snapshot: AppSnapshot) {
        let was_connected = self.connected_profile.is_some();
        let was_query_running = self.query_running;

        self.connect_requested = matches!(
            snapshot.connection.status,
            ConnectionStatus::Connecting | ConnectionStatus::Reconnecting
        );
        match snapshot.connection.status {
            ConnectionStatus::Connected => {
                self.connected_profile = snapshot.connection.profile_name.clone();
                self.connect_requested = false;
                if let Some(profile_name) = snapshot.connection.profile_name.as_deref() {
                    self.active_connection_profile = snapshot
                        .profiles
                        .iter()
                        .find(|profile| profile.name == profile_name)
                        .cloned()
                        .or_else(|| self.last_connect_profile.clone());
                }
                if !was_connected {
                    self.status_line = format!(
                        "Connected to `{}`; select a database to load tables",
                        snapshot
                            .connection
                            .profile_name
                            .as_deref()
                            .unwrap_or("profile")
                    );
                }
            }
            ConnectionStatus::Disconnected => {
                if was_connected {
                    self.status_line = "Disconnected".to_string();
                }
                self.connected_profile = None;
                self.connect_requested = false;
            }
            ConnectionStatus::Connecting
            | ConnectionStatus::Disconnecting
            | ConnectionStatus::Reconnecting => {}
        }

        self.schema_databases = snapshot.schema.databases.clone();
        if let Some(database) = snapshot.schema.selected_database.clone() {
            self.active_database = Some(database.clone());
            self.selection.database = Some(database.clone());
            self.selected_database_index = self
                .schema_databases
                .iter()
                .position(|candidate| candidate == &database)
                .unwrap_or(0);
        }
        self.schema_tables = snapshot.schema.tables.clone();
        if let Some(table) = snapshot.schema.selected_table.clone() {
            self.selection.table = Some(table.clone());
            self.selected_table_index = self
                .schema_tables
                .iter()
                .position(|candidate| candidate == &table)
                .unwrap_or(0);
        } else if self.selection.table.is_none() {
            self.selection.table = self.schema_tables.first().cloned();
        }
        self.schema_column_schemas = snapshot.schema.columns.clone();
        self.schema_columns = self
            .schema_column_schemas
            .iter()
            .map(|column| column.name.clone())
            .collect();
        self.schema_relationships = snapshot.schema.relationships.clone();
        if self.selection.column.is_none() {
            self.selection.column = self.schema_columns.first().cloned();
        }

        if let Some(confirmation) = snapshot.query.confirmation {
            self.application_confirmation = Some(confirmation.operation_id);
            self.query_running = false;
            self.set_active_pane(Pane::QueryEditor);
            self.status_line = format!(
                "Safe mode confirmation required: {}. Press Enter again to confirm.",
                confirmation.reasons.join(", ")
            );
        } else {
            self.query_running = snapshot.query.running;
        }

        self.result_columns = snapshot
            .results
            .columns
            .iter()
            .map(|column| column.name.clone())
            .collect();
        self.results = ResultsRingBuffer::new(RESULT_BUFFER_CAPACITY);
        for row in snapshot.results.rows {
            self.results.push(row);
        }
        self.has_results = !self.results.is_empty();
        if was_query_running && !self.query_running && self.application_confirmation.is_none() {
            self.inflight_query_sql = None;
            self.status_line = format!(
                "Query returned {} rows ({} buffered{})",
                snapshot.results.rows_seen,
                snapshot.results.rows_buffered,
                if snapshot.results.truncated {
                    ", truncated"
                } else {
                    ""
                }
            );
        }

        if let Some(error) = snapshot.last_error {
            if self.error_panel.as_ref().is_none_or(|panel| panel.detail != error.message) {
                let kind = if matches!(
                    error.kind,
                    AppErrorKind::Authentication
                        | AppErrorKind::Connection
                        | AppErrorKind::Tls
                        | AppErrorKind::Timeout
                ) {
                    ErrorKind::Connection
                } else {
                    ErrorKind::Query
                };
                self.open_error_panel(
                    kind,
                    "Application Error",
                    format!("{:?}", error.kind),
                    error.message,
                );
            }
        }
    }
}
