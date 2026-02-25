impl TuiApp {
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
}
