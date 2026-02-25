use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::profiles::{default_profiles_path, ProfilesError};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditOutcome {
    Started,
    Succeeded,
    Failed,
    Cancelled,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuditRecord {
    pub timestamp_unix_ms: u128,
    pub profile_name: Option<String>,
    pub database: Option<String>,
    pub outcome: AuditOutcome,
    pub sql: String,
    pub rows_streamed: Option<u64>,
    pub elapsed_ms: Option<u128>,
    pub error: Option<String>,
}

#[must_use]
pub fn unix_timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub const DEFAULT_AUDIT_MAX_BYTES: u64 = 5 * 1024 * 1024;
pub const DEFAULT_AUDIT_MAX_ARCHIVES: usize = 3;
const ENV_AUDIT_MAX_BYTES: &str = "MYR_AUDIT_MAX_BYTES";
const ENV_AUDIT_MAX_ARCHIVES: &str = "MYR_AUDIT_MAX_ARCHIVES";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuditRetentionPolicy {
    pub max_bytes: u64,
    pub max_archives: usize,
}

impl Default for AuditRetentionPolicy {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_AUDIT_MAX_BYTES,
            max_archives: DEFAULT_AUDIT_MAX_ARCHIVES,
        }
    }
}

impl AuditRetentionPolicy {
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            max_bytes: parse_env_u64(ENV_AUDIT_MAX_BYTES, DEFAULT_AUDIT_MAX_BYTES),
            max_archives: parse_env_usize(ENV_AUDIT_MAX_ARCHIVES, DEFAULT_AUDIT_MAX_ARCHIVES),
        }
    }
}

#[derive(Debug, Error)]
pub enum AuditTrailError {
    #[error("failed to resolve default config path: {0}")]
    Config(#[from] ProfilesError),
    #[error("invalid audit trail path `{0}`")]
    InvalidPath(PathBuf),
    #[error("failed to create audit trail directory at {path}: {source}")]
    CreateDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize audit record: {source}")]
    Serialize {
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to append audit record at {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read audit trail metadata at {path}: {source}")]
    Metadata {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to delete rotated audit trail file at {path}: {source}")]
    Delete {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to rotate audit trail file from {from} to {to}: {source}")]
    Rotate {
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Clone)]
pub struct FileAuditTrail {
    path: PathBuf,
    retention: AuditRetentionPolicy,
}

impl FileAuditTrail {
    pub fn load_default() -> Result<Self, AuditTrailError> {
        Ok(Self {
            path: default_audit_path()?,
            retention: AuditRetentionPolicy::from_env(),
        })
    }

    #[must_use]
    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            retention: AuditRetentionPolicy::default(),
        }
    }

    #[must_use]
    pub fn from_path_with_retention(
        path: impl Into<PathBuf>,
        retention: AuditRetentionPolicy,
    ) -> Self {
        Self {
            path: path.into(),
            retention,
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, record: &AuditRecord) -> Result<(), AuditTrailError> {
        let parent_dir = self
            .path
            .parent()
            .ok_or_else(|| AuditTrailError::InvalidPath(self.path.clone()))?;
        fs::create_dir_all(parent_dir).map_err(|source| AuditTrailError::CreateDir {
            path: parent_dir.to_path_buf(),
            source,
        })?;

        let rendered = serde_json::to_string(record)
            .map_err(|source| AuditTrailError::Serialize { source })?;
        let incoming_bytes = rendered
            .len()
            .saturating_add(1)
            .try_into()
            .unwrap_or(u64::MAX);
        self.rotate_if_needed(incoming_bytes)?;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|source| AuditTrailError::Write {
                path: self.path.clone(),
                source,
            })?;
        writeln!(file, "{rendered}").map_err(|source| AuditTrailError::Write {
            path: self.path.clone(),
            source,
        })
    }

    fn rotate_if_needed(&self, incoming_bytes: u64) -> Result<(), AuditTrailError> {
        let current_size = match fs::metadata(&self.path) {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(source) => {
                return Err(AuditTrailError::Metadata {
                    path: self.path.clone(),
                    source,
                });
            }
        };

        if current_size.saturating_add(incoming_bytes) <= self.retention.max_bytes {
            return Ok(());
        }

        let max_archives = self.retention.max_archives.max(1);
        let oldest = rotated_audit_path(&self.path, max_archives);
        if oldest.exists() {
            fs::remove_file(&oldest).map_err(|source| AuditTrailError::Delete {
                path: oldest.clone(),
                source,
            })?;
        }

        for index in (1..max_archives).rev() {
            let from = rotated_audit_path(&self.path, index);
            if !from.exists() {
                continue;
            }
            let to = rotated_audit_path(&self.path, index + 1);
            fs::rename(&from, &to).map_err(|source| AuditTrailError::Rotate {
                from,
                to,
                source,
            })?;
        }

        if self.path.exists() {
            let to = rotated_audit_path(&self.path, 1);
            fs::rename(&self.path, &to).map_err(|source| AuditTrailError::Rotate {
                from: self.path.clone(),
                to,
                source,
            })?;
        }

        Ok(())
    }
}

fn default_audit_path() -> Result<PathBuf, AuditTrailError> {
    let profiles_path = default_profiles_path()?;
    let Some(config_dir) = profiles_path.parent() else {
        return Err(AuditTrailError::InvalidPath(profiles_path));
    };
    Ok(config_dir.join("audit.ndjson"))
}

fn rotated_audit_path(path: &Path, index: usize) -> PathBuf {
    let mut rendered = path.as_os_str().to_os_string();
    rendered.push(format!(".{index}"));
    PathBuf::from(rendered)
}

fn parse_env_u64(name: &str, fallback: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(fallback)
}

fn parse_env_usize(name: &str, fallback: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(fallback)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{
        rotated_audit_path, unix_timestamp_millis, AuditOutcome, AuditRecord, AuditRetentionPolicy,
        FileAuditTrail,
    };

    #[test]
    fn appends_json_lines_to_file() {
        let temp_dir = TempDir::new().expect("failed to create temp directory");
        let path = temp_dir.path().join("audit.ndjson");
        let trail = FileAuditTrail::from_path(&path);

        let first = AuditRecord {
            timestamp_unix_ms: 1,
            profile_name: Some("local".to_string()),
            database: Some("app".to_string()),
            outcome: AuditOutcome::Started,
            sql: "SELECT 1".to_string(),
            rows_streamed: None,
            elapsed_ms: None,
            error: None,
        };
        trail.append(&first).expect("failed to append first record");

        let second = AuditRecord {
            timestamp_unix_ms: 2,
            profile_name: Some("local".to_string()),
            database: Some("app".to_string()),
            outcome: AuditOutcome::Succeeded,
            sql: "SELECT 1".to_string(),
            rows_streamed: Some(1),
            elapsed_ms: Some(5),
            error: None,
        };
        trail
            .append(&second)
            .expect("failed to append second record");

        let content = std::fs::read_to_string(path).expect("failed to read audit file");
        let mut lines = content.lines();

        let first_loaded: AuditRecord =
            serde_json::from_str(lines.next().expect("missing first line"))
                .expect("failed to parse first line");
        assert_eq!(first_loaded, first);

        let second_loaded: AuditRecord =
            serde_json::from_str(lines.next().expect("missing second line"))
                .expect("failed to parse second line");
        assert_eq!(second_loaded, second);

        assert!(
            lines.next().is_none(),
            "unexpected extra lines in audit file"
        );
    }

    #[test]
    fn timestamp_uses_unix_epoch_millis() {
        assert!(unix_timestamp_millis() > 0);
    }

    #[test]
    fn rotates_files_when_retention_threshold_is_exceeded() {
        let temp_dir = TempDir::new().expect("failed to create temp directory");
        let path = temp_dir.path().join("audit.ndjson");
        let trail = FileAuditTrail::from_path_with_retention(
            &path,
            AuditRetentionPolicy {
                max_bytes: 1,
                max_archives: 2,
            },
        );

        for timestamp in [1_u128, 2, 3, 4] {
            let record = AuditRecord {
                timestamp_unix_ms: timestamp,
                profile_name: Some("local".to_string()),
                database: Some("app".to_string()),
                outcome: AuditOutcome::Started,
                sql: "SELECT 1".to_string(),
                rows_streamed: None,
                elapsed_ms: None,
                error: None,
            };
            trail.append(&record).expect("append should succeed");
        }

        let current = std::fs::read_to_string(&path).expect("read current audit file");
        let archive_1 =
            std::fs::read_to_string(rotated_audit_path(&path, 1)).expect("read first archive");
        let archive_2 =
            std::fs::read_to_string(rotated_audit_path(&path, 2)).expect("read second archive");

        let current: AuditRecord =
            serde_json::from_str(current.trim_end()).expect("parse current record");
        let archive_1: AuditRecord =
            serde_json::from_str(archive_1.trim_end()).expect("parse archive one record");
        let archive_2: AuditRecord =
            serde_json::from_str(archive_2.trim_end()).expect("parse archive two record");

        assert_eq!(current.timestamp_unix_ms, 4);
        assert_eq!(archive_1.timestamp_unix_ms, 3);
        assert_eq!(archive_2.timestamp_unix_ms, 2);
        assert!(
            !rotated_audit_path(&path, 3).exists(),
            "retention should keep at most two archive files"
        );
    }
}
