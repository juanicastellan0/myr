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
pub struct ForeignKeySchema {
    pub constraint_name: String,
    pub column_name: String,
    pub referenced_database: String,
    pub referenced_table: String,
    pub referenced_column: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableSchema {
    pub name: String,
    pub columns: Vec<ColumnSchema>,
    pub foreign_keys: Vec<ForeignKeySchema>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RelationshipDirection {
    Outbound,
    Inbound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableRelationship {
    pub direction: RelationshipDirection,
    pub constraint_name: String,
    pub source_column: String,
    pub related_database: String,
    pub related_table: String,
    pub related_column: String,
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

    pub async fn list_related_tables(
        &mut self,
        database_name: &str,
        table_name: &str,
    ) -> Result<Vec<TableRelationship>, SchemaCacheError> {
        let schema = self.schema().await?;
        let mut relationships = Vec::new();

        let Some(database) = schema.database(database_name) else {
            return Ok(relationships);
        };

        if let Some(table) = database.tables.iter().find(|table| table.name == table_name) {
            for foreign_key in &table.foreign_keys {
                relationships.push(TableRelationship {
                    direction: RelationshipDirection::Outbound,
                    constraint_name: foreign_key.constraint_name.clone(),
                    source_column: foreign_key.column_name.clone(),
                    related_database: foreign_key.referenced_database.clone(),
                    related_table: foreign_key.referenced_table.clone(),
                    related_column: foreign_key.referenced_column.clone(),
                });
            }
        }

        for candidate_table in &database.tables {
            for foreign_key in &candidate_table.foreign_keys {
                if foreign_key.referenced_table == table_name
                    && foreign_key.referenced_database == database_name
                {
                    relationships.push(TableRelationship {
                        direction: RelationshipDirection::Inbound,
                        constraint_name: foreign_key.constraint_name.clone(),
                        source_column: foreign_key.referenced_column.clone(),
                        related_database: database_name.to_string(),
                        related_table: candidate_table.name.clone(),
                        related_column: foreign_key.column_name.clone(),
                    });
                }
            }
        }

        relationships.sort_unstable_by(|left, right| {
            left.related_database
                .cmp(&right.related_database)
                .then_with(|| left.related_table.cmp(&right.related_table))
                .then_with(|| left.related_column.cmp(&right.related_column))
                .then_with(|| left.constraint_name.cmp(&right.constraint_name))
                .then_with(|| left.direction.cmp(&right.direction))
        });

        Ok(relationships)
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
        ColumnSchema, DatabaseSchema, ForeignKeySchema, RelationshipDirection, SchemaBackend,
        SchemaBackendError, SchemaCacheService, SchemaCatalog, TableSchema,
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
                            foreign_keys: Vec::new(),
                        },
                        TableSchema {
                            name: "sessions".to_string(),
                            columns: vec![
                                ColumnSchema {
                                    name: "user_id".to_string(),
                                    data_type: "bigint".to_string(),
                                    nullable: false,
                                    default_value: None,
                                },
                                ColumnSchema {
                                    name: "token".to_string(),
                                    data_type: "varchar(255)".to_string(),
                                    nullable: false,
                                    default_value: None,
                                },
                            ],
                            foreign_keys: vec![ForeignKeySchema {
                                constraint_name: "fk_sessions_users".to_string(),
                                column_name: "user_id".to_string(),
                                referenced_database: "app".to_string(),
                                referenced_table: "users".to_string(),
                                referenced_column: "id".to_string(),
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
                        foreign_keys: Vec::new(),
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

    #[tokio::test]
    async fn list_related_tables_returns_outbound_and_inbound_relationships() {
        let backend = FakeSchemaBackend {
            fetch_count: Arc::new(AtomicUsize::new(0)),
            schema: sample_schema(),
        };
        let mut cache = SchemaCacheService::new(backend, Duration::from_secs(60));

        let related = cache
            .list_related_tables("app", "users")
            .await
            .expect("relationship listing should succeed");

        assert_eq!(related.len(), 1);
        assert_eq!(related[0].direction, RelationshipDirection::Inbound);
        assert_eq!(related[0].related_table, "sessions");
        assert_eq!(related[0].related_column, "user_id");

        let outbound = cache
            .list_related_tables("app", "sessions")
            .await
            .expect("relationship listing should succeed");
        assert_eq!(outbound.len(), 1);
        assert_eq!(outbound[0].direction, RelationshipDirection::Outbound);
        assert_eq!(outbound[0].related_table, "users");
        assert_eq!(outbound[0].related_column, "id");
    }
}
