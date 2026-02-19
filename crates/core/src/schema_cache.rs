use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnSchema {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default_value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableSchema {
    pub name: String,
    pub columns: Vec<ColumnSchema>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseSchema {
    pub name: String,
    pub tables: Vec<TableSchema>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SchemaCatalog {
    pub databases: Vec<DatabaseSchema>,
}

impl SchemaCatalog {
    #[must_use]
    pub fn database(&self, name: &str) -> Option<&DatabaseSchema> {
        self.databases.iter().find(|database| database.name == name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct SchemaBackendError {
    message: String,
}

impl SchemaBackendError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug, Error)]
pub enum SchemaCacheError {
    #[error("schema backend failed: {0}")]
    Backend(#[source] SchemaBackendError),
}

#[async_trait]
pub trait SchemaBackend {
    async fn fetch_schema(&self) -> Result<SchemaCatalog, SchemaBackendError>;
}

#[derive(Debug)]
struct CachedSchema {
    fetched_at: Instant,
    schema: Arc<SchemaCatalog>,
}

#[derive(Debug)]
pub struct SchemaCacheService<B: SchemaBackend> {
    backend: B,
    ttl: Duration,
    cache: Option<CachedSchema>,
}

impl<B: SchemaBackend> SchemaCacheService<B> {
    #[must_use]
    pub fn new(backend: B, ttl: Duration) -> Self {
        Self {
            backend,
            ttl,
            cache: None,
        }
    }

    #[must_use]
    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    pub fn invalidate(&mut self) {
        self.cache = None;
    }

    pub async fn schema(&mut self) -> Result<Arc<SchemaCatalog>, SchemaCacheError> {
        self.schema_at(Instant::now()).await
    }

    pub async fn refresh(&mut self) -> Result<Arc<SchemaCatalog>, SchemaCacheError> {
        self.refresh_at(Instant::now()).await
    }

    pub async fn list_databases(&mut self) -> Result<Vec<String>, SchemaCacheError> {
        let schema = self.schema().await?;
        Ok(schema
            .databases
            .iter()
            .map(|database| database.name.clone())
            .collect())
    }

    pub async fn list_tables(
        &mut self,
        database_name: &str,
    ) -> Result<Vec<String>, SchemaCacheError> {
        let schema = self.schema().await?;
        Ok(schema
            .database(database_name)
            .map(|database| {
                database
                    .tables
                    .iter()
                    .map(|table| table.name.clone())
                    .collect()
            })
            .unwrap_or_default())
    }

    pub async fn list_columns(
        &mut self,
        database_name: &str,
        table_name: &str,
    ) -> Result<Vec<ColumnSchema>, SchemaCacheError> {
        let schema = self.schema().await?;
        let columns = schema
            .database(database_name)
            .and_then(|database| {
                database
                    .tables
                    .iter()
                    .find(|table| table.name == table_name)
            })
            .map(|table| table.columns.clone())
            .unwrap_or_default();
        Ok(columns)
    }

    async fn schema_at(&mut self, now: Instant) -> Result<Arc<SchemaCatalog>, SchemaCacheError> {
        if let Some(cache) = &self.cache {
            if now.duration_since(cache.fetched_at) <= self.ttl {
                return Ok(Arc::clone(&cache.schema));
            }
        }
        self.refresh_at(now).await
    }

    async fn refresh_at(&mut self, now: Instant) -> Result<Arc<SchemaCatalog>, SchemaCacheError> {
        let schema = Arc::new(
            self.backend
                .fetch_schema()
                .await
                .map_err(SchemaCacheError::Backend)?,
        );

        self.cache = Some(CachedSchema {
            fetched_at: now,
            schema: Arc::clone(&schema),
        });
        Ok(schema)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use std::time::Duration;

    use super::{
        ColumnSchema, DatabaseSchema, SchemaBackend, SchemaBackendError, SchemaCacheService,
        SchemaCatalog, TableSchema,
    };

    #[derive(Debug, Clone)]
    struct FakeSchemaBackend {
        fetch_count: Arc<AtomicUsize>,
        schema: SchemaCatalog,
    }

    #[async_trait::async_trait]
    impl SchemaBackend for FakeSchemaBackend {
        async fn fetch_schema(&self) -> Result<SchemaCatalog, SchemaBackendError> {
            self.fetch_count.fetch_add(1, Ordering::Relaxed);
            Ok(self.schema.clone())
        }
    }

    fn sample_schema() -> SchemaCatalog {
        SchemaCatalog {
            databases: vec![
                DatabaseSchema {
                    name: "app".to_string(),
                    tables: vec![
                        TableSchema {
                            name: "users".to_string(),
                            columns: vec![
                                ColumnSchema {
                                    name: "id".to_string(),
                                    data_type: "bigint".to_string(),
                                    nullable: false,
                                    default_value: None,
                                },
                                ColumnSchema {
                                    name: "email".to_string(),
                                    data_type: "varchar(255)".to_string(),
                                    nullable: false,
                                    default_value: None,
                                },
                            ],
                        },
                        TableSchema {
                            name: "sessions".to_string(),
                            columns: vec![ColumnSchema {
                                name: "token".to_string(),
                                data_type: "varchar(255)".to_string(),
                                nullable: false,
                                default_value: None,
                            }],
                        },
                    ],
                },
                DatabaseSchema {
                    name: "analytics".to_string(),
                    tables: vec![TableSchema {
                        name: "events".to_string(),
                        columns: vec![ColumnSchema {
                            name: "occurred_at".to_string(),
                            data_type: "datetime".to_string(),
                            nullable: false,
                            default_value: None,
                        }],
                    }],
                },
            ],
        }
    }

    #[tokio::test]
    async fn uses_cache_within_ttl() {
        let fetch_count = Arc::new(AtomicUsize::new(0));
        let backend = FakeSchemaBackend {
            fetch_count: Arc::clone(&fetch_count),
            schema: sample_schema(),
        };
        let mut cache = SchemaCacheService::new(backend, Duration::from_secs(60));

        let databases = cache
            .list_databases()
            .await
            .expect("first read should load schema");
        let tables = cache
            .list_tables("app")
            .await
            .expect("second read should use cache");

        assert_eq!(fetch_count.load(Ordering::Relaxed), 1);
        assert_eq!(databases, vec!["app".to_string(), "analytics".to_string()]);
        assert_eq!(tables, vec!["users".to_string(), "sessions".to_string()]);
    }

    #[tokio::test]
    async fn zero_ttl_refetches_on_each_request() {
        let fetch_count = Arc::new(AtomicUsize::new(0));
        let backend = FakeSchemaBackend {
            fetch_count: Arc::clone(&fetch_count),
            schema: sample_schema(),
        };
        let mut cache = SchemaCacheService::new(backend, Duration::ZERO);

        cache
            .list_databases()
            .await
            .expect("first read should load schema");
        cache
            .list_databases()
            .await
            .expect("second read should refresh schema");

        assert_eq!(fetch_count.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn list_columns_returns_expected_shape() {
        let backend = FakeSchemaBackend {
            fetch_count: Arc::new(AtomicUsize::new(0)),
            schema: sample_schema(),
        };
        let mut cache = SchemaCacheService::new(backend, Duration::from_secs(60));

        let columns = cache
            .list_columns("app", "users")
            .await
            .expect("column listing should succeed");

        assert_eq!(columns.len(), 2);
        assert_eq!(columns[0].name, "id");
        assert_eq!(columns[1].name, "email");
    }
}
