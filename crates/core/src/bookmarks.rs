use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SavedBookmark {
    pub name: String,
    #[serde(default)]
    pub profile_name: Option<String>,
    #[serde(default)]
    pub database: Option<String>,
    #[serde(default)]
    pub table: Option<String>,
    #[serde(default)]
    pub column: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
}

impl SavedBookmark {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            profile_name: None,
            database: None,
            table: None,
            column: None,
            query: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum BookmarksError {
    #[error("config directory is unavailable for this platform")]
    ConfigDirUnavailable,
    #[error("failed to read bookmarks file at {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse bookmarks file at {path}: {source}")]
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
    #[error("failed to serialize bookmarks: {source}")]
    Serialize {
        #[source]
        source: toml::ser::Error,
    },
    #[error("failed to write bookmarks file at {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct BookmarksDocument {
    #[serde(default)]
    bookmarks: Vec<SavedBookmark>,
}

impl BookmarksDocument {
    fn normalize(&mut self) {
        let mut by_name = std::collections::BTreeMap::new();
        for bookmark in self.bookmarks.drain(..) {
            by_name.insert(bookmark.name.clone(), bookmark);
        }
        self.bookmarks = by_name.into_values().collect();
    }
}

#[derive(Debug, Clone)]
pub struct FileBookmarksStore {
    path: PathBuf,
    bookmarks: Vec<SavedBookmark>,
}

impl FileBookmarksStore {
    pub fn load_default() -> Result<Self, BookmarksError> {
        let path = default_bookmarks_path()?;
        Self::load_from_path(path)
    }

    pub fn load_from_path(path: impl Into<PathBuf>) -> Result<Self, BookmarksError> {
        let path = path.into();
        if !path.exists() {
            return Ok(Self {
                path,
                bookmarks: Vec::new(),
            });
        }

        let raw = fs::read_to_string(&path).map_err(|source| BookmarksError::Read {
            path: path.clone(),
            source,
        })?;

        if raw.trim().is_empty() {
            return Ok(Self {
                path,
                bookmarks: Vec::new(),
            });
        }

        let mut doc: BookmarksDocument =
            toml::from_str(&raw).map_err(|source| BookmarksError::Parse {
                path: path.clone(),
                source,
            })?;
        doc.normalize();

        Ok(Self {
            path,
            bookmarks: doc.bookmarks,
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn bookmarks(&self) -> &[SavedBookmark] {
        &self.bookmarks
    }

    #[must_use]
    pub fn bookmark(&self, name: &str) -> Option<&SavedBookmark> {
        self.bookmarks
            .iter()
            .find(|bookmark| bookmark.name == name)
    }

    pub fn upsert_bookmark(&mut self, bookmark: SavedBookmark) {
        if let Some(existing) = self
            .bookmarks
            .iter_mut()
            .find(|existing| existing.name == bookmark.name)
        {
            *existing = bookmark;
        } else {
            self.bookmarks.push(bookmark);
            self.bookmarks.sort_unstable_by(|a, b| a.name.cmp(&b.name));
        }
    }

    #[must_use]
    pub fn delete_bookmark(&mut self, name: &str) -> bool {
        let original_len = self.bookmarks.len();
        self.bookmarks.retain(|bookmark| bookmark.name != name);
        self.bookmarks.len() != original_len
    }

    pub fn persist(&self) -> Result<(), BookmarksError> {
        if let Some(parent_dir) = self.path.parent() {
            fs::create_dir_all(parent_dir).map_err(|source| BookmarksError::CreateDir {
                path: parent_dir.to_path_buf(),
                source,
            })?;
        }

        let doc = BookmarksDocument {
            bookmarks: self.bookmarks.clone(),
        };
        let rendered =
            toml::to_string_pretty(&doc).map_err(|source| BookmarksError::Serialize { source })?;

        fs::write(&self.path, rendered).map_err(|source| BookmarksError::Write {
            path: self.path.clone(),
            source,
        })
    }
}

pub fn default_bookmarks_path() -> Result<PathBuf, BookmarksError> {
    let base_dir = if let Some(custom) = env::var_os("MYR_CONFIG_DIR") {
        PathBuf::from(custom)
    } else if cfg!(target_os = "windows") {
        env::var_os("APPDATA")
            .map(PathBuf::from)
            .ok_or(BookmarksError::ConfigDirUnavailable)?
    } else if let Some(xdg_config_home) = env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(xdg_config_home)
    } else {
        let home = env::var_os("HOME").ok_or(BookmarksError::ConfigDirUnavailable)?;
        PathBuf::from(home).join(".config")
    };

    Ok(base_dir.join("myr").join("bookmarks.toml"))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::{FileBookmarksStore, SavedBookmark};

    fn temp_bookmarks_path(temp_dir: &TempDir) -> PathBuf {
        temp_dir.path().join("bookmarks.toml")
    }

    #[test]
    fn missing_bookmarks_file_loads_empty_store() {
        let temp_dir = TempDir::new().expect("failed to create temp directory");
        let path = temp_bookmarks_path(&temp_dir);

        let store = FileBookmarksStore::load_from_path(path).expect("failed to load store");
        assert!(store.bookmarks().is_empty());
    }

    #[test]
    fn upsert_persist_reload_and_delete_bookmark() {
        let temp_dir = TempDir::new().expect("failed to create temp directory");
        let path = temp_bookmarks_path(&temp_dir);

        let mut store = FileBookmarksStore::load_from_path(&path).expect("failed to load store");
        let mut bookmark = SavedBookmark::new("users-default");
        bookmark.database = Some("app".to_string());
        bookmark.table = Some("users".to_string());
        bookmark.column = Some("id".to_string());
        bookmark.query = Some("SELECT * FROM `app`.`users` LIMIT 200".to_string());

        store.upsert_bookmark(bookmark.clone());
        store.persist().expect("failed to persist store");

        let mut reloaded = FileBookmarksStore::load_from_path(&path).expect("failed to reload");
        let loaded = reloaded
            .bookmark("users-default")
            .expect("missing bookmark after save");
        assert_eq!(loaded, &bookmark);

        let mut updated = loaded.clone();
        updated.query = Some("SELECT id FROM `app`.`users` LIMIT 20".to_string());
        reloaded.upsert_bookmark(updated.clone());
        reloaded
            .persist()
            .expect("failed to persist updated bookmark");

        let mut reloaded = FileBookmarksStore::load_from_path(&path).expect("failed to reload");
        let loaded = reloaded
            .bookmark("users-default")
            .expect("missing bookmark after update");
        assert_eq!(
            loaded.query.as_deref(),
            Some("SELECT id FROM `app`.`users` LIMIT 20")
        );

        assert!(reloaded.delete_bookmark("users-default"));
        reloaded.persist().expect("failed to persist deletion");

        let reloaded = FileBookmarksStore::load_from_path(path).expect("failed final reload");
        assert!(reloaded.bookmark("users-default").is_none());
        assert!(reloaded.bookmarks().is_empty());
    }
}
