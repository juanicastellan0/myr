impl TuiApp {
    pub(super) fn handle(&mut self, msg: Msg) {
        if self.exit_confirmation
            && !matches!(
                msg,
                Msg::Quit | Msg::TogglePalette | Msg::Tick | Msg::CancelQuery
            )
        {
            self.status_line =
                "Exit pending. Press Ctrl+C to confirm, F10 to exit now, Esc to cancel."
                    .to_string();
            return;
        }

        if self.error_panel.is_some() {
            self.handle_error_panel_input(msg);
            return;
        }

        if self.results_search_mode {
            match msg {
                Msg::InputChar(ch) => {
                    self.results_search_query.push(ch);
                    self.apply_results_search(false);
                    return;
                }
                Msg::Backspace => {
                    self.results_search_query.pop();
                    self.apply_results_search(false);
                    return;
                }
                Msg::ClearInput => {
                    self.results_search_query.clear();
                    self.apply_results_search(false);
                    return;
                }
                Msg::Submit => {
                    self.apply_results_search(true);
                    return;
                }
                Msg::TogglePalette => {
                    self.results_search_mode = false;
                    self.status_line = "Results search canceled".to_string();
                    return;
                }
                _ => {}
            }
        }

        match msg {
            Msg::Quit => {
                self.should_quit = true;
            }
            Msg::GoConnectionWizard => {
                self.set_active_pane(Pane::ConnectionWizard);
                if self.wizard_form.editing {
                    self.cancel_wizard_edit();
                } else {
                    self.status_line = "Returned to Connection Wizard".to_string();
                }
            }
            Msg::GoProfileBookmarkManager => {
                self.open_profile_bookmark_manager();
            }
            Msg::ToggleHelp => self.show_help = !self.show_help,
            Msg::NextPane => {
                if self.pane == Pane::ConnectionWizard && self.wizard_form.editing {
                    self.status_line =
                        "Finish editing field first (Enter to save, Esc to cancel)".to_string();
                } else {
                    let next_pane = self.pane.next();
                    self.set_active_pane(next_pane);
                    self.status_line = format!("Switched pane to {}", self.pane_name());
                }
            }
            Msg::TogglePalette => {
                if self.exit_confirmation {
                    self.exit_confirmation = false;
                    self.status_line = "Exit canceled".to_string();
                    return;
                }
                if self.show_help {
                    self.show_help = false;
                    self.status_line = "Help closed".to_string();
                    return;
                }
                if self.pane == Pane::ConnectionWizard && self.wizard_form.editing {
                    self.cancel_wizard_edit();
                    return;
                }
                if self.pane == Pane::ProfileBookmarks && self.manager_rename_mode {
                    self.cancel_manager_rename();
                    return;
                }
                self.show_palette = !self.show_palette;
                if self.show_palette {
                    self.palette_query.clear();
                    self.palette_selection = 0;
                }
                self.status_line = if self.show_palette {
                    "Command palette opened".to_string()
                } else {
                    "Command palette closed".to_string()
                };
            }
            Msg::TogglePerfOverlay => {
                self.show_perf_overlay = !self.show_perf_overlay;
                self.status_line = if self.show_perf_overlay {
                    "Perf overlay enabled".to_string()
                } else {
                    "Perf overlay disabled".to_string()
                };
            }
            Msg::ToggleSafeMode => {
                let next_enabled = !self.safe_mode_guard.is_enabled();
                self.safe_mode_guard.set_enabled(next_enabled);
                self.pending_confirmation = None;
                self.status_line = if next_enabled {
                    "Safe mode enabled".to_string()
                } else {
                    "Safe mode disabled".to_string()
                };
            }
            Msg::ToggleSchemaColumnView => self.toggle_schema_column_view_mode(),
            Msg::Submit => self.submit(),
            Msg::Connect => self.connect(),
            Msg::CancelQuery => {
                if !self.query_running {
                    if self.exit_confirmation {
                        self.should_quit = true;
                    } else {
                        self.exit_confirmation = true;
                        self.status_line = "No active query. Exit myr? Press Ctrl+C again to confirm, F10 to exit now, Esc to cancel.".to_string();
                    }
                    return;
                }

                self.cancel_requested = true;
                if let Some(cancellation) = &self.query_cancellation {
                    cancellation.cancel();
                    self.status_line = "Cancelling query...".to_string();
                } else {
                    let audit_sql = self.inflight_query_sql.clone().unwrap_or_default();
                    self.query_running = false;
                    self.query_ticks_remaining = 0;
                    self.append_audit_event(
                        AuditOutcome::Cancelled,
                        &audit_sql,
                        None,
                        None,
                        Some("cancel requested"),
                    );
                    self.status_line = "Cancel requested".to_string();
                }
            }
            Msg::Navigate(direction) => self.navigate(direction),
            Msg::InvokeActionSlot(index) => self.invoke_ranked_action(index),
            Msg::InputChar(ch) => self.handle_input_char(ch),
            Msg::InsertNewline => self.handle_insert_newline(),
            Msg::Backspace => self.handle_backspace(),
            Msg::DeleteSelection => self.delete_manager_selection(),
            Msg::ClearInput => self.handle_clear_input(),
            Msg::Tick => self.on_tick(),
        }
    }

    fn handle_error_panel_input(&mut self, msg: Msg) {
        match msg {
            Msg::Tick => self.on_tick(),
            Msg::TogglePalette => {
                self.error_panel = None;
                self.status_line = "Error panel dismissed".to_string();
            }
            Msg::InvokeActionSlot(0) | Msg::Submit => {
                self.run_primary_error_action();
            }
            Msg::Connect => {
                self.reconnect_from_error_panel();
            }
            Msg::GoConnectionWizard => {
                self.error_panel = None;
                self.set_active_pane(Pane::ConnectionWizard);
                self.status_line = "Returned to Connection Wizard".to_string();
            }
            Msg::Quit => {
                self.should_quit = true;
            }
            _ => {
                self.status_line =
                    "Error panel active: 1 primary action | F5 reconnect | F6 wizard | Esc close"
                        .to_string();
            }
        }
    }

    pub(super) fn on_tick(&mut self) {
        self.loading_tick = self.loading_tick.wrapping_add(1);
        self.pane_flash_ticks = self.pane_flash_ticks.saturating_sub(1);
        self.poll_connect_result();
        self.poll_query_result();

        if self.query_running && self.data_backend.is_none() {
            if self.query_ticks_remaining == 0 {
                let audit_sql = self.inflight_query_sql.clone().unwrap_or_default();
                self.query_running = false;
                self.populate_demo_results();
                self.query_retry_attempts = 0;
                self.inflight_query_sql = None;
                self.last_failed_query = None;
                self.finalize_pagination_after_query();
                self.append_audit_event(
                    AuditOutcome::Succeeded,
                    &audit_sql,
                    Some(self.results.len() as u64),
                    None,
                    None,
                );
                self.status_line = "Query completed".to_string();
            } else {
                self.query_ticks_remaining = self.query_ticks_remaining.saturating_sub(1);
            }
        }

        if self.connect_requested {
            let spinner = spinner_char(self.loading_tick);
            self.status_line = if self.connect_intent == ConnectIntent::AutoReconnect {
                format!("Reconnecting... {spinner}")
            } else {
                format!("Connecting... {spinner}")
            };
        } else if self.query_running && self.query_result_rx.is_some() {
            let spinner = spinner_char(self.loading_tick);
            self.status_line = if self.cancel_requested {
                format!("Cancelling query... {spinner}")
            } else {
                format!("Running query... {spinner}")
            };
        }
    }

    pub(super) fn record_render(&mut self, elapsed: Duration) {
        self.last_render_ms = elapsed.as_secs_f64() * 1_000.0;
        self.recent_render_total_ms += self.last_render_ms;
        self.recent_render_count = self.recent_render_count.saturating_add(1);

        let window_elapsed = self.fps_window_started_at.elapsed();
        if window_elapsed >= Duration::from_secs(1) {
            self.fps = f64::from(self.recent_render_count) / window_elapsed.as_secs_f64();
            self.recent_render_total_ms = 0.0;
            self.recent_render_count = 0;
            self.fps_window_started_at = Instant::now();
        }
    }

    pub(super) fn submit(&mut self) {
        if self.show_palette {
            if let Some(action_id) = self.selected_palette_action() {
                self.invoke_action(action_id);
                self.show_palette = false;
            } else {
                self.status_line = "No matching palette action".to_string();
            }
            return;
        }

        match self.pane {
            Pane::ConnectionWizard => {
                if self.wizard_form.editing {
                    self.commit_wizard_edit();
                } else {
                    self.start_wizard_edit();
                }
            }
            Pane::QueryEditor => {
                if let Some((token, sql)) = self.pending_confirmation.take() {
                    match self.safe_mode_guard.confirm(&token, &sql) {
                        Ok(()) => {
                            self.start_query(sql);
                        }
                        Err(error) => {
                            self.status_line = format!("Confirmation failed: {error}");
                        }
                    }
                    return;
                }
                self.invoke_action(ActionId::RunCurrentQuery);
            }
            Pane::ProfileBookmarks => {
                self.open_manager_selection();
            }
            Pane::SchemaExplorer | Pane::Results => {
                self.status_line = "Nothing to submit in this view".to_string();
            }
        }
    }

    fn connect(&mut self) {
        if self.pane == Pane::ConnectionWizard {
            if self.wizard_form.editing {
                self.commit_wizard_edit();
            }
            if self.connect_requested {
                self.status_line = "Already connecting...".to_string();
            } else {
                self.connect_from_wizard();
            }
        } else if self.pane == Pane::ProfileBookmarks {
            if self.connect_requested {
                self.status_line = "Already connecting...".to_string();
            } else {
                self.connect_from_manager();
            }
        } else {
            self.status_line = "Connect is available in wizard or profiles manager".to_string();
        }
    }

    pub(super) fn connect_from_wizard(&mut self) {
        let profile = match self.wizard_profile() {
            Ok(profile) => profile,
            Err(error) => {
                self.status_line = error;
                return;
            }
        };

        self.start_connect_with_profile(profile, ConnectIntent::Manual);
    }

    fn start_connect_with_profile(&mut self, profile: ConnectionProfile, intent: ConnectIntent) {
        if intent == ConnectIntent::Manual {
            self.reconnect_attempts = 0;
        }
        self.error_panel = None;
        let (tx, rx) = mpsc::channel();
        self.connect_result_rx = Some(rx);
        self.connect_requested = true;
        self.connect_intent = intent;
        self.last_connect_profile = Some(profile.clone());
        self.status_line = if intent == ConnectIntent::AutoReconnect {
            format!(
                "Auto-reconnect {}/{} for `{}`...",
                self.reconnect_attempts.max(1),
                AUTO_RECONNECT_LIMIT,
                profile.name
            )
        } else {
            format!(
                "Connecting to {}:{} as {}...",
                profile.host, profile.port, profile.user
            )
        };

        let _connect_worker = thread::spawn(move || {
            let _ = tx.send(run_connect_worker(profile));
        });
    }

    fn wizard_profile(&self) -> Result<ConnectionProfile, String> {
        let port = self
            .wizard_form
            .port
            .parse::<u16>()
            .map_err(|_| "Invalid port in connection wizard".to_string())?;
        let password_source =
            parse_password_source(&self.wizard_form.password_source).ok_or_else(|| {
                "Invalid password source in connection wizard (use env/keyring)".to_string()
            })?;
        let tls_mode = parse_tls_mode(&self.wizard_form.tls_mode).ok_or_else(|| {
            "Invalid TLS mode in connection wizard (use disabled/prefer/require/verify_identity)"
                .to_string()
        })?;
        let read_only = parse_read_only_flag(&self.wizard_form.read_only).ok_or_else(|| {
            "Invalid read-only mode in connection wizard (use yes/no)".to_string()
        })?;

        let mut profile = ConnectionProfile::new(
            self.wizard_form.profile_name.clone(),
            self.wizard_form.host.clone(),
            self.wizard_form.user.clone(),
        );
        profile.port = port;
        profile.database = if self.wizard_form.database.trim().is_empty() {
            None
        } else {
            Some(self.wizard_form.database.clone())
        };
        profile.password_source = password_source;
        profile.tls_mode = tls_mode;
        profile.read_only = read_only;
        Ok(profile)
    }

    fn poll_connect_result(&mut self) {
        let outcome = match self.connect_result_rx.as_ref() {
            Some(receiver) => match receiver.try_recv() {
                Ok(outcome) => Some(outcome),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(ConnectWorkerOutcome::Failure(
                    "connect worker disconnected".to_string(),
                )),
            },
            None => None,
        };

        let Some(outcome) = outcome else {
            return;
        };

        let intent = self.connect_intent;
        self.connect_result_rx = None;
        self.connect_requested = false;
        self.connect_intent = ConnectIntent::Manual;

        match outcome {
            ConnectWorkerOutcome::Success {
                profile,
                connect_latency,
                databases,
                warning,
            } => {
                self.reconnect_attempts = 0;
                self.apply_connected_profile(profile, connect_latency, databases, warning);
                self.error_panel = None;
                if intent == ConnectIntent::AutoReconnect {
                    if let Some(sql) = self.pending_retry_query.take() {
                        self.start_query(sql);
                    } else {
                        self.status_line = "Auto-reconnect succeeded".to_string();
                    }
                }
            }
            ConnectWorkerOutcome::Failure(error) => {
                if intent == ConnectIntent::AutoReconnect
                    && self.reconnect_attempts < AUTO_RECONNECT_LIMIT
                {
                    if let Some(profile) = self
                        .active_connection_profile
                        .clone()
                        .or_else(|| self.last_connect_profile.clone())
                    {
                        self.reconnect_attempts = self.reconnect_attempts.saturating_add(1);
                        self.start_connect_with_profile(profile, ConnectIntent::AutoReconnect);
                        return;
                    }
                }

                self.pending_retry_query = None;
                self.reconnect_attempts = 0;
                self.status_line = format!("Connect failed: {error}");
                let summary = if intent == ConnectIntent::AutoReconnect {
                    "Auto-reconnect attempts were exhausted".to_string()
                } else {
                    "Connection attempt failed".to_string()
                };
                self.open_error_panel(ErrorKind::Connection, "Connection Error", summary, error);
            }
        }
    }

    pub(super) fn apply_connected_profile(
        &mut self,
        profile: ConnectionProfile,
        connect_latency: Duration,
        databases: Vec<String>,
        warning: Option<String>,
    ) {
        self.last_connection_latency = Some(connect_latency);

        // Keep query execution and schema cache on separate pools so runtime-bound
        // schema refreshes cannot invalidate the active query pool.
        let data_backend = MysqlDataBackend::from_profile(&profile);
        let schema_backend = MysqlDataBackend::from_profile(&profile);
        let schema_cache = SchemaCacheService::new(schema_backend, Duration::from_secs(10));

        let mut active_database = profile.database.clone();
        if active_database.is_none() {
            active_database = databases.first().cloned();
        }

        self.active_connection_profile = Some(profile.clone());
        self.last_connect_profile = Some(profile.clone());
        self.wizard_form = wizard_form_from_profile(&profile);
        self.data_backend = Some(data_backend);
        self.schema_cache = Some(schema_cache);
        self.schema_databases = databases;
        self.schema_database_filter.clear();
        self.schema_table_filter.clear();
        self.schema_column_filter.clear();
        self.selected_database_index = active_database
            .as_deref()
            .and_then(|database| {
                self.schema_databases
                    .iter()
                    .position(|candidate| candidate == database)
            })
            .unwrap_or(0);
        self.active_database = active_database.clone();
        self.connected_profile = Some(profile.name.clone());
        self.selection.database = active_database;
        self.schema_tables.clear();
        self.selected_table_index = 0;
        self.selection.table = None;
        self.schema_columns.clear();
        self.schema_column_schemas.clear();
        self.selected_column_index = 0;
        self.selection.column = None;
        self.schema_relationships.clear();
        self.selected_relationship_index = 0;
        self.reload_tables_for_active_database();
        self.schema_lane = if self.schema_tables.is_empty() {
            SchemaLane::Databases
        } else {
            SchemaLane::Tables
        };
        self.clear_pagination_state();
        self.set_query_editor_to_selected_table();
        self.set_active_pane(Pane::SchemaExplorer);

        let mut notes = Vec::new();
        if let Some(warning) = warning {
            notes.push(warning);
        }

        match self.profile_store.as_mut() {
            Some(store) => {
                let mut profile_to_save = profile.clone();
                if let Some(existing) = store.profile(profile.name.as_str()) {
                    profile_to_save.is_default = existing.is_default;
                    profile_to_save.quick_reconnect = existing.quick_reconnect;
                }
                store.upsert_profile(profile_to_save);
                if let Err(error) = store.persist() {
                    notes.push(format!("profile save failed: {error}"));
                }
            }
            None => match FileProfilesStore::load_default() {
                Ok(mut store) => {
                    let mut profile_to_save = profile.clone();
                    if let Some(existing) = store.profile(profile.name.as_str()) {
                        profile_to_save.is_default = existing.is_default;
                        profile_to_save.quick_reconnect = existing.quick_reconnect;
                    }
                    store.upsert_profile(profile_to_save);
                    if let Err(error) = store.persist() {
                        notes.push(format!("profile save failed: {error}"));
                    }
                }
                Err(error) => notes.push(format!("profile load failed: {error}")),
            },
        }

        let mut status = format!("Connected as `{}` in {:.1?}", profile.name, connect_latency);
        if !notes.is_empty() {
            status.push_str(" (");
            status.push_str(&notes.join("; "));
            status.push(')');
        }
        self.status_line = status;
    }

    pub(super) fn poll_query_result(&mut self) {
        let outcome = match self.query_result_rx.as_ref() {
            Some(receiver) => match receiver.try_recv() {
                Ok(outcome) => Some(outcome),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some(QueryWorkerOutcome::Failure(
                    "query worker disconnected".to_string(),
                )),
            },
            None => None,
        };

        let Some(outcome) = outcome else {
            return;
        };

        self.query_result_rx = None;
        self.query_cancellation = None;
        self.query_running = false;
        let audit_sql = self
            .inflight_query_sql
            .clone()
            .or_else(|| {
                (!self.query_editor_text.trim().is_empty()).then(|| self.query_editor_text.clone())
            })
            .unwrap_or_default();

        match outcome {
            QueryWorkerOutcome::Success {
                results,
                rows_streamed,
                was_cancelled,
                elapsed,
            } => {
                self.results = results;
                self.has_results = !self.results.is_empty();
                self.results_cursor = 0;
                self.results_column_cursor = 0;
                self.results_search_mode = false;
                self.results_search_query.clear();
                self.reset_results_column_focus();
                self.query_retry_attempts = 0;
                self.reconnect_attempts = 0;
                self.inflight_query_sql = None;
                self.last_failed_query = None;
                self.finalize_pagination_after_query();
                let audit_outcome = if was_cancelled {
                    AuditOutcome::Cancelled
                } else {
                    AuditOutcome::Succeeded
                };
                self.append_audit_event(
                    audit_outcome,
                    &audit_sql,
                    Some(rows_streamed),
                    Some(elapsed),
                    None,
                );
                self.status_line = if was_cancelled {
                    format!("Query cancelled after {rows_streamed} rows in {elapsed:.1?}")
                } else {
                    format!("Query returned {rows_streamed} rows in {elapsed:.1?}")
                };
            }
            QueryWorkerOutcome::Failure(error) => {
                self.pending_page_transition = None;
                self.has_results = !self.results.is_empty();
                self.results_search_mode = false;
                self.results_search_query.clear();
                let query_sql = if audit_sql.is_empty() {
                    None
                } else {
                    Some(audit_sql.clone())
                };
                let transient = is_transient_query_error(&error);
                let connection_loss = is_connection_lost_error(&error);
                self.append_audit_event(AuditOutcome::Failed, &audit_sql, None, None, Some(&error));

                if transient
                    && !self.cancel_requested
                    && self.query_retry_attempts < QUERY_RETRY_LIMIT
                {
                    if let Some(sql) = query_sql.clone() {
                        self.query_retry_attempts = self.query_retry_attempts.saturating_add(1);
                        self.status_line = format!(
                            "Transient query failure; retrying ({}/{})...",
                            self.query_retry_attempts, QUERY_RETRY_LIMIT
                        );
                        self.start_query_internal(sql, true);
                        self.cancel_requested = false;
                        return;
                    }
                }

                if connection_loss
                    && !self.cancel_requested
                    && self.reconnect_attempts < AUTO_RECONNECT_LIMIT
                {
                    if let Some(profile) = self
                        .active_connection_profile
                        .clone()
                        .or_else(|| self.last_connect_profile.clone())
                    {
                        self.pending_retry_query = query_sql.clone();
                        self.reconnect_attempts = self.reconnect_attempts.saturating_add(1);
                        self.start_connect_with_profile(profile, ConnectIntent::AutoReconnect);
                        self.status_line = format!(
                            "Connection dropped; reconnecting ({}/{})...",
                            self.reconnect_attempts, AUTO_RECONNECT_LIMIT
                        );
                        self.cancel_requested = false;
                        return;
                    }
                }

                self.query_retry_attempts = 0;
                self.reconnect_attempts = 0;
                self.inflight_query_sql = None;
                self.last_failed_query = query_sql;
                self.pending_retry_query = None;
                self.status_line = format!("Query failed: {error}");
                self.open_error_panel(
                    ErrorKind::Query,
                    "Query Error",
                    "Query execution failed".to_string(),
                    error,
                );
            }
        }

        self.cancel_requested = false;
    }

}
