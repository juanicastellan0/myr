impl TuiApp {
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
            .or_else(|| (!self.query_editor_text.trim().is_empty()).then(|| self.query_editor_text.clone()))
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

                if transient && !self.cancel_requested && self.query_retry_attempts < QUERY_RETRY_LIMIT {
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
