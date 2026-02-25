use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParseOutcome {
    Config,
    HelpRequested,
}

#[derive(Debug, Clone)]
pub(crate) struct BenchmarkConfig {
    pub(crate) profile_name: String,
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) user: String,
    pub(crate) database: String,
    pub(crate) sql: String,
    pub(crate) seed_rows: u64,
    pub(crate) assert_first_row_ms: Option<f64>,
    pub(crate) assert_min_rows_per_sec: Option<f64>,
    pub(crate) metrics_output: Option<String>,
    pub(crate) metrics_label: Option<String>,
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
            metrics_output: None,
            metrics_label: None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct QueryMetrics {
    pub(crate) rows_streamed: u64,
    pub(crate) first_row: Option<Duration>,
    pub(crate) elapsed: Duration,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct BenchMetricsSnapshot {
    pub(crate) connect_ms: f64,
    pub(crate) first_row_ms: f64,
    pub(crate) elapsed_ms: f64,
    pub(crate) rows_streamed: u64,
    pub(crate) rows_per_sec: f64,
    pub(crate) peak_memory_bytes: Option<u64>,
}
