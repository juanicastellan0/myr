use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;

use crate::results_buffer::ResultsRingBuffer;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryRow {
    pub values: Vec<String>,
}

impl QueryRow {
    #[must_use]
    pub fn new(values: Vec<String>) -> Self {
        Self { values }
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
            buffer.get(0).map(|row| &row.values[0]),
            Some(&"2".to_string())
        );
        assert_eq!(
            buffer.get(1).map(|row| &row.values[0]),
            Some(&"3".to_string())
        );
        assert!(!cancel_called.load(Ordering::SeqCst));
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
