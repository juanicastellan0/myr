impl TuiApp {
    fn handle_input_char(&mut self, ch: char) {
        if self.show_palette {
            self.palette_query.push(ch);
            self.palette_selection = 0;
            self.status_line = format!("Palette query: {}", self.palette_query);
        } else if self.pane == Pane::ConnectionWizard {
            if !self.wizard_form.editing {
                if ch.eq_ignore_ascii_case(&'e') {
                    self.start_wizard_edit();
                } else {
                    self.status_line = format!(
                        "Selected {}. Press E or Enter to edit",
                        self.wizard_form.active_field.label()
                    );
                }
            } else {
                self.wizard_form.edit_buffer.push(ch);
                self.status_line = format!("Editing {}", self.wizard_form.active_field.label());
            }
        } else if self.pane == Pane::QueryEditor {
            self.insert_text_at_query_cursor(&ch.to_string());
            self.status_line = "Query text updated".to_string();
        }
    }

    fn handle_insert_newline(&mut self) {
        if self.show_palette || self.results_search_mode {
            return;
        }
        if self.pane != Pane::QueryEditor {
            self.status_line = "Newline insert is only available in query editor".to_string();
            return;
        }

        self.insert_text_at_query_cursor("\n");
        self.status_line = "Inserted newline".to_string();
    }

    fn handle_backspace(&mut self) {
        if self.show_palette {
            self.palette_query.pop();
            self.palette_selection = 0;
            self.status_line = format!("Palette query: {}", self.palette_query);
        } else if self.pane == Pane::ConnectionWizard {
            if self.wizard_form.editing {
                self.wizard_form.edit_buffer.pop();
                self.status_line = format!("Editing {}", self.wizard_form.active_field.label());
            } else {
                self.status_line = format!(
                    "Selected {}. Press E or Enter to edit",
                    self.wizard_form.active_field.label()
                );
            }
        } else if self.pane == Pane::QueryEditor {
            self.backspace_query_editor_char();
            self.status_line = "Query text updated".to_string();
        }
    }

    fn handle_clear_input(&mut self) {
        if self.show_palette {
            self.palette_query.clear();
            self.palette_selection = 0;
            self.status_line = "Palette query cleared".to_string();
        } else if self.pane == Pane::ConnectionWizard {
            if self.wizard_form.editing {
                self.wizard_form.edit_buffer.clear();
                self.status_line = format!("Cleared {}", self.wizard_form.active_field.label());
            } else {
                self.status_line = format!(
                    "Selected {}. Press E or Enter to edit",
                    self.wizard_form.active_field.label()
                );
            }
        } else if self.pane == Pane::QueryEditor {
            self.query_editor_text.clear();
            self.query_cursor = 0;
            self.query_history_index = None;
            self.query_history_draft = None;
            self.status_line = "Query cleared".to_string();
        }
    }

    fn insert_text_at_query_cursor(&mut self, text: &str) {
        self.clamp_query_cursor();
        self.query_editor_text.insert_str(self.query_cursor, text);
        self.query_cursor = self
            .query_cursor
            .saturating_add(text.len())
            .min(self.query_editor_text.len());
        self.query_history_index = None;
    }

    fn backspace_query_editor_char(&mut self) {
        self.clamp_query_cursor();
        if self.query_cursor == 0 {
            return;
        }
        let previous = previous_char_boundary(&self.query_editor_text, self.query_cursor);
        self.query_editor_text
            .replace_range(previous..self.query_cursor, "");
        self.query_cursor = previous;
        self.query_history_index = None;
    }

    fn move_query_cursor_left(&mut self) {
        self.clamp_query_cursor();
        self.query_cursor = previous_char_boundary(&self.query_editor_text, self.query_cursor);
        let (line, column) = self.query_cursor_line_col();
        self.status_line = format!("Cursor moved to line {line}, col {column}");
    }

    fn move_query_cursor_right(&mut self) {
        self.clamp_query_cursor();
        self.query_cursor = next_char_boundary(&self.query_editor_text, self.query_cursor);
        let (line, column) = self.query_cursor_line_col();
        self.status_line = format!("Cursor moved to line {line}, col {column}");
    }

    fn use_previous_query_from_history(&mut self) {
        if self.query_history.is_empty() {
            self.status_line = "Query history is empty".to_string();
            return;
        }

        let next_index = match self.query_history_index {
            Some(index) => index.saturating_sub(1),
            None => {
                self.query_history_draft = Some(self.query_editor_text.clone());
                self.query_history.len().saturating_sub(1)
            }
        };

        self.query_history_index = Some(next_index);
        self.query_editor_text = self.query_history[next_index].clone();
        self.query_cursor = self.query_editor_text.len();
        self.status_line = format!("History {} / {}", next_index + 1, self.query_history.len());
    }

    fn use_next_query_from_history(&mut self) {
        let Some(index) = self.query_history_index else {
            self.status_line = "Already at latest editor query".to_string();
            return;
        };

        if index + 1 < self.query_history.len() {
            let next_index = index + 1;
            self.query_history_index = Some(next_index);
            self.query_editor_text = self.query_history[next_index].clone();
            self.query_cursor = self.query_editor_text.len();
            self.status_line = format!("History {} / {}", next_index + 1, self.query_history.len());
            return;
        }

        self.query_history_index = None;
        if let Some(draft) = self.query_history_draft.take() {
            self.query_editor_text = draft;
        }
        self.query_cursor = self.query_editor_text.len();
        self.status_line = "Returned to latest editor query".to_string();
    }

    fn record_query_history(&mut self, sql: &str) {
        let trimmed = sql.trim();
        if trimmed.is_empty() {
            return;
        }

        if self
            .query_history
            .last()
            .is_some_and(|last| last.as_str() == trimmed)
        {
            return;
        }

        self.query_history.push(trimmed.to_string());
        if self.query_history.len() > 100 {
            let overflow = self.query_history.len() - 100;
            self.query_history.drain(0..overflow);
        }
    }

    fn clamp_query_cursor(&mut self) {
        if self.query_cursor > self.query_editor_text.len() {
            self.query_cursor = self.query_editor_text.len();
        }
        while self.query_cursor > 0 && !self.query_editor_text.is_char_boundary(self.query_cursor) {
            self.query_cursor = self.query_cursor.saturating_sub(1);
        }
    }

    pub(super) fn query_cursor_line_col(&self) -> (usize, usize) {
        let cursor = self.query_cursor.min(self.query_editor_text.len());
        let prefix = &self.query_editor_text[..cursor];
        let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
        let column = prefix
            .rsplit('\n')
            .next()
            .map(|segment| segment.chars().count() + 1)
            .unwrap_or(1);
        (line, column)
    }

    fn start_wizard_edit(&mut self) {
        if self.pane != Pane::ConnectionWizard || self.wizard_form.editing {
            return;
        }
        let current_value = self.active_wizard_value().to_string();
        self.wizard_form.editing = true;
        self.wizard_form.edit_buffer = current_value;
        self.status_line = format!(
            "Editing {} (Enter save, Esc cancel, Ctrl+U clear)",
            self.wizard_form.active_field.label()
        );
    }

    fn commit_wizard_edit(&mut self) {
        if self.pane != Pane::ConnectionWizard || !self.wizard_form.editing {
            return;
        }
        let updated_value = self.wizard_form.edit_buffer.clone();
        *self.active_wizard_value_mut() = updated_value;
        self.wizard_form.editing = false;
        self.wizard_form.edit_buffer.clear();
        self.status_line = format!("Saved {}", self.wizard_form.active_field.label());
    }

    fn cancel_wizard_edit(&mut self) {
        if self.pane != Pane::ConnectionWizard || !self.wizard_form.editing {
            return;
        }
        self.wizard_form.editing = false;
        self.wizard_form.edit_buffer.clear();
        self.status_line = format!("Canceled editing {}", self.wizard_form.active_field.label());
    }

    fn active_wizard_value(&self) -> &str {
        match self.wizard_form.active_field {
            WizardField::ProfileName => self.wizard_form.profile_name.as_str(),
            WizardField::Host => self.wizard_form.host.as_str(),
            WizardField::Port => self.wizard_form.port.as_str(),
            WizardField::User => self.wizard_form.user.as_str(),
            WizardField::PasswordSource => self.wizard_form.password_source.as_str(),
            WizardField::Database => self.wizard_form.database.as_str(),
            WizardField::TlsMode => self.wizard_form.tls_mode.as_str(),
            WizardField::ReadOnly => self.wizard_form.read_only.as_str(),
        }
    }

    fn active_wizard_value_mut(&mut self) -> &mut String {
        match self.wizard_form.active_field {
            WizardField::ProfileName => &mut self.wizard_form.profile_name,
            WizardField::Host => &mut self.wizard_form.host,
            WizardField::Port => &mut self.wizard_form.port,
            WizardField::User => &mut self.wizard_form.user,
            WizardField::PasswordSource => &mut self.wizard_form.password_source,
            WizardField::Database => &mut self.wizard_form.database,
            WizardField::TlsMode => &mut self.wizard_form.tls_mode,
            WizardField::ReadOnly => &mut self.wizard_form.read_only,
        }
    }

    fn invoke_ranked_action(&mut self, index: usize) {
        if self.pane == Pane::ConnectionWizard {
            if self.wizard_form.editing {
                let digit = char::from_digit((index + 1) as u32, 10).unwrap_or('0');
                self.wizard_form.edit_buffer.push(digit);
                self.status_line = format!("Editing {}", self.wizard_form.active_field.label());
            } else {
                self.status_line = format!(
                    "Selected {}. Press E or Enter to edit",
                    self.wizard_form.active_field.label()
                );
            }
            return;
        }

        if self.show_palette {
            self.palette_selection = index.min(self.palette_entries().len().saturating_sub(1));
            self.submit();
            return;
        }

        let context = self.action_context();
        let ranked = self.actions.rank_top_n(&context, FOOTER_ACTIONS_LIMIT);
        let Some(action) = ranked.get(index) else {
            self.status_line = format!("No action bound to slot {}", index + 1);
            return;
        };

        self.invoke_action(action.id);
    }

}
