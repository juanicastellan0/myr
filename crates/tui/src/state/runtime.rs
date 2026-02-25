use super::super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectIntent {
    Manual,
    AutoReconnect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ErrorKind {
    Connection,
    Query,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ErrorPanel {
    pub(crate) kind: ErrorKind,
    pub(crate) title: String,
    pub(crate) summary: String,
    pub(crate) detail: String,
}

#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum ConnectWorkerOutcome {
    Success {
        profile: ConnectionProfile,
        connect_latency: Duration,
        databases: Vec<String>,
        warning: Option<String>,
    },
    Failure(String),
}

#[derive(Debug)]
pub(crate) enum QueryWorkerOutcome {
    Success {
        results: ResultsRingBuffer<QueryRow>,
        rows_streamed: u64,
        was_cancelled: bool,
        elapsed: Duration,
    },
    Failure(String),
}
