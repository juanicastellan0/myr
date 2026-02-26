use std::io;
use std::path::Path;

use serde_json::{json, Value};

use crate::io_other;
use crate::model::{BenchMetricsSnapshot, BenchmarkConfig};

#[derive(Debug, Clone)]
pub(crate) struct TrendGuardPolicy {
    pub(crate) label: String,
    pub(crate) baseline_connect_ms: f64,
    pub(crate) baseline_first_row_ms: f64,
    pub(crate) baseline_rows_per_sec: f64,
    pub(crate) max_connect_regression_pct: f64,
    pub(crate) max_first_row_regression_pct: f64,
    pub(crate) max_rows_per_sec_regression_pct: f64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TrendGuardThresholds {
    pub(crate) max_connect_ms: f64,
    pub(crate) max_first_row_ms: f64,
    pub(crate) min_rows_per_sec: f64,
}

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

pub(crate) fn load_trend_guard_policy(path: &str) -> io::Result<TrendGuardPolicy> {
    let raw = std::fs::read_to_string(path)
        .map_err(|error| io_other(format!("failed to read trend policy `{path}`: {error}")))?;
    let policy: Value = serde_json::from_str(&raw)
        .map_err(|error| io_other(format!("failed to parse trend policy `{path}`: {error}")))?;

    let version = required_u64(&policy, "/version")?;
    if version != 1 {
        return Err(io_other(format!(
            "unsupported trend policy version {version}; expected 1"
        )));
    }

    Ok(TrendGuardPolicy {
        label: required_string(&policy, "/label")?.to_string(),
        baseline_connect_ms: required_non_negative_f64(&policy, "/baseline/connect_ms")?,
        baseline_first_row_ms: required_non_negative_f64(&policy, "/baseline/first_row_ms")?,
        baseline_rows_per_sec: required_positive_f64(&policy, "/baseline/rows_per_sec")?,
        max_connect_regression_pct: required_non_negative_f64(
            &policy,
            "/tolerance/connect_ms_regression_pct",
        )?,
        max_first_row_regression_pct: required_non_negative_f64(
            &policy,
            "/tolerance/first_row_ms_regression_pct",
        )?,
        max_rows_per_sec_regression_pct: required_bounded_f64(
            &policy,
            "/tolerance/rows_per_sec_regression_pct",
            0.0,
            100.0,
        )?,
    })
}

pub(crate) fn trend_guard_thresholds(policy: &TrendGuardPolicy) -> TrendGuardThresholds {
    TrendGuardThresholds {
        max_connect_ms: policy.baseline_connect_ms
            * (1.0 + policy.max_connect_regression_pct / 100.0),
        max_first_row_ms: policy.baseline_first_row_ms
            * (1.0 + policy.max_first_row_regression_pct / 100.0),
        min_rows_per_sec: policy.baseline_rows_per_sec
            * (1.0 - policy.max_rows_per_sec_regression_pct / 100.0),
    }
}

pub(crate) fn enforce_trend_guard(
    policy: &TrendGuardPolicy,
    snapshot: &BenchMetricsSnapshot,
) -> io::Result<()> {
    let thresholds = trend_guard_thresholds(policy);

    if snapshot.connect_ms > thresholds.max_connect_ms {
        return Err(io_other(format!(
            "trend guard `{}` failed: connect_ms {:.3} exceeded {:.3} (baseline {:.3} + {:.1}% window)",
            policy.label,
            snapshot.connect_ms,
            thresholds.max_connect_ms,
            policy.baseline_connect_ms,
            policy.max_connect_regression_pct
        )));
    }

    if snapshot.first_row_ms > thresholds.max_first_row_ms {
        return Err(io_other(format!(
            "trend guard `{}` failed: first_row_ms {:.3} exceeded {:.3} (baseline {:.3} + {:.1}% window)",
            policy.label,
            snapshot.first_row_ms,
            thresholds.max_first_row_ms,
            policy.baseline_first_row_ms,
            policy.max_first_row_regression_pct
        )));
    }

    if snapshot.rows_per_sec < thresholds.min_rows_per_sec {
        return Err(io_other(format!(
            "trend guard `{}` failed: rows_per_sec {:.3} below {:.3} (baseline {:.3} - {:.1}% window)",
            policy.label,
            snapshot.rows_per_sec,
            thresholds.min_rows_per_sec,
            policy.baseline_rows_per_sec,
            policy.max_rows_per_sec_regression_pct
        )));
    }

    Ok(())
}

fn required_u64(payload: &Value, pointer: &str) -> io::Result<u64> {
    let value = required_value(payload, pointer)?;
    value.as_u64().ok_or_else(|| {
        io_other(format!(
            "trend policy `{pointer}` must be an unsigned integer"
        ))
    })
}

fn required_string<'a>(payload: &'a Value, pointer: &str) -> io::Result<&'a str> {
    let value = required_value(payload, pointer)?;
    value
        .as_str()
        .ok_or_else(|| io_other(format!("trend policy `{pointer}` must be a string")))
}

fn required_non_negative_f64(payload: &Value, pointer: &str) -> io::Result<f64> {
    let value = required_f64(payload, pointer)?;
    if value < 0.0 {
        return Err(io_other(format!(
            "trend policy `{pointer}` must be greater than or equal to 0"
        )));
    }
    Ok(value)
}

fn required_positive_f64(payload: &Value, pointer: &str) -> io::Result<f64> {
    let value = required_f64(payload, pointer)?;
    if value <= 0.0 {
        return Err(io_other(format!(
            "trend policy `{pointer}` must be greater than 0"
        )));
    }
    Ok(value)
}

fn required_bounded_f64(payload: &Value, pointer: &str, min: f64, max: f64) -> io::Result<f64> {
    let value = required_f64(payload, pointer)?;
    if !(min..=max).contains(&value) {
        return Err(io_other(format!(
            "trend policy `{pointer}` must be within [{min}, {max}]"
        )));
    }
    Ok(value)
}

fn required_f64(payload: &Value, pointer: &str) -> io::Result<f64> {
    let value = required_value(payload, pointer)?;
    let parsed = value
        .as_f64()
        .ok_or_else(|| io_other(format!("trend policy `{pointer}` must be a number")))?;
    if !parsed.is_finite() {
        return Err(io_other(format!(
            "trend policy `{pointer}` must be a finite number"
        )));
    }
    Ok(parsed)
}

fn required_value<'a>(payload: &'a Value, pointer: &str) -> io::Result<&'a Value> {
    payload
        .pointer(pointer)
        .ok_or_else(|| io_other(format!("trend policy missing required field `{pointer}`")))
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
