impl TuiApp {
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
            self.reset_results_column_focus();
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
            self.reset_results_column_focus();
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
                PageTransition::Reset => keyset_first_page_sql(&target, key_column, state.page_size)
                    .map_err(|error| error.to_string()),
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
}
