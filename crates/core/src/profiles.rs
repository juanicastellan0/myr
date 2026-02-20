use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TlsMode {
    Disabled,
    #[default]
    Prefer,
    Require,
    VerifyIdentity,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PasswordSource {
    #[default]
    EnvVar,
    Keyring,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectionProfile {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub database: Option<String>,
    #[serde(default)]
    pub tls_mode: TlsMode,
    #[serde(default)]
    pub password_source: PasswordSource,
    #[serde(default)]
    pub keyring_service: Option<String>,
    #[serde(default)]
    pub keyring_account: Option<String>,
    #[serde(default)]
    pub tls_ca_cert_path: Option<String>,
    #[serde(default)]
    pub tls_client_cert_path: Option<String>,
    #[serde(default)]
    pub tls_client_key_path: Option<String>,
    #[serde(default)]
    pub tls_disable_built_in_roots: bool,
    #[serde(default)]
    pub tls_skip_domain_validation: bool,
    #[serde(default)]
    pub tls_accept_invalid_certs: bool,
    #[serde(default)]
    pub tls_hostname_override: Option<String>,
    #[serde(default)]
    pub read_only: bool,
}

impl ConnectionProfile {
    #[must_use]
    pub fn new(name: impl Into<String>, host: impl Into<String>, user: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            host: host.into(),
            port: 3306,
            user: user.into(),
            database: None,
            tls_mode: TlsMode::Prefer,
            password_source: PasswordSource::EnvVar,
            keyring_service: None,
            keyring_account: None,
            tls_ca_cert_path: None,
            tls_client_cert_path: None,
            tls_client_key_path: None,
            tls_disable_built_in_roots: false,
            tls_skip_domain_validation: false,
            tls_accept_invalid_certs: false,
            tls_hostname_override: None,
            read_only: false,
        }
    }
}

#[derive(Debug, Error)]
pub enum ProfilesError {
    #[error("config directory is unavailable for this platform")]
    ConfigDirUnavailable,
    #[error("failed to read profiles file at {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse profiles file at {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("failed to create config directory at {path}: {source}")]
    CreateDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize profiles: {source}")]
    Serialize {
        #[source]
        source: toml::ser::Error,
    },
    #[error("failed to write profiles file at {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ProfilesDocument {
    #[serde(default)]
    profiles: Vec<ConnectionProfile>,
}

impl ProfilesDocument {
    fn normalize(&mut self) {
        let mut by_name = std::collections::BTreeMap::new();
        for profile in self.profiles.drain(..) {
            by_name.insert(profile.name.clone(), profile);
        }
        self.profiles = by_name.into_values().collect();
    }
}

#[derive(Debug, Clone)]
pub struct FileProfilesStore {
    path: PathBuf,
    profiles: Vec<ConnectionProfile>,
}

impl FileProfilesStore {
    pub fn load_default() -> Result<Self, ProfilesError> {
        let path = default_profiles_path()?;
        Self::load_from_path(path)
    }

    pub fn load_from_path(path: impl Into<PathBuf>) -> Result<Self, ProfilesError> {
        let path = path.into();
        if !path.exists() {
            return Ok(Self {
                path,
                profiles: Vec::new(),
            });
        }

        let raw = fs::read_to_string(&path).map_err(|source| ProfilesError::Read {
            path: path.clone(),
            source,
        })?;

        if raw.trim().is_empty() {
            return Ok(Self {
                path,
                profiles: Vec::new(),
            });
        }

        let mut doc: ProfilesDocument =
            toml::from_str(&raw).map_err(|source| ProfilesError::Parse {
                path: path.clone(),
                source,
            })?;
        doc.normalize();

        Ok(Self {
            path,
            profiles: doc.profiles,
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn profiles(&self) -> &[ConnectionProfile] {
        &self.profiles
    }

    #[must_use]
    pub fn profile(&self, name: &str) -> Option<&ConnectionProfile> {
        self.profiles.iter().find(|profile| profile.name == name)
    }

    pub fn upsert_profile(&mut self, profile: ConnectionProfile) {
        if let Some(existing) = self
            .profiles
            .iter_mut()
            .find(|existing| existing.name == profile.name)
        {
            *existing = profile;
        } else {
            self.profiles.push(profile);
            self.profiles.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        }
    }

    #[must_use]
    pub fn delete_profile(&mut self, name: &str) -> bool {
        let original_len = self.profiles.len();
        self.profiles.retain(|profile| profile.name != name);
        self.profiles.len() != original_len
    }

    pub fn persist(&self) -> Result<(), ProfilesError> {
        if let Some(parent_dir) = self.path.parent() {
            fs::create_dir_all(parent_dir).map_err(|source| ProfilesError::CreateDir {
                path: parent_dir.to_path_buf(),
                source,
            })?;
        }

        let doc = ProfilesDocument {
            profiles: self.profiles.clone(),
        };
        let rendered =
            toml::to_string_pretty(&doc).map_err(|source| ProfilesError::Serialize { source })?;

        fs::write(&self.path, rendered).map_err(|source| ProfilesError::Write {
            path: self.path.clone(),
            source,
        })
    }
}

pub fn default_profiles_path() -> Result<PathBuf, ProfilesError> {
    let base_dir = if let Some(custom) = env::var_os("MYR_CONFIG_DIR") {
        PathBuf::from(custom)
    } else if cfg!(target_os = "windows") {
        env::var_os("APPDATA")
            .map(PathBuf::from)
            .ok_or(ProfilesError::ConfigDirUnavailable)?
    } else if let Some(xdg_config_home) = env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(xdg_config_home)
    } else {
        let home = env::var_os("HOME").ok_or(ProfilesError::ConfigDirUnavailable)?;
        PathBuf::from(home).join(".config")
    };

    Ok(base_dir.join("myr").join("profiles.toml"))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::{ConnectionProfile, FileProfilesStore, TlsMode};

    fn temp_profiles_path(temp_dir: &TempDir) -> PathBuf {
        temp_dir.path().join("profiles.toml")
    }

    #[test]
    fn missing_profiles_file_loads_empty_store() {
        let temp_dir = TempDir::new().expect("failed to create temp directory");
        let path = temp_profiles_path(&temp_dir);

        let store = FileProfilesStore::load_from_path(path).expect("failed to load store");
        assert!(store.profiles().is_empty());
    }

    #[test]
    fn upsert_persist_reload_and_delete_profile() {
        let temp_dir = TempDir::new().expect("failed to create temp directory");
        let path = temp_profiles_path(&temp_dir);

        let mut store = FileProfilesStore::load_from_path(&path).expect("failed to load store");
        let mut profile = ConnectionProfile::new("local", "127.0.0.1", "root");
        profile.database = Some("myr".to_string());
        profile.tls_mode = TlsMode::Require;

        store.upsert_profile(profile.clone());
        store.persist().expect("failed to persist store");

        let mut reloaded = FileProfilesStore::load_from_path(&path).expect("failed to reload");
        let loaded = reloaded
            .profile("local")
            .expect("missing profile after save");
        assert_eq!(loaded, &profile);

        let mut updated = loaded.clone();
        updated.database = Some("myr_dev".to_string());
        reloaded.upsert_profile(updated.clone());
        reloaded
            .persist()
            .expect("failed to persist updated profile");

        let mut reloaded = FileProfilesStore::load_from_path(&path).expect("failed to reload");
        let loaded = reloaded
            .profile("local")
            .expect("missing profile after update");
        assert_eq!(loaded.database.as_deref(), Some("myr_dev"));

        assert!(reloaded.delete_profile("local"));
        reloaded.persist().expect("failed to persist deletion");

        let reloaded = FileProfilesStore::load_from_path(path).expect("failed final reload");
        assert!(reloaded.profile("local").is_none());
        assert!(reloaded.profiles().is_empty());
    }
}
