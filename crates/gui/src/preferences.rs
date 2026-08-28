use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ColorScheme {
    Dark,
    Light,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GuiPreferences {
    pub width: f32,
    pub height: f32,
    pub color_scheme: ColorScheme,
    pub result_page_size: usize,
}

impl Default for GuiPreferences {
    fn default() -> Self {
        Self {
            width: 1_280.0,
            height: 800.0,
            color_scheme: ColorScheme::Dark,
            result_page_size: 100,
        }
    }
}

impl GuiPreferences {
    #[must_use]
    pub fn load_default() -> Self {
        let Ok(path) = default_gui_preferences_path() else {
            return Self::default();
        };
        Self::load(&path).unwrap_or_default()
    }

    /// Loads GUI-only preferences from `path`.
    ///
    /// # Errors
    ///
    /// Returns a descriptive error when the file cannot be read or parsed.
    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        toml::from_str(&raw).map_err(|error| format!("failed to parse {}: {error}", path.display()))
    }

    /// Saves GUI-only preferences to the platform configuration directory.
    ///
    /// # Errors
    ///
    /// Returns a descriptive error when the destination cannot be created or written.
    pub fn save_default(&self) -> Result<(), String> {
        let path = default_gui_preferences_path()?;
        self.save(&path)
    }

    /// Saves GUI-only preferences to `path` using a partial file and rename.
    ///
    /// # Errors
    ///
    /// Returns a descriptive error when serialization or filesystem operations fail.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
        }
        let rendered = toml::to_string_pretty(self)
            .map_err(|error| format!("failed to serialize GUI preferences: {error}"))?;
        let partial = path.with_extension("toml.part");
        fs::write(&partial, rendered)
            .map_err(|error| format!("failed to write {}: {error}", partial.display()))?;
        fs::rename(&partial, path)
            .map_err(|error| format!("failed to finalize {}: {error}", path.display()))
    }
}

/// Resolves the platform-specific `gui.toml` path.
///
/// # Errors
///
/// Returns an error when no suitable user configuration directory is available.
pub fn default_gui_preferences_path() -> Result<PathBuf, String> {
    let base_dir = if let Some(custom) = env::var_os("MYR_CONFIG_DIR") {
        PathBuf::from(custom)
    } else if cfg!(target_os = "windows") {
        env::var_os("APPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| "APPDATA is not available".to_string())?
    } else if let Some(xdg_config_home) = env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(xdg_config_home)
    } else {
        let home = env::var_os("HOME").ok_or_else(|| "HOME is not available".to_string())?;
        PathBuf::from(home).join(".config")
    };
    Ok(base_dir.join("myr").join("gui.toml"))
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn preferences_round_trip_without_touching_profiles() {
        let temp_dir = TempDir::new().expect("temp dir should create");
        let path = temp_dir.path().join("gui.toml");
        let preferences = GuiPreferences {
            width: 1_024.0,
            height: 768.0,
            color_scheme: ColorScheme::Light,
            result_page_size: 50,
        };
        preferences.save(&path).expect("preferences should save");
        assert_eq!(
            GuiPreferences::load(&path).expect("preferences should load"),
            preferences
        );
        assert!(!temp_dir.path().join("profiles.toml").exists());
    }

    #[test]
    fn missing_and_invalid_preferences_have_explicit_outcomes() {
        let temp_dir = TempDir::new().expect("temp dir should create");
        let missing = temp_dir.path().join("missing.toml");
        assert_eq!(
            GuiPreferences::load(&missing).expect("missing preferences use defaults"),
            GuiPreferences::default()
        );

        let invalid = temp_dir.path().join("invalid.toml");
        std::fs::write(&invalid, "width = [not valid").expect("invalid fixture should write");
        assert!(GuiPreferences::load(&invalid)
            .expect_err("invalid TOML should fail")
            .contains("failed to parse"));
        assert!(default_gui_preferences_path()
            .expect("this test environment has a config directory")
            .ends_with("myr/gui.toml"));

        let destination_is_directory = temp_dir.path().join("directory.toml");
        std::fs::create_dir(&destination_is_directory).expect("directory fixture should create");
        assert!(GuiPreferences::default()
            .save(&destination_is_directory)
            .expect_err("rename over a directory should fail")
            .contains("failed to finalize"));
    }
}
