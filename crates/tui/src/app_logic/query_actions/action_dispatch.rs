impl TuiApp {
    pub(super) fn invoke_action(&mut self, action_id: ActionId) {
        let context = self.action_context();
        match self.actions.invoke(action_id, &context) {
            Ok(invocation) => self.apply_invocation(action_id, invocation),
            Err(error) => self.status_line = format!("Action error: {error}"),
        }
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
            ActionInvocation::RunHealthDiagnostics => {
                self.run_health_diagnostics();
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
            Pane::ProfileBookmarks => AppView::ConnectionWizard,
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
}
