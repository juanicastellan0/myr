use myr_adapters::mysql::{MysqlConnectionBackend, MysqlDataBackend};
use myr_core::connection_manager::ConnectionBackend;
use myr_core::profiles::ConnectionProfile;
use myr_core::query_runner::{QueryBackend, QueryRowStream, QueryValue};
use myr_core::schema_cache::SchemaBackend;

fn mysql_integration_enabled() -> bool {
    matches!(
        std::env::var("MYR_RUN_MYSQL_INTEGRATION").ok().as_deref(),
        Some("1")
    )
}

fn integration_profile(database: Option<&str>) -> ConnectionProfile {
    let host = std::env::var("MYR_TEST_DB_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let user = std::env::var("MYR_TEST_DB_USER").unwrap_or_else(|_| "root".to_string());
    let port = std::env::var("MYR_TEST_DB_PORT")
        .ok()
        .and_then(|raw| raw.parse::<u16>().ok())
        .unwrap_or(3306);

    let mut profile = ConnectionProfile::new("adapters-integration", host, user);
    profile.port = port;
    profile.database = database.map(str::to_string);
    profile
}

async fn execute_sql(backend: &MysqlDataBackend, sql: &str) {
    let mut stream = backend.start_query(sql).await.expect("query should start");
    while stream
        .next_row()
        .await
        .expect("query stream should advance")
        .is_some()
    {}
}

#[tokio::test(flavor = "current_thread")]
async fn mysql_backend_connection_schema_and_query_paths() {
    if !mysql_integration_enabled() {
        return;
    }

    let database = "myr_adapters_cov";

    let admin_backend = MysqlDataBackend::from_profile(&integration_profile(None));
    execute_sql(
        &admin_backend,
        &format!("CREATE DATABASE IF NOT EXISTS `{database}`"),
    )
    .await;
    admin_backend
        .disconnect()
        .await
        .expect("admin disconnect should succeed");

    let profile = integration_profile(Some(database));

    let connection_backend = MysqlConnectionBackend;
    let mut connection = connection_backend
        .connect(&profile)
        .await
        .expect("connect should succeed");
    connection_backend
        .ping(&mut connection)
        .await
        .expect("ping should succeed");
    connection_backend
        .disconnect(connection)
        .await
        .expect("disconnect should succeed");

    let backend = MysqlDataBackend::from_profile(&profile);
    execute_sql(&backend, "DROP TABLE IF EXISTS integration_users").await;
    execute_sql(
        &backend,
        "CREATE TABLE integration_users (\
         id BIGINT NOT NULL PRIMARY KEY,\
         email VARCHAR(64) NOT NULL,\
         age INT NULL\
         )",
    )
    .await;
    execute_sql(
        &backend,
        "INSERT INTO integration_users (id, email, age) VALUES \
         (1, 'a@example.com', 22), (2, 'b@example.com', NULL)",
    )
    .await;

    let databases = backend
        .list_databases()
        .await
        .expect("database listing should succeed");
    assert!(databases.iter().any(|candidate| candidate == database));
    let tables = backend
        .list_tables(database)
        .await
        .expect("table listing should succeed");
    assert!(tables.iter().any(|table| table == "integration_users"));
    let columns = backend
        .list_columns(database, "integration_users")
        .await
        .expect("column listing should succeed");
    assert!(columns.iter().any(|column| column.name == "id"));
    assert!(columns.iter().any(|column| column.name == "email"));
    assert!(columns.iter().any(|column| column.name == "age"));

    let mut query_stream = backend
        .start_query("SELECT id, email, age FROM integration_users ORDER BY id")
        .await
        .expect("query stream should start");
    let row_1 = query_stream
        .next_row()
        .await
        .expect("stream should read row")
        .expect("first row expected");
    let row_2 = query_stream
        .next_row()
        .await
        .expect("stream should read row")
        .expect("second row expected");
    let end = query_stream
        .next_row()
        .await
        .expect("stream should end cleanly");

    assert_eq!(row_1.values[0], QueryValue::Int(1));
    assert_eq!(
        row_1.values[1],
        QueryValue::Text("a@example.com".to_string())
    );
    assert_eq!(row_2.values[0], QueryValue::Int(2));
    assert_eq!(row_2.values[2], QueryValue::Null);
    assert!(end.is_none());

    let mut cancellable_stream = backend
        .start_query("SELECT id FROM integration_users ORDER BY id")
        .await
        .expect("stream should start");
    cancellable_stream
        .cancel()
        .await
        .expect("cancel should succeed");
    let cancelled_end = cancellable_stream
        .next_row()
        .await
        .expect("cancelled stream should return none");
    assert!(cancelled_end.is_none());

    execute_sql(&backend, "DROP TABLE IF EXISTS integration_types").await;
    execute_sql(
        &backend,
        "CREATE TABLE integration_types (\
         signed_value BIGINT NOT NULL,\
         unsigned_value BIGINT UNSIGNED NOT NULL,\
         float_value DOUBLE NOT NULL,\
         text_value VARCHAR(32) NOT NULL,\
         bytes_value VARBINARY(8) NOT NULL,\
         datetime_value DATETIME(6) NOT NULL,\
         time_value TIME(6) NOT NULL,\
         null_value INT NULL\
         )",
    )
    .await;
    execute_sql(
        &backend,
        "INSERT INTO integration_types VALUES (\
         -9, 10, 1.25, 'hello', X'00FF41', \
         '2024-05-06 07:08:09.123456', '10:11:12.654321', NULL)",
    )
    .await;
    let mut typed_stream = backend
        .start_query("SELECT * FROM integration_types")
        .await
        .expect("typed query should start");
    let typed_columns = typed_stream
        .columns()
        .expect("typed query should expose column metadata");
    assert_eq!(typed_columns.len(), 8);
    assert_eq!(typed_columns[0].name, "signed_value");
    assert_eq!(typed_columns[0].schema.as_deref(), Some(database));
    assert_eq!(typed_columns[0].table.as_deref(), Some("integration_types"));
    assert_eq!(
        typed_columns[0].original_name.as_deref(),
        Some("signed_value")
    );
    assert!(typed_columns
        .iter()
        .all(|column| !column.mysql_type.is_empty()));
    let typed_row = typed_stream
        .next_row()
        .await
        .expect("typed row should decode")
        .expect("typed row expected");
    assert_eq!(typed_row.values[0], QueryValue::Int(-9));
    assert_eq!(typed_row.values[1], QueryValue::UInt(10));
    assert_eq!(typed_row.values[2], QueryValue::Float(1.25));
    assert_eq!(typed_row.values[3], QueryValue::Text("hello".to_string()));
    assert_eq!(typed_row.values[4], QueryValue::Bytes(vec![0, 255, 65]));
    assert_eq!(
        typed_row.values[5],
        QueryValue::DateTime("2024-05-06 07:08:09.123456".to_string())
    );
    assert_eq!(
        typed_row.values[6],
        QueryValue::Time("000 10:11:12.654321".to_string())
    );
    assert_eq!(typed_row.values[7], QueryValue::Null);
    assert!(typed_stream
        .next_row()
        .await
        .expect("typed stream should finish")
        .is_none());

    execute_sql(&backend, "DROP TABLE IF EXISTS integration_types").await;
    execute_sql(&backend, "DROP TABLE IF EXISTS integration_users").await;
    backend
        .disconnect()
        .await
        .expect("backend disconnect should succeed");
}
