use std::io;

use myr_adapters::mysql::{MysqlConnectionBackend, MysqlDataBackend};
use myr_core::connection_manager::ConnectionManager;
use myr_core::profiles::ConnectionProfile;

#[path = "benchmark/model.rs"]
mod model;
#[path = "benchmark/parser.rs"]
mod parser;
#[path = "benchmark/report.rs"]
mod report;
#[path = "benchmark/runner.rs"]
mod runner;
#[cfg(test)]
#[path = "benchmark/tests.rs"]
mod tests;

use model::BenchMetricsSnapshot;
use parser::parse_args;
use report::{
    enforce_assertions, enforce_trend_guard, load_trend_guard_policy,
    peak_memory_bytes_best_effort, trend_guard_thresholds, write_metrics_file,
};
use runner::{ensure_seed_data, run_query_benchmark};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = parse_args()?;
    let trend_policy = config
        .trend_policy
        .as_deref()
        .map(load_trend_guard_policy)
        .transpose()?;

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
    let peak_memory_bytes = peak_memory_bytes_best_effort();
    if let Some(bytes) = peak_memory_bytes {
        println!("metric.peak_memory_bytes={bytes}");
    } else {
        println!("metric.peak_memory_bytes=n/a");
    }

    let snapshot = BenchMetricsSnapshot {
        connect_ms: connect_latency.as_secs_f64() * 1_000.0,
        first_row_ms,
        elapsed_ms,
        rows_streamed: metrics.rows_streamed,
        rows_per_sec,
        peak_memory_bytes,
    };

    if let Some(path) = config.metrics_output.as_deref() {
        write_metrics_file(path, &config, snapshot)?;
        println!("metric.output_file={path}");
    }

    if let Some(policy) = trend_policy.as_ref() {
        let thresholds = trend_guard_thresholds(policy);
        println!("metric.trend_policy={}", policy.label);
        println!(
            "metric.trend_connect_ms_max={:.3}",
            thresholds.max_connect_ms
        );
        println!(
            "metric.trend_first_row_ms_max={:.3}",
            thresholds.max_first_row_ms
        );
        println!(
            "metric.trend_rows_per_sec_min={:.3}",
            thresholds.min_rows_per_sec
        );
        enforce_trend_guard(policy, &snapshot)?;
    }

    enforce_assertions(&config, first_row_ms, rows_per_sec)?;

    manager.disconnect().await.map_err(io_other)?;
    data_backend.disconnect().await.map_err(io_other)?;

    Ok(())
}

fn io_other(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}
