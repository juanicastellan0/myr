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
}
