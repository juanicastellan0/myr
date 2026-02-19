use std::io;
use std::time::{Duration, Instant};

use myr_adapters::mysql::{MysqlConnectionBackend, MysqlDataBackend};
use myr_core::connection_manager::ConnectionManager;
use myr_core::profiles::ConnectionProfile;
use myr_core::query_runner::{QueryBackend, QueryRowStream};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParseOutcome {
    Config,
    HelpRequested,
}

#[derive(Debug, Clone)]
struct BenchmarkConfig {
    profile_name: String,
    host: String,
    port: u16,
    user: String,
    database: String,
    sql: String,
    seed_rows: u64,
    assert_first_row_ms: Option<f64>,
    assert_min_rows_per_sec: Option<f64>,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            profile_name: "bench-local".to_string(),
            host: "127.0.0.1".to_string(),
            port: 3306,
            user: "root".to_string(),
            database: "myr_bench".to_string(),
            sql: "SELECT id, user_id, category, payload, created_at FROM events ORDER BY id LIMIT 20000"
                .to_string(),
            seed_rows: 0,
            assert_first_row_ms: None,
            assert_min_rows_per_sec: None,
        }
    }
}

#[derive(Debug, Clone)]
struct QueryMetrics {
    rows_streamed: u64,
    first_row: Option<Duration>,
    elapsed: Duration,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = parse_args()?;
    let mut profile = ConnectionProfile::new(
        config.profile_name.clone(),
        config.host.clone(),
        config.user.clone(),
    );
    profile.port = config.port;
    profile.database = Some(config.database.clone());

    let mut manager = ConnectionManager::new(MysqlConnectionBackend);
    let connect_latency = manager.connect(profile.clone()).await.map_err(io_other)?;

    let data_backend = MysqlDataBackend::from_profile(&profile);
    if config.seed_rows > 0 {
        ensure_seed_data(&data_backend, config.seed_rows).await?;
    }

    let metrics = run_query_benchmark(&data_backend, &config.sql).await?;
    let rows_per_sec = if metrics.elapsed.as_secs_f64() > 0.0 {
        metrics.rows_streamed as f64 / metrics.elapsed.as_secs_f64()
    } else {
        0.0
    };

    let first_row_ms = metrics
        .first_row
        .map_or(0.0, |duration| duration.as_secs_f64() * 1_000.0);
    let elapsed_ms = metrics.elapsed.as_secs_f64() * 1_000.0;

    println!(
        "metric.connect_ms={:.3}",
        connect_latency.as_secs_f64() * 1_000.0
    );
    println!("metric.first_row_ms={first_row_ms:.3}");
    println!("metric.rows_streamed={}", metrics.rows_streamed);
    println!("metric.stream_elapsed_ms={elapsed_ms:.3}");
    println!("metric.rows_per_sec={rows_per_sec:.3}");
    if let Some(bytes) = peak_memory_bytes_best_effort() {
        println!("metric.peak_memory_bytes={bytes}");
    } else {
        println!("metric.peak_memory_bytes=n/a");
    }

    enforce_assertions(&config, first_row_ms, rows_per_sec)?;

    manager.disconnect().await.map_err(io_other)?;
    data_backend.disconnect().await.map_err(io_other)?;

    Ok(())
}

async fn run_query_benchmark(backend: &MysqlDataBackend, sql: &str) -> io::Result<QueryMetrics> {
    let mut stream = backend.start_query(sql).await.map_err(io_other)?;
    let started_at = Instant::now();
    let mut first_row = None;
    let mut rows_streamed = 0_u64;

    while let Some(_row) = stream.next_row().await.map_err(io_other)? {
        rows_streamed += 1;
        if first_row.is_none() {
            first_row = Some(started_at.elapsed());
        }
    }

    Ok(QueryMetrics {
        rows_streamed,
        first_row,
        elapsed: started_at.elapsed(),
    })
}

async fn ensure_seed_data(backend: &MysqlDataBackend, target_rows: u64) -> io::Result<()> {
    execute_sql(
        backend,
        "CREATE TABLE IF NOT EXISTS events (\
         id BIGINT NOT NULL AUTO_INCREMENT PRIMARY KEY,\
         user_id INT NOT NULL,\
         category VARCHAR(32) NOT NULL,\
         payload VARCHAR(128) NOT NULL,\
         created_at DATETIME NOT NULL,\
         KEY idx_created_at (created_at),\
         KEY idx_user_id_id (user_id, id)\
         )",
    )
    .await?;

    let existing_rows = query_scalar_u64(backend, "SELECT COUNT(*) FROM events").await?;
    if existing_rows >= target_rows {
        return Ok(());
    }

    let mut next = existing_rows + 1;
    while next <= target_rows {
        let end = (next + 999).min(target_rows);
        execute_sql(backend, &build_insert_batch_sql(next, end)).await?;
        next = end + 1;
    }

    Ok(())
}

async fn execute_sql(backend: &MysqlDataBackend, sql: &str) -> io::Result<()> {
    let mut stream = backend.start_query(sql).await.map_err(io_other)?;
    while stream.next_row().await.map_err(io_other)?.is_some() {}
    Ok(())
}

async fn query_scalar_u64(backend: &MysqlDataBackend, sql: &str) -> io::Result<u64> {
    let mut stream = backend.start_query(sql).await.map_err(io_other)?;
    let row = stream
        .next_row()
        .await
        .map_err(io_other)?
        .ok_or_else(|| io_other("query returned no rows"))?;
    let value = row
        .values
        .first()
        .ok_or_else(|| io_other("query returned no columns"))?;
    value
        .parse::<u64>()
        .map_err(|error| io_other(format!("failed to parse scalar value `{value}`: {error}")))
}

fn build_insert_batch_sql(start: u64, end: u64) -> String {
    let mut values = Vec::with_capacity((end - start + 1) as usize);
    for index in start..=end {
        let user_id = (index % 5_000) + 1;
        let category = match index % 5 {
            0 => "search",
            1 => "play",
            2 => "pause",
            3 => "skip",
            _ => "share",
        };
        let payload = format!("payload-{index}");
        let created_offset = index % 86_400;
        values.push(format!(
            "({user_id}, '{category}', '{payload}', NOW() - INTERVAL {created_offset} SECOND)"
        ));
    }

    format!(
        "INSERT INTO events (user_id, category, payload, created_at) VALUES {}",
        values.join(",")
    )
}

fn enforce_assertions(
    config: &BenchmarkConfig,
    first_row_ms: f64,
    rows_per_sec: f64,
) -> io::Result<()> {
    if let Some(max_first_row_ms) = config.assert_first_row_ms {
        if first_row_ms > max_first_row_ms {
            return Err(io_other(format!(
                "first row latency {:.3}ms exceeded threshold {:.3}ms",
                first_row_ms, max_first_row_ms
            )));
        }
    }

    if let Some(min_rows_per_sec) = config.assert_min_rows_per_sec {
        if rows_per_sec < min_rows_per_sec {
            return Err(io_other(format!(
                "rows/sec {:.3} below threshold {:.3}",
                rows_per_sec, min_rows_per_sec
            )));
        }
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn peak_memory_bytes_best_effort() -> Option<u64> {
    let contents = std::fs::read_to_string("/proc/self/status").ok()?;
    let vm_hwm_line = contents.lines().find(|line| line.starts_with("VmHWM:"))?;
    let kb = vm_hwm_line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    Some(kb * 1_024)
}

#[cfg(not(target_os = "linux"))]
fn peak_memory_bytes_best_effort() -> Option<u64> {
    None
}

fn parse_args() -> io::Result<BenchmarkConfig> {
    let mut config = BenchmarkConfig::default();
    let outcome = parse_args_from(std::env::args().skip(1), &mut config)?;
    if outcome == ParseOutcome::HelpRequested {
        print_help();
        std::process::exit(0);
    }
    Ok(config)
}

fn parse_args_from(
    args: impl IntoIterator<Item = String>,
    config: &mut BenchmarkConfig,
) -> io::Result<ParseOutcome> {
    let mut args = args.into_iter();

    while let Some(flag) = args.next() {
        match flag.as_str() {
            "-h" | "--help" => return Ok(ParseOutcome::HelpRequested),
            "--profile-name" => config.profile_name = next_value(&mut args, "--profile-name")?,
            "--host" => config.host = next_value(&mut args, "--host")?,
            "--port" => {
                config.port = next_value(&mut args, "--port")?
                    .parse::<u16>()
                    .map_err(|error| io_other(format!("invalid --port value: {error}")))?;
            }
            "--user" => config.user = next_value(&mut args, "--user")?,
            "--database" => config.database = next_value(&mut args, "--database")?,
            "--sql" => config.sql = next_value(&mut args, "--sql")?,
            "--seed-rows" => {
                config.seed_rows = next_value(&mut args, "--seed-rows")?
                    .parse::<u64>()
                    .map_err(|error| io_other(format!("invalid --seed-rows value: {error}")))?;
            }
            "--assert-first-row-ms" => {
                config.assert_first_row_ms = Some(
                    next_value(&mut args, "--assert-first-row-ms")?
                        .parse::<f64>()
                        .map_err(|error| {
                            io_other(format!("invalid --assert-first-row-ms value: {error}"))
                        })?,
                );
            }
            "--assert-min-rows-per-sec" => {
                config.assert_min_rows_per_sec = Some(
                    next_value(&mut args, "--assert-min-rows-per-sec")?
                        .parse::<f64>()
                        .map_err(|error| {
                            io_other(format!("invalid --assert-min-rows-per-sec value: {error}"))
                        })?,
                );
            }
            _ => {
                return Err(io_other(format!("unknown argument `{flag}`")));
            }
        }
    }

    Ok(ParseOutcome::Config)
}

fn next_value(args: &mut impl Iterator<Item = String>, flag: &str) -> io::Result<String> {
    args.next()
        .ok_or_else(|| io_other(format!("missing value for `{flag}`")))
}

fn print_help() {
    println!(
        "myr benchmark runner\n\n\
Usage:\n  cargo run -p myr-app --bin benchmark -- [OPTIONS]\n\n\
Options:\n  --profile-name <name>           Profile name used for connection manager (default: bench-local)\n  --host <host>                   MySQL host (default: 127.0.0.1)\n  --port <port>                   MySQL port (default: 3306)\n  --user <user>                   MySQL user (default: root)\n  --database <name>               Database name (default: myr_bench)\n  --sql <query>                   Query to benchmark\n  --seed-rows <count>             Seed `events` table up to count rows before benchmark\n  --assert-first-row-ms <ms>      Fail if first-row latency exceeds threshold\n  --assert-min-rows-per-sec <rps> Fail if throughput is below threshold\n\n\
Environment:\n  MYR_DB_PASSWORD is used for authentication.\n"
    );
}

fn io_other(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

#[cfg(test)]
mod tests {
    use myr_adapters::mysql::MysqlDataBackend;
    use myr_core::profiles::ConnectionProfile;

    use super::{
        build_insert_batch_sql, enforce_assertions, ensure_seed_data, execute_sql, io_other,
        next_value, parse_args_from, query_scalar_u64, run_query_benchmark, BenchmarkConfig,
        ParseOutcome,
    };

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

        let mut profile = ConnectionProfile::new("bench-integration", host, user);
        profile.port = port;
        profile.database = database.map(str::to_string);
        profile
    }

    #[test]
    fn parse_args_from_applies_overrides() {
        let mut config = BenchmarkConfig::default();
        let outcome = parse_args_from(
            vec![
                "--profile-name".to_string(),
                "ci-bench".to_string(),
                "--host".to_string(),
                "db".to_string(),
                "--port".to_string(),
                "33306".to_string(),
                "--user".to_string(),
                "bench_user".to_string(),
                "--database".to_string(),
                "bench_db".to_string(),
                "--sql".to_string(),
                "SELECT * FROM events LIMIT 100".to_string(),
                "--seed-rows".to_string(),
                "12345".to_string(),
                "--assert-first-row-ms".to_string(),
                "1500".to_string(),
                "--assert-min-rows-per-sec".to_string(),
                "4000".to_string(),
            ],
            &mut config,
        )
        .expect("parse should succeed");

        assert_eq!(outcome, ParseOutcome::Config);
        assert_eq!(config.profile_name, "ci-bench");
        assert_eq!(config.host, "db");
        assert_eq!(config.port, 33306);
        assert_eq!(config.user, "bench_user");
        assert_eq!(config.database, "bench_db");
        assert_eq!(config.sql, "SELECT * FROM events LIMIT 100");
        assert_eq!(config.seed_rows, 12345);
        assert_eq!(config.assert_first_row_ms, Some(1500.0));
        assert_eq!(config.assert_min_rows_per_sec, Some(4000.0));
    }

    #[test]
    fn parse_args_from_detects_help() {
        let mut config = BenchmarkConfig::default();
        let outcome = parse_args_from(vec!["--help".to_string()], &mut config).expect("help parse");
        assert_eq!(outcome, ParseOutcome::HelpRequested);
    }

    #[test]
    fn parse_args_from_fails_for_unknown_flag() {
        let mut config = BenchmarkConfig::default();
        let err = parse_args_from(vec!["--bogus".to_string()], &mut config)
            .expect_err("unknown flags should fail");
        assert!(err.to_string().contains("unknown argument"));
    }

    #[test]
    fn next_value_reports_missing_flag_values() {
        let mut args = std::iter::empty::<String>();
        let err = next_value(&mut args, "--port").expect_err("missing value should fail");
        assert!(err.to_string().contains("missing value for `--port`"));
    }

    #[test]
    fn build_insert_batch_sql_emits_expected_rows() {
        let sql = build_insert_batch_sql(1, 3);
        assert!(
            sql.starts_with("INSERT INTO events (user_id, category, payload, created_at) VALUES ")
        );
        assert!(sql.contains("(2, 'play', 'payload-1', NOW() - INTERVAL 1 SECOND)"));
        assert!(sql.contains("(3, 'pause', 'payload-2', NOW() - INTERVAL 2 SECOND)"));
        assert!(sql.contains("(4, 'skip', 'payload-3', NOW() - INTERVAL 3 SECOND)"));
    }

    #[test]
    fn enforce_assertions_validates_thresholds() {
        let config = BenchmarkConfig {
            assert_first_row_ms: Some(50.0),
            assert_min_rows_per_sec: Some(10_000.0),
            ..BenchmarkConfig::default()
        };

        let first_row_err =
            enforce_assertions(&config, 51.0, 20_000.0).expect_err("first-row threshold");
        assert!(first_row_err.to_string().contains("first row latency"));

        let rows_per_sec_err =
            enforce_assertions(&config, 20.0, 9_999.0).expect_err("throughput threshold");
        assert!(rows_per_sec_err.to_string().contains("rows/sec"));
    }

    #[test]
    fn io_other_uses_display_text() {
        let err = io_other("boom");
        assert_eq!(err.to_string(), "boom");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn benchmark_helpers_work_against_mysql() {
        if !mysql_integration_enabled() {
            return;
        }

        let database = "myr_bench_cov";
        let admin_backend = MysqlDataBackend::from_profile(&integration_profile(None));
        execute_sql(
            &admin_backend,
            &format!("CREATE DATABASE IF NOT EXISTS `{database}`"),
        )
        .await
        .expect("create db");
        admin_backend.disconnect().await.expect("disconnect admin");

        let backend = MysqlDataBackend::from_profile(&integration_profile(Some(database)));
        execute_sql(&backend, "DROP TABLE IF EXISTS events")
            .await
            .expect("drop table");
        ensure_seed_data(&backend, 25).await.expect("seed rows");

        let rows = query_scalar_u64(&backend, "SELECT COUNT(*) FROM events")
            .await
            .expect("count rows");
        assert!(rows >= 25);

        let metrics = run_query_benchmark(
            &backend,
            "SELECT id, user_id, category, payload, created_at FROM events ORDER BY id LIMIT 20",
        )
        .await
        .expect("run query benchmark");
        assert!(metrics.rows_streamed > 0);
        assert!(metrics.elapsed > std::time::Duration::ZERO);

        execute_sql(&backend, "DROP TABLE IF EXISTS events")
            .await
            .expect("cleanup table");
        backend.disconnect().await.expect("disconnect");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn query_scalar_reports_parse_errors() {
        if !mysql_integration_enabled() {
            return;
        }

        let database = "myr_bench_cov";
        let admin_backend = MysqlDataBackend::from_profile(&integration_profile(None));
        execute_sql(
            &admin_backend,
            &format!("CREATE DATABASE IF NOT EXISTS `{database}`"),
        )
        .await
        .expect("create db");
        admin_backend.disconnect().await.expect("disconnect admin");

        let backend = MysqlDataBackend::from_profile(&integration_profile(Some(database)));
        let err = query_scalar_u64(&backend, "SELECT 'not-an-int'")
            .await
            .expect_err("parse should fail");
        assert!(err.to_string().contains("failed to parse scalar value"));
        backend.disconnect().await.expect("disconnect");
    }
}
