impl TuiApp {
    pub(super) fn open_profile_bookmark_manager(&mut self) {
        self.manager_rename_mode = false;
        self.manager_rename_buffer.clear();
        self.clamp_manager_cursors();
        self.set_active_pane(Pane::ProfileBookmarks);
        self.status_line = format!(
            "Manager opened: {} profiles | {} bookmarks",
            self.manager_profiles().len(),
            self.manager_bookmarks().len()
        );
    }

    fn navigate_profile_bookmark_manager(&mut self, direction: DirectionKey) {
        if self.manager_rename_mode {
            self.status_line =
                "Rename mode active (Enter save, Esc cancel, Backspace/Ctrl+U edit)".to_string();
            return;
        }

        match direction {
            DirectionKey::Left => {
                self.manager_lane = self.manager_lane.previous();
                self.status_line = format!("Manager focus: {}", self.manager_lane.label());
            }
            DirectionKey::Right => {
                self.manager_lane = self.manager_lane.next();
                self.status_line = format!("Manager focus: {}", self.manager_lane.label());
            }
            DirectionKey::Up => self.move_manager_cursor(-1),
            DirectionKey::Down => self.move_manager_cursor(1),
        }
    }

    fn move_manager_cursor(&mut self, delta: isize) {
        if self.manager_rename_mode {
            self.status_line =
                "Rename mode active (Enter save, Esc cancel, Backspace/Ctrl+U edit)".to_string();
            return;
        }
        self.clamp_manager_cursors();
        let lane_label = self.manager_lane.label();
        let len = self.current_manager_len();
        if len == 0 {
            self.status_line = format!("No {} available", lane_label.to_lowercase());
            return;
        }

        let cursor = self.current_manager_cursor_mut();
        if delta < 0 {
            *cursor = cursor.saturating_sub(delta.unsigned_abs());
        } else {
            *cursor = (*cursor + delta as usize).min(len.saturating_sub(1));
        }
        self.status_line = format!(
            "{} {} / {}",
            lane_label,
            cursor.saturating_add(1),
            len
        );
    }

    pub(super) fn handle_manager_input_char(&mut self, ch: char) {
        if self.pane != Pane::ProfileBookmarks {
            return;
        }

        if self.manager_rename_mode {
            if !ch.is_ascii_graphic() && ch != ' ' {
                return;
            }
            self.manager_rename_buffer.push(ch);
            self.status_line = format!("Renaming to `{}`", self.manager_rename_buffer);
            return;
        }

        match ch.to_ascii_lowercase() {
            'r' => self.start_manager_rename(),
            'd' => self.mark_selected_profile_default(),
            'q' => self.mark_selected_profile_quick_reconnect(),
            _ => {
                self.status_line =
                    "Manager shortcuts: r rename | d default profile | q quick reconnect target"
                        .to_string();
            }
        }
    }

    pub(super) fn handle_manager_backspace(&mut self) {
        if self.pane != Pane::ProfileBookmarks {
            return;
        }
        if !self.manager_rename_mode {
            self.status_line = "Use Del to remove selected entry".to_string();
            return;
        }

        self.manager_rename_buffer.pop();
        self.status_line = format!("Renaming to `{}`", self.manager_rename_buffer);
    }

    pub(super) fn clear_manager_rename_buffer(&mut self) {
        if self.pane != Pane::ProfileBookmarks {
            return;
        }
        if !self.manager_rename_mode {
            self.status_line = "Use Del to remove selected entry".to_string();
            return;
        }

        self.manager_rename_buffer.clear();
        self.status_line = "Rename input cleared".to_string();
    }

    fn start_manager_rename(&mut self) {
        self.clamp_manager_cursors();
        let current_name = match self.manager_lane {
            ManagerLane::Profiles => self
                .manager_profiles()
                .get(self.manager_profile_cursor)
                .map(|profile| profile.name.clone()),
            ManagerLane::Bookmarks => self
                .manager_bookmarks()
                .get(self.manager_bookmark_cursor)
                .map(|bookmark| bookmark.name.clone()),
        };

        let Some(current_name) = current_name else {
            self.status_line = format!("No {} selected", self.manager_lane.label().to_lowercase());
            return;
        };

        self.manager_rename_mode = true;
        self.manager_rename_buffer = current_name.clone();
        self.status_line = format!("Renaming `{current_name}` (Enter save, Esc cancel)");
    }

    pub(super) fn cancel_manager_rename(&mut self) {
        if !self.manager_rename_mode {
            return;
        }
        self.manager_rename_mode = false;
        self.manager_rename_buffer.clear();
        self.status_line = "Rename canceled".to_string();
    }

    pub(super) fn commit_manager_rename(&mut self) {
        if !self.manager_rename_mode {
            self.open_manager_selection();
            return;
        }

        let new_name = self.manager_rename_buffer.trim().to_string();
        if new_name.is_empty() {
            self.status_line = "Rename failed: name cannot be empty".to_string();
            return;
        }

        match self.manager_lane {
            ManagerLane::Profiles => self.rename_selected_profile(new_name.as_str()),
            ManagerLane::Bookmarks => self.rename_selected_bookmark(new_name.as_str()),
        }
    }

    fn rename_selected_profile(&mut self, new_name: &str) {
        let Some(store) = self.profile_store.as_mut() else {
            self.status_line = "Profile storage unavailable on this platform".to_string();
            return;
        };
        if store.profiles().is_empty() {
            self.status_line = "No profiles available".to_string();
            return;
        }

        let index = self
            .manager_profile_cursor
            .min(store.profiles().len().saturating_sub(1));
        let old_name = store.profiles()[index].name.clone();
        if old_name == new_name {
            self.manager_rename_mode = false;
            self.manager_rename_buffer.clear();
            self.status_line = "Profile name unchanged".to_string();
            return;
        }
        if store.profile(new_name).is_some() {
            self.status_line = format!("Rename failed: profile `{new_name}` already exists");
            return;
        }

        let mut updated = store.profiles()[index].clone();
        updated.name = new_name.to_string();
        store.upsert_profile(updated.clone());
        let _deleted = store.delete_profile(old_name.as_str());

        match store.persist() {
            Ok(()) => {
                self.manager_rename_mode = false;
                self.manager_rename_buffer.clear();
                self.manager_profile_cursor = store
                    .profiles()
                    .iter()
                    .position(|profile| profile.name == updated.name)
                    .unwrap_or(0);

                if self.connected_profile.as_deref() == Some(old_name.as_str()) {
                    self.connected_profile = Some(updated.name.clone());
                }
                if self.last_connect_profile.as_ref().map(|profile| profile.name.as_str())
                    == Some(old_name.as_str())
                {
                    self.last_connect_profile = Some(updated.clone());
                }
                if self
                    .active_connection_profile
                    .as_ref()
                    .map(|profile| profile.name.as_str())
                    == Some(old_name.as_str())
                {
                    self.active_connection_profile = Some(updated.clone());
                }
                if self.wizard_form.profile_name == old_name {
                    self.wizard_form.profile_name = updated.name.clone();
                }

                self.status_line =
                    format!("Renamed profile `{old_name}` -> `{}`", updated.name.as_str());
            }
            Err(error) => {
                self.status_line = format!("Profile rename failed: {error}");
            }
        }
    }

    fn rename_selected_bookmark(&mut self, new_name: &str) {
        let Some(store) = self.bookmark_store.as_mut() else {
            self.status_line = "Bookmark storage unavailable on this platform".to_string();
            return;
        };
        if store.bookmarks().is_empty() {
            self.status_line = "No bookmarks available".to_string();
            return;
        }

        let index = self
            .manager_bookmark_cursor
            .min(store.bookmarks().len().saturating_sub(1));
        let old_name = store.bookmarks()[index].name.clone();
        if old_name == new_name {
            self.manager_rename_mode = false;
            self.manager_rename_buffer.clear();
            self.status_line = "Bookmark name unchanged".to_string();
            return;
        }
        if store.bookmark(new_name).is_some() {
            self.status_line = format!("Rename failed: bookmark `{new_name}` already exists");
            return;
        }

        let mut updated = store.bookmarks()[index].clone();
        updated.name = new_name.to_string();
        store.upsert_bookmark(updated.clone());
        let _deleted = store.delete_bookmark(old_name.as_str());

        match store.persist() {
            Ok(()) => {
                self.manager_rename_mode = false;
                self.manager_rename_buffer.clear();
                self.manager_bookmark_cursor = store
                    .bookmarks()
                    .iter()
                    .position(|bookmark| bookmark.name == updated.name)
                    .unwrap_or(0);
                self.status_line =
                    format!("Renamed bookmark `{old_name}` -> `{}`", updated.name.as_str());
            }
            Err(error) => {
                self.status_line = format!("Bookmark rename failed: {error}");
            }
        }
    }

    fn mark_selected_profile_default(&mut self) {
        if self.manager_lane != ManagerLane::Profiles {
            self.status_line = "Default profile marker applies to Profiles lane".to_string();
            return;
        }
        let Some(store) = self.profile_store.as_mut() else {
            self.status_line = "Profile storage unavailable on this platform".to_string();
            return;
        };
        if store.profiles().is_empty() {
            self.status_line = "No profiles available".to_string();
            return;
        }

        let index = self
            .manager_profile_cursor
            .min(store.profiles().len().saturating_sub(1));
        let name = store.profiles()[index].name.clone();
        let _ = store.set_default_profile(name.as_str());
        match store.persist() {
            Ok(()) => {
                self.status_line = format!("Marked `{name}` as default profile");
            }
            Err(error) => {
                self.status_line = format!("Default profile update failed: {error}");
            }
        }
    }

    fn mark_selected_profile_quick_reconnect(&mut self) {
        if self.manager_lane != ManagerLane::Profiles {
            self.status_line = "Quick reconnect marker applies to Profiles lane".to_string();
            return;
        }
        let Some(store) = self.profile_store.as_mut() else {
            self.status_line = "Profile storage unavailable on this platform".to_string();
            return;
        };
        if store.profiles().is_empty() {
            self.status_line = "No profiles available".to_string();
            return;
        }

        let index = self
            .manager_profile_cursor
            .min(store.profiles().len().saturating_sub(1));
        let name = store.profiles()[index].name.clone();
        let _ = store.set_quick_reconnect_profile(name.as_str());
        match store.persist() {
            Ok(()) => {
                self.status_line = format!("Marked `{name}` as quick reconnect target");
            }
            Err(error) => {
                self.status_line = format!("Quick reconnect update failed: {error}");
            }
        }
    }

    pub(super) fn connect_from_manager(&mut self) {
        self.clamp_manager_cursors();
        if self.manager_rename_mode {
            self.status_line =
                "Rename mode active (Enter save, Esc cancel, Backspace/Ctrl+U edit)".to_string();
            return;
        }

        let selected_profile = self
            .manager_profiles()
            .get(self.manager_profile_cursor)
            .cloned();
        let quick_profile = self
            .profile_store
            .as_ref()
            .and_then(|store| store.quick_reconnect_profile().cloned());

        let profile = match self.manager_lane {
            ManagerLane::Profiles => selected_profile.or(quick_profile),
            ManagerLane::Bookmarks => quick_profile.or(selected_profile),
        };

        let Some(profile) = profile else {
            self.status_line =
                "No profile available for connect (select one or mark quick reconnect)".to_string();
            return;
        };

        self.start_connect_with_profile(profile, ConnectIntent::Manual);
    }

    pub(super) fn open_manager_selection(&mut self) {
        if self.manager_rename_mode {
            self.commit_manager_rename();
            return;
        }
        self.clamp_manager_cursors();
        match self.manager_lane {
            ManagerLane::Profiles => self.open_selected_manager_profile(),
            ManagerLane::Bookmarks => self.open_selected_manager_bookmark(),
        }
    }

    fn open_selected_manager_profile(&mut self) {
        let profiles = self.manager_profiles();
        if profiles.is_empty() {
            self.status_line = "No profiles available".to_string();
            return;
        }

        let index = self.manager_profile_cursor.min(profiles.len().saturating_sub(1));
        let profile = profiles[index].clone();
        self.wizard_form = wizard_form_from_profile(&profile);
        self.last_connect_profile = Some(profile.clone());
        self.set_active_pane(Pane::ConnectionWizard);
        self.status_line = format!("Loaded profile `{}` into wizard", profile.name);
    }

    fn open_selected_manager_bookmark(&mut self) {
        let bookmarks = self.manager_bookmarks();
        if bookmarks.is_empty() {
            self.status_line = "No bookmarks available".to_string();
            return;
        }

        let index = self
            .manager_bookmark_cursor
            .min(bookmarks.len().saturating_sub(1));
        self.apply_bookmark(bookmarks[index].clone());
    }

    pub(super) fn delete_manager_selection(&mut self) {
        if self.pane != Pane::ProfileBookmarks {
            self.status_line = "Delete selection is only available in manager view".to_string();
            return;
        }
        if self.manager_rename_mode {
            self.status_line =
                "Rename mode active (Enter save, Esc cancel, Backspace/Ctrl+U edit)".to_string();
            return;
        }

        match self.manager_lane {
            ManagerLane::Profiles => self.delete_selected_profile(),
            ManagerLane::Bookmarks => self.delete_selected_bookmark(),
        }
    }

    fn delete_selected_profile(&mut self) {
        let Some(store) = self.profile_store.as_mut() else {
            self.status_line = "Profile storage unavailable on this platform".to_string();
            return;
        };
        if store.profiles().is_empty() {
            self.status_line = "No profiles available".to_string();
            return;
        }

        let index = self
            .manager_profile_cursor
            .min(store.profiles().len().saturating_sub(1));
        let name = store.profiles()[index].name.clone();
        if !store.delete_profile(name.as_str()) {
            self.status_line = format!("Profile `{name}` could not be deleted");
            return;
        }

        match store.persist() {
            Ok(()) => {
                if self.manager_profile_cursor > 0
                    && self.manager_profile_cursor >= store.profiles().len()
                {
                    self.manager_profile_cursor = self.manager_profile_cursor.saturating_sub(1);
                }
                self.status_line = format!(
                    "Deleted profile `{name}` ({} remaining)",
                    store.profiles().len()
                );
            }
            Err(error) => {
                self.status_line = format!("Profile delete failed: {error}");
            }
        }
    }

    fn delete_selected_bookmark(&mut self) {
        let Some(store) = self.bookmark_store.as_mut() else {
            self.status_line = "Bookmark storage unavailable on this platform".to_string();
            return;
        };
        if store.bookmarks().is_empty() {
            self.status_line = "No bookmarks available".to_string();
            return;
        }

        let index = self
            .manager_bookmark_cursor
            .min(store.bookmarks().len().saturating_sub(1));
        let name = store.bookmarks()[index].name.clone();
        if !store.delete_bookmark(name.as_str()) {
            self.status_line = format!("Bookmark `{name}` could not be deleted");
            return;
        }

        match store.persist() {
            Ok(()) => {
                self.bookmark_cycle_index = 0;
                if self.manager_bookmark_cursor > 0
                    && self.manager_bookmark_cursor >= store.bookmarks().len()
                {
                    self.manager_bookmark_cursor = self.manager_bookmark_cursor.saturating_sub(1);
                }
                self.status_line = format!(
                    "Deleted bookmark `{name}` ({} remaining)",
                    store.bookmarks().len()
                );
            }
            Err(error) => {
                self.status_line = format!("Bookmark delete failed: {error}");
            }
        }
    }

    fn current_manager_len(&self) -> usize {
        match self.manager_lane {
            ManagerLane::Profiles => self.manager_profiles().len(),
            ManagerLane::Bookmarks => self.manager_bookmarks().len(),
        }
    }

    fn current_manager_cursor_mut(&mut self) -> &mut usize {
        match self.manager_lane {
            ManagerLane::Profiles => &mut self.manager_profile_cursor,
            ManagerLane::Bookmarks => &mut self.manager_bookmark_cursor,
        }
    }

    pub(super) fn manager_profiles(&self) -> Vec<ConnectionProfile> {
        self.profile_store
            .as_ref()
            .map_or_else(Vec::new, |store| store.profiles().to_vec())
    }

    pub(super) fn manager_bookmarks(&self) -> Vec<SavedBookmark> {
        self.bookmark_store
            .as_ref()
            .map_or_else(Vec::new, |store| store.bookmarks().to_vec())
    }

    fn clamp_manager_cursors(&mut self) {
        let profile_count = self.manager_profiles().len();
        if profile_count == 0 {
            self.manager_profile_cursor = 0;
        } else {
            self.manager_profile_cursor = self.manager_profile_cursor.min(profile_count - 1);
        }

        let bookmark_count = self.manager_bookmarks().len();
        if bookmark_count == 0 {
            self.manager_bookmark_cursor = 0;
        } else {
            self.manager_bookmark_cursor = self.manager_bookmark_cursor.min(bookmark_count - 1);
        }
    }

}
