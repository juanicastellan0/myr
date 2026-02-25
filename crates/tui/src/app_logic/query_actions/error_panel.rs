impl TuiApp {
    pub(super) fn open_error_panel(
        &mut self,
        kind: ErrorKind,
        title: impl Into<String>,
        summary: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.error_panel = Some(ErrorPanel {
            kind,
            title: title.into(),
            summary: summary.into(),
            detail: detail.into(),
        });
    }

    fn run_primary_error_action(&mut self) {
        let Some(panel) = self.error_panel.as_ref().cloned() else {
            return;
        };

        if panel.kind == ErrorKind::Query {
            if let Some(sql) = self.last_failed_query.clone() {
                self.error_panel = None;
                self.start_query(sql);
                return;
            }
        }

        self.reconnect_from_error_panel()
    }

    fn reconnect_from_error_panel(&mut self) {
        if self.connect_requested {
            self.status_line = "Already connecting...".to_string();
            return;
        }

        let profile = self
            .active_connection_profile
            .clone()
            .or_else(|| self.last_connect_profile.clone())
            .or_else(|| self.wizard_profile().ok());

        let Some(profile) = profile else {
            self.status_line = "Reconnect unavailable: provide a valid connection profile".to_string();
            return;
        };

        self.error_panel = None;
        self.reconnect_attempts = 0;
        self.start_connect_with_profile(profile, ConnectIntent::Manual);
    }

    pub(super) fn can_reconnect_from_error_panel(&self) -> bool {
        self.active_connection_profile.is_some()
            || self.last_connect_profile.is_some()
            || self.wizard_profile().is_ok()
    }
}
