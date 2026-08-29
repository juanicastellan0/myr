use std::path::PathBuf;

use async_trait::async_trait;
use futures_util::StreamExt;
use myr_application::{
    AppErrorKind, ApplicationBackendError, ApplicationBackendFactory, ApplicationSession,
    CancellationToken,
};
use myr_core::connection_manager::{BackendError, ConnectionBackend};
use myr_core::profiles::{ConnectionProfile, PasswordSource, TlsMode};
use myr_core::query_runner::{
    ColumnMeta, QueryBackend, QueryBackendError, QueryRow, QueryRowStream, QueryValue,
};
use myr_core::schema_cache::{
    ColumnSchema, RelationshipDirection, SchemaBackend, SchemaBackendError, TableRelationship,
};
use mysql_async::prelude::{Query, Queryable};
use mysql_async::{
    ClientIdentity, Conn, OptsBuilder, Pool, ResultSetStream, Row, SslOpts, TextProtocol, Value,
};

#[derive(Debug, Clone, Default)]
pub struct MysqlConnectionBackend;

#[derive(Debug, Clone, Default)]
pub struct MysqlApplicationBackendFactory;

#[async_trait]
impl ApplicationBackendFactory for MysqlApplicationBackendFactory {
    async fn connect(
        &self,
        profile: &ConnectionProfile,
    ) -> Result<std::sync::Arc<dyn ApplicationSession>, ApplicationBackendError> {
        let connection_backend = MysqlConnectionBackend;
        let mut connection = connection_backend
            .connect(profile)
            .await
            .map_err(|error| classify_application_error(&error.to_string()))?;
        connection_backend
            .ping(&mut connection)
            .await
            .map_err(|error| classify_application_error(&error.to_string()))?;
        connection_backend
            .disconnect(connection)
            .await
            .map_err(|error| classify_application_error(&error.to_string()))?;
        Ok(std::sync::Arc::new(MysqlDataBackend::from_profile(profile)))
    }
}

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

    async fn start_cancellable_query(
        &self,
        sql: &str,
        cancellation: CancellationToken,
    ) -> Result<MysqlStreamingRowStream, ApplicationBackendError> {
        let connection = tokio::select! {
            biased;
            () = cancellation.cancelled() => {
                return Err(ApplicationBackendError::new(
                    AppErrorKind::Cancellation,
                    "query cancelled",
                ));
            }
            result = self.pool.get_conn() => {
                result.map_err(|error| classify_application_error(&error.to_string()))?
            }
        };
        let connection_id = connection.id();
        let query = sql.to_string().stream::<Row, _>(connection);
        let stream = tokio::select! {
            biased;
            () = cancellation.cancelled() => {
                kill_query(&self.pool, connection_id).await?;
                return Err(ApplicationBackendError::new(
                    AppErrorKind::Cancellation,
                    "query cancelled",
                ));
            }
            result = query => {
                result.map_err(|error| classify_application_error(&error.to_string()))?
            }
        };
        Ok(MysqlStreamingRowStream::new(
            stream,
            self.pool.clone(),
            connection_id,
        ))
    }
}

#[async_trait]
impl ApplicationSession for MysqlDataBackend {
    async fn list_databases(&self) -> Result<Vec<String>, ApplicationBackendError> {
        SchemaBackend::list_databases(self)
            .await
            .map_err(|error| classify_application_error(&error.to_string()))
    }

    async fn list_tables(&self, database: &str) -> Result<Vec<String>, ApplicationBackendError> {
        SchemaBackend::list_tables(self, database)
            .await
            .map_err(|error| classify_application_error(&error.to_string()))
    }

    async fn list_columns(
        &self,
        database: &str,
        table: &str,
    ) -> Result<Vec<ColumnSchema>, ApplicationBackendError> {
        SchemaBackend::list_columns(self, database, table)
            .await
            .map_err(|error| classify_application_error(&error.to_string()))
    }

    async fn list_relationships(
        &self,
        database: &str,
        table: &str,
    ) -> Result<Vec<TableRelationship>, ApplicationBackendError> {
        SchemaBackend::list_relationships(self, database, table)
            .await
            .map_err(|error| classify_application_error(&error.to_string()))
    }

    async fn start_query(
        &self,
        sql: &str,
        cancellation: CancellationToken,
    ) -> Result<Box<dyn QueryRowStream + Send>, ApplicationBackendError> {
        self.start_cancellable_query(sql, cancellation)
            .await
            .map(|stream| Box::new(stream) as Box<dyn QueryRowStream + Send>)
    }

    async fn disconnect(&self) -> Result<(), ApplicationBackendError> {
        MysqlDataBackend::disconnect(self)
            .await
            .map_err(|error| classify_application_error(&error.to_string()))
    }
}

#[async_trait]
impl SchemaBackend for MysqlDataBackend {
    async fn list_databases(&self) -> Result<Vec<String>, SchemaBackendError> {
        let mut conn = self.pool.get_conn().await.map_err(to_schema_error)?;
        let databases = conn
            .query_map("SHOW DATABASES", |database: String| database)
            .await
            .map_err(to_schema_error)?;
        conn.disconnect().await.map_err(to_schema_error)?;
        Ok(databases)
    }

    async fn list_tables(&self, database: &str) -> Result<Vec<String>, SchemaBackendError> {
        let mut conn = self.pool.get_conn().await.map_err(to_schema_error)?;
        let tables = conn
            .exec_map(
                "SELECT TABLE_NAME \
                 FROM information_schema.TABLES \
                 WHERE TABLE_SCHEMA = ? \
                 ORDER BY TABLE_NAME",
                (database,),
                |table_name: String| table_name,
            )
            .await
            .map_err(to_schema_error)?;
        conn.disconnect().await.map_err(to_schema_error)?;
        Ok(tables)
    }

    async fn list_columns(
        &self,
        database: &str,
        table: &str,
    ) -> Result<Vec<ColumnSchema>, SchemaBackendError> {
        let mut conn = self.pool.get_conn().await.map_err(to_schema_error)?;
        let columns = conn
            .exec_map(
                "SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_DEFAULT \
                 FROM information_schema.COLUMNS \
                 WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? \
                 ORDER BY ORDINAL_POSITION",
                (database, table),
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
        conn.disconnect().await.map_err(to_schema_error)?;
        Ok(columns)
    }

    async fn list_relationships(
        &self,
        database: &str,
        table: &str,
    ) -> Result<Vec<TableRelationship>, SchemaBackendError> {
        let mut conn = self.pool.get_conn().await.map_err(to_schema_error)?;
        let relationships = conn
            .exec_map(
                "SELECT 'outbound', CONSTRAINT_NAME, COLUMN_NAME, \
                        REFERENCED_TABLE_SCHEMA, REFERENCED_TABLE_NAME, REFERENCED_COLUMN_NAME \
                 FROM information_schema.KEY_COLUMN_USAGE \
                 WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? \
                   AND REFERENCED_TABLE_NAME IS NOT NULL \
                 UNION ALL \
                 SELECT 'inbound', CONSTRAINT_NAME, REFERENCED_COLUMN_NAME, \
                        TABLE_SCHEMA, TABLE_NAME, COLUMN_NAME \
                 FROM information_schema.KEY_COLUMN_USAGE \
                 WHERE REFERENCED_TABLE_SCHEMA = ? AND REFERENCED_TABLE_NAME = ? \
                 ORDER BY 4, 5, 6, 2",
                (database, table, database, table),
                |(
                    direction,
                    constraint_name,
                    source_column,
                    related_database,
                    related_table,
                    related_column,
                ): (String, String, String, String, String, String)| {
                    TableRelationship {
                        direction: if direction == "outbound" {
                            RelationshipDirection::Outbound
                        } else {
                            RelationshipDirection::Inbound
                        },
                        constraint_name,
                        source_column,
                        related_database,
                        related_table,
                        related_column,
                    }
                },
            )
            .await
            .map_err(to_schema_error)?;
        conn.disconnect().await.map_err(to_schema_error)?;
        Ok(relationships)
    }
}

#[derive(Debug)]
pub struct MysqlStreamingRowStream {
    stream: Option<ResultSetStream<'static, 'static, 'static, Row, TextProtocol>>,
    pool: Pool,
    connection_id: u32,
    cancelled: bool,
    columns: Vec<ColumnMeta>,
}

impl MysqlStreamingRowStream {
    fn new(
        stream: ResultSetStream<'static, 'static, 'static, Row, TextProtocol>,
        pool: Pool,
        connection_id: u32,
    ) -> Self {
        let columns = stream
            .columns_ref()
            .iter()
            .map(|column| ColumnMeta {
                name: column.name_str().into_owned(),
                schema: non_empty_owned(column.schema_str().into_owned()),
                table: non_empty_owned(column.table_str().into_owned()),
                original_table: non_empty_owned(column.org_table_str().into_owned()),
                original_name: non_empty_owned(column.org_name_str().into_owned()),
                mysql_type: format!("{:?}", column.column_type()),
                flags: column.flags().bits(),
                character_set: column.character_set(),
                decimals: column.decimals(),
            })
            .collect();
        Self {
            stream: Some(stream),
            pool,
            connection_id,
            cancelled: false,
            columns,
        }
    }
}

#[async_trait]
impl QueryRowStream for MysqlStreamingRowStream {
    fn columns(&self) -> Option<&[ColumnMeta]> {
        Some(&self.columns)
    }

    async fn next_row(&mut self) -> Result<Option<QueryRow>, QueryBackendError> {
        if self.cancelled {
            return Ok(None);
        }
        let Some(stream) = self.stream.as_mut() else {
            return Ok(None);
        };

        match stream.next().await {
            Some(Ok(row)) => row_to_query_row(row, &self.columns).map(Some),
            Some(Err(error)) => Err(to_query_error(error)),
            None => {
                self.stream = None;
                Ok(None)
            }
        }
    }

    async fn cancel(&mut self) -> Result<(), QueryBackendError> {
        self.cancelled = true;
        kill_query(&self.pool, self.connection_id)
            .await
            .map_err(|error| QueryBackendError::new(error.message))?;
        self.stream = None;
        Ok(())
    }
}

#[async_trait]
impl QueryBackend for MysqlDataBackend {
    type Stream = MysqlStreamingRowStream;

    async fn start_query(&self, sql: &str) -> Result<Self::Stream, QueryBackendError> {
        let connection = self.pool.get_conn().await.map_err(to_query_error)?;
        let connection_id = connection.id();
        let stream = sql
            .to_string()
            .stream::<Row, _>(connection)
            .await
            .map_err(to_query_error)?;
        Ok(MysqlStreamingRowStream::new(
            stream,
            self.pool.clone(),
            connection_id,
        ))
    }
}

async fn kill_query(pool: &Pool, connection_id: u32) -> Result<(), ApplicationBackendError> {
    let mut connection = pool
        .get_conn()
        .await
        .map_err(|error| classify_application_error(&error.to_string()))?;
    connection
        .query_drop(format!("KILL QUERY {connection_id}"))
        .await
        .map_err(|error| classify_application_error(&error.to_string()))?;
    connection
        .disconnect()
        .await
        .map_err(|error| classify_application_error(&error.to_string()))
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

fn non_empty_owned(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
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

fn row_to_query_row(row: Row, columns: &[ColumnMeta]) -> Result<QueryRow, QueryBackendError> {
    row_values_to_typed(row.unwrap_raw(), columns).map(QueryRow::from_values)
}

fn row_values_to_typed(
    values: Vec<Option<Value>>,
    columns: &[ColumnMeta],
) -> Result<Vec<QueryValue>, QueryBackendError> {
    values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            value
                .map(|value| mysql_value_to_query_value(value, columns.get(index)))
                .ok_or_else(|| {
                    QueryBackendError::new(format!(
                        "row decoding failed: missing value at column index {index}"
                    ))
                })
        })
        .collect()
}

fn mysql_value_to_query_value(value: Value, column: Option<&ColumnMeta>) -> QueryValue {
    match value {
        Value::NULL => QueryValue::Null,
        Value::Bytes(bytes) => bytes_to_query_value(bytes, column),
        Value::Int(value) => QueryValue::Int(value),
        Value::UInt(value) => QueryValue::UInt(value),
        Value::Float(value) => QueryValue::Float(f64::from(value)),
        Value::Double(value) => QueryValue::Float(value),
        Value::Date(year, month, day, hour, minute, second, micros) => {
            QueryValue::DateTime(format!(
                "{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}.{:06}",
                micros
            ))
        }
        Value::Time(is_negative, days, hours, minutes, seconds, micros) => {
            let sign = if is_negative { "-" } else { "" };
            QueryValue::Time(format!(
                "{sign}{days:03} {hours:02}:{minutes:02}:{seconds:02}.{:06}",
                micros
            ))
        }
    }
}

fn bytes_to_query_value(bytes: Vec<u8>, column: Option<&ColumnMeta>) -> QueryValue {
    let Some(column) = column else {
        return QueryValue::Text(String::from_utf8_lossy(&bytes).into_owned());
    };
    let rendered = std::str::from_utf8(&bytes).ok();
    if matches!(
        column.mysql_type.as_str(),
        "MYSQL_TYPE_TINY"
            | "MYSQL_TYPE_SHORT"
            | "MYSQL_TYPE_LONG"
            | "MYSQL_TYPE_LONGLONG"
            | "MYSQL_TYPE_INT24"
            | "MYSQL_TYPE_YEAR"
    ) {
        if column.flags & 32 != 0 {
            if let Some(value) = rendered.and_then(|value| value.parse::<u64>().ok()) {
                return QueryValue::UInt(value);
            }
        } else if let Some(value) = rendered.and_then(|value| value.parse::<i64>().ok()) {
            return QueryValue::Int(value);
        }
    }
    if matches!(
        column.mysql_type.as_str(),
        "MYSQL_TYPE_FLOAT" | "MYSQL_TYPE_DOUBLE" | "MYSQL_TYPE_DECIMAL" | "MYSQL_TYPE_NEWDECIMAL"
    ) {
        if let Some(value) = rendered.and_then(|value| value.parse::<f64>().ok()) {
            return QueryValue::Float(value);
        }
    }
    if matches!(
        column.mysql_type.as_str(),
        "MYSQL_TYPE_DATE"
            | "MYSQL_TYPE_NEWDATE"
            | "MYSQL_TYPE_DATETIME"
            | "MYSQL_TYPE_DATETIME2"
            | "MYSQL_TYPE_TIMESTAMP"
            | "MYSQL_TYPE_TIMESTAMP2"
    ) {
        return QueryValue::DateTime(String::from_utf8_lossy(&bytes).into_owned());
    }
    if matches!(
        column.mysql_type.as_str(),
        "MYSQL_TYPE_TIME" | "MYSQL_TYPE_TIME2"
    ) {
        return QueryValue::Time(String::from_utf8_lossy(&bytes).into_owned());
    }

    if column.character_set == 63 {
        QueryValue::Bytes(bytes)
    } else {
        QueryValue::Text(String::from_utf8_lossy(&bytes).into_owned())
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

fn classify_application_error(message: &str) -> ApplicationBackendError {
    let normalized = message.to_ascii_lowercase();
    let kind = if normalized.contains("access denied")
        || normalized.contains("authentication")
        || normalized.contains("password")
    {
        AppErrorKind::Authentication
    } else if normalized.contains("tls")
        || normalized.contains("ssl")
        || normalized.contains("certificate")
    {
        AppErrorKind::Tls
    } else if normalized.contains("timeout") || normalized.contains("timed out") {
        AppErrorKind::Timeout
    } else {
        AppErrorKind::Connection
    };
    ApplicationBackendError::new(kind, message)
}

#[cfg(test)]
mod tests {
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    use std::time::{SystemTime, UNIX_EPOCH};

    use myr_core::profiles::{ConnectionProfile, TlsMode};
    use myr_core::query_runner::{ColumnMeta, QueryValue};
    use mysql_async::Value;

    use super::{
        client_identity_from_profile, mysql_value_to_query_value, opts_from_profile,
        profile_requests_tls, row_values_to_typed,
    };

    #[test]
    fn value_conversion_is_human_readable() {
        let text_column = ColumnMeta::new("value");
        assert_eq!(
            mysql_value_to_query_value(Value::NULL, None),
            QueryValue::Null
        );
        assert_eq!(
            mysql_value_to_query_value(Value::Bytes(b"hello".to_vec()), Some(&text_column)),
            QueryValue::Text("hello".to_string())
        );
        assert_eq!(
            mysql_value_to_query_value(Value::Int(-8), None),
            QueryValue::Int(-8)
        );
        assert_eq!(
            mysql_value_to_query_value(Value::UInt(8), None),
            QueryValue::UInt(8)
        );

        let mut integer_column = ColumnMeta::new("count");
        integer_column.mysql_type = "MYSQL_TYPE_LONGLONG".to_string();
        integer_column.character_set = 63;
        assert_eq!(
            mysql_value_to_query_value(Value::Bytes(b"42".to_vec()), Some(&integer_column)),
            QueryValue::Int(42)
        );
        integer_column.flags = 32;
        assert_eq!(
            mysql_value_to_query_value(Value::Bytes(b"42".to_vec()), Some(&integer_column)),
            QueryValue::UInt(42)
        );

        let mut datetime_column = ColumnMeta::new("created_at");
        datetime_column.mysql_type = "MYSQL_TYPE_DATETIME".to_string();
        datetime_column.character_set = 63;
        assert!(matches!(
            mysql_value_to_query_value(
                Value::Bytes(b"2024-05-06 07:08:09".to_vec()),
                Some(&datetime_column)
            ),
            QueryValue::DateTime(_)
        ));
    }

    #[test]
    fn row_value_mapping_reports_missing_columns_without_panicking() {
        let error = row_values_to_typed(vec![Some(Value::Int(1)), None], &[])
            .expect_err("missing row values should return an error");
        assert_eq!(
            error.to_string(),
            "row decoding failed: missing value at column index 1"
        );
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

    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    fn keyring_smoke_enabled() -> bool {
        matches!(
            std::env::var("MYR_RUN_KEYRING_SMOKE").ok().as_deref(),
            Some("1")
        )
    }

    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    fn keyring_smoke_profile() -> ConnectionProfile {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos());
        let mut profile = ConnectionProfile::new("keyring-smoke", "127.0.0.1", "root");
        profile.keyring_service = Some("myr-ci-keyring-smoke".to_string());
        profile.keyring_account = Some(format!("myr-ci-keyring-smoke-{unique_suffix}"));
        profile
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    fn keyring_password_round_trip_when_enabled() {
        if !keyring_smoke_enabled() {
            return;
        }

        let profile = keyring_smoke_profile();
        let expected_password = format!(
            "myr-smoke-password-{}",
            profile.keyring_account.as_deref().unwrap_or("default")
        );
        let entry = super::keyring_entry(&profile).expect("keyring entry should be created");
        let _ = entry.delete_credential();

        super::store_keyring_password(&profile, &expected_password);
        let loaded = super::load_keyring_password(&profile);
        assert_eq!(loaded.as_deref(), Some(expected_password.as_str()));

        entry
            .delete_credential()
            .expect("keyring credential cleanup should succeed");
    }
}
