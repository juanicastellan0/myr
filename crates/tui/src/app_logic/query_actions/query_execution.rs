impl TuiApp {
    fn append_audit_event(
        &self,
        outcome: AuditOutcome,
        sql: &str,
        rows_streamed: Option<u64>,
        elapsed: Option<Duration>,
        error: Option<&str>,
    ) {
        let Some(audit_trail) = self.audit_trail.as_ref() else {
            return;
        };

        let record = AuditRecord {
            timestamp_unix_ms: unix_timestamp_millis(),
            profile_name: self
                .active_connection_profile
                .as_ref()
                .map(|profile| profile.name.clone())
                .or_else(|| self.connected_profile.clone()),
            database: self.selection.database.clone().or_else(|| {
                self.active_connection_profile
                    .as_ref()
                    .and_then(|profile| profile.database.clone())
            }),
            outcome,
            sql: compact_sql_for_audit(sql),
            rows_streamed,
            elapsed_ms: elapsed.map(|duration| duration.as_millis()),
            error: error.map(|value| truncate_for_audit(value, AUDIT_ERROR_MAX_CHARS)),
        };
        let _ = audit_trail.append(&record);
    }

    fn start_query(&mut self, sql: String) {
        self.start_query_internal(sql, false);
    }

    fn start_query_internal(&mut self, sql: String, retrying: bool) {
        if !retrying {
            self.query_retry_attempts = 0;
            self.last_failed_query = None;
            self.pending_retry_query = None;
            self.reconnect_attempts = 0;
            self.record_query_history(&sql);
            self.query_history_index = None;
            self.query_history_draft = None;
        }
        self.inflight_query_sql = Some(sql.clone());
        self.query_editor_text = sql;
        self.query_cursor = self.query_editor_text.len();
        self.append_audit_event(
            AuditOutcome::Started,
            &self.query_editor_text,
            None,
            None,
            None,
        );
        self.set_active_pane(Pane::Results);
        self.results_search_mode = false;
        self.results_search_query.clear();
        self.error_panel = None;
        self.cancel_requested = false;
        self.has_results = false;
        self.query_cancellation = None;
        self.query_result_rx = None;

        if let Some(data_backend) = &self.data_backend {
            self.query_running = true;
            self.query_ticks_remaining = 0;
            self.results = ResultsRingBuffer::new(RESULT_BUFFER_CAPACITY);
            let cancellation = CancellationToken::new();
            self.query_cancellation = Some(cancellation.clone());
            let backend = data_backend.clone();
            let sql = self.query_editor_text.clone();
            let (tx, rx) = mpsc::channel();
            self.query_result_rx = Some(rx);

            let _query_worker = thread::spawn(move || {
                let _ = tx.send(run_query_worker(backend, sql, cancellation));
            });

            self.status_line = "Running query...".to_string();
            return;
        }

        self.query_running = true;
        self.query_ticks_remaining = QUERY_DURATION_TICKS;
        self.status_line = "Running query...".to_string();
    }

    fn current_profile_read_only(&self) -> bool {
        self.active_connection_profile
            .as_ref()
            .or(self.last_connect_profile.as_ref())
            .is_some_and(|profile| profile.read_only)
    }

    fn execute_sql_with_guard(&mut self, sql: String) {
        if self.current_profile_read_only() {
            let assessment = assess_sql_safety(&sql);
            if !assessment.is_safe_read_only() {
                self.pending_confirmation = None;
                let blocked_message =
                    "Blocked by read-only profile mode: write/DDL SQL is disabled".to_string();
                self.append_audit_event(
                    AuditOutcome::Blocked,
                    &sql,
                    None,
                    None,
                    Some(&blocked_message),
                );
                self.status_line = blocked_message;
                return;
            }
        }

        match self.safe_mode_guard.evaluate(&sql) {
            GuardDecision::Allow { .. } => {
                self.pending_confirmation = None;
                self.start_query(sql);
            }
            GuardDecision::RequireConfirmation { token, assessment } => {
                self.pending_confirmation = Some((token, sql.clone()));
                self.query_editor_text = sql;
                self.query_cursor = self.query_editor_text.len();
                self.set_active_pane(Pane::QueryEditor);
                self.status_line = format!(
                    "Safe mode confirmation required: {:?}. Press Enter again to confirm.",
                    assessment.reasons
                );
            }
        }
    }

    fn run_health_diagnostics(&mut self) {
        let Some(data_backend) = self.data_backend.clone() else {
            self.status_line = "Health diagnostics failed: not connected".to_string();
            self.open_error_panel(
                ErrorKind::Connection,
                "Health Diagnostics",
                "Connection check failed".to_string(),
                "Connection check: FAILED (no active database connection)\nSchema check: skipped\nQuery smoke: skipped"
                    .to_string(),
            );
            return;
        };

        let schema_ready = self.schema_cache.is_some() && !self.schema_databases.is_empty();
        let schema_message = if schema_ready {
            format!(
                "Schema check: OK ({} databases loaded)",
                self.schema_databases.len()
            )
        } else {
            "Schema check: FAILED (schema cache/databases not loaded)".to_string()
        };

        match run_query_worker(
            data_backend,
            "SELECT 1 AS health_check".to_string(),
            CancellationToken::new(),
        ) {
            QueryWorkerOutcome::Success {
                rows_streamed,
                elapsed,
                ..
            } if schema_ready => {
                self.error_panel = None;
                self.status_line = format!(
                    "Health diagnostics passed: connection OK, schema OK, query smoke {rows_streamed} row(s) in {elapsed:.1?}"
                );
            }
            QueryWorkerOutcome::Success {
                rows_streamed,
                elapsed,
                ..
            } => {
                self.status_line = "Health diagnostics failed: schema check failed".to_string();
                self.open_error_panel(
                    ErrorKind::Connection,
                    "Health Diagnostics",
                    "Schema check failed".to_string(),
                    format!(
                        "Connection check: OK\n{schema_message}\nQuery smoke: OK ({rows_streamed} row(s) in {elapsed:.1?})"
                    ),
                );
            }
            QueryWorkerOutcome::Failure(error) => {
                self.status_line = format!("Health diagnostics failed: {error}");
                self.open_error_panel(
                    ErrorKind::Query,
                    "Health Diagnostics",
                    "Query smoke failed".to_string(),
                    format!(
                        "Connection check: OK\n{schema_message}\nQuery smoke: FAILED ({error})"
                    ),
                );
            }
        }
    }
}
