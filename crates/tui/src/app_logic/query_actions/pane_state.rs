impl TuiApp {
    pub(super) fn pane_tab_index(&self) -> usize {
        match self.pane {
            Pane::ConnectionWizard => 0,
            Pane::SchemaExplorer => 1,
            Pane::Results => 2,
            Pane::QueryEditor => 3,
            Pane::ProfileBookmarks => 4,
        }
    }

    pub(super) fn runtime_state_label(&self) -> &'static str {
        if self.connect_requested || self.query_running {
            "BUSY"
        } else {
            "IDLE"
        }
    }

    pub(super) fn connection_state_label(&self) -> &'static str {
        if self.connect_requested {
            if self.connect_intent == ConnectIntent::AutoReconnect {
                "RECONNECTING"
            } else {
                "CONNECTING"
            }
        } else if self.data_backend.is_some() {
            "CONNECTED"
        } else {
            "DISCONNECTED"
        }
    }

    pub(super) fn pane_name(&self) -> &'static str {
        match self.pane {
            Pane::ConnectionWizard => "Connection Wizard",
            Pane::SchemaExplorer => "Schema Explorer",
            Pane::Results => "Results",
            Pane::QueryEditor => "Query Editor",
            Pane::ProfileBookmarks => "Profiles & Bookmarks",
        }
    }

    fn set_active_pane(&mut self, pane: Pane) {
        if self.pane != pane {
            self.pane = pane;
            self.pane_flash_ticks = PANE_FLASH_DURATION_TICKS;
            if pane != Pane::Results {
                self.results_search_mode = false;
            }
        }
    }
}
