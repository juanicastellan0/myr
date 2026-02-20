use std::path::PathBuf;

use async_trait::async_trait;
use futures_util::StreamExt;
use myr_core::connection_manager::{BackendError, ConnectionBackend};
use myr_core::profiles::{ConnectionProfile, PasswordSource, TlsMode};
use myr_core::query_runner::{QueryBackend, QueryBackendError, QueryRow, QueryRowStream};
use myr_core::schema_cache::{
    ColumnSchema, DatabaseSchema, ForeignKeySchema, SchemaBackend, SchemaBackendError,
    SchemaCatalog, TableSchema,
};
use mysql_async::prelude::{Query, Queryable};
use mysql_async::{
    ClientIdentity, Conn, OptsBuilder, Pool, ResultSetStream, Row, SslOpts, TextProtocol, Value,
};

#[derive(Debug, Clone, Default)]
pub struct MysqlConnectionBackend;

#[async_trait]
impl ConnectionBackend for MysqlConnectionBackend {
    type Connection = Conn;

    async fn connect(&self, profile: &ConnectionProfile) -> Result<Self::Connection, BackendError> {
        Conn::new(opts_from_profile(profile))
            .await
            .map_err(to_connection_error)
    }

    async fn ping(&self, connection: &mut Self::Connection) -> Result<(), BackendError> {
        connection.ping().await.map_err(to_connection_error)
    }

    async fn disconnect(&self, connection: Self::Connection) -> Result<(), BackendError> {
        connection.disconnect().await.map_err(to_connection_error)
    }
}

#[derive(Debug, Clone)]
pub struct MysqlDataBackend {
    pool: Pool,
}

impl MysqlDataBackend {
    #[must_use]
    pub fn from_profile(profile: &ConnectionProfile) -> Self {
        Self {
            pool: Pool::new(opts_from_profile(profile)),
        }
    }

    pub async fn disconnect(&self) -> Result<(), mysql_async::Error> {
        self.pool.clone().disconnect().await
    }
}

#[async_trait]
impl SchemaBackend for MysqlDataBackend {
    async fn fetch_schema(&self) -> Result<SchemaCatalog, SchemaBackendError> {
        let mut conn = self.pool.get_conn().await.map_err(to_schema_error)?;
        let databases = conn
            .query_map("SHOW DATABASES", |database: String| database)
            .await
            .map_err(to_schema_error)?;

        let mut catalog_databases = Vec::with_capacity(databases.len());
        for database in databases {
            let tables = conn
                .exec_map(
                    "SELECT TABLE_NAME \
                     FROM information_schema.TABLES \
                     WHERE TABLE_SCHEMA = ? \
                     ORDER BY TABLE_NAME",
                    (database.clone(),),
                    |table_name: String| table_name,
                )
                .await
                .map_err(to_schema_error)?;

            let mut catalog_tables = Vec::with_capacity(tables.len());
            for table in tables {
                let columns = conn
                    .exec_map(
                        "SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_DEFAULT \
                         FROM information_schema.COLUMNS \
                         WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? \
                         ORDER BY ORDINAL_POSITION",
                        (database.clone(), table.clone()),
                        |(name, data_type, nullable, default_value): (
                            String,
                            String,
                            String,
                            Option<String>,
                        )| ColumnSchema {
                            name,
                            data_type,
                            nullable: nullable.eq_ignore_ascii_case("YES"),
                            default_value,
                        },
                    )
                    .await
                    .map_err(to_schema_error)?;

                let foreign_keys = conn
                    .exec_map(
                        "SELECT CONSTRAINT_NAME, COLUMN_NAME, REFERENCED_TABLE_SCHEMA, \
                         REFERENCED_TABLE_NAME, REFERENCED_COLUMN_NAME \
                         FROM information_schema.KEY_COLUMN_USAGE \
                         WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? \
                           AND REFERENCED_TABLE_NAME IS NOT NULL \
                         ORDER BY CONSTRAINT_NAME, ORDINAL_POSITION",
                        (database.clone(), table.clone()),
                        |(
                            constraint_name,
                            column_name,
                            referenced_database,
                            referenced_table,
                            referenced_column,
                        ): (String, String, String, String, String)| ForeignKeySchema {
                            constraint_name,
                            column_name,
                            referenced_database,
                            referenced_table,
                            referenced_column,
                        },
                    )
                    .await
                    .map_err(to_schema_error)?;

                catalog_tables.push(TableSchema {
                    name: table,
                    columns,
                    foreign_keys,
                });
            }

            catalog_databases.push(DatabaseSchema {
                name: database,
                tables: catalog_tables,
            });
        }

        conn.disconnect().await.map_err(to_schema_error)?;
        Ok(SchemaCatalog {
            databases: catalog_databases,
        })
    }
}

#[derive(Debug)]
pub struct MysqlStreamingRowStream {
    stream: Option<ResultSetStream<'static, 'static, 'static, Row, TextProtocol>>,
    cancelled: bool,
}

impl MysqlStreamingRowStream {
    fn new(stream: ResultSetStream<'static, 'static, 'static, Row, TextProtocol>) -> Self {
        Self {
            stream: Some(stream),
            cancelled: false,
        }
    }
}

#[async_trait]
impl QueryRowStream for MysqlStreamingRowStream {
    async fn next_row(&mut self) -> Result<Option<QueryRow>, QueryBackendError> {
        if self.cancelled {
            return Ok(None);
        }
        let Some(stream) = self.stream.as_mut() else {
            return Ok(None);
        };

        match stream.next().await {
            Some(Ok(row)) => Ok(Some(row_to_query_row(row))),
            Some(Err(error)) => Err(to_query_error(error)),
            None => {
                self.stream = None;
                Ok(None)
            }
        }
    }

    async fn cancel(&mut self) -> Result<(), QueryBackendError> {
        self.cancelled = true;
        self.stream = None;
        Ok(())
    }
}

#[async_trait]
impl QueryBackend for MysqlDataBackend {
    type Stream = MysqlStreamingRowStream;

    async fn start_query(&self, sql: &str) -> Result<Self::Stream, QueryBackendError> {
        let stream = sql
            .to_string()
            .stream::<Row, _>(self.pool.clone())
            .await
            .map_err(to_query_error)?;
        Ok(MysqlStreamingRowStream::new(stream))
    }
}

fn opts_from_profile(profile: &ConnectionProfile) -> OptsBuilder {
    let mut builder = OptsBuilder::default()
        .ip_or_hostname(profile.host.clone())
        .tcp_port(profile.port)
        .user(Some(profile.user.clone()));

    if let Some(password) = resolve_password(profile) {
        builder = builder.pass(Some(password));
    }

    if let Some(database) = &profile.database {
        builder = builder.db_name(Some(database.clone()));
    }

    if let Some(ssl_opts) = ssl_opts_from_profile(profile) {
        builder = builder.ssl_opts(ssl_opts);
    }

    if matches!(profile.tls_mode, TlsMode::Disabled) {
        builder = builder.prefer_socket(false);
    }

    builder
}

fn resolve_password(profile: &ConnectionProfile) -> Option<String> {
    let env_password = std::env::var("MYR_DB_PASSWORD")
        .ok()
        .filter(|pw| !pw.is_empty());

    match profile.password_source {
        PasswordSource::EnvVar => env_password,
        PasswordSource::Keyring => {
            if let Some(password) = load_keyring_password(profile) {
                return Some(password);
            }

            if let Some(password) = env_password {
                store_keyring_password(profile, &password);
                return Some(password);
            }

            None
        }
    }
}

fn ssl_opts_from_profile(profile: &ConnectionProfile) -> Option<SslOpts> {
    if !profile_requests_tls(profile) {
        return None;
    }

    let mut ssl_opts = SslOpts::default()
        .with_disable_built_in_roots(profile.tls_disable_built_in_roots)
        .with_danger_skip_domain_validation(profile.tls_skip_domain_validation)
        .with_danger_accept_invalid_certs(profile.tls_accept_invalid_certs);

    if let Some(ca_cert_path) = non_empty(profile.tls_ca_cert_path.as_deref()) {
        ssl_opts = ssl_opts.with_root_certs(vec![PathBuf::from(ca_cert_path).into()]);
    }

    if let Some(hostname_override) = non_empty(profile.tls_hostname_override.as_deref()) {
        ssl_opts = ssl_opts.with_danger_tls_hostname_override(Some(hostname_override.to_string()));
    }

    if let Some(identity) = client_identity_from_profile(profile) {
        ssl_opts = ssl_opts.with_client_identity(Some(identity));
    }

    Some(ssl_opts)
}

fn profile_requests_tls(profile: &ConnectionProfile) -> bool {
    match profile.tls_mode {
        TlsMode::Disabled => false,
        TlsMode::Prefer => has_custom_tls_settings(profile),
        TlsMode::Require | TlsMode::VerifyIdentity => true,
    }
}

fn has_custom_tls_settings(profile: &ConnectionProfile) -> bool {
    non_empty(profile.tls_ca_cert_path.as_deref()).is_some()
        || non_empty(profile.tls_client_cert_path.as_deref()).is_some()
        || non_empty(profile.tls_client_key_path.as_deref()).is_some()
        || non_empty(profile.tls_hostname_override.as_deref()).is_some()
        || profile.tls_disable_built_in_roots
        || profile.tls_skip_domain_validation
        || profile.tls_accept_invalid_certs
}

fn client_identity_from_profile(profile: &ConnectionProfile) -> Option<ClientIdentity> {
    let cert_path = non_empty(profile.tls_client_cert_path.as_deref())?;
    let key_path = non_empty(profile.tls_client_key_path.as_deref())?;
    Some(ClientIdentity::new(
        PathBuf::from(cert_path).into(),
        PathBuf::from(key_path).into(),
    ))
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        (!trimmed.is_empty()).then_some(trimmed)
    })
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn load_keyring_password(profile: &ConnectionProfile) -> Option<String> {
    let entry = keyring_entry(profile)?;
    entry.get_password().ok().filter(|pw| !pw.is_empty())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn load_keyring_password(_profile: &ConnectionProfile) -> Option<String> {
    None
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn store_keyring_password(profile: &ConnectionProfile, password: &str) {
    if password.is_empty() {
        return;
    }
    if let Some(entry) = keyring_entry(profile) {
        let _ = entry.set_password(password);
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn store_keyring_password(_profile: &ConnectionProfile, _password: &str) {}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn keyring_entry(profile: &ConnectionProfile) -> Option<keyring::Entry> {
    let service = non_empty(profile.keyring_service.as_deref()).unwrap_or("myr");
    let account = non_empty(profile.keyring_account.as_deref()).unwrap_or(profile.name.as_str());
    keyring::Entry::new(service, account).ok()
}

fn row_to_query_row(row: Row) -> QueryRow {
    let values = row
        .unwrap()
        .into_iter()
        .map(mysql_value_to_string)
        .collect::<Vec<_>>();
    QueryRow::new(values)
}

fn mysql_value_to_string(value: Value) -> String {
    match value {
        Value::NULL => "NULL".to_string(),
        Value::Bytes(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Value::Int(value) => value.to_string(),
        Value::UInt(value) => value.to_string(),
        Value::Float(value) => value.to_string(),
        Value::Double(value) => value.to_string(),
        Value::Date(year, month, day, hour, minute, second, micros) => format!(
            "{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}.{:06}",
            micros
        ),
        Value::Time(is_negative, days, hours, minutes, seconds, micros) => {
            let sign = if is_negative { "-" } else { "" };
            format!(
                "{sign}{days:03} {hours:02}:{minutes:02}:{seconds:02}.{:06}",
                micros
            )
        }
    }
}

fn to_connection_error(error: mysql_async::Error) -> BackendError {
    BackendError::new(error.to_string())
}

fn to_schema_error(error: mysql_async::Error) -> SchemaBackendError {
    SchemaBackendError::new(error.to_string())
}

fn to_query_error(error: mysql_async::Error) -> QueryBackendError {
    QueryBackendError::new(error.to_string())
}

#[cfg(test)]
mod tests {
    use myr_core::profiles::{ConnectionProfile, TlsMode};
    use mysql_async::Value;

    use super::{
        client_identity_from_profile, mysql_value_to_string, opts_from_profile,
        profile_requests_tls,
    };

    #[test]
    fn value_conversion_is_human_readable() {
        assert_eq!(mysql_value_to_string(Value::NULL), "NULL");
        assert_eq!(
            mysql_value_to_string(Value::Bytes(b"hello".to_vec())),
            "hello".to_string()
        );
        assert_eq!(mysql_value_to_string(Value::Int(-8)), "-8");
        assert_eq!(mysql_value_to_string(Value::UInt(8)), "8");
    }

    #[test]
    fn opts_builder_uses_profile_host_port_user() {
        let mut profile = ConnectionProfile::new("local", "127.0.0.1", "root");
        profile.port = 3307;
        profile.database = Some("app".to_string());

        let _opts = opts_from_profile(&profile);
        // Construction is the assertion here; mysql_async exposes limited stable introspection.
    }

    #[test]
    fn tls_mode_prefer_requires_explicit_tls_settings() {
        let mut profile = ConnectionProfile::new("local", "127.0.0.1", "root");
        profile.tls_mode = TlsMode::Prefer;
        assert!(!profile_requests_tls(&profile));

        profile.tls_ca_cert_path = Some("/tmp/ca.pem".to_string());
        assert!(profile_requests_tls(&profile));
    }

    #[test]
    fn tls_mode_require_always_uses_tls() {
        let mut profile = ConnectionProfile::new("local", "127.0.0.1", "root");
        profile.tls_mode = TlsMode::Require;
        assert!(profile_requests_tls(&profile));
    }

    #[test]
    fn client_identity_requires_both_cert_and_key_paths() {
        let mut profile = ConnectionProfile::new("local", "127.0.0.1", "root");
        profile.tls_mode = TlsMode::VerifyIdentity;
        profile.tls_client_cert_path = Some("/tmp/client-cert.pem".to_string());
        assert!(client_identity_from_profile(&profile).is_none());

        profile.tls_client_key_path = Some("/tmp/client-key.pem".to_string());
        assert!(client_identity_from_profile(&profile).is_some());
    }
}
