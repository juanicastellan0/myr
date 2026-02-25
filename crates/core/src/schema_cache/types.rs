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
