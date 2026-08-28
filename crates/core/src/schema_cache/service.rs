use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::{ColumnSchema, SchemaBackend, SchemaCacheError, SchemaScope, TableRelationship};

#[derive(Debug)]
struct Cached<T> {
    fetched_at: Instant,
    value: T,
}

#[derive(Debug)]
pub struct SchemaCacheService<B: SchemaBackend> {
    backend: B,
    ttl: Duration,
    databases: Option<Cached<Vec<String>>>,
    tables: HashMap<String, Cached<Vec<String>>>,
    columns: HashMap<(String, String), Cached<Vec<ColumnSchema>>>,
    relationships: HashMap<(String, String), Cached<Vec<TableRelationship>>>,
}

impl<B: SchemaBackend> SchemaCacheService<B> {
    #[must_use]
    pub fn new(backend: B, ttl: Duration) -> Self {
        Self {
            backend,
            ttl,
            databases: None,
            tables: HashMap::new(),
            columns: HashMap::new(),
            relationships: HashMap::new(),
        }
    }

    #[must_use]
    pub fn ttl(&self) -> Duration {
        self.ttl
    }

    pub fn invalidate(&mut self) {
        self.invalidate_scope(&SchemaScope::All);
    }

    pub fn invalidate_scope(&mut self, scope: &SchemaScope) {
        match scope {
            SchemaScope::All => {
                self.databases = None;
                self.tables.clear();
                self.columns.clear();
                self.relationships.clear();
            }
            SchemaScope::Databases => self.databases = None,
            SchemaScope::Tables { database } => {
                self.tables.remove(database);
                self.columns.retain(|(db, _), _| db != database);
                self.relationships.retain(|(db, _), _| db != database);
            }
            SchemaScope::Table { database, table } => {
                let key = (database.clone(), table.clone());
                self.columns.remove(&key);
                self.relationships.remove(&key);
            }
        }
    }

    pub fn prime_databases(&mut self, databases: Vec<String>) {
        self.databases = Some(Cached {
            fetched_at: Instant::now(),
            value: databases,
        });
    }

    pub async fn list_databases(&mut self) -> Result<Vec<String>, SchemaCacheError> {
        let now = Instant::now();
        if let Some(cached) = &self.databases {
            if is_fresh(cached, now, self.ttl) {
                return Ok(cached.value.clone());
            }
        }

        let value = self
            .backend
            .list_databases()
            .await
            .map_err(SchemaCacheError::Backend)?;
        self.databases = Some(Cached {
            fetched_at: Instant::now(),
            value: value.clone(),
        });
        Ok(value)
    }

    pub async fn list_tables(
        &mut self,
        database_name: &str,
    ) -> Result<Vec<String>, SchemaCacheError> {
        let now = Instant::now();
        if let Some(cached) = self.tables.get(database_name) {
            if is_fresh(cached, now, self.ttl) {
                return Ok(cached.value.clone());
            }
        }

        let value = self
            .backend
            .list_tables(database_name)
            .await
            .map_err(SchemaCacheError::Backend)?;
        self.tables.insert(
            database_name.to_string(),
            Cached {
                fetched_at: Instant::now(),
                value: value.clone(),
            },
        );
        Ok(value)
    }

    pub async fn list_columns(
        &mut self,
        database_name: &str,
        table_name: &str,
    ) -> Result<Vec<ColumnSchema>, SchemaCacheError> {
        let now = Instant::now();
        let key = (database_name.to_string(), table_name.to_string());
        if let Some(cached) = self.columns.get(&key) {
            if is_fresh(cached, now, self.ttl) {
                return Ok(cached.value.clone());
            }
        }

        let value = self
            .backend
            .list_columns(database_name, table_name)
            .await
            .map_err(SchemaCacheError::Backend)?;
        self.columns.insert(
            key,
            Cached {
                fetched_at: Instant::now(),
                value: value.clone(),
            },
        );
        Ok(value)
    }

    pub async fn list_relationships(
        &mut self,
        database_name: &str,
        table_name: &str,
    ) -> Result<Vec<TableRelationship>, SchemaCacheError> {
        let now = Instant::now();
        let key = (database_name.to_string(), table_name.to_string());
        if let Some(cached) = self.relationships.get(&key) {
            if is_fresh(cached, now, self.ttl) {
                return Ok(cached.value.clone());
            }
        }

        let value = self
            .backend
            .list_relationships(database_name, table_name)
            .await
            .map_err(SchemaCacheError::Backend)?;
        self.relationships.insert(
            key,
            Cached {
                fetched_at: Instant::now(),
                value: value.clone(),
            },
        );
        Ok(value)
    }

    pub async fn list_related_tables(
        &mut self,
        database_name: &str,
        table_name: &str,
    ) -> Result<Vec<TableRelationship>, SchemaCacheError> {
        self.list_relationships(database_name, table_name).await
    }
}

fn is_fresh<T>(cached: &Cached<T>, now: Instant, ttl: Duration) -> bool {
    now.duration_since(cached.fetched_at) <= ttl
}
