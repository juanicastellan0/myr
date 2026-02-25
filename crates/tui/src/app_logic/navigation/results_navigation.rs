impl TuiApp {
    pub(super) fn navigate_results(&mut self, direction: DirectionKey) {
        let row_count = self.results.len();
        if row_count == 0 {
            self.status_line = "No buffered rows yet".to_string();
            return;
        }

        let column_count = self.result_column_count();
        match direction {
            DirectionKey::Up => {
                self.results_cursor = self.results_cursor.saturating_sub(1);
            }
            DirectionKey::Down => {
                self.results_cursor = (self.results_cursor + 1).min(row_count.saturating_sub(1));
            }
            DirectionKey::Left => {
                self.results_column_cursor = self.results_column_cursor.saturating_sub(1);
            }
            DirectionKey::Right => {
                self.results_column_cursor =
                    (self.results_column_cursor + 1).min(column_count.saturating_sub(1));
            }
        }
        self.sync_results_column_selection();

        let selected_column = self
            .selection
            .column
            .as_deref()
            .unwrap_or("col?");

        self.status_line = format!(
            "Results cursor: row {} / {} | col {} / {} ({selected_column})",
            self.results_cursor + 1,
            row_count,
            self.results_column_cursor + 1,
            column_count.max(1),
        );
    }

    fn start_results_search(&mut self) {
        if self.results.is_empty() {
            self.results_search_mode = false;
            self.status_line = "No buffered rows yet".to_string();
            return;
        }

        self.set_active_pane(Pane::Results);
        self.results_search_mode = true;
        self.apply_results_search(false);
    }

    fn apply_results_search(&mut self, find_next: bool) {
        let query = self.results_search_query.trim();
        if query.is_empty() {
            self.status_line = "Search results: type text, Enter next, Esc cancel".to_string();
            return;
        }

        let row_count = self.results.len();
        if row_count == 0 {
            self.status_line = "No buffered rows yet".to_string();
            return;
        }

        let start_index = if find_next {
            (self.results_cursor + 1) % row_count
        } else {
            0
        };

        if let Some(index) = self.find_results_match_index(query, start_index) {
            self.results_cursor = index;
            self.status_line = format!(
                "Search matched row {} / {} for `{query}` (Enter next, Esc cancel)",
                index + 1,
                row_count
            );
        } else {
            self.status_line = format!("No match for `{query}` in {row_count} buffered rows");
        }
    }

    fn find_results_match_index(&self, query: &str, start_index: usize) -> Option<usize> {
        if self.results.is_empty() {
            return None;
        }

        let needle = query.to_ascii_lowercase();
        let row_count = self.results.len();
        for offset in 0..row_count {
            let index = (start_index + offset) % row_count;
            let Some(row) = self.results.get(index) else {
                continue;
            };
            if row
                .values
                .iter()
                .any(|value| value.to_ascii_lowercase().contains(&needle))
            {
                return Some(index);
            }
        }

        None
    }

    pub(super) fn populate_demo_results(&mut self) {
        self.results = ResultsRingBuffer::new(RESULT_BUFFER_CAPACITY);
        self.results_cursor = 0;
        self.results_column_cursor = 0;
        self.results_search_mode = false;
        self.results_search_query.clear();
        self.result_columns = vec![
            "id".to_string(),
            "value".to_string(),
            "observed_at".to_string(),
        ];

        let selected_table = self
            .selection
            .table
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        for index in 1..=500 {
            let row = QueryRow::new(vec![
                index.to_string(),
                format!("{selected_table}-value-{index}"),
                format!("2026-02-{day:02}", day = ((index - 1) % 28) + 1),
            ]);
            self.results.push(row);
        }

        self.has_results = true;
        self.results_column_cursor = self
            .result_columns
            .iter()
            .position(|column| column == "value")
            .unwrap_or(0);
        self.sync_results_column_selection();
    }

    pub(super) fn reset_results_column_focus(&mut self) {
        let column_count = self.result_column_count();
        if column_count == 0 {
            self.results_column_cursor = 0;
            self.selection.column = None;
            return;
        }

        if let Some(selected) = self
            .selection
            .column
            .as_deref()
            .and_then(|column| self.result_columns.iter().position(|candidate| candidate == column))
        {
            self.results_column_cursor = selected;
        } else {
            self.results_column_cursor = self.results_column_cursor.min(column_count - 1);
        }
        self.sync_results_column_selection();
    }

    fn sync_results_column_selection(&mut self) {
        let column_count = self.result_column_count();
        if column_count == 0 {
            self.results_column_cursor = 0;
            self.selection.column = None;
            return;
        }

        self.results_column_cursor = self.results_column_cursor.min(column_count - 1);
        self.selection.column = Some(self.result_column_name(self.results_column_cursor));
    }

    fn result_column_count(&self) -> usize {
        self.result_columns
            .len()
            .max(self.results.get(self.results_cursor).map_or(0, |row| row.values.len()))
    }

    fn result_column_name(&self, index: usize) -> String {
        self.result_columns
            .get(index)
            .cloned()
            .unwrap_or_else(|| format!("col{}", index + 1))
    }

}
