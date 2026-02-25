impl TuiApp {
    fn navigate_schema(&mut self, direction: DirectionKey) {
        match direction {
            DirectionKey::Left => {
                self.schema_lane = self.schema_lane.previous();
                self.status_line = format!(
                    "Schema focus: {} (filter: `{}`)",
                    self.schema_lane.label(),
                    self.active_schema_filter()
                );
            }
            DirectionKey::Right => {
                self.schema_lane = self.schema_lane.next();
                self.status_line = format!(
                    "Schema focus: {} (filter: `{}`)",
                    self.schema_lane.label(),
                    self.active_schema_filter()
                );
            }
            DirectionKey::Up | DirectionKey::Down => match self.schema_lane {
                SchemaLane::Databases => self.navigate_schema_databases(direction),
                SchemaLane::Tables => self.navigate_schema_tables(direction),
                SchemaLane::Columns => self.navigate_schema_columns(direction),
            },
        }
    }

    pub(super) fn append_schema_filter_char(&mut self, ch: char) {
        if !ch.is_ascii_graphic() && ch != ' ' {
            return;
        }
        self.active_schema_filter_mut().push(ch);
        self.apply_active_schema_filter();
    }

    pub(super) fn backspace_schema_filter(&mut self) {
        self.active_schema_filter_mut().pop();
        self.apply_active_schema_filter();
    }

    pub(super) fn clear_schema_filter(&mut self) {
        self.active_schema_filter_mut().clear();
        self.apply_active_schema_filter();
    }

    pub(super) fn toggle_schema_column_view_mode(&mut self) {
        if self.pane != Pane::SchemaExplorer {
            self.status_line =
                "Schema column view toggle is available in Schema Explorer".to_string();
            return;
        }

        self.schema_column_view_mode = self.schema_column_view_mode.toggle();
        self.status_line = format!(
            "Schema columns view: {}",
            self.schema_column_view_mode.label()
        );
    }

    fn active_schema_filter(&self) -> &str {
        match self.schema_lane {
            SchemaLane::Databases => self.schema_database_filter.as_str(),
            SchemaLane::Tables => self.schema_table_filter.as_str(),
            SchemaLane::Columns => self.schema_column_filter.as_str(),
        }
    }

    fn active_schema_filter_mut(&mut self) -> &mut String {
        match self.schema_lane {
            SchemaLane::Databases => &mut self.schema_database_filter,
            SchemaLane::Tables => &mut self.schema_table_filter,
            SchemaLane::Columns => &mut self.schema_column_filter,
        }
    }

    fn apply_active_schema_filter(&mut self) {
        match self.schema_lane {
            SchemaLane::Databases => self.apply_database_filter(),
            SchemaLane::Tables => self.apply_table_filter(),
            SchemaLane::Columns => self.apply_column_filter(),
        }
    }

    fn apply_database_filter(&mut self) {
        if self.schema_databases.is_empty() {
            self.status_line = "No databases available".to_string();
            return;
        }

        let filtered = filtered_schema_indices(
            &self.schema_databases,
            self.schema_database_filter.as_str(),
        );
        if filtered.is_empty() {
            self.status_line = format!(
                "Database filter `{}` matched 0 entries",
                self.schema_database_filter
            );
            return;
        }

        self.selected_database_index = filtered[0];
        self.active_database = self
            .schema_databases
            .get(self.selected_database_index)
            .cloned();
        self.selection.database = self.active_database.clone();
        self.reload_tables_for_active_database();
        self.clear_pagination_state();
        self.set_query_editor_to_selected_table();
        self.status_line = format!(
            "Database filter `{}` matched {} entries",
            self.schema_database_filter,
            filtered.len()
        );
    }

    fn apply_table_filter(&mut self) {
        if self.schema_tables.is_empty() {
            self.status_line = "No tables available".to_string();
            return;
        }

        let filtered =
            filtered_schema_indices(&self.schema_tables, self.schema_table_filter.as_str());
        if filtered.is_empty() {
            self.status_line = format!("Table filter `{}` matched 0 entries", self.schema_table_filter);
            return;
        }

        self.selected_table_index = filtered[0];
        self.selection.table = self.schema_tables.get(self.selected_table_index).cloned();
        self.reload_columns_for_selected_table();
        self.clear_pagination_state();
        self.set_query_editor_to_selected_table();
        self.status_line = format!(
            "Table filter `{}` matched {} entries",
            self.schema_table_filter,
            filtered.len()
        );
    }

    fn apply_column_filter(&mut self) {
        if self.schema_columns.is_empty() {
            self.status_line = "No columns available".to_string();
            return;
        }

        let filtered =
            filtered_schema_indices(&self.schema_columns, self.schema_column_filter.as_str());
        if filtered.is_empty() {
            self.status_line = format!(
                "Column filter `{}` matched 0 entries",
                self.schema_column_filter
            );
            return;
        }

        self.selected_column_index = filtered[0];
        self.selection.column = self.schema_columns.get(self.selected_column_index).cloned();
        self.clear_pagination_state();
        let selected = self.selection.column.as_deref().unwrap_or("-");
        self.status_line = format!(
            "Column filter `{}` matched {} entries (active `{selected}`)",
            self.schema_column_filter,
            filtered.len()
        );
    }

    fn navigate_schema_databases(&mut self, direction: DirectionKey) {
        if self.schema_databases.is_empty() {
            self.status_line = "No databases available".to_string();
            return;
        }

        let filtered = filtered_schema_indices(
            &self.schema_databases,
            self.schema_database_filter.as_str(),
        );
        if filtered.is_empty() {
            self.status_line = format!(
                "No databases match filter `{}`",
                self.schema_database_filter
            );
            return;
        }

        match direction {
            DirectionKey::Up => {
                self.selected_database_index = previous_filtered_index(
                    &filtered,
                    self.selected_database_index,
                );
            }
            DirectionKey::Down => {
                self.selected_database_index = next_filtered_index(&filtered, self.selected_database_index);
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

        let filtered = filtered_schema_indices(&self.schema_tables, self.schema_table_filter.as_str());
        if filtered.is_empty() {
            self.status_line = format!("No tables match filter `{}`", self.schema_table_filter);
            return;
        }

        match direction {
            DirectionKey::Up => {
                self.selected_table_index = previous_filtered_index(&filtered, self.selected_table_index);
            }
            DirectionKey::Down => {
                self.selected_table_index = next_filtered_index(&filtered, self.selected_table_index);
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

        let filtered =
            filtered_schema_indices(&self.schema_columns, self.schema_column_filter.as_str());
        if filtered.is_empty() {
            self.status_line = format!("No columns match filter `{}`", self.schema_column_filter);
            return;
        }

        match direction {
            DirectionKey::Up => {
                self.selected_column_index =
                    previous_filtered_index(&filtered, self.selected_column_index);
            }
            DirectionKey::Down => {
                self.selected_column_index = next_filtered_index(&filtered, self.selected_column_index);
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
        if let Some(filtered_index) =
            first_filtered_index(&self.schema_tables, self.schema_table_filter.as_str())
        {
            self.selected_table_index = filtered_index;
            self.selection.table = self.schema_tables.get(filtered_index).cloned();
        }
        self.reload_columns_for_selected_table();
    }

    pub(super) fn reload_columns_for_selected_table(&mut self) {
        let Some(table_name) = self.selection.table.clone() else {
            self.schema_columns.clear();
            self.schema_column_schemas.clear();
            self.selected_column_index = 0;
            self.selection.column = None;
            self.schema_relationships.clear();
            self.selected_relationship_index = 0;
            return;
        };

        if let Some(schema_cache) = self.schema_cache.as_mut() {
            if let Some(database_name) = self.active_database.clone() {
                self.schema_column_schemas =
                    match block_on_result(schema_cache.list_columns(&database_name, &table_name)) {
                        Ok(columns) => columns,
                        Err(error) => {
                            self.status_line = format!("Column fetch failed: {error}");
                            Vec::new()
                        }
                    };
                self.schema_columns = self
                    .schema_column_schemas
                    .iter()
                    .map(|column| column.name.clone())
                    .collect();
            } else {
                self.schema_columns.clear();
                self.schema_column_schemas.clear();
            }
        } else {
            self.schema_column_schemas = demo_column_schemas();
            self.schema_columns = self
                .schema_column_schemas
                .iter()
                .map(|column| column.name.clone())
                .collect();
        }

        self.selected_column_index = 0;
        self.selection.column = self.schema_columns.first().cloned();
        if let Some(filtered_index) =
            first_filtered_index(&self.schema_columns, self.schema_column_filter.as_str())
        {
            self.selected_column_index = filtered_index;
            self.selection.column = self.schema_columns.get(filtered_index).cloned();
        }
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

}
