use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::results_buffer::ResultsRingBuffer;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum QueryValue {
    Null,
    Int(i64),
    UInt(u64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
    DateTime(String),
    Time(String),
}

impl QueryValue {
    #[must_use]
    pub fn display_text(&self) -> String {
        match self {
            Self::Null => "NULL".to_string(),
            Self::Int(value) => value.to_string(),
            Self::UInt(value) => value.to_string(),
            Self::Float(value) => value.to_string(),
            Self::Text(value) | Self::DateTime(value) | Self::Time(value) => value.clone(),
            Self::Bytes(bytes) => {
                let mut rendered = String::with_capacity(bytes.len().saturating_mul(2) + 2);
                rendered.push_str("0x");
                for byte in bytes {
                    use std::fmt::Write as _;
                    let _ = write!(rendered, "{byte:02x}");
                }
                rendered
            }
        }
    }

    #[must_use]
    pub fn typed_json_value(&self) -> serde_json::Value {
        match self {
            Self::Null => serde_json::Value::Null,
            Self::Int(value) => (*value).into(),
            Self::UInt(value) => (*value).into(),
            Self::Float(value) => serde_json::Number::from_f64(*value)
                .map_or(serde_json::Value::Null, serde_json::Value::Number),
            Self::Text(value) | Self::DateTime(value) | Self::Time(value) => value.clone().into(),
            Self::Bytes(value) => serde_json::json!({
                "encoding": "hex",
                "data": self.display_text().trim_start_matches("0x"),
                "bytes": value.len(),
            }),
        }
    }
}

impl std::fmt::Display for QueryValue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.display_text())
    }
}

impl From<String> for QueryValue {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for QueryValue {
    fn from(value: &str) -> Self {
        Self::Text(value.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnMeta {
    pub name: String,
    pub schema: Option<String>,
    pub table: Option<String>,
    pub original_table: Option<String>,
    pub original_name: Option<String>,
    pub mysql_type: String,
    pub flags: u16,
    pub character_set: u16,
    pub decimals: u8,
}

impl ColumnMeta {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            schema: None,
            table: None,
            original_table: None,
            original_name: None,
            mysql_type: String::new(),
            flags: 0,
            character_set: 0,
            decimals: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryRow {
    pub values: Vec<QueryValue>,
}

impl QueryRow {
    #[must_use]
    pub fn new(values: Vec<String>) -> Self {
        Self {
            values: values.into_iter().map(QueryValue::Text).collect(),
        }
    }

    #[must_use]
    pub fn from_values(values: Vec<QueryValue>) -> Self {
        Self { values }
    }

    #[must_use]
    pub fn display_values(&self) -> Vec<String> {
        self.values.iter().map(QueryValue::display_text).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryResultBatch {
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<QueryRow>,
    pub rows_seen: u64,
    pub rows_buffered: usize,
    pub truncated: bool,
}

impl QueryResultBatch {
    #[must_use]
    pub fn new(columns: Vec<ColumnMeta>, rows: Vec<QueryRow>, rows_seen: u64) -> Self {
        let rows_buffered = rows.len();
        Self {
            columns,
            rows,
            rows_seen,
            rows_buffered,
            truncated: rows_seen > u64::try_from(rows_buffered).unwrap_or(u64::MAX),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct QueryBackendError {
    message: String,
}

impl QueryBackendError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug, Error)]
pub enum QueryRunnerError {
    #[error("query backend failed: {0}")]
    Backend(#[source] QueryBackendError),
}

#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryExecutionSummary {
    pub rows_streamed: u64,
    pub was_cancelled: bool,
    pub elapsed: Duration,
}

#[async_trait]
pub trait QueryRowStream: Send {
    fn columns(&self) -> Option<&[ColumnMeta]> {
        None
    }

    async fn next_row(&mut self) -> Result<Option<QueryRow>, QueryBackendError>;

    async fn cancel(&mut self) -> Result<(), QueryBackendError> {
        Ok(())
    }
}

#[async_trait]
pub trait QueryBackend {
    type Stream: QueryRowStream + Send;

    async fn start_query(&self, sql: &str) -> Result<Self::Stream, QueryBackendError>;
}

#[derive(Debug)]
pub struct QueryRunner<B: QueryBackend> {
    backend: B,
}

impl<B: QueryBackend> QueryRunner<B> {
    #[must_use]
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    pub async fn execute_streaming(
        &self,
        sql: &str,
        buffer: &mut ResultsRingBuffer<QueryRow>,
        cancellation: &CancellationToken,
    ) -> Result<QueryExecutionSummary, QueryRunnerError> {
        let started_at = std::time::Instant::now();
        let mut stream = self
            .backend
            .start_query(sql)
            .await
            .map_err(QueryRunnerError::Backend)?;

        let mut rows_streamed = 0_u64;
        let mut was_cancelled = false;

        while !cancellation.is_cancelled() {
            let maybe_row = stream.next_row().await.map_err(QueryRunnerError::Backend)?;
            let Some(row) = maybe_row else {
                return Ok(QueryExecutionSummary {
                    rows_streamed,
                    was_cancelled,
                    elapsed: started_at.elapsed(),
                });
            };

            buffer.push(row);
            rows_streamed += 1;
        }

        stream.cancel().await.map_err(QueryRunnerError::Backend)?;
        was_cancelled = true;

        Ok(QueryExecutionSummary {
            rows_streamed,
            was_cancelled,
            elapsed: started_at.elapsed(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    };

    use super::{
        CancellationToken, QueryBackend, QueryBackendError, QueryRow, QueryRowStream, QueryRunner,
    };
    use crate::results_buffer::ResultsRingBuffer;

    #[derive(Debug, Clone)]
    struct FakeQueryBackend {
        rows: Vec<QueryRow>,
        cancel_called: Arc<AtomicBool>,
    }

    #[derive(Debug)]
    struct FakeStream {
        rows: VecDeque<QueryRow>,
        cancel_called: Arc<AtomicBool>,
        _state: Mutex<usize>,
    }

    #[async_trait::async_trait]
    impl QueryRowStream for FakeStream {
        async fn next_row(&mut self) -> Result<Option<QueryRow>, QueryBackendError> {
            Ok(self.rows.pop_front())
        }

        async fn cancel(&mut self) -> Result<(), QueryBackendError> {
            self.cancel_called.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl QueryBackend for FakeQueryBackend {
        type Stream = FakeStream;

        async fn start_query(&self, _sql: &str) -> Result<Self::Stream, QueryBackendError> {
            Ok(FakeStream {
                rows: self.rows.iter().cloned().collect(),
                cancel_called: Arc::clone(&self.cancel_called),
                _state: Mutex::new(0),
            })
        }
    }

    #[tokio::test]
    async fn streams_rows_into_buffer_with_bounded_memory() {
        let cancel_called = Arc::new(AtomicBool::new(false));
        let backend = FakeQueryBackend {
            rows: vec![
                QueryRow::new(vec!["1".to_string()]),
                QueryRow::new(vec!["2".to_string()]),
                QueryRow::new(vec!["3".to_string()]),
            ],
            cancel_called: Arc::clone(&cancel_called),
        };
        let runner = QueryRunner::new(backend);
        let cancellation = CancellationToken::new();
        let mut buffer = ResultsRingBuffer::new(2);

        let summary = runner
            .execute_streaming("select * from users", &mut buffer, &cancellation)
            .await
            .expect("query should succeed");

        assert_eq!(summary.rows_streamed, 3);
        assert!(!summary.was_cancelled);
        assert_eq!(buffer.len(), 2);
        assert_eq!(
            buffer.get(0).map(|row| row.values[0].display_text()),
            Some("2".to_string())
        );
        assert_eq!(
            buffer.get(1).map(|row| row.values[0].display_text()),
            Some("3".to_string())
        );
        assert!(!cancel_called.load(Ordering::SeqCst));
    }

    #[test]
    fn typed_values_preserve_display_compatibility_and_json_types() {
        use super::QueryValue;

        assert_eq!(QueryValue::Null.display_text(), "NULL");
        assert_eq!(QueryValue::Int(-3).display_text(), "-3");
        assert_eq!(QueryValue::Bytes(vec![0, 255]).display_text(), "0x00ff");
        assert_eq!(QueryValue::UInt(7).typed_json_value(), serde_json::json!(7));
        assert_eq!(QueryValue::Null.typed_json_value(), serde_json::Value::Null);
    }

    #[tokio::test]
    async fn cancellation_short_circuits_stream_and_invokes_backend_cancel() {
        let cancel_called = Arc::new(AtomicBool::new(false));
        let backend = FakeQueryBackend {
            rows: vec![
                QueryRow::new(vec!["1".to_string()]),
                QueryRow::new(vec!["2".to_string()]),
            ],
            cancel_called: Arc::clone(&cancel_called),
        };
        let runner = QueryRunner::new(backend);
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let mut buffer = ResultsRingBuffer::new(2);

        let summary = runner
            .execute_streaming("select * from users", &mut buffer, &cancellation)
            .await
            .expect("query should cancel cleanly");

        assert_eq!(summary.rows_streamed, 0);
        assert!(summary.was_cancelled);
        assert!(cancel_called.load(Ordering::SeqCst));
        assert!(buffer.is_empty());
    }
}
