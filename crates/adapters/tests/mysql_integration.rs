use myr_adapters::mysql::{MysqlConnectionBackend, MysqlDataBackend};
use myr_core::connection_manager::ConnectionBackend;
use myr_core::profiles::ConnectionProfile;
use myr_core::query_runner::{QueryBackend, QueryRowStream};
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

    let schema = backend
        .fetch_schema()
        .await
        .expect("schema fetch should succeed");
    let db = schema
        .databases
        .iter()
        .find(|db| db.name == database)
        .expect("database should be listed");
    let table = db
        .tables
        .iter()
        .find(|table| table.name == "integration_users")
        .expect("table should be listed");
    assert!(table.columns.iter().any(|column| column.name == "id"));
    assert!(table.columns.iter().any(|column| column.name == "email"));
    assert!(table.columns.iter().any(|column| column.name == "age"));

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

    assert_eq!(row_1.values[0], "1");
    assert_eq!(row_1.values[1], "a@example.com");
    assert_eq!(row_2.values[0], "2");
    assert_eq!(row_2.values[2], "NULL");
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

    execute_sql(&backend, "DROP TABLE IF EXISTS integration_users").await;
    backend
        .disconnect()
        .await
        .expect("backend disconnect should succeed");
}
