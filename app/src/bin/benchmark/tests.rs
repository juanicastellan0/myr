use myr_adapters::mysql::MysqlDataBackend;
use myr_core::profiles::ConnectionProfile;

use crate::io_other;
use crate::model::{BenchMetricsSnapshot, BenchmarkConfig, ParseOutcome};
use crate::parser::{next_value, parse_args_from};
use crate::report::{
    enforce_assertions, enforce_trend_guard, load_trend_guard_policy, trend_guard_thresholds,
    write_metrics_file, TrendGuardPolicy,
};
use crate::runner::{
    build_insert_batch_sql, ensure_seed_data, execute_sql, query_scalar_u64, run_query_benchmark,
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
            "--trend-policy".to_string(),
            "bench/perf-trend-policy.json".to_string(),
            "--metrics-output".to_string(),
            "target/perf/bench.json".to_string(),
            "--metrics-label".to_string(),
            "ci-main".to_string(),
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
    assert_eq!(
        config.trend_policy.as_deref(),
        Some("bench/perf-trend-policy.json")
    );
    assert_eq!(
        config.metrics_output.as_deref(),
        Some("target/perf/bench.json")
    );
    assert_eq!(config.metrics_label.as_deref(), Some("ci-main"));
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
    assert!(sql.starts_with("INSERT INTO events (user_id, category, payload, created_at) VALUES "));
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

fn snapshot_with(rows_per_sec: f64, connect_ms: f64, first_row_ms: f64) -> BenchMetricsSnapshot {
    BenchMetricsSnapshot {
        connect_ms,
        first_row_ms,
        elapsed_ms: 30.0,
        rows_streamed: 42,
        rows_per_sec,
        peak_memory_bytes: Some(123_456),
    }
}

#[test]
fn trend_guard_policy_loads_and_computes_thresholds() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let policy_path = temp_dir.path().join("trend-policy.json");
    std::fs::write(
        &policy_path,
        r#"{
  "version": 1,
  "label": "unit-test-policy",
  "baseline": {
    "connect_ms": 100,
    "first_row_ms": 200,
    "rows_per_sec": 1000
  },
  "tolerance": {
    "connect_ms_regression_pct": 25,
    "first_row_ms_regression_pct": 50,
    "rows_per_sec_regression_pct": 40
  }
}"#,
    )
    .expect("write policy");

    let policy = load_trend_guard_policy(policy_path.to_string_lossy().as_ref()).expect("load");
    assert_eq!(policy.label, "unit-test-policy");
    assert_eq!(policy.baseline_connect_ms, 100.0);
    assert_eq!(policy.baseline_first_row_ms, 200.0);
    assert_eq!(policy.baseline_rows_per_sec, 1000.0);

    let thresholds = trend_guard_thresholds(&policy);
    assert_eq!(thresholds.max_connect_ms, 125.0);
    assert_eq!(thresholds.max_first_row_ms, 300.0);
    assert_eq!(thresholds.min_rows_per_sec, 600.0);
}

#[test]
fn enforce_trend_guard_rejects_regressions() {
    let policy = TrendGuardPolicy {
        label: "ci-smoke".to_string(),
        baseline_connect_ms: 100.0,
        baseline_first_row_ms: 200.0,
        baseline_rows_per_sec: 1_000.0,
        max_connect_regression_pct: 20.0,
        max_first_row_regression_pct: 30.0,
        max_rows_per_sec_regression_pct: 25.0,
    };

    enforce_trend_guard(&policy, &snapshot_with(900.0, 110.0, 230.0))
        .expect("within tolerance should pass");

    let connect_err = enforce_trend_guard(&policy, &snapshot_with(900.0, 130.0, 220.0))
        .expect_err("connect regression");
    assert!(connect_err.to_string().contains("connect_ms"));

    let first_row_err = enforce_trend_guard(&policy, &snapshot_with(900.0, 110.0, 270.0))
        .expect_err("first row regression");
    assert!(first_row_err.to_string().contains("first_row_ms"));

    let throughput_err = enforce_trend_guard(&policy, &snapshot_with(700.0, 100.0, 200.0))
        .expect_err("throughput regression");
    assert!(throughput_err.to_string().contains("rows_per_sec"));
}

#[test]
fn trend_guard_policy_rejects_invalid_ranges() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let policy_path = temp_dir.path().join("trend-policy.json");
    std::fs::write(
        &policy_path,
        r#"{
  "version": 1,
  "label": "bad-policy",
  "baseline": {
    "connect_ms": 100,
    "first_row_ms": 200,
    "rows_per_sec": 1000
  },
  "tolerance": {
    "connect_ms_regression_pct": 25,
    "first_row_ms_regression_pct": 50,
    "rows_per_sec_regression_pct": 101
  }
}"#,
    )
    .expect("write policy");

    let err = load_trend_guard_policy(policy_path.to_string_lossy().as_ref())
        .expect_err("invalid policy");
    assert!(err
        .to_string()
        .contains("/tolerance/rows_per_sec_regression_pct"));
}

#[test]
fn io_other_uses_display_text() {
    let err = io_other("boom");
    assert_eq!(err.to_string(), "boom");
}

#[test]
fn metrics_writer_emits_json_payload() {
    let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
    let output_path = temp_dir.path().join("bench.json");
    let config = BenchmarkConfig {
        metrics_label: Some("ci-smoke".to_string()),
        ..BenchmarkConfig::default()
    };
    let snapshot = BenchMetricsSnapshot {
        connect_ms: 10.0,
        first_row_ms: 20.0,
        elapsed_ms: 30.0,
        rows_streamed: 42,
        rows_per_sec: 2_000.0,
        peak_memory_bytes: Some(123_456),
    };

    write_metrics_file(output_path.to_string_lossy().as_ref(), &config, snapshot)
        .expect("write metrics");

    let raw = std::fs::read_to_string(output_path).expect("read metrics file");
    let parsed: serde_json::Value = serde_json::from_str(&raw).expect("parse metrics json");
    assert_eq!(parsed["label"], "ci-smoke");
    assert_eq!(parsed["metrics"]["rows_streamed"], 42);
    assert_eq!(parsed["metrics"]["rows_per_sec"], 2_000.0);
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
