use std::io;
use std::path::Path;

use serde_json::json;

use crate::io_other;
use crate::model::{BenchMetricsSnapshot, BenchmarkConfig};

pub(crate) fn enforce_assertions(
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

pub(crate) fn write_metrics_file(
    path: &str,
    config: &BenchmarkConfig,
    snapshot: BenchMetricsSnapshot,
) -> io::Result<()> {
    let path_ref = Path::new(path);
    if let Some(parent) = path_ref.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let started_unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis())
        .unwrap_or(0);

    let payload = json!({
        "label": config.metrics_label.clone().unwrap_or_else(|| "benchmark".to_string()),
        "started_unix_ms": started_unix_ms,
        "profile_name": config.profile_name,
        "host": config.host,
        "port": config.port,
        "database": config.database,
        "sql": config.sql,
        "seed_rows": config.seed_rows,
        "metrics": {
            "connect_ms": snapshot.connect_ms,
            "first_row_ms": snapshot.first_row_ms,
            "stream_elapsed_ms": snapshot.elapsed_ms,
            "rows_streamed": snapshot.rows_streamed,
            "rows_per_sec": snapshot.rows_per_sec,
            "peak_memory_bytes": snapshot.peak_memory_bytes,
        }
    });

    let rendered = serde_json::to_string_pretty(&payload).map_err(io_other)?;
    std::fs::write(path_ref, rendered).map_err(io_other)
}

#[cfg(target_os = "linux")]
pub(crate) fn peak_memory_bytes_best_effort() -> Option<u64> {
    let contents = std::fs::read_to_string("/proc/self/status").ok()?;
    let vm_hwm_line = contents.lines().find(|line| line.starts_with("VmHWM:"))?;
    let kb = vm_hwm_line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    Some(kb * 1_024)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn peak_memory_bytes_best_effort() -> Option<u64> {
    None
}
