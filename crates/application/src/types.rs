use std::path::PathBuf;

use myr_core::profiles::ConnectionProfile;
use myr_core::query_runner::{ColumnMeta, QueryRow};
use myr_core::schema_cache::{ColumnSchema, SchemaScope, TableRelationship};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OperationId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    Connection,
    Schema,
    Query,
    Export,
    Profile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppErrorKind {
    Authentication,
    Connection,
    Tls,
    Schema,
    Query,
    Timeout,
    Cancellation,
    Profile,
    Export,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppError {
    pub kind: AppErrorKind,
    pub message: String,
    pub operation_id: Option<OperationId>,
    pub retryable: bool,
}

impl AppError {
    #[must_use]
    pub fn new(kind: AppErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            operation_id: None,
            retryable: false,
        }
    }

    #[must_use]
    pub fn for_operation(mut self, operation_id: OperationId) -> Self {
        self.operation_id = Some(operation_id);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ConnectionStatus {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Disconnecting,
    Reconnecting,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ConnectionSnapshot {
    pub status: ConnectionStatus,
    pub profile_name: Option<String>,
    pub operation_id: Option<OperationId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SchemaSnapshot {
    pub databases: Vec<String>,
    pub selected_database: Option<String>,
    pub tables: Vec<String>,
    pub selected_table: Option<String>,
    pub columns: Vec<ColumnSchema>,
    pub relationships: Vec<TableRelationship>,
    pub loading: bool,
    pub operation_id: Option<OperationId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmationSnapshot {
    pub operation_id: OperationId,
    pub sql: String,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QuerySnapshot {
    pub sql: String,
    pub running: bool,
    pub operation_id: Option<OperationId>,
    pub confirmation: Option<ConfirmationSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ResultsSnapshot {
    pub columns: Vec<ColumnMeta>,
    pub rows: Vec<QueryRow>,
    pub rows_seen: u64,
    pub rows_buffered: usize,
    pub truncated: bool,
    pub search: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Csv,
    Json,
    JsonLines,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportScope {
    LoadedRows,
    FullQuery { sql: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRequest {
    pub path: PathBuf,
    pub format: ExportFormat,
    pub scope: ExportScope,
    pub typed_values: bool,
    pub gzip: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExportSnapshot {
    pub running: bool,
    pub operation_id: Option<OperationId>,
    pub destination: Option<PathBuf>,
    pub rows_written: u64,
    pub bytes_written: u64,
    pub columns_written: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AppSnapshot {
    pub connection: ConnectionSnapshot,
    pub profiles: Vec<ConnectionProfile>,
    pub schema: SchemaSnapshot,
    pub query: QuerySnapshot,
    pub results: ResultsSnapshot,
    pub export: ExportSnapshot,
    pub last_error: Option<AppError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppCommand {
    Connect {
        profile_name: String,
    },
    ConnectProfile {
        profile: ConnectionProfile,
    },
    Disconnect,
    SelectDatabase {
        database: String,
    },
    SelectTable {
        database: String,
        table: String,
    },
    ReloadSchema {
        scope: SchemaScope,
    },
    ExecuteSql {
        sql: String,
    },
    ConfirmSql {
        operation_id: OperationId,
    },
    Cancel {
        operation_id: Option<OperationId>,
    },
    SearchResults {
        query: String,
    },
    PreviewTable {
        database: String,
        table: String,
        limit: u32,
        offset: u64,
    },
    Export(ExportRequest),
    UpsertProfile {
        profile: ConnectionProfile,
    },
    DeleteProfile {
        profile_name: String,
    },
    SetDefaultProfile {
        profile_name: String,
    },
    SetQuickReconnectProfile {
        profile_name: String,
    },
    ClearError,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationProgress {
    pub operation_id: OperationId,
    pub kind: OperationKind,
    pub rows: u64,
    pub bytes: u64,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppEvent {
    SnapshotChanged(Box<AppSnapshot>),
    ResultsBatch {
        operation_id: OperationId,
        columns: Vec<ColumnMeta>,
        rows: Vec<QueryRow>,
        rows_seen: u64,
    },
    Progress(OperationProgress),
    ConfirmationRequired(ConfirmationSnapshot),
    Finished {
        operation_id: OperationId,
        kind: OperationKind,
    },
    Error(AppError),
}
