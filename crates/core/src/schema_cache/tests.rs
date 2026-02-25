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
