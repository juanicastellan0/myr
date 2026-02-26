use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const PROFILES_FORMAT_VERSION: u32 = 1;

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
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub quick_reconnect: bool,
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
            is_default: false,
            quick_reconnect: false,
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
    #[serde(default = "profiles_format_version")]
    version: u32,
    #[serde(default)]
    profiles: Vec<ConnectionProfile>,
}

impl ProfilesDocument {
    fn new(profiles: Vec<ConnectionProfile>) -> Self {
        Self {
            version: PROFILES_FORMAT_VERSION,
            profiles,
        }
    }

    fn normalize(&mut self) {
        let mut by_name = std::collections::BTreeMap::new();
        for profile in self.profiles.drain(..) {
            by_name.insert(profile.name.clone(), profile);
        }
        self.profiles = by_name.into_values().collect();
    }
}

fn profiles_format_version() -> u32 {
    PROFILES_FORMAT_VERSION
}

fn decode_profiles_document(raw: &str) -> Result<(ProfilesDocument, bool), toml::de::Error> {
    let mut value: toml::Value = toml::from_str(raw)?;
    let migrated = migrate_profiles_document(&mut value);
    let doc: ProfilesDocument = value.try_into()?;
    Ok((doc, migrated))
}

fn migrate_profiles_document(value: &mut toml::Value) -> bool {
    let Some(root) = value.as_table_mut() else {
        return false;
    };

    let version = root
        .get("version")
        .map_or(0, |value| match value.as_integer() {
            Some(raw) => u32::try_from(raw).unwrap_or(PROFILES_FORMAT_VERSION.saturating_add(1)),
            None => PROFILES_FORMAT_VERSION.saturating_add(1),
        });

    if version != 0 {
        return false;
    }

    let _ = rename_table_key(root, "connections", "profiles");

    if let Some(profiles) = root.get_mut("profiles").and_then(toml::Value::as_array_mut) {
        for profile in profiles {
            if let Some(profile_table) = profile.as_table_mut() {
                let _ = migrate_profile_fields(profile_table);
            }
        }
    }

    root.insert(
        "version".to_string(),
        toml::Value::Integer(PROFILES_FORMAT_VERSION.into()),
    );
    true
}

fn migrate_profile_fields(profile: &mut toml::map::Map<String, toml::Value>) -> bool {
    let mut migrated = false;
    migrated |= rename_table_key(profile, "default", "is_default");
    migrated |= rename_table_key(profile, "quick", "quick_reconnect");
    migrated |= rename_table_key(profile, "quick_connect", "quick_reconnect");
    migrated |= rename_table_key(profile, "password_provider", "password_source");
    migrated |= rename_table_key(profile, "tls_ca_cert", "tls_ca_cert_path");
    migrated |= rename_table_key(profile, "tls_client_cert", "tls_client_cert_path");
    migrated |= rename_table_key(profile, "tls_client_key", "tls_client_key_path");
    migrated |= rename_table_key(profile, "read_only_mode", "read_only");
    migrated
}

fn rename_table_key(table: &mut toml::map::Map<String, toml::Value>, from: &str, to: &str) -> bool {
    let Some(value) = table.remove(from) else {
        return false;
    };

    if !table.contains_key(to) {
        table.insert(to.to_string(), value);
    }
    true
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

        let (mut doc, migrated) =
            decode_profiles_document(&raw).map_err(|source| ProfilesError::Parse {
                path: path.clone(),
                source,
            })?;
        doc.normalize();

        let store = Self {
            path,
            profiles: doc.profiles,
        };

        if migrated {
            store.persist()?;
        }

        Ok(store)
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

    #[must_use]
    pub fn default_profile(&self) -> Option<&ConnectionProfile> {
        self.profiles.iter().find(|profile| profile.is_default)
    }

    #[must_use]
    pub fn quick_reconnect_profile(&self) -> Option<&ConnectionProfile> {
        self.profiles.iter().find(|profile| profile.quick_reconnect)
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
    pub fn set_default_profile(&mut self, name: &str) -> bool {
        if !self.profiles.iter().any(|profile| profile.name == name) {
            return false;
        }

        for profile in &mut self.profiles {
            profile.is_default = profile.name == name;
        }
        true
    }

    #[must_use]
    pub fn set_quick_reconnect_profile(&mut self, name: &str) -> bool {
        if !self.profiles.iter().any(|profile| profile.name == name) {
            return false;
        }

        for profile in &mut self.profiles {
            profile.quick_reconnect = profile.name == name;
        }
        true
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

        let doc = ProfilesDocument::new(self.profiles.clone());
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
    use std::fs;
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::{ConnectionProfile, FileProfilesStore, PasswordSource, TlsMode};

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

    #[test]
    fn default_and_quick_reconnect_profile_markers_are_exclusive() {
        let temp_dir = TempDir::new().expect("failed to create temp directory");
        let path = temp_profiles_path(&temp_dir);

        let mut store = FileProfilesStore::load_from_path(&path).expect("failed to load store");
        let local = ConnectionProfile::new("local", "127.0.0.1", "root");
        let prod = ConnectionProfile::new("prod", "10.0.0.8", "app");
        store.upsert_profile(local);
        store.upsert_profile(prod);

        assert!(store.set_default_profile("prod"));
        assert!(store.set_quick_reconnect_profile("local"));
        store.persist().expect("failed to persist store");

        let reloaded = FileProfilesStore::load_from_path(path).expect("failed to reload store");
        assert_eq!(
            reloaded
                .default_profile()
                .map(|profile| profile.name.as_str()),
            Some("prod")
        );
        assert_eq!(
            reloaded
                .quick_reconnect_profile()
                .map(|profile| profile.name.as_str()),
            Some("local")
        );
        assert_eq!(
            reloaded
                .profiles()
                .iter()
                .filter(|profile| profile.is_default)
                .count(),
            1
        );
        assert_eq!(
            reloaded
                .profiles()
                .iter()
                .filter(|profile| profile.quick_reconnect)
                .count(),
            1
        );
    }

    #[test]
    fn persist_writes_profiles_format_version() {
        let temp_dir = TempDir::new().expect("failed to create temp directory");
        let path = temp_profiles_path(&temp_dir);
        let mut store = FileProfilesStore::load_from_path(&path).expect("failed to load store");
        store.upsert_profile(ConnectionProfile::new("local", "127.0.0.1", "root"));
        store.persist().expect("failed to persist store");

        let raw = fs::read_to_string(path).expect("failed to read persisted profile file");
        assert!(raw.contains("version = 1"));
    }

    #[test]
    fn load_migrates_legacy_profile_document_and_rewrites_file() {
        let temp_dir = TempDir::new().expect("failed to create temp directory");
        let path = temp_profiles_path(&temp_dir);
        let legacy = r#"
[[connections]]
name = "legacy"
host = "127.0.0.1"
port = 3307
user = "root"
default = true
quick_connect = true
password_provider = "keyring"
tls_ca_cert = "/tmp/ca.pem"
tls_client_cert = "/tmp/client-cert.pem"
tls_client_key = "/tmp/client-key.pem"
read_only_mode = true
"#;
        fs::write(&path, legacy).expect("failed to write legacy profile file");

        let store = FileProfilesStore::load_from_path(&path).expect("failed to load legacy file");
        let profile = store
            .profile("legacy")
            .expect("legacy profile should be migrated");
        assert!(profile.is_default);
        assert!(profile.quick_reconnect);
        assert!(profile.read_only);
        assert_eq!(profile.password_source, PasswordSource::Keyring);
        assert_eq!(profile.tls_ca_cert_path.as_deref(), Some("/tmp/ca.pem"));
        assert_eq!(
            profile.tls_client_cert_path.as_deref(),
            Some("/tmp/client-cert.pem")
        );
        assert_eq!(
            profile.tls_client_key_path.as_deref(),
            Some("/tmp/client-key.pem")
        );

        let migrated = fs::read_to_string(path).expect("failed to read migrated profile file");
        assert!(migrated.contains("version = 1"));
        assert!(migrated.contains("[[profiles]]"));
        assert!(migrated.contains("is_default = true"));
        assert!(migrated.contains("quick_reconnect = true"));
        assert!(!migrated.contains("[[connections]]"));
        assert!(!migrated.contains("\nquick_connect = "));
        assert!(!migrated.contains("\npassword_provider = "));
        assert!(!migrated.contains("\nread_only_mode = "));
    }

    #[test]
    fn load_accepts_future_profile_version_without_overwriting_file() {
        let temp_dir = TempDir::new().expect("failed to create temp directory");
        let path = temp_profiles_path(&temp_dir);
        let future = r#"
version = 9

[[profiles]]
name = "future"
host = "127.0.0.1"
port = 3306
user = "root"
future_flag = "enabled"
"#;
        fs::write(&path, future).expect("failed to write future profile file");

        let store = FileProfilesStore::load_from_path(&path).expect("failed to load future file");
        let profile = store
            .profile("future")
            .expect("future profile should still load");
        assert_eq!(profile.host, "127.0.0.1");

        let untouched = fs::read_to_string(path).expect("failed to read future profile file");
        assert!(untouched.contains("version = 9"));
        assert!(untouched.contains("future_flag = \"enabled\""));
    }
}
