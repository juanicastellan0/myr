impl TuiApp {
    pub(super) fn navigate(&mut self, direction: DirectionKey) {
        if self.show_palette {
            self.navigate_palette(direction);
            return;
        }

        match self.pane {
            Pane::ConnectionWizard => {
                if self.wizard_form.editing {
                    self.status_line =
                        "Finish editing field first (Enter to save, Esc to cancel)".to_string();
                    return;
                }
                match direction {
                    DirectionKey::Up => {
                        self.wizard_form.active_field = self.previous_wizard_field();
                        self.status_line =
                            format!("Wizard field: {}", self.wizard_form.active_field.label());
                    }
                    DirectionKey::Down => {
                        self.wizard_form.active_field = self.wizard_form.active_field.next();
                        self.status_line =
                            format!("Wizard field: {}", self.wizard_form.active_field.label());
                    }
                    DirectionKey::Left | DirectionKey::Right => {
                        self.status_line = "Use Up/Down to select a wizard field".to_string();
                    }
                }
            }
            Pane::SchemaExplorer => self.navigate_schema(direction),
            Pane::Results => self.navigate_results(direction),
            Pane::QueryEditor => match direction {
                DirectionKey::Left => self.move_query_cursor_left(),
                DirectionKey::Right => self.move_query_cursor_right(),
                DirectionKey::Up => self.use_previous_query_from_history(),
                DirectionKey::Down => self.use_next_query_from_history(),
            },
        }
    }

    fn navigate_palette(&mut self, direction: DirectionKey) {
        let entry_count = self.palette_entries().len();
        if entry_count == 0 {
            self.palette_selection = 0;
            return;
        }

        match direction {
            DirectionKey::Up | DirectionKey::Left => {
                self.palette_selection = self.palette_selection.saturating_sub(1);
            }
            DirectionKey::Down | DirectionKey::Right => {
                self.palette_selection = (self.palette_selection + 1).min(entry_count - 1);
            }
        }

        self.status_line = format!("Palette selection: {}", self.palette_selection + 1);
    }

    fn previous_wizard_field(&self) -> WizardField {
        match self.wizard_form.active_field {
            WizardField::ProfileName => WizardField::ReadOnly,
            WizardField::Host => WizardField::ProfileName,
            WizardField::Port => WizardField::Host,
            WizardField::User => WizardField::Port,
            WizardField::PasswordSource => WizardField::User,
            WizardField::Database => WizardField::PasswordSource,
            WizardField::TlsMode => WizardField::Database,
            WizardField::ReadOnly => WizardField::TlsMode,
        }
    }

    fn navigate_schema(&mut self, direction: DirectionKey) {
        match direction {
            DirectionKey::Left => {
                self.schema_lane = self.schema_lane.previous();
                self.status_line = format!("Schema focus: {}", self.schema_lane.label());
            }
            DirectionKey::Right => {
                self.schema_lane = self.schema_lane.next();
                self.status_line = format!("Schema focus: {}", self.schema_lane.label());
            }
            DirectionKey::Up | DirectionKey::Down => match self.schema_lane {
                SchemaLane::Databases => self.navigate_schema_databases(direction),
                SchemaLane::Tables => self.navigate_schema_tables(direction),
                SchemaLane::Columns => self.navigate_schema_columns(direction),
            },
        }
    }

    fn navigate_schema_databases(&mut self, direction: DirectionKey) {
        if self.schema_databases.is_empty() {
            self.status_line = "No databases available".to_string();
            return;
        }

        match direction {
            DirectionKey::Up => {
                self.selected_database_index = self.selected_database_index.saturating_sub(1);
            }
            DirectionKey::Down => {
                let max_index = self.schema_databases.len() - 1;
                self.selected_database_index = (self.selected_database_index + 1).min(max_index);
            }
            DirectionKey::Left | DirectionKey::Right => {}
        }

        self.active_database = self
            .schema_databases
            .get(self.selected_database_index)
            .cloned();
        self.selection.database = self.active_database.clone();
        self.reload_tables_for_active_database();
        self.clear_pagination_state();
        self.set_query_editor_to_selected_table();

        if let Some(database) = &self.active_database {
            self.status_line = format!("Selected database `{database}`");
        }
    }

    fn navigate_schema_tables(&mut self, direction: DirectionKey) {
        if self.schema_tables.is_empty() {
            self.status_line = "No tables available".to_string();
            return;
        }

        match direction {
            DirectionKey::Up => {
                self.selected_table_index = self.selected_table_index.saturating_sub(1);
            }
            DirectionKey::Down => {
                let max_index = self.schema_tables.len() - 1;
                self.selected_table_index = (self.selected_table_index + 1).min(max_index);
            }
            DirectionKey::Left | DirectionKey::Right => {}
        }

        self.selection.table = self.schema_tables.get(self.selected_table_index).cloned();
        self.reload_columns_for_selected_table();
        self.clear_pagination_state();
        self.set_query_editor_to_selected_table();

        if let Some(table) = &self.selection.table {
            self.status_line = format!("Selected table `{table}`");
        }
    }

    fn navigate_schema_columns(&mut self, direction: DirectionKey) {
        if self.schema_columns.is_empty() {
            self.status_line = "No columns available".to_string();
            return;
        }

        match direction {
            DirectionKey::Up => {
                self.selected_column_index = self.selected_column_index.saturating_sub(1);
            }
            DirectionKey::Down => {
                let max_index = self.schema_columns.len() - 1;
                self.selected_column_index = (self.selected_column_index + 1).min(max_index);
            }
            DirectionKey::Left | DirectionKey::Right => {}
        }

        self.selection.column = self.schema_columns.get(self.selected_column_index).cloned();
        if let Some(column) = &self.selection.column {
            self.status_line = format!("Selected column `{column}`");
        }
    }

    fn reload_tables_for_active_database(&mut self) {
        let Some(database_name) = self.active_database.clone() else {
            self.schema_tables.clear();
            self.selected_table_index = 0;
            self.selection.table = None;
            self.reload_columns_for_selected_table();
            return;
        };

        if let Some(schema_cache) = self.schema_cache.as_mut() {
            self.schema_tables = match block_on_result(schema_cache.list_tables(&database_name)) {
                Ok(tables) => tables,
                Err(error) => {
                    self.status_line = format!("Table fetch failed: {error}");
                    Vec::new()
                }
            };
        } else if self.schema_tables.is_empty() {
            self.schema_tables = DEMO_SCHEMA_TABLES
                .iter()
                .map(|table| (*table).to_string())
                .collect();
        }

        self.selected_table_index = 0;
        self.selection.table = self.schema_tables.first().cloned();
        self.reload_columns_for_selected_table();
    }

    pub(super) fn reload_columns_for_selected_table(&mut self) {
        let Some(table_name) = self.selection.table.clone() else {
            self.schema_columns.clear();
            self.selected_column_index = 0;
            self.selection.column = None;
            self.schema_relationships.clear();
            self.selected_relationship_index = 0;
            return;
        };

        if let Some(schema_cache) = self.schema_cache.as_mut() {
            if let Some(database_name) = self.active_database.clone() {
                self.schema_columns =
                    match block_on_result(schema_cache.list_columns(&database_name, &table_name)) {
                        Ok(columns) => columns.into_iter().map(|column| column.name).collect(),
                        Err(error) => {
                            self.status_line = format!("Column fetch failed: {error}");
                            Vec::new()
                        }
                    };
            } else {
                self.schema_columns.clear();
            }
        } else {
            self.schema_columns = DEMO_SCHEMA_COLUMNS
                .iter()
                .map(|column| (*column).to_string())
                .collect();
        }

        self.selected_column_index = 0;
        self.selection.column = self.schema_columns.first().cloned();
        self.reload_relationships_for_selected_table();
    }

    fn reload_relationships_for_selected_table(&mut self) {
        let Some(table_name) = self.selection.table.clone() else {
            self.schema_relationships.clear();
            self.selected_relationship_index = 0;
            return;
        };

        if let Some(schema_cache) = self.schema_cache.as_mut() {
            if let Some(database_name) = self.active_database.clone() {
                self.schema_relationships = match block_on_result(
                    schema_cache.list_related_tables(&database_name, &table_name),
                ) {
                    Ok(relationships) => relationships,
                    Err(error) => {
                        self.status_line = format!("Relationship fetch failed: {error}");
                        Vec::new()
                    }
                };
            } else {
                self.schema_relationships.clear();
            }
        } else {
            self.schema_relationships =
                demo_relationships(self.active_database.as_deref(), Some(table_name.as_str()));
        }

        self.selected_relationship_index = 0;
    }

    fn set_query_editor_to_selected_table(&mut self) {
        let Some(table) = self.selection.table.as_deref() else {
            return;
        };

        let table_sql = quote_identifier(table);
        if let Some(database) = self.selection.database.as_deref() {
            let database_sql = quote_identifier(database);
            self.query_editor_text = format!("SELECT * FROM {database_sql}.{table_sql}");
        } else {
            self.query_editor_text = format!("SELECT * FROM {table_sql}");
        }
        self.query_cursor = self.query_editor_text.len();
        self.query_history_index = None;
        self.query_history_draft = None;
    }

    pub(super) fn navigate_results(&mut self, direction: DirectionKey) {
        let row_count = self.results.len();
        if row_count == 0 {
            self.status_line = "No buffered rows yet".to_string();
            return;
        }

        match direction {
            DirectionKey::Up | DirectionKey::Left => {
                self.results_cursor = self.results_cursor.saturating_sub(1);
            }
            DirectionKey::Down | DirectionKey::Right => {
                self.results_cursor = (self.results_cursor + 1).min(row_count.saturating_sub(1));
            }
        }

        self.status_line = format!(
            "Results cursor: {} / {}",
            self.results_cursor + 1,
            row_count
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
        self.selection.column = Some("value".to_string());
    }

    pub(super) fn export_results(&mut self, format: myr_core::actions_engine::ExportFormat) {
        if !self.has_results || self.results.is_empty() {
            self.status_line = "No results available to export".to_string();
            return;
        }

        let rows = (0..self.results.len())
            .filter_map(|index| self.results.get(index))
            .map(|row| row.values.clone())
            .collect::<Vec<_>>();
        let file_path = export_file_path(match format {
            myr_core::actions_engine::ExportFormat::Csv => "csv",
            myr_core::actions_engine::ExportFormat::Json => "json",
            myr_core::actions_engine::ExportFormat::CsvGzip => "csv.gz",
            myr_core::actions_engine::ExportFormat::JsonGzip => "json.gz",
            myr_core::actions_engine::ExportFormat::JsonLines => "jsonl",
            myr_core::actions_engine::ExportFormat::JsonLinesGzip => "jsonl.gz",
        });

        let result = match format {
            myr_core::actions_engine::ExportFormat::Csv => {
                export_rows_to_csv(&file_path, &self.result_columns, &rows)
            }
            myr_core::actions_engine::ExportFormat::Json => {
                export_rows_to_json(&file_path, &self.result_columns, &rows)
            }
            myr_core::actions_engine::ExportFormat::CsvGzip => export_rows_to_csv_with_options(
                &file_path,
                &self.result_columns,
                &rows,
                ExportCompression::Gzip,
            ),
            myr_core::actions_engine::ExportFormat::JsonGzip => export_rows_to_json_with_options(
                &file_path,
                &self.result_columns,
                &rows,
                JsonExportFormat::Array,
                ExportCompression::Gzip,
            ),
            myr_core::actions_engine::ExportFormat::JsonLines => export_rows_to_json_with_options(
                &file_path,
                &self.result_columns,
                &rows,
                JsonExportFormat::JsonLines,
                ExportCompression::None,
            ),
            myr_core::actions_engine::ExportFormat::JsonLinesGzip => {
                export_rows_to_json_with_options(
                    &file_path,
                    &self.result_columns,
                    &rows,
                    JsonExportFormat::JsonLines,
                    ExportCompression::Gzip,
                )
            }
        };

        match result {
            Ok(row_count) => {
                self.status_line = format!("Exported {row_count} rows to {}", file_path.display());
            }
            Err(error) => {
                self.status_line = format!("Export failed: {error}");
            }
        }
    }

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

    pub(super) fn jump_to_next_related_table(&mut self) {
        if self.schema_relationships.is_empty() {
            self.status_line = "No related tables discovered for the current selection".to_string();
            return;
        }

        let index = self
            .selected_relationship_index
            .min(self.schema_relationships.len().saturating_sub(1));
        let relationship = self.schema_relationships[index].clone();
        self.selected_relationship_index = (index + 1) % self.schema_relationships.len();

        if let Some(database_index) = self
            .schema_databases
            .iter()
            .position(|candidate| candidate == &relationship.related_database)
        {
            self.selected_database_index = database_index;
        }
        self.active_database = Some(relationship.related_database.clone());
        self.selection.database = Some(relationship.related_database.clone());
        self.reload_tables_for_active_database();

        if let Some(table_index) = self
            .schema_tables
            .iter()
            .position(|candidate| candidate == &relationship.related_table)
        {
            self.selected_table_index = table_index;
        }
        self.selection.table = Some(relationship.related_table.clone());
        self.reload_columns_for_selected_table();

        if let Some(column_index) = self
            .schema_columns
            .iter()
            .position(|candidate| candidate == &relationship.related_column)
        {
            self.selected_column_index = column_index;
        }
        self.selection.column = Some(relationship.related_column.clone());
        self.clear_pagination_state();
        self.set_query_editor_to_selected_table();
        self.set_active_pane(Pane::SchemaExplorer);

        let direction = relationship_direction_label(relationship.direction);
        self.status_line = format!(
            "Jumped {direction} `{}`.`{}` via {} ({} -> {})",
            relationship.related_database,
            relationship.related_table,
            relationship.constraint_name,
            relationship.source_column,
            relationship.related_column
        );
    }

    pub(super) fn has_saved_bookmarks(&self) -> bool {
        self.bookmark_store
            .as_ref()
            .is_some_and(|store| !store.bookmarks().is_empty())
    }

    pub(super) fn palette_entries(&self) -> Vec<ActionId> {
        let query = self.palette_query.trim().to_ascii_lowercase();
        let ranked = self.actions.rank_top_n(&self.action_context(), 50);
        if query.is_empty() {
            return ranked.into_iter().map(|action| action.id).collect();
        }

        let mut matches = ranked
            .into_iter()
            .filter_map(|ranked_action| {
                let metadata = self.actions.registry().find(ranked_action.id);
                let title = ranked_action.title.to_ascii_lowercase();
                let description = metadata
                    .map_or("", |action| action.description)
                    .to_ascii_lowercase();
                let search_score = palette_match_score(
                    query.as_str(),
                    title.as_str(),
                    description.as_str(),
                    palette_aliases(ranked_action.id),
                )?;
                let combined_score = search_score * 10_000 + ranked_action.score;
                Some((combined_score, ranked_action.title, ranked_action.id))
            })
            .collect::<Vec<_>>();

        matches.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| left.1.cmp(right.1))
        });
        matches.into_iter().map(|(_, _, action_id)| action_id).collect()
    }

    fn selected_palette_action(&self) -> Option<ActionId> {
        let entries = self.palette_entries();
        entries.get(self.palette_selection).copied()
    }
}

fn palette_aliases(action_id: ActionId) -> &'static [&'static str] {
    match action_id {
        ActionId::PreviewTable => &["preview", "peek", "sample", "pvw"],
        ActionId::JumpToRelatedTable => &["fk", "foreign key", "relationship", "related"],
        ActionId::PreviousPage => &["prev", "back", "page back"],
        ActionId::NextPage => &["next", "forward", "more"],
        ActionId::DescribeTable => &["describe", "desc", "columns", "schema"],
        ActionId::ShowIndexes => &["index", "indexes", "keys"],
        ActionId::ShowCreateTable => &["ddl", "create", "show create"],
        ActionId::CountEstimate => &["count", "estimate", "rows"],
        ActionId::RunCurrentQuery => &["run", "execute", "query"],
        ActionId::ApplyLimit200 => &["limit", "cap rows", "preview limit"],
        ActionId::ExplainQuery => &["explain", "plan", "query plan"],
        ActionId::BuildFilterSortQuery => &["filter", "sort", "where", "order by"],
        ActionId::InsertSelectSnippet => &["snippet", "select template"],
        ActionId::InsertJoinSnippet => &["snippet", "join template"],
        ActionId::CancelRunningQuery => &["cancel", "stop", "abort"],
        ActionId::ExportCsv => &["csv", "export csv"],
        ActionId::ExportJson => &["json", "export json"],
        ActionId::ExportCsvGzip => &["csv.gz", "gzip csv", "compressed csv"],
        ActionId::ExportJsonGzip => &["json.gz", "gzip json", "compressed json"],
        ActionId::ExportJsonLines => &["jsonl", "ndjson", "json lines"],
        ActionId::ExportJsonLinesGzip => &["jsonl.gz", "gzip jsonl", "compressed jsonl"],
        ActionId::SaveBookmark => &["bookmark save", "save view", "favorite"],
        ActionId::OpenBookmark => &["bookmark open", "open view", "load bookmark"],
        ActionId::CopyCell => &["copy cell", "clipboard cell"],
        ActionId::CopyRow => &["copy row", "clipboard row"],
        ActionId::SearchResults => &["search", "find", "grep"],
        ActionId::FocusQueryEditor => &["editor", "sql", "go query editor"],
    }
}

fn palette_match_score(
    query: &str,
    title: &str,
    description: &str,
    aliases: &[&str],
) -> Option<i32> {
    let title_score = text_match_score(query, title).map(|score| score + 30);
    let description_score = text_match_score(query, description);
    let alias_score = aliases
        .iter()
        .filter_map(|alias| text_match_score(query, alias))
        .max()
        .map(|score| score + 15);
    [title_score, description_score, alias_score]
        .into_iter()
        .flatten()
        .max()
}

fn text_match_score(query: &str, text: &str) -> Option<i32> {
    if query.is_empty() || text.is_empty() {
        return None;
    }
    if text == query {
        return Some(1_000);
    }
    if text.starts_with(query) {
        return Some(900);
    }
    if text
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|word| !word.is_empty() && word.starts_with(query))
    {
        return Some(820);
    }
    if text.contains(query) {
        return Some(760);
    }
    fuzzy_subsequence_score(query, text)
}

fn fuzzy_subsequence_score(query: &str, text: &str) -> Option<i32> {
    let mut query_chars = query.chars();
    let mut current = query_chars.next()?;
    let mut matched = 0usize;
    let mut previous_index = 0usize;
    let mut gap_penalty = 0i32;

    for (index, ch) in text.chars().enumerate() {
        if ch != current {
            continue;
        }

        if matched > 0 {
            let gap = index.saturating_sub(previous_index + 1);
            gap_penalty += i32::try_from(gap.min(12)).unwrap_or(12);
        }

        matched += 1;
        previous_index = index;

        if let Some(next) = query_chars.next() {
            current = next;
            continue;
        }

        let length_bonus = i32::try_from(query.chars().count().min(12)).unwrap_or(12) * 8;
        return Some((620 + length_bonus - gap_penalty).max(500));
    }

    None
}
