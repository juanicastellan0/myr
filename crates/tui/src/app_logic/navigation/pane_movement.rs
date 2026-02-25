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
            Pane::ProfileBookmarks => self.navigate_profile_bookmark_manager(direction),
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

}
