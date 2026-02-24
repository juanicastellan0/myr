impl TuiApp {
    pub(super) fn invoke_action(&mut self, action_id: ActionId) {
        let context = self.action_context();
        match self.actions.invoke(action_id, &context) {
            Ok(invocation) => self.apply_invocation(action_id, invocation),
            Err(error) => self.status_line = format!("Action error: {error}"),
        }
    }

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

    pub(super) fn start_preview_paged_query(&mut self, fallback_sql: String) {
        let Some(state) = self.build_preview_pagination_state() else {
            self.clear_pagination_state();
            self.execute_sql_with_guard(fallback_sql);
            return;
        };

        let sql = match self.pagination_sql(&state, PageTransition::Reset) {
            Ok(sql) => sql,
            Err(error) => {
                self.clear_pagination_state();
                self.status_line = format!("Pagination setup failed: {error}");
                return;
            }
        };

        if !self.schema_columns.is_empty() {
            self.result_columns = self.schema_columns.clone();
        }
        self.pagination_state = Some(state);
        self.pending_page_transition = Some(PageTransition::Reset);
        self.execute_sql_with_guard(sql);
    }

    fn run_pagination_transition(&mut self, transition: PageTransition) {
        let Some(state) = self.pagination_state.clone() else {
            self.status_line = "Pagination is not active for the current result set".to_string();
            return;
        };

        if matches!(transition, PageTransition::Previous) && state.page_index == 0 {
            self.status_line = "Already at the first page".to_string();
            return;
        }

        let sql = match self.pagination_sql(&state, transition) {
            Ok(sql) => sql,
            Err(error) => {
                self.status_line = format!("Pagination unavailable: {error}");
                return;
            }
        };

        if !self.schema_columns.is_empty() {
            self.result_columns = self.schema_columns.clone();
        }
        self.pending_page_transition = Some(transition);
        self.execute_sql_with_guard(sql);
    }

    fn pagination_sql(
        &self,
        state: &PaginationState,
        transition: PageTransition,
    ) -> Result<String, String> {
        let target = SqlTarget::new(state.database.as_deref(), state.table.as_str())
            .map_err(|error| error.to_string())?;

        match &state.plan {
            PaginationPlan::Keyset {
                key_column,
                first_key,
                last_key,
            } => match transition {
                PageTransition::Reset => {
                    keyset_first_page_sql(&target, key_column, state.page_size)
                        .map_err(|error| error.to_string())
                }
                PageTransition::Next => {
                    let Some(boundary) = last_key.as_deref() else {
                        return Err("missing keyset boundary for next page".to_string());
                    };
                    keyset_page_sql(
                        &target,
                        key_column,
                        boundary,
                        PaginationDirection::Next,
                        state.page_size,
                    )
                    .map_err(|error| error.to_string())
                }
                PageTransition::Previous => {
                    let Some(boundary) = first_key.as_deref() else {
                        return Err("missing keyset boundary for previous page".to_string());
                    };
                    keyset_page_sql(
                        &target,
                        key_column,
                        boundary,
                        PaginationDirection::Previous,
                        state.page_size,
                    )
                    .map_err(|error| error.to_string())
                }
            },
            PaginationPlan::Offset => {
                let next_index = match transition {
                    PageTransition::Reset => 0,
                    PageTransition::Next => state.page_index.saturating_add(1),
                    PageTransition::Previous => state.page_index.saturating_sub(1),
                };
                let offset = next_index.saturating_mul(state.page_size);
                Ok(offset_page_sql(&target, state.page_size, offset))
            }
        }
    }

    fn build_preview_pagination_state(&self) -> Option<PaginationState> {
        let table = self.selection.table.clone()?;
        let plan = match candidate_key_column(&self.schema_columns) {
            Some(key_column) => PaginationPlan::Keyset {
                key_column,
                first_key: None,
                last_key: None,
            },
            None => PaginationPlan::Offset,
        };

        Some(PaginationState {
            database: self.selection.database.clone(),
            table,
            page_size: PREVIEW_PAGE_SIZE,
            page_index: 0,
            last_page_row_count: 0,
            plan,
        })
    }

    fn finalize_pagination_after_query(&mut self) {
        let Some(transition) = self.pending_page_transition.take() else {
            return;
        };

        let row_count = self.results.len();
        let key_bounds = self
            .pagination_state
            .as_ref()
            .and_then(|state| match &state.plan {
                PaginationPlan::Keyset { key_column, .. } => Some(extract_key_bounds(
                    &self.results,
                    &self.result_columns,
                    key_column,
                )),
                PaginationPlan::Offset => None,
            });

        let Some(state) = self.pagination_state.as_mut() else {
            return;
        };

        state.last_page_row_count = row_count;
        match transition {
            PageTransition::Reset => state.page_index = 0,
            PageTransition::Next => {
                if row_count > 0 {
                    state.page_index = state.page_index.saturating_add(1);
                }
            }
            PageTransition::Previous => {
                if row_count > 0 {
                    state.page_index = state.page_index.saturating_sub(1);
                }
            }
        }

        if let (
            PaginationPlan::Keyset {
                first_key,
                last_key,
                ..
            },
            Some((first, last)),
        ) = (&mut state.plan, key_bounds)
        {
            *first_key = first;
            *last_key = last;
        }
    }

    fn clear_pagination_state(&mut self) {
        self.pagination_state = None;
        self.pending_page_transition = None;
    }

    fn pagination_capabilities(&self) -> (bool, bool, bool) {
        let Some(state) = self.pagination_state.as_ref() else {
            return (false, false, false);
        };

        let can_page_next = self.has_results && state.last_page_row_count >= state.page_size;
        let can_page_previous = state.page_index > 0;
        (true, can_page_next, can_page_previous)
    }

    pub(super) fn apply_invocation(&mut self, action_id: ActionId, invocation: ActionInvocation) {
        match invocation {
            ActionInvocation::RunSql(sql) => {
                if action_id == ActionId::PreviewTable {
                    self.start_preview_paged_query(sql);
                } else {
                    self.clear_pagination_state();
                    self.execute_sql_with_guard(sql);
                }
            }
            ActionInvocation::PaginatePrevious => {
                self.run_pagination_transition(PageTransition::Previous);
            }
            ActionInvocation::PaginateNext => {
                self.run_pagination_transition(PageTransition::Next);
            }
            ActionInvocation::ReplaceQueryEditorText(query) => {
                self.query_editor_text = query;
                self.query_cursor = self.query_editor_text.len();
                self.query_history_index = None;
                self.query_history_draft = None;
                self.set_active_pane(Pane::QueryEditor);
                self.status_line = "Query editor updated".to_string();
            }
            ActionInvocation::InsertQueryEditorText(snippet) => {
                self.set_active_pane(Pane::QueryEditor);
                self.insert_text_at_query_cursor(&snippet);
                self.status_line = "Inserted query snippet".to_string();
            }
            ActionInvocation::CancelQuery => {
                let audit_sql = self.inflight_query_sql.clone().unwrap_or_default();
                self.query_running = false;
                self.query_ticks_remaining = 0;
                self.cancel_requested = true;
                self.append_audit_event(
                    AuditOutcome::Cancelled,
                    &audit_sql,
                    None,
                    None,
                    Some("cancel action"),
                );
                self.status_line = "Query cancelled".to_string();
            }
            ActionInvocation::ExportResults(format) => {
                self.export_results(format);
            }
            ActionInvocation::SaveBookmark => {
                self.save_current_bookmark();
            }
            ActionInvocation::OpenBookmark => {
                self.open_next_bookmark();
            }
            ActionInvocation::JumpToRelatedTable => {
                self.jump_to_next_related_table();
            }
            ActionInvocation::CopyToClipboard(target) => {
                self.status_line = format!("Copy requested: {target:?}");
            }
            ActionInvocation::OpenView(view) => {
                let pane = match view {
                    AppView::ConnectionWizard => Pane::ConnectionWizard,
                    AppView::SchemaExplorer => Pane::SchemaExplorer,
                    AppView::Results => Pane::Results,
                    AppView::QueryEditor => Pane::QueryEditor,
                    AppView::CommandPalette => self.pane,
                };
                self.set_active_pane(pane);
                self.status_line = format!("Switched view to {}", self.pane_name());
            }
            ActionInvocation::SearchBufferedResults => {
                self.start_results_search();
            }
        }
    }

    pub(super) fn action_context(&self) -> ActionContext {
        let view = match self.pane {
            Pane::ConnectionWizard => AppView::ConnectionWizard,
            Pane::SchemaExplorer => AppView::SchemaExplorer,
            Pane::Results => AppView::Results,
            Pane::QueryEditor => AppView::QueryEditor,
        };

        let query_text = if matches!(self.pane, Pane::QueryEditor) || self.query_running {
            Some(self.query_editor_text.clone())
        } else {
            None
        };
        let (pagination_enabled, can_page_next, can_page_previous) = self.pagination_capabilities();

        ActionContext {
            view,
            selection: self.selection.clone(),
            query_text,
            query_running: self.query_running,
            has_results: self.has_results,
            has_related_tables: !self.schema_relationships.is_empty(),
            has_saved_bookmarks: self.has_saved_bookmarks(),
            pagination_enabled,
            can_page_next,
            can_page_previous,
        }
    }

    pub(super) fn open_error_panel(
        &mut self,
        kind: ErrorKind,
        title: impl Into<String>,
        summary: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.error_panel = Some(ErrorPanel {
            kind,
            title: title.into(),
            summary: summary.into(),
            detail: detail.into(),
        });
    }

    fn run_primary_error_action(&mut self) {
        let Some(panel) = self.error_panel.as_ref().cloned() else {
            return;
        };

        if panel.kind == ErrorKind::Query {
            if let Some(sql) = self.last_failed_query.clone() {
                self.error_panel = None;
                self.start_query(sql);
                return;
            }
        }

        self.reconnect_from_error_panel();
    }

    fn reconnect_from_error_panel(&mut self) {
        if self.connect_requested {
            self.status_line = "Already connecting...".to_string();
            return;
        }

        let profile = self
            .active_connection_profile
            .clone()
            .or_else(|| self.last_connect_profile.clone())
            .or_else(|| self.wizard_profile().ok());

        let Some(profile) = profile else {
            self.status_line =
                "Reconnect unavailable: provide a valid connection profile".to_string();
            return;
        };

        self.error_panel = None;
        self.reconnect_attempts = 0;
        self.start_connect_with_profile(profile, ConnectIntent::Manual);
    }

    pub(super) fn can_reconnect_from_error_panel(&self) -> bool {
        self.active_connection_profile.is_some()
            || self.last_connect_profile.is_some()
            || self.wizard_profile().is_ok()
    }

    pub(super) fn pane_tab_index(&self) -> usize {
        match self.pane {
            Pane::ConnectionWizard => 0,
            Pane::SchemaExplorer => 1,
            Pane::Results => 2,
            Pane::QueryEditor => 3,
        }
    }

    pub(super) fn runtime_state_label(&self) -> &'static str {
        if self.connect_requested || self.query_running {
            "BUSY"
        } else {
            "IDLE"
        }
    }

    pub(super) fn connection_state_label(&self) -> &'static str {
        if self.connect_requested {
            if self.connect_intent == ConnectIntent::AutoReconnect {
                "RECONNECTING"
            } else {
                "CONNECTING"
            }
        } else if self.data_backend.is_some() {
            "CONNECTED"
        } else {
            "DISCONNECTED"
        }
    }

    pub(super) fn pane_name(&self) -> &'static str {
        match self.pane {
            Pane::ConnectionWizard => "Connection Wizard",
            Pane::SchemaExplorer => "Schema Explorer",
            Pane::Results => "Results",
            Pane::QueryEditor => "Query Editor",
        }
    }

    fn set_active_pane(&mut self, pane: Pane) {
        if self.pane != pane {
            self.pane = pane;
            self.pane_flash_ticks = PANE_FLASH_DURATION_TICKS;
            if pane != Pane::Results {
                self.results_search_mode = false;
            }
        }
    }
}
