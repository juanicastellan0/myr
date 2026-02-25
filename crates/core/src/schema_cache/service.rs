use std::sync::Arc;
use std::time::{Duration, Instant};

use super::relationships::collect_table_relationships;
use super::{ColumnSchema, SchemaBackend, SchemaCacheError, SchemaCatalog, TableRelationship};

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
        Ok(collect_table_relationships(
            schema.as_ref(),
            database_name,
            table_name,
        ))
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
