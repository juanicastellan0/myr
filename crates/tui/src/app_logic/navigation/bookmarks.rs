impl TuiApp {
    pub(super) fn save_current_bookmark(&mut self) {
        let query_trimmed = self.query_editor_text.trim();
        if self.selection.table.is_none() && query_trimmed.is_empty() {
            self.status_line =
                "Nothing to bookmark yet (select a table or write a query)".to_string();
            return;
        }

        let mut bookmark = SavedBookmark::new(String::new());
        bookmark.profile_name = self.connected_profile.clone();
        bookmark.database = self.selection.database.clone();
        bookmark.table = self.selection.table.clone();
        bookmark.column = self.selection.column.clone();
        if !query_trimmed.is_empty() {
            bookmark.query = Some(self.query_editor_text.clone());
        }

        let base_name = bookmark_base_name(
            self.connected_profile.as_deref(),
            self.selection.database.as_deref(),
            self.selection.table.as_deref(),
        );

        let Some(store) = self.bookmark_store.as_mut() else {
            self.status_line = "Bookmark storage unavailable on this platform".to_string();
            return;
        };

        let name = next_bookmark_name(store.bookmarks(), &base_name);
        bookmark.name = name.clone();
        store.upsert_bookmark(bookmark);

        match store.persist() {
            Ok(()) => {
                self.bookmark_cycle_index = 0;
                self.status_line = format!(
                    "Saved bookmark `{name}` ({} total)",
                    store.bookmarks().len()
                );
            }
            Err(error) => {
                self.status_line = format!("Bookmark save failed: {error}");
            }
        }
    }

    pub(super) fn open_next_bookmark(&mut self) {
        let bookmarks = match self.bookmark_store.as_ref() {
            Some(store) => store.bookmarks().to_vec(),
            None => {
                self.status_line = "Bookmark storage unavailable on this platform".to_string();
                return;
            }
        };

        if bookmarks.is_empty() {
            self.status_line = "No saved bookmarks found".to_string();
            return;
        }

        let index = self.bookmark_cycle_index % bookmarks.len();
        let bookmark = bookmarks[index].clone();
        self.bookmark_cycle_index = (index + 1) % bookmarks.len();
        self.apply_bookmark(bookmark);
    }

    fn apply_bookmark(&mut self, bookmark: SavedBookmark) {
        if let Some(database) = bookmark.database.as_deref() {
            if let Some(index) = self
                .schema_databases
                .iter()
                .position(|candidate| candidate == database)
            {
                self.selected_database_index = index;
            }
            self.active_database = Some(database.to_string());
            self.selection.database = Some(database.to_string());
            self.reload_tables_for_active_database();
        }

        if let Some(table) = bookmark.table.as_deref() {
            if let Some(index) = self
                .schema_tables
                .iter()
                .position(|candidate| candidate == table)
            {
                self.selected_table_index = index;
            }
            self.selection.table = Some(table.to_string());
            self.reload_columns_for_selected_table();
        }

        if let Some(column) = bookmark.column.as_deref() {
            if let Some(index) = self
                .schema_columns
                .iter()
                .position(|candidate| candidate == column)
            {
                self.selected_column_index = index;
            }
            self.selection.column = Some(column.to_string());
        }

        if let Some(query) = bookmark
            .query
            .as_deref()
            .filter(|query| !query.trim().is_empty())
        {
            self.query_editor_text = query.to_string();
            self.query_cursor = self.query_editor_text.len();
            self.query_history_index = None;
            self.query_history_draft = None;
            self.set_active_pane(Pane::QueryEditor);
        } else {
            self.set_query_editor_to_selected_table();
            self.set_active_pane(Pane::SchemaExplorer);
        }

        self.clear_pagination_state();
        self.status_line = format!("Opened bookmark `{}`", bookmark.name);
    }
    pub(super) fn has_saved_bookmarks(&self) -> bool {
        self.bookmark_store
            .as_ref()
            .is_some_and(|store| !store.bookmarks().is_empty())
    }

}
