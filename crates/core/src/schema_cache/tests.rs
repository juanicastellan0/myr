use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

use super::{
    ColumnSchema, DatabaseSchema, ForeignKeySchema, RelationshipDirection, SchemaBackend,
    SchemaBackendError, SchemaCacheService, SchemaCatalog, SchemaScope, TableRelationship,
    TableSchema,
};

#[derive(Debug, Default)]
struct BackendCounts {
    databases: AtomicUsize,
    tables: AtomicUsize,
    columns: AtomicUsize,
    relationships: AtomicUsize,
}

#[derive(Debug, Clone)]
struct FakeSchemaBackend {
    counts: Arc<BackendCounts>,
    schema: SchemaCatalog,
}

#[async_trait::async_trait]
impl SchemaBackend for FakeSchemaBackend {
    async fn list_databases(&self) -> Result<Vec<String>, SchemaBackendError> {
        self.counts.databases.fetch_add(1, Ordering::Relaxed);
        Ok(self
            .schema
            .databases
            .iter()
            .map(|database| database.name.clone())
            .collect())
    }

    async fn list_tables(&self, database: &str) -> Result<Vec<String>, SchemaBackendError> {
        self.counts.tables.fetch_add(1, Ordering::Relaxed);
        Ok(self
            .schema
            .database(database)
            .map(|database| {
                database
                    .tables
                    .iter()
                    .map(|table| table.name.clone())
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn list_columns(
        &self,
        database: &str,
        table: &str,
    ) -> Result<Vec<ColumnSchema>, SchemaBackendError> {
        self.counts.columns.fetch_add(1, Ordering::Relaxed);
        Ok(self
            .schema
            .database(database)
            .and_then(|database| {
                database
                    .tables
                    .iter()
                    .find(|candidate| candidate.name == table)
            })
            .map(|table| table.columns.clone())
            .unwrap_or_default())
    }

    async fn list_relationships(
        &self,
        database: &str,
        table: &str,
    ) -> Result<Vec<TableRelationship>, SchemaBackendError> {
        self.counts.relationships.fetch_add(1, Ordering::Relaxed);
        let mut output = Vec::new();
        let Some(database_schema) = self.schema.database(database) else {
            return Ok(output);
        };

        if let Some(table_schema) = database_schema
            .tables
            .iter()
            .find(|candidate| candidate.name == table)
        {
            output.extend(
                table_schema
                    .foreign_keys
                    .iter()
                    .map(|foreign_key| TableRelationship {
                        direction: RelationshipDirection::Outbound,
                        constraint_name: foreign_key.constraint_name.clone(),
                        source_column: foreign_key.column_name.clone(),
                        related_database: foreign_key.referenced_database.clone(),
                        related_table: foreign_key.referenced_table.clone(),
                        related_column: foreign_key.referenced_column.clone(),
                    }),
            );
        }
        for candidate in &database_schema.tables {
            output.extend(
                candidate
                    .foreign_keys
                    .iter()
                    .filter(|foreign_key| {
                        foreign_key.referenced_database == database
                            && foreign_key.referenced_table == table
                    })
                    .map(|foreign_key| TableRelationship {
                        direction: RelationshipDirection::Inbound,
                        constraint_name: foreign_key.constraint_name.clone(),
                        source_column: foreign_key.referenced_column.clone(),
                        related_database: database.to_string(),
                        related_table: candidate.name.clone(),
                        related_column: foreign_key.column_name.clone(),
                    }),
            );
        }
        Ok(output)
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
    let counts = Arc::new(BackendCounts::default());
    let backend = FakeSchemaBackend {
        counts: Arc::clone(&counts),
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

    assert_eq!(counts.databases.load(Ordering::Relaxed), 1);
    assert_eq!(counts.tables.load(Ordering::Relaxed), 1);
    assert_eq!(counts.columns.load(Ordering::Relaxed), 0);
    assert_eq!(counts.relationships.load(Ordering::Relaxed), 0);
    assert_eq!(databases, vec!["app".to_string(), "analytics".to_string()]);
    assert_eq!(tables, vec!["users".to_string(), "sessions".to_string()]);
}

#[tokio::test]
async fn zero_ttl_refetches_on_each_request() {
    let counts = Arc::new(BackendCounts::default());
    let backend = FakeSchemaBackend {
        counts: Arc::clone(&counts),
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

    assert_eq!(counts.databases.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn list_columns_returns_expected_shape() {
    let backend = FakeSchemaBackend {
        counts: Arc::new(BackendCounts::default()),
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
        counts: Arc::new(BackendCounts::default()),
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

#[tokio::test]
async fn scopes_load_lazily_and_invalidation_is_targeted() {
    let counts = Arc::new(BackendCounts::default());
    let backend = FakeSchemaBackend {
        counts: Arc::clone(&counts),
        schema: sample_schema(),
    };
    let mut cache = SchemaCacheService::new(backend, Duration::from_secs(60));

    cache.list_databases().await.expect("databases should load");
    assert_eq!(counts.tables.load(Ordering::Relaxed), 0);
    assert_eq!(counts.columns.load(Ordering::Relaxed), 0);

    cache.list_tables("app").await.expect("tables should load");
    assert_eq!(counts.columns.load(Ordering::Relaxed), 0);

    cache
        .list_columns("app", "users")
        .await
        .expect("columns should load");
    cache.invalidate_scope(&SchemaScope::Table {
        database: "app".to_string(),
        table: "users".to_string(),
    });
    cache
        .list_columns("app", "users")
        .await
        .expect("invalidated columns should reload");

    assert_eq!(counts.databases.load(Ordering::Relaxed), 1);
    assert_eq!(counts.tables.load(Ordering::Relaxed), 1);
    assert_eq!(counts.columns.load(Ordering::Relaxed), 2);
}
