use async_trait::async_trait;
use myr_core::profiles::ConnectionProfile;
use myr_core::query_runner::QueryRowStream;
use myr_core::schema_cache::{ColumnSchema, TableRelationship};

use crate::AppErrorKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationBackendError {
    pub kind: AppErrorKind,
    pub message: String,
}

impl ApplicationBackendError {
    #[must_use]
    pub fn new(kind: AppErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

#[async_trait]
pub trait ApplicationBackendFactory: Send + Sync + 'static {
    async fn connect(
        &self,
        profile: &ConnectionProfile,
    ) -> Result<std::sync::Arc<dyn ApplicationSession>, ApplicationBackendError>;
}

#[async_trait]
pub trait ApplicationSession: Send + Sync + 'static {
    async fn list_databases(&self) -> Result<Vec<String>, ApplicationBackendError>;

    async fn list_tables(&self, database: &str) -> Result<Vec<String>, ApplicationBackendError>;

    async fn list_columns(
        &self,
        database: &str,
        table: &str,
    ) -> Result<Vec<ColumnSchema>, ApplicationBackendError>;

    async fn list_relationships(
        &self,
        database: &str,
        table: &str,
    ) -> Result<Vec<TableRelationship>, ApplicationBackendError>;

    async fn start_query(
        &self,
        sql: &str,
    ) -> Result<Box<dyn QueryRowStream + Send>, ApplicationBackendError>;

    async fn disconnect(&self) -> Result<(), ApplicationBackendError>;
}
