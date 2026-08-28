use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use flate2::write::GzEncoder;
use flate2::Compression;
use myr_core::audit_trail::{unix_timestamp_millis, AuditOutcome, AuditRecord, FileAuditTrail};
use myr_core::profiles::{ConnectionProfile, FileProfilesStore};
use myr_core::query_runner::{CancellationToken, ColumnMeta, QueryRow, QueryValue};
use myr_core::safe_mode::{ConfirmationToken, GuardDecision, SafeModeGuard};
use myr_core::schema_cache::{ColumnSchema, SchemaScope, TableRelationship};
use tokio::sync::{broadcast, mpsc, watch};

use crate::ports::{ApplicationBackendError, ApplicationBackendFactory, ApplicationSession};
use crate::types::{
    AppCommand, AppError, AppErrorKind, AppEvent, AppSnapshot, ConfirmationSnapshot,
    ConnectionStatus, ExportFormat, ExportRequest, ExportScope, OperationId, OperationKind,
    OperationProgress, ResultsSnapshot, SchemaSnapshot,
};

const RESULT_BUFFER_CAPACITY: usize = 2_000;
const QUERY_BATCH_CAPACITY: usize = 2_000;
const VISUAL_UPDATE_INTERVAL: Duration = Duration::from_millis(100);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const QUERY_RETRY_LIMIT: u8 = 1;
const QUERY_RECONNECT_LIMIT: u8 = 2;

#[derive(Clone)]
pub struct ApplicationHandle {
    command_tx: mpsc::Sender<AppCommand>,
    snapshot_rx: watch::Receiver<AppSnapshot>,
    event_tx: broadcast::Sender<AppEvent>,
}

impl ApplicationHandle {
    /// Sends a command to the application actor, waiting for queue capacity.
    ///
    /// # Errors
    ///
    /// Returns an internal application error if the actor has stopped.
    pub async fn command(&self, command: AppCommand) -> Result<(), AppError> {
        self.command_tx
            .send(command)
            .await
            .map_err(|_| AppError::new(AppErrorKind::Internal, "application actor is not running"))
    }

    /// Tries to send a command without waiting for queue capacity.
    ///
    /// # Errors
    ///
    /// Returns an internal application error if the queue is full or the actor has stopped.
    pub fn try_command(&self, command: AppCommand) -> Result<(), AppError> {
        self.command_tx.try_send(command).map_err(|error| {
            AppError::new(
                AppErrorKind::Internal,
                format!("application command queue rejected the command: {error}"),
            )
        })
    }

    #[must_use]
    pub fn snapshot(&self) -> AppSnapshot {
        self.snapshot_rx.borrow().clone()
    }

    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.event_tx.subscribe()
    }
}

pub fn spawn_application(
    factory: Arc<dyn ApplicationBackendFactory>,
    profiles: FileProfilesStore,
) -> ApplicationHandle {
    let audit_path = profiles
        .path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("audit.ndjson");
    let initial_snapshot = AppSnapshot {
        profiles: profiles.profiles().to_vec(),
        ..AppSnapshot::default()
    };
    let (command_tx, command_rx) = mpsc::channel(128);
    let (snapshot_tx, snapshot_rx) = watch::channel(initial_snapshot.clone());
    let (event_tx, _) = broadcast::channel(128);
    let (worker_tx, worker_rx) = mpsc::unbounded_channel();

    let actor = ApplicationActor {
        factory,
        profiles,
        session: None,
        active_profile: None,
        snapshot: initial_snapshot,
        command_rx,
        snapshot_tx,
        event_tx: event_tx.clone(),
        worker_tx,
        worker_rx,
        next_operation_id: 0,
        safe_mode: SafeModeGuard::new(true),
        audit_trail: FileAuditTrail::from_path(audit_path),
        pending_confirmation: None,
        active_query: None,
        active_query_started: None,
        query_retry_attempts: 0,
        query_reconnect_attempts: 0,
        active_export: None,
    };
    tokio::spawn(actor.run());

    ApplicationHandle {
        command_tx,
        snapshot_rx,
        event_tx,
    }
}

struct PendingConfirmation {
    operation_id: OperationId,
    token: ConfirmationToken,
    sql: String,
}

struct ApplicationActor {
    factory: Arc<dyn ApplicationBackendFactory>,
    profiles: FileProfilesStore,
    session: Option<Arc<dyn ApplicationSession>>,
    active_profile: Option<ConnectionProfile>,
    snapshot: AppSnapshot,
    command_rx: mpsc::Receiver<AppCommand>,
    snapshot_tx: watch::Sender<AppSnapshot>,
    event_tx: broadcast::Sender<AppEvent>,
    worker_tx: mpsc::UnboundedSender<WorkerEvent>,
    worker_rx: mpsc::UnboundedReceiver<WorkerEvent>,
    next_operation_id: u64,
    safe_mode: SafeModeGuard,
    audit_trail: FileAuditTrail,
    pending_confirmation: Option<PendingConfirmation>,
    active_query: Option<(OperationId, CancellationToken)>,
    active_query_started: Option<(OperationId, Instant)>,
    query_retry_attempts: u8,
    query_reconnect_attempts: u8,
    active_export: Option<(OperationId, CancellationToken)>,
}

enum WorkerEvent {
    Connected {
        operation_id: OperationId,
        profile: ConnectionProfile,
        result: Result<(Arc<dyn ApplicationSession>, Vec<String>), AppError>,
    },
    Disconnected {
        operation_id: OperationId,
        result: Result<(), AppError>,
    },
    TablesLoaded {
        operation_id: OperationId,
        database: String,
        result: Result<Vec<String>, AppError>,
    },
    TableLoaded {
        operation_id: OperationId,
        database: String,
        table: String,
        result: Result<(Vec<ColumnSchema>, Vec<TableRelationship>), AppError>,
    },
    QueryProgress {
        operation_id: OperationId,
        columns: Vec<ColumnMeta>,
        rows: Vec<QueryRow>,
        rows_seen: u64,
    },
    QueryFinished {
        operation_id: OperationId,
        rows_seen: u64,
        cancelled: bool,
    },
    QueryFailed {
        operation_id: OperationId,
        error: AppError,
    },
    QueryReconnected {
        operation_id: OperationId,
        result: Result<(Arc<dyn ApplicationSession>, Vec<String>), AppError>,
    },
    ExportProgress {
        operation_id: OperationId,
        rows: u64,
        bytes: u64,
    },
    ExportFinished {
        operation_id: OperationId,
        result: Result<ExportOutcome, AppError>,
    },
}

impl ApplicationActor {
    async fn run(mut self) {
        loop {
            tokio::select! {
                command = self.command_rx.recv() => {
                    let Some(command) = command else { break; };
                    self.handle_command(command);
                }
                worker = self.worker_rx.recv() => {
                    let Some(worker) = worker else { break; };
                    self.handle_worker_event(worker);
                }
            }
        }

        if let Some((_, cancellation)) = self.active_query.take() {
            cancellation.cancel();
        }
        if let Some((_, cancellation)) = self.active_export.take() {
            cancellation.cancel();
        }
    }

    fn handle_command(&mut self, command: AppCommand) {
        match command {
            AppCommand::Connect { profile_name } => {
                let Some(profile) = self.profiles.profile(&profile_name).cloned() else {
                    self.emit_error(AppError::new(
                        AppErrorKind::Profile,
                        format!("connection profile `{profile_name}` was not found"),
                    ));
                    return;
                };
                self.begin_connect(profile);
            }
            AppCommand::ConnectProfile { profile } => self.begin_connect(profile),
            AppCommand::Disconnect => self.begin_disconnect(),
            AppCommand::SelectDatabase { database } => self.load_tables(database),
            AppCommand::SelectTable { database, table } => self.load_table(database, table),
            AppCommand::ReloadSchema { scope } => self.reload_schema(scope),
            AppCommand::ExecuteSql { sql } => self.evaluate_query(sql),
            AppCommand::ConfirmSql { operation_id } => self.confirm_query(operation_id),
            AppCommand::Cancel { operation_id } => self.cancel_operation(operation_id),
            AppCommand::SearchResults { query } => {
                self.snapshot.results.search = query;
                self.publish_snapshot();
            }
            AppCommand::PreviewTable {
                database,
                table,
                limit,
                offset,
            } => {
                let sql = format!(
                    "SELECT * FROM {}.{} LIMIT {} OFFSET {}",
                    quote_identifier(&database),
                    quote_identifier(&table),
                    limit.clamp(1, 2_000),
                    offset
                );
                self.evaluate_query(sql);
            }
            AppCommand::Export(request) => self.begin_export(request),
            AppCommand::UpsertProfile { profile } => {
                self.update_profiles(|profiles| {
                    profiles.upsert_profile(profile.clone());
                    if profile.is_default {
                        let _ = profiles.set_default_profile(&profile.name);
                    }
                    if profile.quick_reconnect {
                        let _ = profiles.set_quick_reconnect_profile(&profile.name);
                    }
                });
            }
            AppCommand::DeleteProfile { profile_name } => {
                let deleted =
                    self.update_profiles(|profiles| profiles.delete_profile(&profile_name));
                if deleted == Some(false) {
                    self.emit_error(AppError::new(
                        AppErrorKind::Profile,
                        format!("connection profile `{profile_name}` was not found"),
                    ));
                }
            }
            AppCommand::SetDefaultProfile { profile_name } => {
                let updated =
                    self.update_profiles(|profiles| profiles.set_default_profile(&profile_name));
                if updated == Some(false) {
                    self.emit_error(AppError::new(
                        AppErrorKind::Profile,
                        format!("connection profile `{profile_name}` was not found"),
                    ));
                }
            }
            AppCommand::SetQuickReconnectProfile { profile_name } => {
                let updated = self.update_profiles(|profiles| {
                    profiles.set_quick_reconnect_profile(&profile_name)
                });
                if updated == Some(false) {
                    self.emit_error(AppError::new(
                        AppErrorKind::Profile,
                        format!("connection profile `{profile_name}` was not found"),
                    ));
                }
            }
            AppCommand::ClearError => {
                self.snapshot.last_error = None;
                self.publish_snapshot();
            }
        }
    }

    fn begin_connect(&mut self, profile: ConnectionProfile) {
        self.cancel_operation(None);
        let operation_id = self.next_id();
        self.snapshot.last_error = None;
        self.snapshot.connection.status = ConnectionStatus::Connecting;
        self.snapshot.connection.profile_name = Some(profile.name.clone());
        self.snapshot.connection.operation_id = Some(operation_id);
        self.snapshot.schema = SchemaSnapshot::default();
        self.publish_snapshot();

        let factory = Arc::clone(&self.factory);
        let worker_tx = self.worker_tx.clone();
        tokio::spawn(async move {
            let result = tokio::time::timeout(CONNECT_TIMEOUT, async {
                let session = factory.connect(&profile).await.map_err(app_backend_error)?;
                let databases = session.list_databases().await.map_err(app_backend_error)?;
                Ok::<_, AppError>((session, databases))
            })
            .await
            .unwrap_or_else(|_| Err(AppError::new(AppErrorKind::Timeout, "connection timed out")))
            .map_err(|error| error.for_operation(operation_id));
            let _ = worker_tx.send(WorkerEvent::Connected {
                operation_id,
                profile,
                result,
            });
        });
    }

    fn begin_disconnect(&mut self) {
        if let Some((_, cancellation)) = self.active_query.take() {
            cancellation.cancel();
        }
        if let Some((_, cancellation)) = self.active_export.take() {
            cancellation.cancel();
        }
        let Some(session) = self.session.take() else {
            self.snapshot.connection = crate::types::ConnectionSnapshot::default();
            self.snapshot.schema = SchemaSnapshot::default();
            self.publish_snapshot();
            return;
        };

        let operation_id = self.next_id();
        self.snapshot.connection.status = ConnectionStatus::Disconnecting;
        self.snapshot.connection.operation_id = Some(operation_id);
        self.publish_snapshot();
        let worker_tx = self.worker_tx.clone();
        tokio::spawn(async move {
            let result = session
                .disconnect()
                .await
                .map_err(app_backend_error)
                .map_err(|error| error.for_operation(operation_id));
            let _ = worker_tx.send(WorkerEvent::Disconnected {
                operation_id,
                result,
            });
        });
    }

    fn load_tables(&mut self, database: String) {
        let Some(session) = self.session.clone() else {
            self.emit_error(AppError::new(AppErrorKind::Connection, "not connected"));
            return;
        };
        let operation_id = self.next_id();
        self.snapshot.schema.selected_database = Some(database.clone());
        self.snapshot.schema.tables.clear();
        self.snapshot.schema.selected_table = None;
        self.snapshot.schema.columns.clear();
        self.snapshot.schema.relationships.clear();
        self.snapshot.schema.loading = true;
        self.snapshot.schema.operation_id = Some(operation_id);
        self.publish_snapshot();
        let worker_tx = self.worker_tx.clone();
        tokio::spawn(async move {
            let result = session
                .list_tables(&database)
                .await
                .map_err(app_backend_error)
                .map_err(|error| error.for_operation(operation_id));
            let _ = worker_tx.send(WorkerEvent::TablesLoaded {
                operation_id,
                database,
                result,
            });
        });
    }

    fn load_table(&mut self, database: String, table: String) {
        let Some(session) = self.session.clone() else {
            self.emit_error(AppError::new(AppErrorKind::Connection, "not connected"));
            return;
        };
        let operation_id = self.next_id();
        self.snapshot.schema.selected_database = Some(database.clone());
        self.snapshot.schema.selected_table = Some(table.clone());
        self.snapshot.schema.columns.clear();
        self.snapshot.schema.relationships.clear();
        self.snapshot.schema.loading = true;
        self.snapshot.schema.operation_id = Some(operation_id);
        self.publish_snapshot();
        let worker_tx = self.worker_tx.clone();
        tokio::spawn(async move {
            let (columns, relationships) = tokio::join!(
                session.list_columns(&database, &table),
                session.list_relationships(&database, &table),
            );
            let result = columns
                .map_err(app_backend_error)
                .and_then(|columns| {
                    relationships
                        .map(|relationships| (columns, relationships))
                        .map_err(app_backend_error)
                })
                .map_err(|error| error.for_operation(operation_id));
            let _ = worker_tx.send(WorkerEvent::TableLoaded {
                operation_id,
                database,
                table,
                result,
            });
        });
    }

    fn reload_schema(&mut self, scope: SchemaScope) {
        match scope {
            SchemaScope::All | SchemaScope::Databases => {
                if let Some(profile) = self.active_profile.clone() {
                    self.begin_connect(profile);
                }
            }
            SchemaScope::Tables { database } => self.load_tables(database),
            SchemaScope::Table { database, table } => self.load_table(database, table),
        }
    }

    fn evaluate_query(&mut self, sql: String) {
        if sql.trim().is_empty() {
            self.emit_error(AppError::new(AppErrorKind::Query, "SQL must not be empty"));
            return;
        }
        if self.session.is_none() {
            self.emit_error(AppError::new(AppErrorKind::Connection, "not connected"));
            return;
        }

        let operation_id = self.next_id();
        let assessment = myr_core::safe_mode::assess_sql_safety(&sql);
        if self
            .active_profile
            .as_ref()
            .is_some_and(|profile| profile.read_only)
            && !assessment.is_safe_read_only()
        {
            self.append_audit_event(
                AuditOutcome::Blocked,
                &sql,
                None,
                None,
                Some("read-only profile blocked a statement with side effects"),
            );
            self.emit_error(
                AppError::new(
                    AppErrorKind::Query,
                    "read-only profile blocked a statement with side effects",
                )
                .for_operation(operation_id),
            );
            return;
        }

        match self.safe_mode.evaluate(&sql) {
            GuardDecision::Allow { .. } => self.begin_query(operation_id, sql),
            GuardDecision::RequireConfirmation { token, assessment } => {
                let confirmation = ConfirmationSnapshot {
                    operation_id,
                    sql: sql.clone(),
                    reasons: assessment
                        .reasons
                        .iter()
                        .map(|reason| format!("{reason:?}"))
                        .collect(),
                };
                self.pending_confirmation = Some(PendingConfirmation {
                    operation_id,
                    token,
                    sql,
                });
                self.snapshot.query.confirmation = Some(confirmation.clone());
                self.publish_snapshot();
                let _ = self
                    .event_tx
                    .send(AppEvent::ConfirmationRequired(confirmation));
            }
        }
    }

    fn confirm_query(&mut self, operation_id: OperationId) {
        let Some(pending) = self.pending_confirmation.take() else {
            self.emit_error(AppError::new(
                AppErrorKind::Query,
                "no SQL confirmation is pending",
            ));
            return;
        };
        if pending.operation_id != operation_id {
            self.pending_confirmation = Some(pending);
            self.emit_error(AppError::new(
                AppErrorKind::Query,
                "confirmation belongs to a stale operation",
            ));
            return;
        }
        if let Err(error) = self.safe_mode.confirm(&pending.token, &pending.sql) {
            self.emit_error(
                AppError::new(AppErrorKind::Query, error.to_string()).for_operation(operation_id),
            );
            return;
        }
        self.snapshot.query.confirmation = None;
        self.begin_query(operation_id, pending.sql);
    }

    fn begin_query(&mut self, operation_id: OperationId, sql: String) {
        let Some(session) = self.session.clone() else {
            return;
        };
        if let Some((_, cancellation)) = self.active_query.take() {
            cancellation.cancel();
            if let Some((_, started)) = self.active_query_started.take() {
                self.append_audit_event(
                    AuditOutcome::Cancelled,
                    &self.snapshot.query.sql,
                    Some(self.snapshot.results.rows_seen),
                    Some(started.elapsed()),
                    Some("superseded by a newer query"),
                );
            }
        }
        let cancellation = CancellationToken::new();
        self.active_query = Some((operation_id, cancellation.clone()));
        self.active_query_started = Some((operation_id, Instant::now()));
        self.query_retry_attempts = 0;
        self.query_reconnect_attempts = 0;
        self.snapshot.query.sql.clone_from(&sql);
        self.snapshot.query.running = true;
        self.snapshot.query.operation_id = Some(operation_id);
        self.snapshot.query.confirmation = None;
        self.snapshot.results = ResultsSnapshot::default();
        self.snapshot.last_error = None;
        self.append_audit_event(AuditOutcome::Started, &sql, None, None, None);
        self.publish_snapshot();

        self.spawn_query_task(session, operation_id, sql, cancellation);
    }

    fn spawn_query_task(
        &self,
        session: Arc<dyn ApplicationSession>,
        operation_id: OperationId,
        sql: String,
        cancellation: CancellationToken,
    ) {
        tokio::spawn(run_query(
            session,
            operation_id,
            sql,
            cancellation,
            self.worker_tx.clone(),
        ));
    }

    fn retry_query(&mut self, operation_id: OperationId) {
        let Some(session) = self.session.clone() else {
            return;
        };
        let cancellation = CancellationToken::new();
        self.active_query = Some((operation_id, cancellation.clone()));
        let _ = self.event_tx.send(AppEvent::Progress(OperationProgress {
            operation_id,
            kind: OperationKind::Query,
            rows: self.snapshot.results.rows_seen,
            bytes: 0,
            message: format!(
                "retrying query ({}/{QUERY_RETRY_LIMIT})",
                self.query_retry_attempts
            ),
        }));
        self.spawn_query_task(
            session,
            operation_id,
            self.snapshot.query.sql.clone(),
            cancellation,
        );
    }

    fn reconnect_query(&mut self, operation_id: OperationId) {
        let Some(profile) = self.active_profile.clone() else {
            return;
        };
        self.session = None;
        self.snapshot.connection.status = ConnectionStatus::Reconnecting;
        self.snapshot.connection.operation_id = Some(operation_id);
        self.publish_snapshot();
        let _ = self.event_tx.send(AppEvent::Progress(OperationProgress {
            operation_id,
            kind: OperationKind::Connection,
            rows: self.snapshot.results.rows_seen,
            bytes: 0,
            message: format!(
                "reconnecting ({}/{QUERY_RECONNECT_LIMIT})",
                self.query_reconnect_attempts
            ),
        }));
        let factory = Arc::clone(&self.factory);
        let worker_tx = self.worker_tx.clone();
        tokio::spawn(async move {
            let result = tokio::time::timeout(CONNECT_TIMEOUT, async {
                let session = factory.connect(&profile).await.map_err(app_backend_error)?;
                let databases = session.list_databases().await.map_err(app_backend_error)?;
                Ok::<_, AppError>((session, databases))
            })
            .await
            .unwrap_or_else(|_| Err(AppError::new(AppErrorKind::Timeout, "reconnect timed out")))
            .map_err(|error| error.for_operation(operation_id));
            let _ = worker_tx.send(WorkerEvent::QueryReconnected {
                operation_id,
                result,
            });
        });
    }

    fn cancel_operation(&mut self, requested: Option<OperationId>) {
        if let Some((operation_id, cancellation)) = &self.active_query {
            if requested.is_none_or(|requested| requested == *operation_id) {
                cancellation.cancel();
            }
        }
        if let Some((operation_id, cancellation)) = &self.active_export {
            if requested.is_none_or(|requested| requested == *operation_id) {
                cancellation.cancel();
            }
        }
    }

    fn begin_export(&mut self, request: ExportRequest) {
        if matches!(&request.scope, ExportScope::FullQuery { sql } if !myr_core::safe_mode::assess_sql_safety(sql).is_safe_read_only())
        {
            self.emit_error(AppError::new(
                AppErrorKind::Export,
                "full-query export only accepts read-only SQL",
            ));
            return;
        }
        if matches!(&request.scope, ExportScope::FullQuery { .. }) && self.session.is_none() {
            self.emit_error(AppError::new(AppErrorKind::Connection, "not connected"));
            return;
        }

        let operation_id = self.next_id();
        if let Some((_, cancellation)) = self.active_export.take() {
            cancellation.cancel();
        }
        let cancellation = CancellationToken::new();
        self.active_export = Some((operation_id, cancellation.clone()));
        self.snapshot.export.running = true;
        self.snapshot.export.operation_id = Some(operation_id);
        self.snapshot.export.destination = Some(request.path.clone());
        self.snapshot.export.rows_written = 0;
        self.snapshot.export.bytes_written = 0;
        self.snapshot.export.columns_written = 0;
        self.publish_snapshot();

        let worker_tx = self.worker_tx.clone();
        let session = self.session.clone();
        let columns = self.snapshot.results.columns.clone();
        let rows = self.snapshot.results.rows.clone();
        tokio::spawn(run_export(
            operation_id,
            request,
            session,
            columns,
            rows,
            cancellation,
            worker_tx,
        ));
    }

    fn update_profiles<R>(
        &mut self,
        update: impl FnOnce(&mut FileProfilesStore) -> R,
    ) -> Option<R> {
        match self.profiles.update_locked(update) {
            Ok(result) => {
                self.snapshot.profiles = self.profiles.profiles().to_vec();
                self.snapshot.last_error = None;
                self.publish_snapshot();
                Some(result)
            }
            Err(error) => {
                self.emit_error(AppError::new(AppErrorKind::Profile, error.to_string()));
                None
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn handle_worker_event(&mut self, event: WorkerEvent) {
        match event {
            WorkerEvent::Connected {
                operation_id,
                profile,
                result,
            } => {
                if self.snapshot.connection.operation_id != Some(operation_id) {
                    return;
                }
                match result {
                    Ok((session, databases)) => {
                        self.session = Some(session);
                        self.active_profile = Some(profile.clone());
                        self.snapshot.connection.status = ConnectionStatus::Connected;
                        self.snapshot.connection.profile_name = Some(profile.name);
                        self.snapshot.connection.operation_id = None;
                        self.snapshot.schema.databases = databases;
                        self.snapshot.schema.loading = false;
                        self.publish_snapshot();
                        self.emit_finished(operation_id, OperationKind::Connection);
                    }
                    Err(error) => {
                        self.session = None;
                        self.active_profile = None;
                        self.snapshot.connection.status = ConnectionStatus::Disconnected;
                        self.snapshot.connection.operation_id = None;
                        self.emit_error(error);
                    }
                }
            }
            WorkerEvent::Disconnected {
                operation_id,
                result,
            } => {
                if self.snapshot.connection.operation_id != Some(operation_id) {
                    return;
                }
                self.active_profile = None;
                self.snapshot.connection = crate::types::ConnectionSnapshot::default();
                self.snapshot.schema = SchemaSnapshot::default();
                self.snapshot.query.running = false;
                self.publish_snapshot();
                match result {
                    Ok(()) => self.emit_finished(operation_id, OperationKind::Connection),
                    Err(error) => self.emit_error(error),
                }
            }
            WorkerEvent::TablesLoaded {
                operation_id,
                database,
                result,
            } => {
                if self.snapshot.schema.operation_id != Some(operation_id)
                    || self.snapshot.schema.selected_database.as_deref() != Some(&database)
                {
                    return;
                }
                self.snapshot.schema.loading = false;
                self.snapshot.schema.operation_id = None;
                match result {
                    Ok(tables) => {
                        self.snapshot.schema.tables = tables;
                        self.publish_snapshot();
                        self.emit_finished(operation_id, OperationKind::Schema);
                    }
                    Err(error) => self.emit_error(error),
                }
            }
            WorkerEvent::TableLoaded {
                operation_id,
                database,
                table,
                result,
            } => {
                if self.snapshot.schema.operation_id != Some(operation_id)
                    || self.snapshot.schema.selected_database.as_deref() != Some(&database)
                    || self.snapshot.schema.selected_table.as_deref() != Some(&table)
                {
                    return;
                }
                self.snapshot.schema.loading = false;
                self.snapshot.schema.operation_id = None;
                match result {
                    Ok((columns, relationships)) => {
                        self.snapshot.schema.columns = columns;
                        self.snapshot.schema.relationships = relationships;
                        self.publish_snapshot();
                        self.emit_finished(operation_id, OperationKind::Schema);
                    }
                    Err(error) => self.emit_error(error),
                }
            }
            WorkerEvent::QueryProgress {
                operation_id,
                columns,
                rows,
                rows_seen,
            } => {
                if self.snapshot.query.operation_id != Some(operation_id) {
                    return;
                }
                if self.snapshot.results.columns.is_empty() {
                    self.snapshot.results.columns.clone_from(&columns);
                }
                let _ = self.event_tx.send(AppEvent::ResultsBatch {
                    operation_id,
                    columns: columns.clone(),
                    rows: rows.clone(),
                    rows_seen,
                });
                let mut buffered = self
                    .snapshot
                    .results
                    .rows
                    .drain(..)
                    .collect::<VecDeque<_>>();
                buffered.extend(rows);
                while buffered.len() > RESULT_BUFFER_CAPACITY {
                    buffered.pop_front();
                }
                self.snapshot.results.rows = buffered.into_iter().collect();
                self.snapshot.results.rows_seen = rows_seen;
                self.snapshot.results.rows_buffered = self.snapshot.results.rows.len();
                self.snapshot.results.truncated = rows_seen
                    > u64::try_from(self.snapshot.results.rows_buffered).unwrap_or(u64::MAX);
                self.publish_snapshot();
                let _ = self.event_tx.send(AppEvent::Progress(OperationProgress {
                    operation_id,
                    kind: OperationKind::Query,
                    rows: rows_seen,
                    bytes: 0,
                    message: "query streaming".to_string(),
                }));
            }
            WorkerEvent::QueryFinished {
                operation_id,
                rows_seen,
                cancelled,
            } => {
                if self.snapshot.query.operation_id != Some(operation_id) {
                    return;
                }
                self.active_query = None;
                self.query_retry_attempts = 0;
                self.query_reconnect_attempts = 0;
                let elapsed = self
                    .active_query_started
                    .take()
                    .filter(|(active_id, _)| *active_id == operation_id)
                    .map(|(_, started)| started.elapsed());
                self.snapshot.query.running = false;
                self.snapshot.query.operation_id = None;
                self.snapshot.results.rows_seen = rows_seen;
                self.snapshot.results.rows_buffered = self.snapshot.results.rows.len();
                self.snapshot.results.truncated = rows_seen
                    > u64::try_from(self.snapshot.results.rows_buffered).unwrap_or(u64::MAX);
                self.publish_snapshot();
                if cancelled {
                    self.append_audit_event(
                        AuditOutcome::Cancelled,
                        &self.snapshot.query.sql,
                        Some(rows_seen),
                        elapsed,
                        Some("query cancelled"),
                    );
                    self.emit_error(
                        AppError::new(AppErrorKind::Cancellation, "query cancelled")
                            .for_operation(operation_id),
                    );
                } else {
                    self.append_audit_event(
                        AuditOutcome::Succeeded,
                        &self.snapshot.query.sql,
                        Some(rows_seen),
                        elapsed,
                        None,
                    );
                    self.emit_finished(operation_id, OperationKind::Query);
                }
            }
            WorkerEvent::QueryFailed {
                operation_id,
                error,
            } => {
                if self.snapshot.query.operation_id != Some(operation_id) {
                    return;
                }
                if error.retryable && self.query_retry_attempts < QUERY_RETRY_LIMIT {
                    self.query_retry_attempts = self.query_retry_attempts.saturating_add(1);
                    self.retry_query(operation_id);
                    return;
                }
                if error.kind == AppErrorKind::Connection
                    && self.query_reconnect_attempts < QUERY_RECONNECT_LIMIT
                    && self.active_profile.is_some()
                {
                    self.query_reconnect_attempts = self.query_reconnect_attempts.saturating_add(1);
                    self.query_retry_attempts = 0;
                    self.reconnect_query(operation_id);
                    return;
                }
                self.active_query = None;
                let elapsed = self
                    .active_query_started
                    .take()
                    .filter(|(active_id, _)| *active_id == operation_id)
                    .map(|(_, started)| started.elapsed());
                self.snapshot.query.running = false;
                self.snapshot.query.operation_id = None;
                self.append_audit_event(
                    AuditOutcome::Failed,
                    &self.snapshot.query.sql,
                    Some(self.snapshot.results.rows_seen),
                    elapsed,
                    Some(&error.message),
                );
                self.emit_error(error);
            }
            WorkerEvent::QueryReconnected {
                operation_id,
                result,
            } => {
                if self.snapshot.query.operation_id != Some(operation_id) {
                    return;
                }
                match result {
                    Ok((session, databases)) => {
                        self.session = Some(session.clone());
                        self.snapshot.connection.status = ConnectionStatus::Connected;
                        self.snapshot.connection.operation_id = None;
                        self.snapshot.schema.databases = databases;
                        self.publish_snapshot();
                        let cancellation = self
                            .active_query
                            .as_ref()
                            .map_or_else(CancellationToken::new, |(_, token)| token.clone());
                        if cancellation.is_cancelled() {
                            let _ = self.worker_tx.send(WorkerEvent::QueryFinished {
                                operation_id,
                                rows_seen: self.snapshot.results.rows_seen,
                                cancelled: true,
                            });
                        } else {
                            self.spawn_query_task(
                                session,
                                operation_id,
                                self.snapshot.query.sql.clone(),
                                cancellation,
                            );
                        }
                    }
                    Err(error) => {
                        if self.query_reconnect_attempts < QUERY_RECONNECT_LIMIT {
                            self.query_reconnect_attempts =
                                self.query_reconnect_attempts.saturating_add(1);
                            self.reconnect_query(operation_id);
                        } else {
                            self.active_query = None;
                            self.snapshot.query.running = false;
                            self.snapshot.query.operation_id = None;
                            self.snapshot.connection.status = ConnectionStatus::Disconnected;
                            self.snapshot.connection.operation_id = None;
                            self.emit_error(error);
                        }
                    }
                }
            }
            WorkerEvent::ExportProgress {
                operation_id,
                rows,
                bytes,
            } => {
                if self.snapshot.export.operation_id != Some(operation_id) {
                    return;
                }
                self.snapshot.export.rows_written = rows;
                self.snapshot.export.bytes_written = bytes;
                self.publish_snapshot();
                let _ = self.event_tx.send(AppEvent::Progress(OperationProgress {
                    operation_id,
                    kind: OperationKind::Export,
                    rows,
                    bytes,
                    message: "export streaming".to_string(),
                }));
            }
            WorkerEvent::ExportFinished {
                operation_id,
                result,
            } => {
                if self.snapshot.export.operation_id != Some(operation_id) {
                    return;
                }
                self.active_export = None;
                self.snapshot.export.running = false;
                self.snapshot.export.operation_id = None;
                match result {
                    Ok(outcome) => {
                        self.snapshot.export.rows_written = outcome.rows;
                        self.snapshot.export.bytes_written = outcome.bytes;
                        self.snapshot.export.columns_written = outcome.columns;
                        self.publish_snapshot();
                        self.emit_finished(operation_id, OperationKind::Export);
                    }
                    Err(error) => self.emit_error(error),
                }
            }
        }
    }

    fn next_id(&mut self) -> OperationId {
        self.next_operation_id = self.next_operation_id.saturating_add(1);
        OperationId(self.next_operation_id)
    }

    fn publish_snapshot(&self) {
        self.snapshot_tx.send_replace(self.snapshot.clone());
        let _ = self
            .event_tx
            .send(AppEvent::SnapshotChanged(Box::new(self.snapshot.clone())));
    }

    fn emit_error(&mut self, error: AppError) {
        self.snapshot.last_error = Some(error.clone());
        self.publish_snapshot();
        let _ = self.event_tx.send(AppEvent::Error(error));
    }

    fn emit_finished(&self, operation_id: OperationId, kind: OperationKind) {
        let _ = self
            .event_tx
            .send(AppEvent::Finished { operation_id, kind });
    }

    fn append_audit_event(
        &self,
        outcome: AuditOutcome,
        sql: &str,
        rows_streamed: Option<u64>,
        elapsed: Option<Duration>,
        error: Option<&str>,
    ) {
        let record = AuditRecord {
            timestamp_unix_ms: unix_timestamp_millis(),
            profile_name: self
                .active_profile
                .as_ref()
                .map(|profile| profile.name.clone()),
            database: self.snapshot.schema.selected_database.clone().or_else(|| {
                self.active_profile
                    .as_ref()
                    .and_then(|p| p.database.clone())
            }),
            outcome,
            sql: sql.chars().take(1_000).collect(),
            rows_streamed,
            elapsed_ms: elapsed.map(|duration| duration.as_millis()),
            error: error.map(|message| message.chars().take(400).collect()),
        };
        let _ = self.audit_trail.append(&record);
    }
}

async fn run_query(
    session: Arc<dyn ApplicationSession>,
    operation_id: OperationId,
    sql: String,
    cancellation: CancellationToken,
    worker_tx: mpsc::UnboundedSender<WorkerEvent>,
) {
    let mut stream = match session.start_query(&sql).await {
        Ok(stream) => stream,
        Err(error) => {
            let _ = worker_tx.send(WorkerEvent::QueryFailed {
                operation_id,
                error: app_backend_error(error).for_operation(operation_id),
            });
            return;
        }
    };
    let columns = stream.columns().unwrap_or_default().to_vec();
    let mut pending_rows = Vec::new();
    let mut rows_seen = 0_u64;
    let mut last_update = Instant::now();

    loop {
        if cancellation.is_cancelled() {
            let _ = stream.cancel().await;
            send_query_progress(
                &worker_tx,
                operation_id,
                &columns,
                &mut pending_rows,
                rows_seen,
            );
            let _ = worker_tx.send(WorkerEvent::QueryFinished {
                operation_id,
                rows_seen,
                cancelled: true,
            });
            return;
        }

        match stream.next_row().await {
            Ok(Some(row)) => {
                rows_seen = rows_seen.saturating_add(1);
                pending_rows.push(row);
                if pending_rows.len() >= QUERY_BATCH_CAPACITY {
                    if let Some(delay) = VISUAL_UPDATE_INTERVAL.checked_sub(last_update.elapsed()) {
                        tokio::time::sleep(delay).await;
                    }
                }
                if last_update.elapsed() >= VISUAL_UPDATE_INTERVAL {
                    send_query_progress(
                        &worker_tx,
                        operation_id,
                        &columns,
                        &mut pending_rows,
                        rows_seen,
                    );
                    last_update = Instant::now();
                }
            }
            Ok(None) => {
                send_query_progress(
                    &worker_tx,
                    operation_id,
                    &columns,
                    &mut pending_rows,
                    rows_seen,
                );
                let _ = worker_tx.send(WorkerEvent::QueryFinished {
                    operation_id,
                    rows_seen,
                    cancelled: false,
                });
                return;
            }
            Err(error) => {
                let _ = worker_tx.send(WorkerEvent::QueryFailed {
                    operation_id,
                    error: AppError::new(AppErrorKind::Query, error.to_string())
                        .for_operation(operation_id),
                });
                return;
            }
        }
    }
}

fn send_query_progress(
    worker_tx: &mpsc::UnboundedSender<WorkerEvent>,
    operation_id: OperationId,
    columns: &[ColumnMeta],
    pending_rows: &mut Vec<QueryRow>,
    rows_seen: u64,
) {
    if pending_rows.is_empty() && rows_seen > 0 {
        return;
    }
    let rows = std::mem::take(pending_rows);
    let _ = worker_tx.send(WorkerEvent::QueryProgress {
        operation_id,
        columns: columns.to_vec(),
        rows,
        rows_seen,
    });
}

#[derive(Debug)]
struct ExportOutcome {
    rows: u64,
    bytes: u64,
    columns: usize,
}

async fn run_export(
    operation_id: OperationId,
    request: ExportRequest,
    session: Option<Arc<dyn ApplicationSession>>,
    loaded_columns: Vec<ColumnMeta>,
    loaded_rows: Vec<QueryRow>,
    cancellation: CancellationToken,
    worker_tx: mpsc::UnboundedSender<WorkerEvent>,
) {
    let result = export_rows(
        operation_id,
        &request,
        session,
        loaded_columns,
        loaded_rows,
        &cancellation,
        &worker_tx,
    )
    .await
    .map_err(|error| error.for_operation(operation_id));
    let _ = worker_tx.send(WorkerEvent::ExportFinished {
        operation_id,
        result,
    });
}

#[allow(clippy::too_many_lines)]
async fn export_rows(
    operation_id: OperationId,
    request: &ExportRequest,
    session: Option<Arc<dyn ApplicationSession>>,
    loaded_columns: Vec<ColumnMeta>,
    loaded_rows: Vec<QueryRow>,
    cancellation: &CancellationToken,
    worker_tx: &mpsc::UnboundedSender<WorkerEvent>,
) -> Result<ExportOutcome, AppError> {
    let partial_path = partial_path(&request.path);
    let result = async {
        let mut stream = match &request.scope {
            ExportScope::LoadedRows => None,
            ExportScope::FullQuery { sql } => Some(
                session
                    .ok_or_else(|| AppError::new(AppErrorKind::Connection, "not connected"))?
                    .start_query(sql)
                    .await
                    .map_err(app_backend_error)?,
            ),
        };
        let columns = stream
            .as_ref()
            .and_then(|stream| stream.columns())
            .map_or(loaded_columns, <[ColumnMeta]>::to_vec);
        if columns.is_empty() {
            return Err(AppError::new(
                AppErrorKind::Export,
                "export requires a result set with at least one column",
            ));
        }
        let headers = columns
            .iter()
            .enumerate()
            .map(|(index, column)| {
                if column.name.is_empty() {
                    format!("col_{}", index + 1)
                } else {
                    column.name.clone()
                }
            })
            .collect::<Vec<_>>();
        let mut writer = StreamingExportWriter::create(
            &partial_path,
            request.format,
            &headers,
            request.typed_values,
            request.gzip,
        )?;
        let mut rows = 0_u64;
        let mut last_update = Instant::now();

        match stream.as_mut() {
            None => {
                for row in &loaded_rows {
                    if cancellation.is_cancelled() {
                        return Err(AppError::new(
                            AppErrorKind::Cancellation,
                            "export cancelled",
                        ));
                    }
                    writer.write_row(row)?;
                    rows = rows.saturating_add(1);
                }
            }
            Some(stream) => loop {
                if cancellation.is_cancelled() {
                    let _ = stream.cancel().await;
                    return Err(AppError::new(
                        AppErrorKind::Cancellation,
                        "export cancelled",
                    ));
                }
                match stream.next_row().await {
                    Ok(Some(row)) => {
                        writer.write_row(&row)?;
                        rows = rows.saturating_add(1);
                        if last_update.elapsed() >= VISUAL_UPDATE_INTERVAL {
                            writer.flush()?;
                            let bytes = file_size(&partial_path);
                            let _ = worker_tx.send(WorkerEvent::ExportProgress {
                                operation_id,
                                rows,
                                bytes,
                            });
                            last_update = Instant::now();
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        return Err(AppError::new(AppErrorKind::Export, error.to_string()));
                    }
                }
            },
        }

        writer.finish()?;
        let bytes = file_size(&partial_path);
        fs::rename(&partial_path, &request.path).map_err(|error| {
            AppError::new(
                AppErrorKind::Export,
                format!(
                    "failed to finalize export {}: {error}",
                    request.path.display()
                ),
            )
        })?;
        Ok(ExportOutcome {
            rows,
            bytes,
            columns: headers.len(),
        })
    }
    .await;

    if result.is_err() {
        let _ = fs::remove_file(&partial_path);
    }
    result
}

struct StreamingExportWriter {
    path: PathBuf,
    format: ExportFormat,
    writer: ExportSink,
    headers: Vec<String>,
    wrote_row: bool,
    typed_values: bool,
}

enum ExportSink {
    Plain(BufWriter<File>),
    Gzip(GzEncoder<BufWriter<File>>),
}

impl Write for ExportSink {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(writer) => writer.write(buffer),
            Self::Gzip(writer) => writer.write(buffer),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Plain(writer) => writer.flush(),
            Self::Gzip(writer) => writer.flush(),
        }
    }
}

impl ExportSink {
    fn finish(self) -> std::io::Result<()> {
        match self {
            Self::Plain(mut writer) => writer.flush(),
            Self::Gzip(writer) => writer.finish().map(|_| ()),
        }
    }
}

impl StreamingExportWriter {
    fn create(
        path: &Path,
        format: ExportFormat,
        headers: &[String],
        typed_values: bool,
        gzip: bool,
    ) -> Result<Self, AppError> {
        let file = File::create(path).map_err(|error| export_io_error(path, &error))?;
        let buffered = BufWriter::new(file);
        let mut output = Self {
            path: path.to_path_buf(),
            format,
            writer: if gzip {
                ExportSink::Gzip(GzEncoder::new(buffered, Compression::default()))
            } else {
                ExportSink::Plain(buffered)
            },
            headers: headers.to_vec(),
            wrote_row: false,
            typed_values,
        };
        match format {
            ExportFormat::Csv => {
                let header = headers
                    .iter()
                    .map(|header| csv_escape(header))
                    .collect::<Vec<_>>()
                    .join(",");
                output.write_bytes(header.as_bytes())?;
                output.write_bytes(b"\n")?;
            }
            ExportFormat::Json => output.write_bytes(b"[")?,
            ExportFormat::JsonLines => {}
        }
        Ok(output)
    }

    fn write_row(&mut self, row: &QueryRow) -> Result<(), AppError> {
        match self.format {
            ExportFormat::Csv => {
                let rendered = (0..self.headers.len())
                    .map(|index| {
                        row.values
                            .get(index)
                            .map_or_else(String::new, QueryValue::display_text)
                    })
                    .map(|value| csv_escape(&value))
                    .collect::<Vec<_>>()
                    .join(",");
                self.write_bytes(rendered.as_bytes())?;
                self.write_bytes(b"\n")?;
            }
            ExportFormat::Json | ExportFormat::JsonLines => {
                if self.format == ExportFormat::Json && self.wrote_row {
                    self.write_bytes(b",")?;
                }
                let object = self
                    .headers
                    .iter()
                    .enumerate()
                    .map(|(index, header)| {
                        let value =
                            row.values
                                .get(index)
                                .map_or(serde_json::Value::Null, |value| {
                                    if self.typed_values {
                                        value.typed_json_value()
                                    } else {
                                        serde_json::Value::String(value.display_text())
                                    }
                                });
                        (header.clone(), value)
                    })
                    .collect::<serde_json::Map<_, _>>();
                serde_json::to_writer(&mut self.writer, &object)
                    .map_err(|error| AppError::new(AppErrorKind::Export, error.to_string()))?;
                if self.format == ExportFormat::JsonLines {
                    self.write_bytes(b"\n")?;
                }
            }
        }
        self.wrote_row = true;
        Ok(())
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), AppError> {
        self.writer
            .write_all(bytes)
            .map_err(|error| export_io_error(&self.path, &error))
    }

    fn flush(&mut self) -> Result<(), AppError> {
        self.writer
            .flush()
            .map_err(|error| export_io_error(&self.path, &error))
    }

    fn finish(mut self) -> Result<(), AppError> {
        if self.format == ExportFormat::Json {
            self.write_bytes(b"]\n")?;
        }
        self.writer
            .finish()
            .map_err(|error| export_io_error(&self.path, &error))
    }
}

fn app_backend_error(error: ApplicationBackendError) -> AppError {
    AppError {
        kind: error.kind,
        message: error.message,
        operation_id: None,
        retryable: matches!(error.kind, AppErrorKind::Connection | AppErrorKind::Timeout),
    }
}

fn quote_identifier(identifier: &str) -> String {
    format!("`{}`", identifier.replace('`', "``"))
}

fn partial_path(destination: &Path) -> PathBuf {
    let mut name = destination.as_os_str().to_os_string();
    name.push(".part");
    PathBuf::from(name)
}

fn file_size(path: &Path) -> u64 {
    fs::metadata(path).map_or(0, |metadata| metadata.len())
}

fn export_io_error(path: &Path, error: &std::io::Error) -> AppError {
    AppError::new(
        AppErrorKind::Export,
        format!("failed to write {}: {error}", path.display()),
    )
}

fn csv_escape(value: &str) -> String {
    if value.contains([',', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::io::{BufRead, BufReader, Read};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use async_trait::async_trait;
    use flate2::read::GzDecoder;
    use myr_core::profiles::ConnectionProfile;
    use myr_core::query_runner::{
        ColumnMeta, QueryBackendError, QueryRow, QueryRowStream, QueryValue,
    };
    use myr_core::schema_cache::{ColumnSchema, RelationshipDirection, TableRelationship};
    use tempfile::TempDir;

    use super::*;

    #[derive(Debug, Default)]
    struct Counts {
        connects: AtomicUsize,
        databases: AtomicUsize,
        tables: AtomicUsize,
        columns: AtomicUsize,
        relationships: AtomicUsize,
        queries: AtomicUsize,
    }

    #[derive(Debug)]
    struct FakeFactory {
        session: Arc<FakeSession>,
    }

    #[derive(Debug)]
    struct FakeSession {
        counts: Arc<Counts>,
        default_rows: Mutex<Vec<QueryRow>>,
    }

    #[derive(Debug)]
    struct FakeStream {
        columns: Vec<ColumnMeta>,
        rows: VecDeque<QueryRow>,
        generated_remaining: u64,
        delay: Duration,
        cancelled: bool,
    }

    #[async_trait]
    impl QueryRowStream for FakeStream {
        fn columns(&self) -> Option<&[ColumnMeta]> {
            Some(&self.columns)
        }

        async fn next_row(&mut self) -> Result<Option<QueryRow>, QueryBackendError> {
            if self.cancelled {
                return Ok(None);
            }
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            if let Some(row) = self.rows.pop_front() {
                return Ok(Some(row));
            }
            if self.generated_remaining > 0 {
                let value = 1_000_000_u64.saturating_sub(self.generated_remaining);
                self.generated_remaining = self.generated_remaining.saturating_sub(1);
                return Ok(Some(QueryRow::from_values(vec![QueryValue::UInt(value)])));
            }
            Ok(None)
        }

        async fn cancel(&mut self) -> Result<(), QueryBackendError> {
            self.cancelled = true;
            Ok(())
        }
    }

    #[async_trait]
    impl ApplicationBackendFactory for FakeFactory {
        async fn connect(
            &self,
            _profile: &ConnectionProfile,
        ) -> Result<Arc<dyn ApplicationSession>, ApplicationBackendError> {
            self.session.counts.connects.fetch_add(1, Ordering::Relaxed);
            Ok(self.session.clone())
        }
    }

    #[async_trait]
    impl ApplicationSession for FakeSession {
        async fn list_databases(&self) -> Result<Vec<String>, ApplicationBackendError> {
            self.counts.databases.fetch_add(1, Ordering::Relaxed);
            Ok(vec!["app".to_string(), "analytics".to_string()])
        }

        async fn list_tables(
            &self,
            database: &str,
        ) -> Result<Vec<String>, ApplicationBackendError> {
            self.counts.tables.fetch_add(1, Ordering::Relaxed);
            Ok(if database == "app" {
                vec!["users".to_string()]
            } else {
                Vec::new()
            })
        }

        async fn list_columns(
            &self,
            _database: &str,
            _table: &str,
        ) -> Result<Vec<ColumnSchema>, ApplicationBackendError> {
            self.counts.columns.fetch_add(1, Ordering::Relaxed);
            Ok(vec![ColumnSchema {
                name: "id".to_string(),
                data_type: "bigint".to_string(),
                nullable: false,
                default_value: None,
            }])
        }

        async fn list_relationships(
            &self,
            _database: &str,
            _table: &str,
        ) -> Result<Vec<TableRelationship>, ApplicationBackendError> {
            self.counts.relationships.fetch_add(1, Ordering::Relaxed);
            Ok(vec![TableRelationship {
                direction: RelationshipDirection::Outbound,
                constraint_name: "fk".to_string(),
                source_column: "account_id".to_string(),
                related_database: "app".to_string(),
                related_table: "accounts".to_string(),
                related_column: "id".to_string(),
            }])
        }

        async fn start_query(
            &self,
            sql: &str,
        ) -> Result<Box<dyn QueryRowStream + Send>, ApplicationBackendError> {
            let attempt = self.counts.queries.fetch_add(1, Ordering::Relaxed);
            if sql.contains("retry-once") && attempt == 0 {
                return Err(ApplicationBackendError::new(
                    AppErrorKind::Timeout,
                    "temporary timeout",
                ));
            }
            if sql.contains("reconnect") && attempt < 2 {
                return Err(ApplicationBackendError::new(
                    AppErrorKind::Connection,
                    "connection dropped",
                ));
            }
            let (rows, generated_remaining, delay) = if sql.contains("million") {
                (Vec::new(), 1_000_000, Duration::ZERO)
            } else if sql.contains("slow") {
                (
                    (0..20)
                        .map(|index| QueryRow::new(vec![format!("slow-{index}")]))
                        .collect(),
                    0,
                    Duration::from_millis(5),
                )
            } else if sql.contains("second") {
                (
                    vec![QueryRow::from_values(vec![QueryValue::UInt(2)])],
                    0,
                    Duration::ZERO,
                )
            } else {
                (
                    self.default_rows
                        .lock()
                        .expect("rows mutex should not be poisoned")
                        .clone(),
                    0,
                    Duration::ZERO,
                )
            };
            Ok(Box::new(FakeStream {
                columns: vec![ColumnMeta::new("id")],
                rows: rows.into(),
                generated_remaining,
                delay,
                cancelled: false,
            }))
        }

        async fn disconnect(&self) -> Result<(), ApplicationBackendError> {
            Ok(())
        }
    }

    fn test_application(
        temp_dir: &TempDir,
        rows: Vec<QueryRow>,
        read_only: bool,
    ) -> (ApplicationHandle, Arc<Counts>) {
        let counts = Arc::new(Counts::default());
        let session = Arc::new(FakeSession {
            counts: counts.clone(),
            default_rows: Mutex::new(rows),
        });
        let factory = Arc::new(FakeFactory { session });
        let mut store = FileProfilesStore::load_from_path(temp_dir.path().join("profiles.toml"))
            .expect("profile store should load");
        let mut profile = ConnectionProfile::new("local", "localhost", "root");
        profile.read_only = read_only;
        store.upsert_profile(profile);
        store.persist().expect("profile store should persist");
        (spawn_application(factory, store), counts)
    }

    async fn wait_for(
        handle: &ApplicationHandle,
        predicate: impl Fn(&AppSnapshot) -> bool,
    ) -> AppSnapshot {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let snapshot = handle.snapshot();
                if predicate(&snapshot) {
                    return snapshot;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("application state did not converge")
    }

    async fn connect(handle: &ApplicationHandle) {
        handle
            .command(AppCommand::Connect {
                profile_name: "local".to_string(),
            })
            .await
            .expect("connect command should send");
        wait_for(handle, |snapshot| {
            snapshot.connection.status == ConnectionStatus::Connected
        })
        .await;
    }

    #[tokio::test]
    async fn connect_loads_only_databases() {
        let temp_dir = TempDir::new().expect("temp dir should create");
        let (handle, counts) = test_application(&temp_dir, Vec::new(), false);
        connect(&handle).await;

        assert_eq!(counts.connects.load(Ordering::Relaxed), 1);
        assert_eq!(counts.databases.load(Ordering::Relaxed), 1);
        assert_eq!(counts.tables.load(Ordering::Relaxed), 0);
        assert_eq!(counts.columns.load(Ordering::Relaxed), 0);
        assert_eq!(handle.snapshot().schema.databases.len(), 2);
    }

    #[tokio::test]
    async fn schema_selection_loads_scopes_on_demand() {
        let temp_dir = TempDir::new().expect("temp dir should create");
        let (handle, counts) = test_application(&temp_dir, Vec::new(), false);
        connect(&handle).await;
        handle
            .command(AppCommand::SelectDatabase {
                database: "app".to_string(),
            })
            .await
            .expect("select database should send");
        wait_for(&handle, |snapshot| {
            !snapshot.schema.loading && !snapshot.schema.tables.is_empty()
        })
        .await;
        assert_eq!(counts.tables.load(Ordering::Relaxed), 1);
        assert_eq!(counts.columns.load(Ordering::Relaxed), 0);

        handle
            .command(AppCommand::SelectTable {
                database: "app".to_string(),
                table: "users".to_string(),
            })
            .await
            .expect("select table should send");
        let snapshot = wait_for(&handle, |snapshot| {
            !snapshot.schema.loading && !snapshot.schema.columns.is_empty()
        })
        .await;
        assert_eq!(counts.columns.load(Ordering::Relaxed), 1);
        assert_eq!(counts.relationships.load(Ordering::Relaxed), 1);
        assert_eq!(snapshot.schema.columns[0].name, "id");
    }

    #[tokio::test]
    async fn risky_sql_requires_confirmation_before_execution() {
        let temp_dir = TempDir::new().expect("temp dir should create");
        let (handle, counts) = test_application(&temp_dir, Vec::new(), false);
        connect(&handle).await;
        handle
            .command(AppCommand::ExecuteSql {
                sql: "DELETE FROM users".to_string(),
            })
            .await
            .expect("query command should send");
        let snapshot = wait_for(&handle, |snapshot| snapshot.query.confirmation.is_some()).await;
        assert_eq!(counts.queries.load(Ordering::Relaxed), 0);
        let operation_id = snapshot
            .query
            .confirmation
            .expect("confirmation should exist")
            .operation_id;
        handle
            .command(AppCommand::ConfirmSql { operation_id })
            .await
            .expect("confirmation should send");
        wait_for(&handle, |snapshot| {
            !snapshot.query.running && snapshot.query.confirmation.is_none()
        })
        .await;
        assert_eq!(counts.queries.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn read_only_profile_blocks_write_without_confirmation() {
        let temp_dir = TempDir::new().expect("temp dir should create");
        let (handle, counts) = test_application(&temp_dir, Vec::new(), true);
        connect(&handle).await;
        handle
            .command(AppCommand::ExecuteSql {
                sql: "UPDATE users SET active = 0".to_string(),
            })
            .await
            .expect("query command should send");
        let snapshot = wait_for(&handle, |snapshot| snapshot.last_error.is_some()).await;
        assert_eq!(
            snapshot.last_error.expect("error expected").kind,
            AppErrorKind::Query
        );
        assert_eq!(counts.queries.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn query_buffer_is_bounded_and_reports_truncation() {
        let temp_dir = TempDir::new().expect("temp dir should create");
        let rows = (0..2_050)
            .map(|index| QueryRow::from_values(vec![QueryValue::UInt(index)]))
            .collect();
        let (handle, _) = test_application(&temp_dir, rows, false);
        connect(&handle).await;
        handle
            .command(AppCommand::ExecuteSql {
                sql: "SELECT id FROM users".to_string(),
            })
            .await
            .expect("query command should send");
        let snapshot = wait_for(&handle, |snapshot| {
            !snapshot.query.running && snapshot.results.rows_seen == 2_050
        })
        .await;
        assert_eq!(snapshot.results.rows_buffered, RESULT_BUFFER_CAPACITY);
        assert!(snapshot.results.truncated);
        assert_eq!(snapshot.results.rows[0].values[0], QueryValue::UInt(50));
    }

    #[tokio::test]
    async fn stale_query_updates_are_ignored() {
        let temp_dir = TempDir::new().expect("temp dir should create");
        let (handle, _) = test_application(&temp_dir, Vec::new(), false);
        connect(&handle).await;
        handle
            .command(AppCommand::ExecuteSql {
                sql: "SELECT slow".to_string(),
            })
            .await
            .expect("slow query should send");
        wait_for(&handle, |snapshot| snapshot.query.running).await;
        handle
            .command(AppCommand::ExecuteSql {
                sql: "SELECT second".to_string(),
            })
            .await
            .expect("second query should send");
        let snapshot = wait_for(&handle, |snapshot| {
            !snapshot.query.running && snapshot.query.sql == "SELECT second"
        })
        .await;
        assert_eq!(snapshot.results.rows.len(), 1);
        assert_eq!(snapshot.results.rows[0].values[0], QueryValue::UInt(2));
    }

    #[tokio::test]
    async fn loaded_export_is_atomic_and_typed() {
        let temp_dir = TempDir::new().expect("temp dir should create");
        let rows = vec![QueryRow::from_values(vec![QueryValue::UInt(7)])];
        let (handle, _) = test_application(&temp_dir, rows, false);
        connect(&handle).await;
        handle
            .command(AppCommand::ExecuteSql {
                sql: "SELECT id FROM users".to_string(),
            })
            .await
            .expect("query should send");
        wait_for(&handle, |snapshot| {
            !snapshot.query.running && snapshot.results.rows_seen == 1
        })
        .await;

        let path = temp_dir.path().join("loaded.jsonl");
        handle
            .command(AppCommand::Export(ExportRequest {
                path: path.clone(),
                format: ExportFormat::JsonLines,
                scope: ExportScope::LoadedRows,
                typed_values: true,
                gzip: false,
            }))
            .await
            .expect("export should send");
        wait_for(&handle, |snapshot| {
            !snapshot.export.running && snapshot.export.rows_written == 1
        })
        .await;
        assert_eq!(
            fs::read_to_string(&path).expect("export should be readable"),
            "{\"id\":7}\n"
        );
        assert!(!partial_path(&path).exists());
    }

    #[tokio::test]
    async fn loaded_export_requires_a_result_set() {
        let temp_dir = TempDir::new().expect("temp dir should create");
        let (handle, _) = test_application(&temp_dir, Vec::new(), false);
        let path = temp_dir.path().join("empty.jsonl");

        handle
            .command(AppCommand::Export(ExportRequest {
                path: path.clone(),
                format: ExportFormat::JsonLines,
                scope: ExportScope::LoadedRows,
                typed_values: true,
                gzip: false,
            }))
            .await
            .expect("export should send");

        let snapshot = wait_for(&handle, |snapshot| snapshot.last_error.is_some()).await;
        assert_eq!(
            snapshot.last_error.expect("error expected").kind,
            AppErrorKind::Export
        );
        assert!(!path.exists());
        assert!(!partial_path(&path).exists());
    }

    #[tokio::test]
    async fn full_export_rejects_write_sql() {
        let temp_dir = TempDir::new().expect("temp dir should create");
        let (handle, _) = test_application(&temp_dir, Vec::new(), false);
        connect(&handle).await;
        handle
            .command(AppCommand::Export(ExportRequest {
                path: temp_dir.path().join("blocked.csv"),
                format: ExportFormat::Csv,
                scope: ExportScope::FullQuery {
                    sql: "DELETE FROM users".to_string(),
                },
                typed_values: true,
                gzip: false,
            }))
            .await
            .expect("export should send");
        let snapshot = wait_for(&handle, |snapshot| snapshot.last_error.is_some()).await;
        assert_eq!(
            snapshot.last_error.expect("error expected").kind,
            AppErrorKind::Export
        );
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn profile_commands_disconnect_and_schema_reload_share_one_actor() {
        let temp_dir = TempDir::new().expect("temp dir should create");
        let (handle, counts) = test_application(&temp_dir, Vec::new(), false);
        let mut events = handle.subscribe();

        handle
            .try_command(AppCommand::Connect {
                profile_name: "missing".to_string(),
            })
            .expect("command queue should accept a command");
        wait_for(&handle, |snapshot| snapshot.last_error.is_some()).await;
        handle
            .command(AppCommand::ClearError)
            .await
            .expect("clear error should send");
        wait_for(&handle, |snapshot| snapshot.last_error.is_none()).await;

        connect(&handle).await;
        handle
            .command(AppCommand::ReloadSchema {
                scope: SchemaScope::Databases,
            })
            .await
            .expect("database reload should send");
        wait_for(&handle, |snapshot| {
            snapshot.connection.status == ConnectionStatus::Connected
                && counts.connects.load(Ordering::Relaxed) == 2
        })
        .await;
        handle
            .command(AppCommand::ReloadSchema {
                scope: SchemaScope::Tables {
                    database: "app".to_string(),
                },
            })
            .await
            .expect("table reload should send");
        wait_for(&handle, |snapshot| !snapshot.schema.tables.is_empty()).await;
        handle
            .command(AppCommand::ReloadSchema {
                scope: SchemaScope::Table {
                    database: "app".to_string(),
                    table: "users".to_string(),
                },
            })
            .await
            .expect("column reload should send");
        wait_for(&handle, |snapshot| !snapshot.schema.columns.is_empty()).await;

        let mut profile = ConnectionProfile::new("team", "db.internal", "reader");
        profile.is_default = true;
        profile.quick_reconnect = true;
        handle
            .command(AppCommand::UpsertProfile { profile })
            .await
            .expect("profile upsert should send");
        wait_for(&handle, |snapshot| {
            snapshot
                .profiles
                .iter()
                .any(|profile| profile.name == "team")
        })
        .await;
        handle
            .command(AppCommand::SetDefaultProfile {
                profile_name: "local".to_string(),
            })
            .await
            .expect("default marker should send");
        handle
            .command(AppCommand::SetQuickReconnectProfile {
                profile_name: "local".to_string(),
            })
            .await
            .expect("quick marker should send");
        wait_for(&handle, |snapshot| {
            snapshot.profiles.iter().any(|profile| {
                profile.name == "local" && profile.is_default && profile.quick_reconnect
            })
        })
        .await;
        handle
            .command(AppCommand::DeleteProfile {
                profile_name: "team".to_string(),
            })
            .await
            .expect("profile delete should send");
        wait_for(&handle, |snapshot| {
            !snapshot
                .profiles
                .iter()
                .any(|profile| profile.name == "team")
        })
        .await;

        handle
            .command(AppCommand::Disconnect)
            .await
            .expect("disconnect should send");
        wait_for(&handle, |snapshot| {
            snapshot.connection.status == ConnectionStatus::Disconnected
        })
        .await;
        handle
            .command(AppCommand::Disconnect)
            .await
            .expect("idempotent disconnect should send");
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(matches!(
            events.try_recv(),
            Ok(AppEvent::SnapshotChanged(_) | AppEvent::Finished { .. })
        ));
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn invalid_commands_confirmations_and_cancellation_are_typed() {
        let temp_dir = TempDir::new().expect("temp dir should create");
        let (handle, _) = test_application(&temp_dir, Vec::new(), false);

        handle
            .command(AppCommand::SelectDatabase {
                database: "app".to_string(),
            })
            .await
            .expect("select database should send");
        wait_for(&handle, |snapshot| snapshot.last_error.is_some()).await;
        handle
            .command(AppCommand::ClearError)
            .await
            .expect("clear should send");
        handle
            .command(AppCommand::SelectTable {
                database: "app".to_string(),
                table: "users".to_string(),
            })
            .await
            .expect("select table should send");
        wait_for(&handle, |snapshot| snapshot.last_error.is_some()).await;
        handle
            .command(AppCommand::ClearError)
            .await
            .expect("clear should send");
        handle
            .command(AppCommand::ExecuteSql {
                sql: "  ".to_string(),
            })
            .await
            .expect("empty SQL should send");
        wait_for(&handle, |snapshot| snapshot.last_error.is_some()).await;
        handle
            .command(AppCommand::ClearError)
            .await
            .expect("clear should send");
        handle
            .command(AppCommand::ConfirmSql {
                operation_id: OperationId(999),
            })
            .await
            .expect("confirmation should send");
        wait_for(&handle, |snapshot| snapshot.last_error.is_some()).await;

        connect(&handle).await;
        handle
            .command(AppCommand::ExecuteSql {
                sql: "DELETE FROM users".to_string(),
            })
            .await
            .expect("risky SQL should send");
        let pending = wait_for(&handle, |snapshot| snapshot.query.confirmation.is_some()).await;
        let operation_id = pending
            .query
            .confirmation
            .expect("confirmation should exist")
            .operation_id;
        handle
            .command(AppCommand::ConfirmSql {
                operation_id: OperationId(operation_id.0 + 1),
            })
            .await
            .expect("stale confirmation should send");
        wait_for(&handle, |snapshot| {
            snapshot
                .last_error
                .as_ref()
                .is_some_and(|error| error.message.contains("stale operation"))
        })
        .await;
        handle
            .command(AppCommand::ClearError)
            .await
            .expect("clear should send");
        handle
            .command(AppCommand::ConfirmSql { operation_id })
            .await
            .expect("valid confirmation should send");
        wait_for(&handle, |snapshot| !snapshot.query.running).await;

        handle
            .command(AppCommand::ExecuteSql {
                sql: "SELECT slow".to_string(),
            })
            .await
            .expect("slow query should send");
        let running = wait_for(&handle, |snapshot| snapshot.query.running).await;
        let active_id = running.query.operation_id.expect("operation should exist");
        handle
            .command(AppCommand::Cancel {
                operation_id: Some(OperationId(active_id.0 + 1)),
            })
            .await
            .expect("unmatched cancellation should send");
        handle
            .command(AppCommand::Cancel { operation_id: None })
            .await
            .expect("cancellation should send");
        let cancelled = wait_for(&handle, |snapshot| {
            snapshot
                .last_error
                .as_ref()
                .is_some_and(|error| error.kind == AppErrorKind::Cancellation)
        })
        .await;
        assert!(!cancelled.query.running);
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn loaded_and_full_exports_cover_formats_and_remove_cancelled_partials() {
        let temp_dir = TempDir::new().expect("temp dir should create");
        let rows = vec![
            QueryRow::from_values(vec![QueryValue::Text("a,b".to_string())]),
            QueryRow::from_values(vec![QueryValue::Null]),
        ];
        let (handle, _) = test_application(&temp_dir, rows, false);

        let disconnected_path = temp_dir.path().join("disconnected.csv");
        handle
            .command(AppCommand::Export(ExportRequest {
                path: disconnected_path,
                format: ExportFormat::Csv,
                scope: ExportScope::FullQuery {
                    sql: "SELECT 1".to_string(),
                },
                typed_values: true,
                gzip: false,
            }))
            .await
            .expect("disconnected export should send");
        wait_for(&handle, |snapshot| snapshot.last_error.is_some()).await;
        handle
            .command(AppCommand::ClearError)
            .await
            .expect("clear should send");
        connect(&handle).await;
        handle
            .command(AppCommand::ExecuteSql {
                sql: "SELECT value FROM sample".to_string(),
            })
            .await
            .expect("query should send");
        wait_for(&handle, |snapshot| snapshot.results.rows_seen == 2).await;

        let csv_path = temp_dir.path().join("loaded.csv");
        handle
            .command(AppCommand::Export(ExportRequest {
                path: csv_path.clone(),
                format: ExportFormat::Csv,
                scope: ExportScope::LoadedRows,
                typed_values: true,
                gzip: false,
            }))
            .await
            .expect("CSV export should send");
        wait_for(&handle, |snapshot| {
            !snapshot.export.running && snapshot.export.rows_written == 2
        })
        .await;
        assert_eq!(
            fs::read_to_string(&csv_path).expect("CSV should read"),
            "id\n\"a,b\"\nNULL\n"
        );

        let json_path = temp_dir.path().join("loaded.json");
        handle
            .command(AppCommand::Export(ExportRequest {
                path: json_path.clone(),
                format: ExportFormat::Json,
                scope: ExportScope::LoadedRows,
                typed_values: true,
                gzip: false,
            }))
            .await
            .expect("JSON export should send");
        wait_for(&handle, |snapshot| {
            !snapshot.export.running
                && snapshot.export.destination.as_ref() == Some(&json_path)
                && snapshot.export.rows_written == 2
        })
        .await;
        assert_eq!(
            fs::read_to_string(&json_path).expect("JSON should read"),
            "[{\"id\":\"a,b\"},{\"id\":null}]\n"
        );

        let legacy_path = temp_dir.path().join("legacy.jsonl.gz");
        handle
            .command(AppCommand::Export(ExportRequest {
                path: legacy_path.clone(),
                format: ExportFormat::JsonLines,
                scope: ExportScope::LoadedRows,
                typed_values: false,
                gzip: true,
            }))
            .await
            .expect("legacy gzip export should send");
        wait_for(&handle, |snapshot| {
            !snapshot.export.running
                && snapshot.export.destination.as_ref() == Some(&legacy_path)
                && snapshot.export.rows_written == 2
        })
        .await;
        let mut legacy = String::new();
        GzDecoder::new(File::open(&legacy_path).expect("gzip export should open"))
            .read_to_string(&mut legacy)
            .expect("gzip export should decode");
        assert_eq!(legacy, "{\"id\":\"a,b\"}\n{\"id\":\"NULL\"}\n");

        let full_path = temp_dir.path().join("full.jsonl");
        handle
            .command(AppCommand::Export(ExportRequest {
                path: full_path.clone(),
                format: ExportFormat::JsonLines,
                scope: ExportScope::FullQuery {
                    sql: "SELECT value FROM sample".to_string(),
                },
                typed_values: true,
                gzip: false,
            }))
            .await
            .expect("full export should send");
        wait_for(&handle, |snapshot| {
            !snapshot.export.running
                && snapshot.export.destination.as_ref() == Some(&full_path)
                && snapshot.export.rows_written == 2
        })
        .await;
        assert!(full_path.exists());

        let cancelled_path = temp_dir.path().join("cancelled.jsonl");
        handle
            .command(AppCommand::Export(ExportRequest {
                path: cancelled_path.clone(),
                format: ExportFormat::JsonLines,
                scope: ExportScope::FullQuery {
                    sql: "SELECT slow".to_string(),
                },
                typed_values: true,
                gzip: false,
            }))
            .await
            .expect("slow export should send");
        wait_for(&handle, |snapshot| snapshot.export.running).await;
        handle
            .command(AppCommand::Cancel { operation_id: None })
            .await
            .expect("export cancellation should send");
        wait_for(&handle, |snapshot| {
            !snapshot.export.running
                && snapshot
                    .last_error
                    .as_ref()
                    .is_some_and(|error| error.kind == AppErrorKind::Cancellation)
        })
        .await;
        assert!(!cancelled_path.exists());
        assert!(!partial_path(&cancelled_path).exists());
    }

    #[test]
    fn application_helpers_keep_errors_identifiers_and_csv_explicit() {
        assert_eq!(crate::application_name(), "myr-application");
        let backend = ApplicationBackendError::new(AppErrorKind::Timeout, "too slow");
        let mapped = app_backend_error(backend);
        assert_eq!(mapped.kind, AppErrorKind::Timeout);
        assert!(mapped.retryable);
        assert_eq!(quote_identifier("odd`name"), "`odd``name`");
        assert_eq!(csv_escape("plain"), "plain");
        assert_eq!(csv_escape("a,\"b\""), "\"a,\"\"b\"\"\"");
        assert_eq!(file_size(Path::new("/definitely/missing/myr-file")), 0);
    }

    #[tokio::test]
    async fn transient_queries_retry_and_reconnect_with_a_single_operation() {
        let retry_dir = TempDir::new().expect("temp dir should create");
        let retry_rows = vec![QueryRow::from_values(vec![QueryValue::UInt(1)])];
        let (retry_handle, retry_counts) = test_application(&retry_dir, retry_rows, false);
        connect(&retry_handle).await;
        retry_handle
            .command(AppCommand::ExecuteSql {
                sql: "SELECT retry-once".to_string(),
            })
            .await
            .expect("retryable query should send");
        let retried = wait_for(&retry_handle, |snapshot| {
            !snapshot.query.running && snapshot.results.rows_seen == 1
        })
        .await;
        assert!(retried.last_error.is_none());
        assert_eq!(retry_counts.queries.load(Ordering::Relaxed), 2);

        let reconnect_dir = TempDir::new().expect("temp dir should create");
        let reconnect_rows = vec![QueryRow::from_values(vec![QueryValue::UInt(2)])];
        let (reconnect_handle, reconnect_counts) =
            test_application(&reconnect_dir, reconnect_rows, false);
        connect(&reconnect_handle).await;
        reconnect_handle
            .command(AppCommand::ExecuteSql {
                sql: "SELECT reconnect".to_string(),
            })
            .await
            .expect("reconnecting query should send");
        let reconnected = wait_for(&reconnect_handle, |snapshot| {
            !snapshot.query.running && snapshot.results.rows_seen == 1
        })
        .await;
        assert_eq!(reconnected.connection.status, ConnectionStatus::Connected);
        assert!(reconnected.last_error.is_none());
        assert_eq!(reconnect_counts.queries.load(Ordering::Relaxed), 3);
        assert_eq!(reconnect_counts.connects.load(Ordering::Relaxed), 2);

        let audit = fs::read_to_string(reconnect_dir.path().join("audit.ndjson"))
            .expect("shared application should write the audit trail");
        let records = audit
            .lines()
            .map(|line| serde_json::from_str::<AuditRecord>(line).expect("valid audit record"))
            .collect::<Vec<_>>();
        assert_eq!(
            records.first().map(|record| &record.outcome),
            Some(&AuditOutcome::Started)
        );
        assert_eq!(
            records.last().map(|record| &record.outcome),
            Some(&AuditOutcome::Succeeded)
        );
        assert!(records
            .iter()
            .all(|record| record.profile_name.as_deref() == Some("local")));
    }

    #[tokio::test]
    async fn full_export_streams_one_million_rows_without_a_partial_left_behind() {
        let temp_dir = TempDir::new().expect("temp dir should create");
        let (handle, _) = test_application(&temp_dir, Vec::new(), false);
        connect(&handle).await;
        let path = temp_dir.path().join("million.jsonl");
        handle
            .command(AppCommand::Export(ExportRequest {
                path: path.clone(),
                format: ExportFormat::JsonLines,
                scope: ExportScope::FullQuery {
                    sql: "SELECT million".to_string(),
                },
                typed_values: true,
                gzip: false,
            }))
            .await
            .expect("million-row export should send");
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let snapshot = handle.snapshot();
                if !snapshot.export.running && snapshot.export.rows_written == 1_000_000 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("million-row export should finish");

        let file = File::open(&path).expect("final export should exist");
        assert_eq!(BufReader::new(file).lines().count(), 1_000_000);
        assert!(!partial_path(&path).exists());
    }
}
