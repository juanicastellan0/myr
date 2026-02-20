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
}

#[derive(Debug, Clone)]
pub struct FileAuditTrail {
    path: PathBuf,
}

impl FileAuditTrail {
    pub fn load_default() -> Result<Self, AuditTrailError> {
        Ok(Self {
            path: default_audit_path()?,
        })
    }

    #[must_use]
    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
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
}

fn default_audit_path() -> Result<PathBuf, AuditTrailError> {
    let profiles_path = default_profiles_path()?;
    let Some(config_dir) = profiles_path.parent() else {
        return Err(AuditTrailError::InvalidPath(profiles_path));
    };
    Ok(config_dir.join("audit.ndjson"))
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{unix_timestamp_millis, AuditOutcome, AuditRecord, FileAuditTrail};

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
}
