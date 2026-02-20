use std::io::{self, Stdout};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use myr_adapters::export::{export_rows_to_csv, export_rows_to_json};
use myr_adapters::mysql::{MysqlConnectionBackend, MysqlDataBackend};
use myr_core::actions_engine::{
    ActionContext, ActionId, ActionInvocation, ActionsEngine, AppView, SchemaSelection,
};
use myr_core::connection_manager::ConnectionManager;
use myr_core::profiles::{ConnectionProfile, FileProfilesStore};
use myr_core::query_runner::{CancellationToken, QueryRow, QueryRunner};
use myr_core::results_buffer::ResultsRingBuffer;
use myr_core::safe_mode::{ConfirmationToken, GuardDecision, SafeModeGuard};
use myr_core::schema_cache::SchemaCacheService;
use myr_core::sql_generator::{
    keyset_first_page_sql, keyset_page_sql, offset_page_sql, PaginationDirection, SqlTarget,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Tabs};
use ratatui::{Frame, Terminal};
use thiserror::Error;

const TICK_RATE: Duration = Duration::from_millis(120);
const QUERY_DURATION_TICKS: u8 = 10;
const FOOTER_ACTIONS_LIMIT: usize = 7;
const RESULT_BUFFER_CAPACITY: usize = 2_000;
const PREVIEW_PAGE_SIZE: usize = 200;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const QUERY_TIMEOUT: Duration = Duration::from_secs(20);
const QUERY_RETRY_LIMIT: u8 = 1;
const AUTO_RECONNECT_LIMIT: u8 = 2;
const PANE_FLASH_DURATION_TICKS: u8 = 8;

const DEMO_SCHEMA_TABLES: [&str; 4] = ["users", "sessions", "playlists", "events"];
const DEMO_SCHEMA_COLUMNS: [&str; 4] = ["id", "email", "created_at", "updated_at"];

#[derive(Debug, Error)]
pub enum TuiError {
    #[error("i/o error: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pane {
    ConnectionWizard,
    SchemaExplorer,
    Results,
    QueryEditor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SchemaLane {
    Databases,
    Tables,
    Columns,
}

impl SchemaLane {
    fn next(self) -> Self {
        match self {
            Self::Databases => Self::Tables,
            Self::Tables => Self::Columns,
            Self::Columns => Self::Databases,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Databases => Self::Columns,
            Self::Tables => Self::Databases,
            Self::Columns => Self::Tables,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Databases => "Databases",
            Self::Tables => "Tables",
            Self::Columns => "Columns",
        }
    }
}

impl Pane {
    fn next(self) -> Self {
        match self {
            Self::ConnectionWizard => Self::SchemaExplorer,
            Self::SchemaExplorer => Self::Results,
            Self::Results => Self::QueryEditor,
            Self::QueryEditor => Self::SchemaExplorer,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WizardField {
    ProfileName,
    Host,
    Port,
    User,
    Database,
}

impl WizardField {
    fn next(self) -> Self {
        match self {
            Self::ProfileName => Self::Host,
            Self::Host => Self::Port,
            Self::Port => Self::User,
            Self::User => Self::Database,
            Self::Database => Self::ProfileName,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::ProfileName => "Profile",
            Self::Host => "Host",
            Self::Port => "Port",
            Self::User => "User",
            Self::Database => "Database",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConnectionWizardForm {
    profile_name: String,
    host: String,
    port: String,
    user: String,
    database: String,
    active_field: WizardField,
    editing: bool,
    edit_buffer: String,
}

impl Default for ConnectionWizardForm {
    fn default() -> Self {
        Self {
            profile_name: "local-dev".to_string(),
            host: "127.0.0.1".to_string(),
            port: "3306".to_string(),
            user: "root".to_string(),
            database: "app".to_string(),
            active_field: WizardField::ProfileName,
            editing: false,
            edit_buffer: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectionKey {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Msg {
    Quit,
    GoConnectionWizard,
    ToggleHelp,
    NextPane,
    TogglePalette,
    TogglePerfOverlay,
    ToggleSafeMode,
    Submit,
    CancelQuery,
    Navigate(DirectionKey),
    InvokeActionSlot(usize),
    InputChar(char),
    Backspace,
    ClearInput,
    Connect,
    Tick,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectIntent {
    Manual,
    AutoReconnect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ErrorKind {
    Connection,
    Query,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ErrorPanel {
    kind: ErrorKind,
    title: String,
    summary: String,
    detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PageTransition {
    Reset,
    Next,
    Previous,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PaginationPlan {
    Keyset {
        key_column: String,
        first_key: Option<String>,
        last_key: Option<String>,
    },
    Offset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PaginationState {
    database: Option<String>,
    table: String,
    page_size: usize,
    page_index: usize,
    last_page_row_count: usize,
    plan: PaginationPlan,
}

#[derive(Debug)]
enum ConnectWorkerOutcome {
    Success {
        profile: ConnectionProfile,
        connect_latency: Duration,
        databases: Vec<String>,
        warning: Option<String>,
    },
    Failure(String),
}

#[derive(Debug)]
enum QueryWorkerOutcome {
    Success {
        results: ResultsRingBuffer<QueryRow>,
        rows_streamed: u64,
        was_cancelled: bool,
        elapsed: Duration,
    },
    Failure(String),
}

struct TuiApp {
    actions: ActionsEngine,
    pane: Pane,
    wizard_form: ConnectionWizardForm,
    connected_profile: Option<String>,
    last_connection_latency: Option<Duration>,
    data_backend: Option<MysqlDataBackend>,
    schema_cache: Option<SchemaCacheService<MysqlDataBackend>>,
    schema_databases: Vec<String>,
    selected_database_index: usize,
    active_database: Option<String>,
    schema_tables: Vec<String>,
    selected_table_index: usize,
    schema_columns: Vec<String>,
    selected_column_index: usize,
    schema_lane: SchemaLane,
    show_help: bool,
    show_palette: bool,
    palette_query: String,
    palette_selection: usize,
    show_perf_overlay: bool,
    last_render_ms: f64,
    recent_render_total_ms: f64,
    recent_render_count: u32,
    fps: f64,
    fps_window_started_at: Instant,
    should_quit: bool,
    query_running: bool,
    query_ticks_remaining: u8,
    safe_mode_guard: SafeModeGuard,
    pending_confirmation: Option<(ConfirmationToken, String)>,
    has_results: bool,
    result_columns: Vec<String>,
    results_cursor: usize,
    results_search_mode: bool,
    results_search_query: String,
    results: ResultsRingBuffer<QueryRow>,
    pagination_state: Option<PaginationState>,
    pending_page_transition: Option<PageTransition>,
    cancel_requested: bool,
    connect_requested: bool,
    connect_intent: ConnectIntent,
    connect_result_rx: Option<Receiver<ConnectWorkerOutcome>>,
    query_result_rx: Option<Receiver<QueryWorkerOutcome>>,
    query_cancellation: Option<CancellationToken>,
    active_connection_profile: Option<ConnectionProfile>,
    last_connect_profile: Option<ConnectionProfile>,
    pending_retry_query: Option<String>,
    reconnect_attempts: u8,
    query_retry_attempts: u8,
    inflight_query_sql: Option<String>,
    last_failed_query: Option<String>,
    error_panel: Option<ErrorPanel>,
    loading_tick: usize,
    pane_flash_ticks: u8,
    exit_confirmation: bool,
    status_line: String,
    query_editor_text: String,
    selection: SchemaSelection,
}

impl Default for TuiApp {
    fn default() -> Self {
        Self {
            actions: ActionsEngine::new(),
            pane: Pane::ConnectionWizard,
            wizard_form: ConnectionWizardForm::default(),
            connected_profile: None,
            last_connection_latency: None,
            data_backend: None,
            schema_cache: None,
            schema_databases: vec!["app".to_string()],
            selected_database_index: 0,
            active_database: Some("app".to_string()),
            schema_tables: DEMO_SCHEMA_TABLES
                .iter()
                .map(|table| (*table).to_string())
                .collect(),
            selected_table_index: 0,
            schema_columns: DEMO_SCHEMA_COLUMNS
                .iter()
                .map(|column| (*column).to_string())
                .collect(),
            selected_column_index: 0,
            schema_lane: SchemaLane::Tables,
            show_help: false,
            show_palette: false,
            palette_query: String::new(),
            palette_selection: 0,
            show_perf_overlay: false,
            last_render_ms: 0.0,
            recent_render_total_ms: 0.0,
            recent_render_count: 0,
            fps: 0.0,
            fps_window_started_at: Instant::now(),
            should_quit: false,
            query_running: false,
            query_ticks_remaining: 0,
            safe_mode_guard: SafeModeGuard::new(true),
            pending_confirmation: None,
            has_results: false,
            result_columns: vec![
                "id".to_string(),
                "value".to_string(),
                "observed_at".to_string(),
            ],
            results_cursor: 0,
            results_search_mode: false,
            results_search_query: String::new(),
            results: ResultsRingBuffer::new(RESULT_BUFFER_CAPACITY),
            pagination_state: None,
            pending_page_transition: None,
            cancel_requested: false,
            connect_requested: false,
            connect_intent: ConnectIntent::Manual,
            connect_result_rx: None,
            query_result_rx: None,
            query_cancellation: None,
            active_connection_profile: None,
            last_connect_profile: None,
            pending_retry_query: None,
            reconnect_attempts: 0,
            query_retry_attempts: 0,
            inflight_query_sql: None,
            last_failed_query: None,
            error_panel: None,
            loading_tick: 0,
            pane_flash_ticks: 0,
            exit_confirmation: false,
            status_line: "Select a field with Up/Down, press E to edit, F5 to connect".to_string(),
            query_editor_text: "SELECT * FROM `users`".to_string(),
            selection: SchemaSelection {
                database: Some("app".to_string()),
                table: Some("users".to_string()),
                column: Some("id".to_string()),
            },
        }
    }
}

impl TuiApp {
    fn handle(&mut self, msg: Msg) {
        if self.exit_confirmation
            && !matches!(msg, Msg::Quit | Msg::TogglePalette | Msg::Tick | Msg::CancelQuery)
        {
            self.status_line =
                "Exit pending. Press Ctrl+C to confirm, F10 to exit now, Esc to cancel."
                    .to_string();
            return;
        }

        if self.error_panel.is_some() {
            self.handle_error_panel_input(msg);
            return;
        }

        if self.results_search_mode {
            match msg {
                Msg::InputChar(ch) => {
                    self.results_search_query.push(ch);
                    self.apply_results_search(false);
                    return;
                }
                Msg::Backspace => {
                    self.results_search_query.pop();
                    self.apply_results_search(false);
                    return;
                }
                Msg::ClearInput => {
                    self.results_search_query.clear();
                    self.apply_results_search(false);
                    return;
                }
                Msg::Submit => {
                    self.apply_results_search(true);
                    return;
                }
                Msg::TogglePalette => {
                    self.results_search_mode = false;
                    self.status_line = "Results search canceled".to_string();
                    return;
                }
                _ => {}
            }
        }

        match msg {
            Msg::Quit => {
                self.should_quit = true;
            }
            Msg::GoConnectionWizard => {
                self.set_active_pane(Pane::ConnectionWizard);
                if self.wizard_form.editing {
                    self.cancel_wizard_edit();
                } else {
                    self.status_line = "Returned to Connection Wizard".to_string();
                }
            }
            Msg::ToggleHelp => self.show_help = !self.show_help,
            Msg::NextPane => {
                if self.pane == Pane::ConnectionWizard && self.wizard_form.editing {
                    self.status_line =
                        "Finish editing field first (Enter to save, Esc to cancel)".to_string();
                } else {
                    let next_pane = self.pane.next();
                    self.set_active_pane(next_pane);
                    self.status_line = format!("Switched pane to {}", self.pane_name());
                }
            }
            Msg::TogglePalette => {
                if self.exit_confirmation {
                    self.exit_confirmation = false;
                    self.status_line = "Exit canceled".to_string();
                    return;
                }
                if self.show_help {
                    self.show_help = false;
                    self.status_line = "Help closed".to_string();
                    return;
                }
                if self.pane == Pane::ConnectionWizard && self.wizard_form.editing {
                    self.cancel_wizard_edit();
                    return;
                }
                self.show_palette = !self.show_palette;
                if self.show_palette {
                    self.palette_query.clear();
                    self.palette_selection = 0;
                }
                self.status_line = if self.show_palette {
                    "Command palette opened".to_string()
                } else {
                    "Command palette closed".to_string()
                };
            }
            Msg::TogglePerfOverlay => {
                self.show_perf_overlay = !self.show_perf_overlay;
                self.status_line = if self.show_perf_overlay {
                    "Perf overlay enabled".to_string()
                } else {
                    "Perf overlay disabled".to_string()
                };
            }
            Msg::ToggleSafeMode => {
                let next_enabled = !self.safe_mode_guard.is_enabled();
                self.safe_mode_guard.set_enabled(next_enabled);
                self.pending_confirmation = None;
                self.status_line = if next_enabled {
                    "Safe mode enabled".to_string()
                } else {
                    "Safe mode disabled".to_string()
                };
            }
            Msg::Submit => self.submit(),
            Msg::Connect => self.connect(),
            Msg::CancelQuery => {
                if !self.query_running {
                    if self.exit_confirmation {
                        self.should_quit = true;
                    } else {
                        self.exit_confirmation = true;
                        self.status_line = "No active query. Exit myr? Press Ctrl+C again to confirm, F10 to exit now, Esc to cancel.".to_string();
                    }
                    return;
                }

                self.cancel_requested = true;
                if let Some(cancellation) = &self.query_cancellation {
                    cancellation.cancel();
                    self.status_line = "Cancelling query...".to_string();
                } else {
                    self.query_running = false;
                    self.query_ticks_remaining = 0;
                    self.status_line = "Cancel requested".to_string();
                }
            }
            Msg::Navigate(direction) => self.navigate(direction),
            Msg::InvokeActionSlot(index) => self.invoke_ranked_action(index),
            Msg::InputChar(ch) => self.handle_input_char(ch),
            Msg::Backspace => self.handle_backspace(),
            Msg::ClearInput => self.handle_clear_input(),
            Msg::Tick => self.on_tick(),
        }
    }

    fn handle_error_panel_input(&mut self, msg: Msg) {
        match msg {
            Msg::Tick => self.on_tick(),
            Msg::TogglePalette => {
                self.error_panel = None;
                self.status_line = "Error panel dismissed".to_string();
            }
            Msg::InvokeActionSlot(0) | Msg::Submit => {
                self.run_primary_error_action();
            }
            Msg::Connect => {
                self.reconnect_from_error_panel();
            }
            Msg::GoConnectionWizard => {
                self.error_panel = None;
                self.set_active_pane(Pane::ConnectionWizard);
                self.status_line = "Returned to Connection Wizard".to_string();
            }
            Msg::Quit => {
                self.should_quit = true;
            }
            _ => {
                self.status_line =
                    "Error panel active: 1 primary action | F5 reconnect | F6 wizard | Esc close"
                        .to_string();
            }
        }
    }

    fn on_tick(&mut self) {
        self.loading_tick = self.loading_tick.wrapping_add(1);
        self.pane_flash_ticks = self.pane_flash_ticks.saturating_sub(1);
        self.poll_connect_result();
        self.poll_query_result();

        if self.query_running && self.data_backend.is_none() {
            if self.query_ticks_remaining == 0 {
                self.query_running = false;
                self.populate_demo_results();
                self.query_retry_attempts = 0;
                self.inflight_query_sql = None;
                self.last_failed_query = None;
                self.finalize_pagination_after_query();
                self.status_line = "Query completed".to_string();
            } else {
                self.query_ticks_remaining = self.query_ticks_remaining.saturating_sub(1);
            }
        }

        if self.connect_requested {
            let spinner = spinner_char(self.loading_tick);
            self.status_line = if self.connect_intent == ConnectIntent::AutoReconnect {
                format!("Reconnecting... {spinner}")
            } else {
                format!("Connecting... {spinner}")
            };
        } else if self.query_running && self.query_result_rx.is_some() {
            let spinner = spinner_char(self.loading_tick);
            self.status_line = if self.cancel_requested {
                format!("Cancelling query... {spinner}")
            } else {
                format!("Running query... {spinner}")
            };
        }
    }

    fn record_render(&mut self, elapsed: Duration) {
        self.last_render_ms = elapsed.as_secs_f64() * 1_000.0;
        self.recent_render_total_ms += self.last_render_ms;
        self.recent_render_count = self.recent_render_count.saturating_add(1);

        let window_elapsed = self.fps_window_started_at.elapsed();
        if window_elapsed >= Duration::from_secs(1) {
            self.fps = f64::from(self.recent_render_count) / window_elapsed.as_secs_f64();
            self.recent_render_total_ms = 0.0;
            self.recent_render_count = 0;
            self.fps_window_started_at = Instant::now();
        }
    }

    fn submit(&mut self) {
        if self.show_palette {
            if let Some(action_id) = self.selected_palette_action() {
                self.invoke_action(action_id);
                self.show_palette = false;
            } else {
                self.status_line = "No matching palette action".to_string();
            }
            return;
        }

        match self.pane {
            Pane::ConnectionWizard => {
                if self.wizard_form.editing {
                    self.commit_wizard_edit();
                } else {
                    self.start_wizard_edit();
                }
            }
            Pane::QueryEditor => {
                if let Some((token, sql)) = self.pending_confirmation.take() {
                    match self.safe_mode_guard.confirm(&token, &sql) {
                        Ok(()) => {
                            self.start_query(sql);
                        }
                        Err(error) => {
                            self.status_line = format!("Confirmation failed: {error}");
                        }
                    }
                    return;
                }
                self.invoke_action(ActionId::RunCurrentQuery);
            }
            Pane::SchemaExplorer | Pane::Results => {
                self.status_line = "Nothing to submit in this view".to_string();
            }
        }
    }

    fn connect(&mut self) {
        if self.pane == Pane::ConnectionWizard {
            if self.wizard_form.editing {
                self.commit_wizard_edit();
            }
            if self.connect_requested {
                self.status_line = "Already connecting...".to_string();
            } else {
                self.connect_from_wizard();
            }
        } else {
            self.status_line = "Connect is only available in connection wizard".to_string();
        }
    }

    fn connect_from_wizard(&mut self) {
        let profile = match self.wizard_profile() {
            Ok(profile) => profile,
            Err(error) => {
                self.status_line = error;
                return;
            }
        };

        self.start_connect_with_profile(profile, ConnectIntent::Manual);
    }

    fn start_connect_with_profile(&mut self, profile: ConnectionProfile, intent: ConnectIntent) {
        if intent == ConnectIntent::Manual {
            self.reconnect_attempts = 0;
        }
        self.error_panel = None;
        let (tx, rx) = mpsc::channel();
        self.connect_result_rx = Some(rx);
        self.connect_requested = true;
        self.connect_intent = intent;
        self.last_connect_profile = Some(profile.clone());
        self.status_line = if intent == ConnectIntent::AutoReconnect {
            format!(
                "Auto-reconnect {}/{} for `{}`...",
                self.reconnect_attempts.max(1),
                AUTO_RECONNECT_LIMIT,
                profile.name
            )
        } else {
            format!(
                "Connecting to {}:{} as {}...",
                profile.host, profile.port, profile.user
            )
        };

        let _connect_worker = thread::spawn(move || {
            let _ = tx.send(run_connect_worker(profile));
        });
    }

    fn wizard_profile(&self) -> Result<ConnectionProfile, String> {
        let port = self
            .wizard_form
            .port
            .parse::<u16>()
            .map_err(|_| "Invalid port in connection wizard".to_string())?;

        let mut profile = ConnectionProfile::new(
            self.wizard_form.profile_name.clone(),
            self.wizard_form.host.clone(),
            self.wizard_form.user.clone(),
        );
        profile.port = port;
        profile.database = if self.wizard_form.database.trim().is_empty() {
            None
        } else {
            Some(self.wizard_form.database.clone())
        };
        Ok(profile)
    }

    fn poll_connect_result(&mut self) {
        let outcome = match self.connect_result_rx.as_ref() {
            Some(receiver) => match receiver.try_recv() {
                Ok(outcome) => Some(outcome),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => {
                    Some(ConnectWorkerOutcome::Failure("connect worker disconnected".to_string()))
                }
            },
            None => None,
        };

        let Some(outcome) = outcome else {
            return;
        };

        let intent = self.connect_intent;
        self.connect_result_rx = None;
        self.connect_requested = false;
        self.connect_intent = ConnectIntent::Manual;

        match outcome {
            ConnectWorkerOutcome::Success {
                profile,
                connect_latency,
                databases,
                warning,
            } => {
                self.reconnect_attempts = 0;
                self.apply_connected_profile(profile, connect_latency, databases, warning);
                self.error_panel = None;
                if intent == ConnectIntent::AutoReconnect {
                    if let Some(sql) = self.pending_retry_query.take() {
                        self.start_query(sql);
                    } else {
                        self.status_line = "Auto-reconnect succeeded".to_string();
                    }
                }
            }
            ConnectWorkerOutcome::Failure(error) => {
                if intent == ConnectIntent::AutoReconnect
                    && self.reconnect_attempts < AUTO_RECONNECT_LIMIT
                {
                    if let Some(profile) = self
                        .active_connection_profile
                        .clone()
                        .or_else(|| self.last_connect_profile.clone())
                    {
                        self.reconnect_attempts = self.reconnect_attempts.saturating_add(1);
                        self.start_connect_with_profile(profile, ConnectIntent::AutoReconnect);
                        return;
                    }
                }

                self.pending_retry_query = None;
                self.reconnect_attempts = 0;
                self.status_line = format!("Connect failed: {error}");
                let summary = if intent == ConnectIntent::AutoReconnect {
                    "Auto-reconnect attempts were exhausted".to_string()
                } else {
                    "Connection attempt failed".to_string()
                };
                self.open_error_panel(ErrorKind::Connection, "Connection Error", summary, error);
            }
        }
    }

    fn apply_connected_profile(
        &mut self,
        profile: ConnectionProfile,
        connect_latency: Duration,
        databases: Vec<String>,
        warning: Option<String>,
    ) {
        self.last_connection_latency = Some(connect_latency);

        // Keep query execution and schema cache on separate pools so runtime-bound
        // schema refreshes cannot invalidate the active query pool.
        let data_backend = MysqlDataBackend::from_profile(&profile);
        let schema_backend = MysqlDataBackend::from_profile(&profile);
        let schema_cache = SchemaCacheService::new(schema_backend, Duration::from_secs(10));

        let mut active_database = profile.database.clone();
        if active_database.is_none() {
            active_database = databases.first().cloned();
        }

        self.active_connection_profile = Some(profile.clone());
        self.last_connect_profile = Some(profile.clone());
        self.data_backend = Some(data_backend);
        self.schema_cache = Some(schema_cache);
        self.schema_databases = databases;
        self.selected_database_index = active_database
            .as_deref()
            .and_then(|database| {
                self.schema_databases
                    .iter()
                    .position(|candidate| candidate == database)
            })
            .unwrap_or(0);
        self.active_database = active_database.clone();
        self.connected_profile = Some(profile.name.clone());
        self.selection.database = active_database;
        self.schema_tables.clear();
        self.selected_table_index = 0;
        self.selection.table = None;
        self.schema_columns.clear();
        self.selected_column_index = 0;
        self.selection.column = None;
        self.reload_tables_for_active_database();
        self.schema_lane = if self.schema_tables.is_empty() {
            SchemaLane::Databases
        } else {
            SchemaLane::Tables
        };
        self.clear_pagination_state();
        self.set_query_editor_to_selected_table();
        self.set_active_pane(Pane::SchemaExplorer);

        let mut notes = Vec::new();
        if let Some(warning) = warning {
            notes.push(warning);
        }

        match FileProfilesStore::load_default() {
            Ok(mut store) => {
                store.upsert_profile(profile.clone());
                if let Err(error) = store.persist() {
                    notes.push(format!("profile save failed: {error}"));
                }
            }
            Err(error) => notes.push(format!("profile load failed: {error}")),
        }

        let mut status = format!("Connected as `{}` in {:.1?}", profile.name, connect_latency);
        if !notes.is_empty() {
            status.push_str(" (");
            status.push_str(&notes.join("; "));
            status.push(')');
        }
        self.status_line = status;
    }

    fn poll_query_result(&mut self) {
        let outcome = match self.query_result_rx.as_ref() {
            Some(receiver) => match receiver.try_recv() {
                Ok(outcome) => Some(outcome),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => {
                    Some(QueryWorkerOutcome::Failure("query worker disconnected".to_string()))
                }
            },
            None => None,
        };

        let Some(outcome) = outcome else {
            return;
        };

        self.query_result_rx = None;
        self.query_cancellation = None;
        self.query_running = false;

        match outcome {
            QueryWorkerOutcome::Success {
                results,
                rows_streamed,
                was_cancelled,
                elapsed,
            } => {
                self.results = results;
                self.has_results = !self.results.is_empty();
                self.results_cursor = 0;
                self.results_search_mode = false;
                self.results_search_query.clear();
                self.query_retry_attempts = 0;
                self.reconnect_attempts = 0;
                self.inflight_query_sql = None;
                self.last_failed_query = None;
                self.finalize_pagination_after_query();
                self.status_line = if was_cancelled {
                    format!("Query cancelled after {rows_streamed} rows in {elapsed:.1?}")
                } else {
                    format!("Query returned {rows_streamed} rows in {elapsed:.1?}")
                };
            }
            QueryWorkerOutcome::Failure(error) => {
                self.pending_page_transition = None;
                self.has_results = !self.results.is_empty();
                self.results_search_mode = false;
                self.results_search_query.clear();
                let query_sql = self
                    .inflight_query_sql
                    .clone()
                    .or_else(|| (!self.query_editor_text.trim().is_empty()).then(|| self.query_editor_text.clone()));
                let transient = is_transient_query_error(&error);
                let connection_loss = is_connection_lost_error(&error);

                if transient
                    && !self.cancel_requested
                    && self.query_retry_attempts < QUERY_RETRY_LIMIT
                {
                    if let Some(sql) = query_sql.clone() {
                        self.query_retry_attempts = self.query_retry_attempts.saturating_add(1);
                        self.status_line = format!(
                            "Transient query failure; retrying ({}/{})...",
                            self.query_retry_attempts,
                            QUERY_RETRY_LIMIT
                        );
                        self.start_query_internal(sql, true);
                        self.cancel_requested = false;
                        return;
                    }
                }

                if connection_loss
                    && !self.cancel_requested
                    && self.reconnect_attempts < AUTO_RECONNECT_LIMIT
                {
                    if let Some(profile) = self
                        .active_connection_profile
                        .clone()
                        .or_else(|| self.last_connect_profile.clone())
                    {
                        self.pending_retry_query = query_sql.clone();
                        self.reconnect_attempts = self.reconnect_attempts.saturating_add(1);
                        self.start_connect_with_profile(profile, ConnectIntent::AutoReconnect);
                        self.status_line = format!(
                            "Connection dropped; reconnecting ({}/{})...",
                            self.reconnect_attempts,
                            AUTO_RECONNECT_LIMIT
                        );
                        self.cancel_requested = false;
                        return;
                    }
                }

                self.query_retry_attempts = 0;
                self.reconnect_attempts = 0;
                self.inflight_query_sql = None;
                self.last_failed_query = query_sql;
                self.pending_retry_query = None;
                self.status_line = format!("Query failed: {error}");
                self.open_error_panel(
                    ErrorKind::Query,
                    "Query Error",
                    "Query execution failed".to_string(),
                    error,
                );
            }
        }

        self.cancel_requested = false;
    }

    fn navigate(&mut self, direction: DirectionKey) {
        if self.show_palette {
            self.navigate_palette(direction);
            return;
        }

        match self.pane {
            Pane::ConnectionWizard => {
                if self.wizard_form.editing {
                    self.status_line =
                        "Finish editing field first (Enter to save, Esc to cancel)".to_string();
                    return;
                }
                match direction {
                    DirectionKey::Up => {
                        self.wizard_form.active_field = self.previous_wizard_field();
                        self.status_line =
                            format!("Wizard field: {}", self.wizard_form.active_field.label());
                    }
                    DirectionKey::Down => {
                        self.wizard_form.active_field = self.wizard_form.active_field.next();
                        self.status_line =
                            format!("Wizard field: {}", self.wizard_form.active_field.label());
                    }
                    DirectionKey::Left | DirectionKey::Right => {
                        self.status_line = "Use Up/Down to select a wizard field".to_string();
                    }
                }
            }
            Pane::SchemaExplorer => self.navigate_schema(direction),
            Pane::Results => self.navigate_results(direction),
            Pane::QueryEditor => {
                self.status_line = format!("Navigation in editor: {direction:?}");
            }
        }
    }

    fn navigate_palette(&mut self, direction: DirectionKey) {
        let entry_count = self.palette_entries().len();
        if entry_count == 0 {
            self.palette_selection = 0;
            return;
        }

        match direction {
            DirectionKey::Up | DirectionKey::Left => {
                self.palette_selection = self.palette_selection.saturating_sub(1);
            }
            DirectionKey::Down | DirectionKey::Right => {
                self.palette_selection = (self.palette_selection + 1).min(entry_count - 1);
            }
        }

        self.status_line = format!("Palette selection: {}", self.palette_selection + 1);
    }

    fn previous_wizard_field(&self) -> WizardField {
        match self.wizard_form.active_field {
            WizardField::ProfileName => WizardField::Database,
            WizardField::Host => WizardField::ProfileName,
            WizardField::Port => WizardField::Host,
            WizardField::User => WizardField::Port,
            WizardField::Database => WizardField::User,
        }
    }

    fn navigate_schema(&mut self, direction: DirectionKey) {
        match direction {
            DirectionKey::Left => {
                self.schema_lane = self.schema_lane.previous();
                self.status_line = format!("Schema focus: {}", self.schema_lane.label());
            }
            DirectionKey::Right => {
                self.schema_lane = self.schema_lane.next();
                self.status_line = format!("Schema focus: {}", self.schema_lane.label());
            }
            DirectionKey::Up | DirectionKey::Down => match self.schema_lane {
                SchemaLane::Databases => self.navigate_schema_databases(direction),
                SchemaLane::Tables => self.navigate_schema_tables(direction),
                SchemaLane::Columns => self.navigate_schema_columns(direction),
            },
        }
    }

    fn navigate_schema_databases(&mut self, direction: DirectionKey) {
        if self.schema_databases.is_empty() {
            self.status_line = "No databases available".to_string();
            return;
        }

        match direction {
            DirectionKey::Up => {
                self.selected_database_index = self.selected_database_index.saturating_sub(1);
            }
            DirectionKey::Down => {
                let max_index = self.schema_databases.len() - 1;
                self.selected_database_index = (self.selected_database_index + 1).min(max_index);
            }
            DirectionKey::Left | DirectionKey::Right => {}
        }

        self.active_database = self
            .schema_databases
            .get(self.selected_database_index)
            .cloned();
        self.selection.database = self.active_database.clone();
        self.reload_tables_for_active_database();
        self.clear_pagination_state();
        self.set_query_editor_to_selected_table();

        if let Some(database) = &self.active_database {
            self.status_line = format!("Selected database `{database}`");
        }
    }

    fn navigate_schema_tables(&mut self, direction: DirectionKey) {
        if self.schema_tables.is_empty() {
            self.status_line = "No tables available".to_string();
            return;
        }

        match direction {
            DirectionKey::Up => {
                self.selected_table_index = self.selected_table_index.saturating_sub(1);
            }
            DirectionKey::Down => {
                let max_index = self.schema_tables.len() - 1;
                self.selected_table_index = (self.selected_table_index + 1).min(max_index);
            }
            DirectionKey::Left | DirectionKey::Right => {}
        }

        self.selection.table = self.schema_tables.get(self.selected_table_index).cloned();
        self.reload_columns_for_selected_table();
        self.clear_pagination_state();
        self.set_query_editor_to_selected_table();

        if let Some(table) = &self.selection.table {
            self.status_line = format!("Selected table `{table}`");
        }
    }

    fn navigate_schema_columns(&mut self, direction: DirectionKey) {
        if self.schema_columns.is_empty() {
            self.status_line = "No columns available".to_string();
            return;
        }

        match direction {
            DirectionKey::Up => {
                self.selected_column_index = self.selected_column_index.saturating_sub(1);
            }
            DirectionKey::Down => {
                let max_index = self.schema_columns.len() - 1;
                self.selected_column_index = (self.selected_column_index + 1).min(max_index);
            }
            DirectionKey::Left | DirectionKey::Right => {}
        }

        self.selection.column = self.schema_columns.get(self.selected_column_index).cloned();
        if let Some(column) = &self.selection.column {
            self.status_line = format!("Selected column `{column}`");
        }
    }

    fn reload_tables_for_active_database(&mut self) {
        let Some(database_name) = self.active_database.clone() else {
            self.schema_tables.clear();
            self.selected_table_index = 0;
            self.selection.table = None;
            self.reload_columns_for_selected_table();
            return;
        };

        if let Some(schema_cache) = self.schema_cache.as_mut() {
            self.schema_tables = match block_on_result(schema_cache.list_tables(&database_name)) {
                Ok(tables) => tables,
                Err(error) => {
                    self.status_line = format!("Table fetch failed: {error}");
                    Vec::new()
                }
            };
        } else if self.schema_tables.is_empty() {
            self.schema_tables = DEMO_SCHEMA_TABLES
                .iter()
                .map(|table| (*table).to_string())
                .collect();
        }

        self.selected_table_index = 0;
        self.selection.table = self.schema_tables.first().cloned();
        self.reload_columns_for_selected_table();
    }

    fn reload_columns_for_selected_table(&mut self) {
        let Some(table_name) = self.selection.table.clone() else {
            self.schema_columns.clear();
            self.selected_column_index = 0;
            self.selection.column = None;
            return;
        };

        if let Some(schema_cache) = self.schema_cache.as_mut() {
            if let Some(database_name) = self.active_database.clone() {
                self.schema_columns =
                    match block_on_result(schema_cache.list_columns(&database_name, &table_name)) {
                        Ok(columns) => columns.into_iter().map(|column| column.name).collect(),
                        Err(error) => {
                            self.status_line = format!("Column fetch failed: {error}");
                            Vec::new()
                        }
                    };
            } else {
                self.schema_columns.clear();
            }
        } else {
            self.schema_columns = DEMO_SCHEMA_COLUMNS
                .iter()
                .map(|column| (*column).to_string())
                .collect();
        }

        self.selected_column_index = 0;
        self.selection.column = self.schema_columns.first().cloned();
    }

    fn set_query_editor_to_selected_table(&mut self) {
        let Some(table) = self.selection.table.as_deref() else {
            return;
        };

        let table_sql = quote_identifier(table);
        if let Some(database) = self.selection.database.as_deref() {
            let database_sql = quote_identifier(database);
            self.query_editor_text = format!("SELECT * FROM {database_sql}.{table_sql}");
        } else {
            self.query_editor_text = format!("SELECT * FROM {table_sql}");
        }
    }

    fn navigate_results(&mut self, direction: DirectionKey) {
        let row_count = self.results.len();
        if row_count == 0 {
            self.status_line = "No buffered rows yet".to_string();
            return;
        }

        match direction {
            DirectionKey::Up | DirectionKey::Left => {
                self.results_cursor = self.results_cursor.saturating_sub(1);
            }
            DirectionKey::Down | DirectionKey::Right => {
                self.results_cursor = (self.results_cursor + 1).min(row_count.saturating_sub(1));
            }
        }

        self.status_line = format!(
            "Results cursor: {} / {}",
            self.results_cursor + 1,
            row_count
        );
    }

    fn start_results_search(&mut self) {
        if self.results.is_empty() {
            self.results_search_mode = false;
            self.status_line = "No buffered rows yet".to_string();
            return;
        }

        self.set_active_pane(Pane::Results);
        self.results_search_mode = true;
        self.apply_results_search(false);
    }

    fn apply_results_search(&mut self, find_next: bool) {
        let query = self.results_search_query.trim();
        if query.is_empty() {
            self.status_line = "Search results: type text, Enter next, Esc cancel".to_string();
            return;
        }

        let row_count = self.results.len();
        if row_count == 0 {
            self.status_line = "No buffered rows yet".to_string();
            return;
        }

        let start_index = if find_next {
            (self.results_cursor + 1) % row_count
        } else {
            0
        };

        if let Some(index) = self.find_results_match_index(query, start_index) {
            self.results_cursor = index;
            self.status_line = format!(
                "Search matched row {} / {} for `{query}` (Enter next, Esc cancel)",
                index + 1,
                row_count
            );
        } else {
            self.status_line = format!("No match for `{query}` in {row_count} buffered rows");
        }
    }

    fn find_results_match_index(&self, query: &str, start_index: usize) -> Option<usize> {
        if self.results.is_empty() {
            return None;
        }

        let needle = query.to_ascii_lowercase();
        let row_count = self.results.len();
        for offset in 0..row_count {
            let index = (start_index + offset) % row_count;
            let Some(row) = self.results.get(index) else {
                continue;
            };
            if row
                .values
                .iter()
                .any(|value| value.to_ascii_lowercase().contains(&needle))
            {
                return Some(index);
            }
        }

        None
    }

    fn populate_demo_results(&mut self) {
        self.results = ResultsRingBuffer::new(RESULT_BUFFER_CAPACITY);
        self.results_cursor = 0;
        self.results_search_mode = false;
        self.results_search_query.clear();
        self.result_columns = vec![
            "id".to_string(),
            "value".to_string(),
            "observed_at".to_string(),
        ];

        let selected_table = self
            .selection
            .table
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        for index in 1..=500 {
            let row = QueryRow::new(vec![
                index.to_string(),
                format!("{selected_table}-value-{index}"),
                format!("2026-02-{day:02}", day = ((index - 1) % 28) + 1),
            ]);
            self.results.push(row);
        }

        self.has_results = true;
        self.selection.column = Some("value".to_string());
    }

    fn export_results(&mut self, format: myr_core::actions_engine::ExportFormat) {
        if !self.has_results || self.results.is_empty() {
            self.status_line = "No results available to export".to_string();
            return;
        }

        let rows = (0..self.results.len())
            .filter_map(|index| self.results.get(index))
            .map(|row| row.values.clone())
            .collect::<Vec<_>>();
        let file_path = export_file_path(match format {
            myr_core::actions_engine::ExportFormat::Csv => "csv",
            myr_core::actions_engine::ExportFormat::Json => "json",
        });

        let result = match format {
            myr_core::actions_engine::ExportFormat::Csv => {
                export_rows_to_csv(&file_path, &self.result_columns, &rows)
            }
            myr_core::actions_engine::ExportFormat::Json => {
                export_rows_to_json(&file_path, &self.result_columns, &rows)
            }
        };

        match result {
            Ok(row_count) => {
                self.status_line = format!("Exported {row_count} rows to {}", file_path.display());
            }
            Err(error) => {
                self.status_line = format!("Export failed: {error}");
            }
        }
    }

    fn palette_entries(&self) -> Vec<ActionId> {
        let query = self.palette_query.to_ascii_lowercase();
        self.actions
            .rank_top_n(&self.action_context(), 50)
            .into_iter()
            .filter(|action| {
                if query.is_empty() {
                    true
                } else {
                    action.title.to_ascii_lowercase().contains(&query)
                }
            })
            .map(|action| action.id)
            .collect()
    }

    fn selected_palette_action(&self) -> Option<ActionId> {
        let entries = self.palette_entries();
        entries.get(self.palette_selection).copied()
    }

    fn handle_input_char(&mut self, ch: char) {
        if self.show_palette {
            self.palette_query.push(ch);
            self.palette_selection = 0;
            self.status_line = format!("Palette query: {}", self.palette_query);
        } else if self.pane == Pane::ConnectionWizard {
            if !self.wizard_form.editing {
                if ch.eq_ignore_ascii_case(&'e') {
                    self.start_wizard_edit();
                } else {
                    self.status_line = format!(
                        "Selected {}. Press E or Enter to edit",
                        self.wizard_form.active_field.label()
                    );
                }
            } else {
                self.wizard_form.edit_buffer.push(ch);
                self.status_line = format!("Editing {}", self.wizard_form.active_field.label());
            }
        } else if self.pane == Pane::QueryEditor {
            self.query_editor_text.push(ch);
            self.status_line = "Query text updated".to_string();
        }
    }

    fn handle_backspace(&mut self) {
        if self.show_palette {
            self.palette_query.pop();
            self.palette_selection = 0;
            self.status_line = format!("Palette query: {}", self.palette_query);
        } else if self.pane == Pane::ConnectionWizard {
            if self.wizard_form.editing {
                self.wizard_form.edit_buffer.pop();
                self.status_line = format!("Editing {}", self.wizard_form.active_field.label());
            } else {
                self.status_line = format!(
                    "Selected {}. Press E or Enter to edit",
                    self.wizard_form.active_field.label()
                );
            }
        } else if self.pane == Pane::QueryEditor {
            self.query_editor_text.pop();
            self.status_line = "Query text updated".to_string();
        }
    }

    fn handle_clear_input(&mut self) {
        if self.show_palette {
            self.palette_query.clear();
            self.palette_selection = 0;
            self.status_line = "Palette query cleared".to_string();
        } else if self.pane == Pane::ConnectionWizard {
            if self.wizard_form.editing {
                self.wizard_form.edit_buffer.clear();
                self.status_line = format!("Cleared {}", self.wizard_form.active_field.label());
            } else {
                self.status_line = format!(
                    "Selected {}. Press E or Enter to edit",
                    self.wizard_form.active_field.label()
                );
            }
        } else if self.pane == Pane::QueryEditor {
            self.query_editor_text.clear();
            self.status_line = "Query cleared".to_string();
        }
    }

    fn start_wizard_edit(&mut self) {
        if self.pane != Pane::ConnectionWizard || self.wizard_form.editing {
            return;
        }
        let current_value = self.active_wizard_value().to_string();
        self.wizard_form.editing = true;
        self.wizard_form.edit_buffer = current_value;
        self.status_line = format!(
            "Editing {} (Enter save, Esc cancel, Ctrl+U clear)",
            self.wizard_form.active_field.label()
        );
    }

    fn commit_wizard_edit(&mut self) {
        if self.pane != Pane::ConnectionWizard || !self.wizard_form.editing {
            return;
        }
        let updated_value = self.wizard_form.edit_buffer.clone();
        *self.active_wizard_value_mut() = updated_value;
        self.wizard_form.editing = false;
        self.wizard_form.edit_buffer.clear();
        self.status_line = format!("Saved {}", self.wizard_form.active_field.label());
    }

    fn cancel_wizard_edit(&mut self) {
        if self.pane != Pane::ConnectionWizard || !self.wizard_form.editing {
            return;
        }
        self.wizard_form.editing = false;
        self.wizard_form.edit_buffer.clear();
        self.status_line = format!("Canceled editing {}", self.wizard_form.active_field.label());
    }

    fn active_wizard_value(&self) -> &str {
        match self.wizard_form.active_field {
            WizardField::ProfileName => self.wizard_form.profile_name.as_str(),
            WizardField::Host => self.wizard_form.host.as_str(),
            WizardField::Port => self.wizard_form.port.as_str(),
            WizardField::User => self.wizard_form.user.as_str(),
            WizardField::Database => self.wizard_form.database.as_str(),
        }
    }

    fn active_wizard_value_mut(&mut self) -> &mut String {
        match self.wizard_form.active_field {
            WizardField::ProfileName => &mut self.wizard_form.profile_name,
            WizardField::Host => &mut self.wizard_form.host,
            WizardField::Port => &mut self.wizard_form.port,
            WizardField::User => &mut self.wizard_form.user,
            WizardField::Database => &mut self.wizard_form.database,
        }
    }

    fn invoke_ranked_action(&mut self, index: usize) {
        if self.pane == Pane::ConnectionWizard {
            if self.wizard_form.editing {
                let digit = char::from_digit((index + 1) as u32, 10).unwrap_or('0');
                self.wizard_form.edit_buffer.push(digit);
                self.status_line = format!("Editing {}", self.wizard_form.active_field.label());
            } else {
                self.status_line = format!(
                    "Selected {}. Press E or Enter to edit",
                    self.wizard_form.active_field.label()
                );
            }
            return;
        }

        if self.show_palette {
            self.palette_selection = index.min(self.palette_entries().len().saturating_sub(1));
            self.submit();
            return;
        }

        let context = self.action_context();
        let ranked = self.actions.rank_top_n(&context, FOOTER_ACTIONS_LIMIT);
        let Some(action) = ranked.get(index) else {
            self.status_line = format!("No action bound to slot {}", index + 1);
            return;
        };

        self.invoke_action(action.id);
    }

    fn invoke_action(&mut self, action_id: ActionId) {
        let context = self.action_context();
        match self.actions.invoke(action_id, &context) {
            Ok(invocation) => self.apply_invocation(action_id, invocation),
            Err(error) => self.status_line = format!("Action error: {error}"),
        }
    }

    fn start_query(&mut self, sql: String) {
        self.start_query_internal(sql, false);
    }

    fn start_query_internal(&mut self, sql: String, retrying: bool) {
        if !retrying {
            self.query_retry_attempts = 0;
            self.last_failed_query = None;
            self.pending_retry_query = None;
            self.reconnect_attempts = 0;
        }
        self.inflight_query_sql = Some(sql.clone());
        self.query_editor_text = sql;
        self.set_active_pane(Pane::Results);
        self.results_search_mode = false;
        self.results_search_query.clear();
        self.error_panel = None;
        self.cancel_requested = false;
        self.has_results = false;
        self.query_cancellation = None;
        self.query_result_rx = None;

        if let Some(data_backend) = &self.data_backend {
            self.query_running = true;
            self.query_ticks_remaining = 0;
            self.results = ResultsRingBuffer::new(RESULT_BUFFER_CAPACITY);
            let cancellation = CancellationToken::new();
            self.query_cancellation = Some(cancellation.clone());
            let backend = data_backend.clone();
            let sql = self.query_editor_text.clone();
            let (tx, rx) = mpsc::channel();
            self.query_result_rx = Some(rx);

            let _query_worker = thread::spawn(move || {
                let _ = tx.send(run_query_worker(backend, sql, cancellation));
            });

            self.status_line = "Running query...".to_string();
            return;
        }

        self.query_running = true;
        self.query_ticks_remaining = QUERY_DURATION_TICKS;
        self.status_line = "Running query...".to_string();
    }

    fn execute_sql_with_guard(&mut self, sql: String) {
        match self.safe_mode_guard.evaluate(&sql) {
            GuardDecision::Allow { .. } => {
                self.pending_confirmation = None;
                self.start_query(sql);
            }
            GuardDecision::RequireConfirmation { token, assessment } => {
                self.pending_confirmation = Some((token, sql.clone()));
                self.query_editor_text = sql;
                self.set_active_pane(Pane::QueryEditor);
                self.status_line = format!(
                    "Safe mode confirmation required: {:?}. Press Enter again to confirm.",
                    assessment.reasons
                );
            }
        }
    }

    fn start_preview_paged_query(&mut self, fallback_sql: String) {
        let Some(state) = self.build_preview_pagination_state() else {
            self.clear_pagination_state();
            self.execute_sql_with_guard(fallback_sql);
            return;
        };

        let sql = match self.pagination_sql(&state, PageTransition::Reset) {
            Ok(sql) => sql,
            Err(error) => {
                self.clear_pagination_state();
                self.status_line = format!("Pagination setup failed: {error}");
                return;
            }
        };

        if !self.schema_columns.is_empty() {
            self.result_columns = self.schema_columns.clone();
        }
        self.pagination_state = Some(state);
        self.pending_page_transition = Some(PageTransition::Reset);
        self.execute_sql_with_guard(sql);
    }

    fn run_pagination_transition(&mut self, transition: PageTransition) {
        let Some(state) = self.pagination_state.clone() else {
            self.status_line = "Pagination is not active for the current result set".to_string();
            return;
        };

        if matches!(transition, PageTransition::Previous) && state.page_index == 0 {
            self.status_line = "Already at the first page".to_string();
            return;
        }

        let sql = match self.pagination_sql(&state, transition) {
            Ok(sql) => sql,
            Err(error) => {
                self.status_line = format!("Pagination unavailable: {error}");
                return;
            }
        };

        if !self.schema_columns.is_empty() {
            self.result_columns = self.schema_columns.clone();
        }
        self.pending_page_transition = Some(transition);
        self.execute_sql_with_guard(sql);
    }

    fn pagination_sql(
        &self,
        state: &PaginationState,
        transition: PageTransition,
    ) -> Result<String, String> {
        let target = SqlTarget::new(state.database.as_deref(), state.table.as_str())
            .map_err(|error| error.to_string())?;

        match &state.plan {
            PaginationPlan::Keyset {
                key_column,
                first_key,
                last_key,
            } => match transition {
                PageTransition::Reset => {
                    keyset_first_page_sql(&target, key_column, state.page_size)
                        .map_err(|error| error.to_string())
                }
                PageTransition::Next => {
                    let Some(boundary) = last_key.as_deref() else {
                        return Err("missing keyset boundary for next page".to_string());
                    };
                    keyset_page_sql(
                        &target,
                        key_column,
                        boundary,
                        PaginationDirection::Next,
                        state.page_size,
                    )
                    .map_err(|error| error.to_string())
                }
                PageTransition::Previous => {
                    let Some(boundary) = first_key.as_deref() else {
                        return Err("missing keyset boundary for previous page".to_string());
                    };
                    keyset_page_sql(
                        &target,
                        key_column,
                        boundary,
                        PaginationDirection::Previous,
                        state.page_size,
                    )
                    .map_err(|error| error.to_string())
                }
            },
            PaginationPlan::Offset => {
                let next_index = match transition {
                    PageTransition::Reset => 0,
                    PageTransition::Next => state.page_index.saturating_add(1),
                    PageTransition::Previous => state.page_index.saturating_sub(1),
                };
                let offset = next_index.saturating_mul(state.page_size);
                Ok(offset_page_sql(&target, state.page_size, offset))
            }
        }
    }

    fn build_preview_pagination_state(&self) -> Option<PaginationState> {
        let table = self.selection.table.clone()?;
        let plan = match candidate_key_column(&self.schema_columns) {
            Some(key_column) => PaginationPlan::Keyset {
                key_column,
                first_key: None,
                last_key: None,
            },
            None => PaginationPlan::Offset,
        };

        Some(PaginationState {
            database: self.selection.database.clone(),
            table,
            page_size: PREVIEW_PAGE_SIZE,
            page_index: 0,
            last_page_row_count: 0,
            plan,
        })
    }

    fn finalize_pagination_after_query(&mut self) {
        let Some(transition) = self.pending_page_transition.take() else {
            return;
        };

        let row_count = self.results.len();
        let key_bounds = self
            .pagination_state
            .as_ref()
            .and_then(|state| match &state.plan {
                PaginationPlan::Keyset { key_column, .. } => Some(extract_key_bounds(
                    &self.results,
                    &self.result_columns,
                    key_column,
                )),
                PaginationPlan::Offset => None,
            });

        let Some(state) = self.pagination_state.as_mut() else {
            return;
        };

        state.last_page_row_count = row_count;
        match transition {
            PageTransition::Reset => state.page_index = 0,
            PageTransition::Next => {
                if row_count > 0 {
                    state.page_index = state.page_index.saturating_add(1);
                }
            }
            PageTransition::Previous => {
                if row_count > 0 {
                    state.page_index = state.page_index.saturating_sub(1);
                }
            }
        }

        if let (
            PaginationPlan::Keyset {
                first_key,
                last_key,
                ..
            },
            Some((first, last)),
        ) = (&mut state.plan, key_bounds)
        {
            *first_key = first;
            *last_key = last;
        }
    }

    fn clear_pagination_state(&mut self) {
        self.pagination_state = None;
        self.pending_page_transition = None;
    }

    fn pagination_capabilities(&self) -> (bool, bool, bool) {
        let Some(state) = self.pagination_state.as_ref() else {
            return (false, false, false);
        };

        let can_page_next = self.has_results && state.last_page_row_count >= state.page_size;
        let can_page_previous = state.page_index > 0;
        (true, can_page_next, can_page_previous)
    }

    fn apply_invocation(&mut self, action_id: ActionId, invocation: ActionInvocation) {
        match invocation {
            ActionInvocation::RunSql(sql) => {
                if action_id == ActionId::PreviewTable {
                    self.start_preview_paged_query(sql);
                } else {
                    self.clear_pagination_state();
                    self.execute_sql_with_guard(sql);
                }
            }
            ActionInvocation::PaginatePrevious => {
                self.run_pagination_transition(PageTransition::Previous);
            }
            ActionInvocation::PaginateNext => {
                self.run_pagination_transition(PageTransition::Next);
            }
            ActionInvocation::ReplaceQueryEditorText(query) => {
                self.query_editor_text = query;
                self.status_line = "Applied LIMIT suggestion".to_string();
            }
            ActionInvocation::CancelQuery => {
                self.query_running = false;
                self.query_ticks_remaining = 0;
                self.cancel_requested = true;
                self.status_line = "Query cancelled".to_string();
            }
            ActionInvocation::ExportResults(format) => {
                self.export_results(format);
            }
            ActionInvocation::CopyToClipboard(target) => {
                self.status_line = format!("Copy requested: {target:?}");
            }
            ActionInvocation::OpenView(view) => {
                let pane = match view {
                    AppView::ConnectionWizard => Pane::ConnectionWizard,
                    AppView::SchemaExplorer => Pane::SchemaExplorer,
                    AppView::Results => Pane::Results,
                    AppView::QueryEditor => Pane::QueryEditor,
                    AppView::CommandPalette => self.pane,
                };
                self.set_active_pane(pane);
                self.status_line = format!("Switched view to {}", self.pane_name());
            }
            ActionInvocation::SearchBufferedResults => {
                self.start_results_search();
            }
        }
    }

    fn action_context(&self) -> ActionContext {
        let view = match self.pane {
            Pane::ConnectionWizard => AppView::ConnectionWizard,
            Pane::SchemaExplorer => AppView::SchemaExplorer,
            Pane::Results => AppView::Results,
            Pane::QueryEditor => AppView::QueryEditor,
        };

        let query_text = if matches!(self.pane, Pane::QueryEditor) || self.query_running {
            Some(self.query_editor_text.clone())
        } else {
            None
        };
        let (pagination_enabled, can_page_next, can_page_previous) = self.pagination_capabilities();

        ActionContext {
            view,
            selection: self.selection.clone(),
            query_text,
            query_running: self.query_running,
            has_results: self.has_results,
            pagination_enabled,
            can_page_next,
            can_page_previous,
        }
    }

    fn open_error_panel(
        &mut self,
        kind: ErrorKind,
        title: impl Into<String>,
        summary: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.error_panel = Some(ErrorPanel {
            kind,
            title: title.into(),
            summary: summary.into(),
            detail: detail.into(),
        });
    }

    fn run_primary_error_action(&mut self) {
        let Some(panel) = self.error_panel.as_ref().cloned() else {
            return;
        };

        if panel.kind == ErrorKind::Query {
            if let Some(sql) = self.last_failed_query.clone() {
                self.error_panel = None;
                self.start_query(sql);
                return;
            }
        }

        self.reconnect_from_error_panel();
    }

    fn reconnect_from_error_panel(&mut self) {
        if self.connect_requested {
            self.status_line = "Already connecting...".to_string();
            return;
        }

        let profile = self
            .active_connection_profile
            .clone()
            .or_else(|| self.last_connect_profile.clone())
            .or_else(|| self.wizard_profile().ok());

        let Some(profile) = profile else {
            self.status_line = "Reconnect unavailable: provide a valid connection profile".to_string();
            return;
        };

        self.error_panel = None;
        self.reconnect_attempts = 0;
        self.start_connect_with_profile(profile, ConnectIntent::Manual);
    }

    fn can_reconnect_from_error_panel(&self) -> bool {
        self.active_connection_profile.is_some()
            || self.last_connect_profile.is_some()
            || self.wizard_profile().is_ok()
    }

    fn pane_tab_index(&self) -> usize {
        match self.pane {
            Pane::ConnectionWizard => 0,
            Pane::SchemaExplorer => 1,
            Pane::Results => 2,
            Pane::QueryEditor => 3,
        }
    }

    fn runtime_state_label(&self) -> &'static str {
        if self.connect_requested || self.query_running {
            "BUSY"
        } else {
            "IDLE"
        }
    }

    fn connection_state_label(&self) -> &'static str {
        if self.connect_requested {
            if self.connect_intent == ConnectIntent::AutoReconnect {
                "RECONNECTING"
            } else {
                "CONNECTING"
            }
        } else if self.data_backend.is_some() {
            "CONNECTED"
        } else {
            "DISCONNECTED"
        }
    }

    fn pane_name(&self) -> &'static str {
        match self.pane {
            Pane::ConnectionWizard => "Connection Wizard",
            Pane::SchemaExplorer => "Schema Explorer",
            Pane::Results => "Results",
            Pane::QueryEditor => "Query Editor",
        }
    }

    fn set_active_pane(&mut self, pane: Pane) {
        if self.pane != pane {
            self.pane = pane;
            self.pane_flash_ticks = PANE_FLASH_DURATION_TICKS;
            if pane != Pane::Results {
                self.results_search_mode = false;
            }
        }
    }
}

#[must_use]
pub fn ui_name() -> &'static str {
    "myr-tui"
}

pub fn run() -> Result<(), TuiError> {
    let mut terminal = setup_terminal()?;
    let run_result = run_loop(&mut terminal);
    let restore_result = restore_terminal(&mut terminal);

    if let Err(error) = run_result {
        restore_result?;
        return Err(error);
    }

    restore_result?;
    Ok(())
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>, TuiError> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<(), TuiError> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

fn run_loop(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<(), TuiError> {
    let mut app = TuiApp::default();
    let mut last_tick = Instant::now();

    loop {
        let render_started = Instant::now();
        terminal.draw(|frame| render(frame, &app))?;
        app.record_render(render_started.elapsed());

        let timeout = TICK_RATE
            .checked_sub(last_tick.elapsed())
            .unwrap_or(Duration::ZERO);
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    if let Some(message) = map_key_event(key) {
                        app.handle(message);
                    }
                }
            }
        }

        if last_tick.elapsed() >= TICK_RATE {
            app.handle(Msg::Tick);
            last_tick = Instant::now();
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

fn render(frame: &mut Frame<'_>, app: &TuiApp) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Min(8),
            Constraint::Length(4),
        ])
        .split(frame.area());
    let top_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(3)])
        .split(chunks[0]);

    let latency_text = app
        .last_connection_latency
        .map_or("n/a".to_string(), |latency| format!("{latency:.1?}"));
    let heartbeat = spinner_char(app.loading_tick);
    let loading_text = if app.connect_requested && app.connect_intent == ConnectIntent::AutoReconnect
    {
        "reconnecting"
    } else if app.connect_requested {
        "connecting"
    } else if app.query_running && app.cancel_requested {
        "cancelling query"
    } else if app.query_running {
        "querying"
    } else {
        "idle"
    };
    let runtime_state = app.runtime_state_label();
    let runtime_state_color = if runtime_state == "BUSY" {
        Color::Yellow
    } else {
        Color::Green
    };
    let connection_state = app.connection_state_label();
    let (connection_badge, connection_marker) =
        connection_badge_and_marker(connection_state, app.loading_tick);
    let connection_color = match connection_state {
        "CONNECTED" => {
            if app.loading_tick % 2 == 0 {
                Color::Green
            } else {
                Color::Cyan
            }
        }
        "CONNECTING" | "RECONNECTING" => Color::Yellow,
        _ => Color::Red,
    };

    let runtime_bar = Paragraph::new(Line::from(vec![
        Span::styled(
            format!(" APP {heartbeat} "),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" | "),
        Span::raw("State: "),
        Span::styled(
            runtime_state,
            Style::default()
                .fg(runtime_state_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" | "),
        Span::raw("DB: "),
        Span::styled(
            format!("{connection_badge} {connection_state} {connection_marker}"),
            Style::default()
                .fg(connection_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" | "),
        Span::raw(format!(
            "Profile: {}",
            app.connected_profile.as_deref().unwrap_or("not connected")
        )),
        Span::raw(" | "),
        Span::raw(format!(
            "DB: {}",
            app.selection.database.as_deref().unwrap_or("-")
        )),
        Span::raw(" | "),
        Span::raw(format!("Latency: {latency_text}")),
        Span::raw(" | "),
        Span::raw(format!(
            "Query: {}",
            if app.query_running { "running" } else { "idle" }
        )),
        Span::raw(" | "),
        Span::raw(format!("Load: {loading_text}")),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Runtime"),
    );
    frame.render_widget(runtime_bar, top_chunks[0]);

    let tab_focus_marker = pulse_char(app.loading_tick);
    let tabs_title = if app.pane_flash_ticks > 0 {
        format!(
            "Panes (Tab cycles, F6 returns to Connection Wizard) | Active: {} {}",
            app.pane_name(),
            tab_focus_marker
        )
    } else {
        "Panes (Tab cycles, F6 returns to Connection Wizard)".to_string()
    };
    let tab_labels = [
        (Pane::ConnectionWizard, "Connection Wizard"),
        (Pane::SchemaExplorer, "Schema Explorer"),
        (Pane::Results, "Results"),
        (Pane::QueryEditor, "Query Editor"),
    ]
    .into_iter()
    .map(|(pane, label)| {
        if pane == app.pane && app.pane_flash_ticks > 0 {
            Line::from(format!("{label} {tab_focus_marker}"))
        } else {
            Line::from(label)
        }
    })
    .collect::<Vec<_>>();
    let tab_highlight_style = if app.pane_flash_ticks > 0 {
        let flash_bg = if app.loading_tick % 2 == 0 {
            Color::Yellow
        } else {
            Color::Cyan
        };
        Style::default()
            .fg(Color::Black)
            .bg(flash_bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    };

    let tabs = Tabs::new(tab_labels)
    .select(app.pane_tab_index())
    .style(Style::default().fg(Color::DarkGray))
    .highlight_style(tab_highlight_style)
    .divider(" | ")
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(tabs_title),
    );
    frame.render_widget(tabs, top_chunks[1]);

    let body_text = match app.pane {
        Pane::ConnectionWizard => {
            let fields = [
                (
                    WizardField::ProfileName,
                    "Profile",
                    app.wizard_form.profile_name.as_str(),
                ),
                (WizardField::Host, "Host", app.wizard_form.host.as_str()),
                (WizardField::Port, "Port", app.wizard_form.port.as_str()),
                (WizardField::User, "User", app.wizard_form.user.as_str()),
                (
                    WizardField::Database,
                    "Database",
                    app.wizard_form.database.as_str(),
                ),
            ];

            let mut lines = vec![
                Line::from("Connection Wizard"),
                Line::from("Up/Down: select field"),
                Line::from("E or Enter: edit field | Enter: save field"),
                Line::from("Esc: cancel edit | Ctrl+U: clear field | F5: connect"),
                Line::from(""),
            ];
            for (field, label, value) in fields {
                let marker = if app.wizard_form.active_field == field {
                    ">"
                } else {
                    " "
                };
                let editing_active =
                    app.wizard_form.editing && app.wizard_form.active_field == field;
                let active = app.wizard_form.active_field == field;
                let display_label = if editing_active {
                    format!("{label} [EDIT]")
                } else {
                    label.to_string()
                };
                let display_value = if editing_active {
                    app.wizard_form.edit_buffer.as_str()
                } else {
                    value
                };
                let line = format!("{marker} {display_label}: {display_value}");
                if editing_active {
                    lines.push(Line::from(Span::styled(
                        line,
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    )));
                } else if active {
                    lines.push(Line::from(Span::styled(
                        line,
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )));
                } else {
                    lines.push(Line::from(line));
                }
            }
            lines
        }
        Pane::SchemaExplorer => {
            let mut lines = vec![
                Line::from("Schema Explorer"),
                Line::from("Left/Right switches focus lane, Up/Down changes selection."),
                Line::from(format!(
                    "Focus lane: {} (press 1 for preview action).",
                    app.schema_lane.label()
                )),
                Line::from(""),
            ];

            if app.schema_databases.is_empty() {
                lines.push(Line::from("Databases: (none loaded)"));
            } else {
                lines.push(Line::from("Databases:"));
                for (index, database) in app.schema_databases.iter().enumerate() {
                    let marker = if app.schema_lane == SchemaLane::Databases
                        && index == app.selected_database_index
                    {
                        ">"
                    } else {
                        " "
                    };
                    lines.push(Line::from(format!("{marker} {database}")));
                }
            }
            lines.push(Line::from(""));
            lines.push(Line::from(format!(
                "Active DB: {}",
                app.active_database.as_deref().unwrap_or("-")
            )));
            lines.push(Line::from("Tables:"));

            for (index, table) in app.schema_tables.iter().enumerate() {
                let marker =
                    if app.schema_lane == SchemaLane::Tables && index == app.selected_table_index {
                        ">"
                    } else {
                        " "
                    };
                lines.push(Line::from(format!("{marker} {table}")));
            }

            if app.schema_tables.is_empty() {
                lines.push(Line::from("  (none)"));
            }

            lines.push(Line::from(""));
            lines.push(Line::from("Columns:"));
            for (index, column) in app.schema_columns.iter().enumerate() {
                let marker = if app.schema_lane == SchemaLane::Columns
                    && index == app.selected_column_index
                {
                    ">"
                } else {
                    " "
                };
                lines.push(Line::from(format!("{marker} {column}")));
            }
            if app.schema_columns.is_empty() {
                lines.push(Line::from("  (none)"));
            }
            lines
        }
        Pane::Results => {
            let visible_limit = usize::from(chunks[1].height.saturating_sub(3)).max(1);
            let window_start = app.results_cursor.saturating_sub(visible_limit / 2);
            let rows = app.results.visible_rows(window_start, visible_limit);
            let no_rows = rows.is_empty();

            let mut lines = vec![
                Line::from("Results View (virtualized)"),
                Line::from("Use arrows / hjkl to move cursor."),
            ];
            if app.results_search_mode {
                let query = if app.results_search_query.is_empty() {
                    "(type to search)".to_string()
                } else {
                    app.results_search_query.clone()
                };
                lines.push(Line::from(format!(
                    "Search: {query} (Enter next, Esc cancel)"
                )));
            } else {
                lines.push(Line::from("Press 7 to search buffered rows."));
            }
            if let Some(state) = &app.pagination_state {
                let strategy = match &state.plan {
                    PaginationPlan::Keyset { key_column, .. } => {
                        format!("keyset by `{key_column}`")
                    }
                    PaginationPlan::Offset => "offset fallback".to_string(),
                };
                lines.push(Line::from(format!(
                    "Page {} | Strategy: {} | Page size {}",
                    state.page_index + 1,
                    strategy,
                    state.page_size
                )));
            }

            for (offset, row) in rows.into_iter().enumerate() {
                let absolute_index = window_start + offset;
                let cursor = if absolute_index == app.results_cursor {
                    ">"
                } else {
                    " "
                };
                lines.push(Line::from(format!(
                    "{cursor} {:04} | {}",
                    absolute_index + 1,
                    row.values.join(" | ")
                )));
            }

            if no_rows {
                lines.push(Line::from(if app.query_running {
                    "Query running... waiting for rows"
                } else {
                    "No rows buffered. Tab to Query Editor + Enter, or use 1 in Schema Explorer."
                }));
            }
            lines
        }
        Pane::QueryEditor => vec![
            Line::from("Query Editor"),
            Line::from(app.query_editor_text.as_str()),
            Line::from("Enter to run query, 1..7 for ranked actions."),
            Line::from("Tab: next pane | F6: connection wizard"),
            Line::from("Ctrl+P opens palette placeholder."),
        ],
    };

    let body = Paragraph::new(body_text)
        .block(Block::default().borders(Borders::ALL).title("Workspace"))
        .alignment(Alignment::Left);
    frame.render_widget(body, chunks[1]);

    let footer_line = if app.pane == Pane::ConnectionWizard {
        "F5: connect | E/Enter: edit | Enter: save edit | Esc: cancel edit | F10: quit"
            .to_string()
    } else {
        let actions = app
            .actions
            .rank_top_n(&app.action_context(), FOOTER_ACTIONS_LIMIT);
        if actions.is_empty() {
            "No available actions in this context".to_string()
        } else {
            actions
                .iter()
                .enumerate()
                .map(|(index, action)| format!("{}:{} ", index + 1, action.title))
                .collect::<Vec<_>>()
                .join("| ")
        }
    };
    let footer = Paragraph::new(vec![
        Line::from(footer_line),
        Line::from(format!("Status: {}", app.status_line)),
    ])
    .block(Block::default().borders(Borders::ALL).title("Next Actions"));
    frame.render_widget(footer, chunks[2]);

    if app.show_palette {
        render_palette_popup(frame, app);
    }
    if app.show_help {
        render_help_popup(frame);
    }
    if app.exit_confirmation {
        render_exit_popup(frame);
    }
    if app.error_panel.is_some() {
        render_error_popup(frame, app);
    }
}

fn render_help_popup(frame: &mut Frame<'_>) {
    let area = centered_rect(70, 60, frame.area());
    frame.render_widget(Clear, area);
    let help = Paragraph::new(vec![
        Line::from("Global keymap"),
        Line::from("F10: quit immediately"),
        Line::from("F6: go to connection wizard"),
        Line::from("?: toggle help"),
        Line::from("Tab: cycle panes"),
        Line::from("Connection wizard: E/Enter edit, F5 connect"),
        Line::from("Enter: run query (Query Editor)"),
        Line::from("F2: toggle perf overlay"),
        Line::from("F3: toggle safe mode"),
        Line::from("Ctrl+P: command palette"),
        Line::from("Ctrl+U: clear current input"),
        Line::from("Ctrl+C: cancel query (or request exit if idle)"),
        Line::from("Arrows (or Alt+h/j/k/l): navigation"),
        Line::from("1..7: invoke ranked action slot"),
    ])
    .block(Block::default().borders(Borders::ALL).title("Help"));
    frame.render_widget(help, area);
}

fn render_exit_popup(frame: &mut Frame<'_>) {
    let area = centered_rect(50, 25, frame.area());
    frame.render_widget(Clear, area);
    let content = Paragraph::new(vec![
        Line::from("Exit myr now?"),
        Line::from(""),
        Line::from("Ctrl+C: confirm exit"),
        Line::from("F10: exit now"),
        Line::from("Esc: cancel and return"),
    ])
    .block(Block::default().borders(Borders::ALL).title("Confirm Exit"))
    .alignment(Alignment::Center);
    frame.render_widget(content, area);
}

fn render_error_popup(frame: &mut Frame<'_>, app: &TuiApp) {
    let Some(panel) = app.error_panel.as_ref() else {
        return;
    };

    let area = centered_rect(76, 50, frame.area());
    frame.render_widget(Clear, area);

    let mut lines = vec![
        Line::from(Span::styled(
            panel.summary.as_str(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(format!("Detail: {}", panel.detail)),
        Line::from(""),
        Line::from("Recovery actions:"),
    ];

    if panel.kind == ErrorKind::Query && app.last_failed_query.is_some() {
        lines.push(Line::from("1 or Enter: retry last query"));
    }
    if app.can_reconnect_from_error_panel() {
        lines.push(Line::from("F5: reconnect now"));
    }
    lines.push(Line::from("F6: open connection wizard"));
    lines.push(Line::from("Esc: dismiss panel"));

    let error_panel = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(panel.title.as_str()),
    );
    frame.render_widget(error_panel, area);
}

fn render_palette_popup(frame: &mut Frame<'_>, app: &TuiApp) {
    let area = centered_rect(70, 60, frame.area());
    frame.render_widget(Clear, area);

    let entries = app.palette_entries();
    let mut lines = vec![
        Line::from("Command Palette"),
        Line::from(format!("Query: {}", app.palette_query)),
        Line::from(""),
    ];

    if entries.is_empty() {
        lines.push(Line::from("No actions match current query"));
    } else {
        for (index, action_id) in entries.iter().take(10).enumerate() {
            let title = app
                .actions
                .registry()
                .find(*action_id)
                .map_or("unknown action", |action| action.title);
            let marker = if index == app.palette_selection {
                ">"
            } else {
                " "
            };
            lines.push(Line::from(format!("{marker} {title}")));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(
        "Type to filter, arrows to navigate, Enter to run",
    ));

    let palette = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title("Palette (Ctrl+P / Esc)"),
    );
    frame.render_widget(palette, area);
}

fn centered_rect(width_percent: u16, height_percent: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100_u16 - height_percent) / 2),
            Constraint::Percentage(height_percent),
            Constraint::Percentage((100_u16 - height_percent) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100_u16 - width_percent) / 2),
            Constraint::Percentage(width_percent),
            Constraint::Percentage((100_u16 - width_percent) / 2),
        ])
        .split(vertical[1])[1]
}

fn spinner_char(tick: usize) -> char {
    const FRAMES: [char; 4] = ['|', '/', '-', '\\'];
    FRAMES[tick % FRAMES.len()]
}

fn pulse_char(tick: usize) -> char {
    const FRAMES: [char; 4] = ['.', 'o', 'O', 'o'];
    FRAMES[tick % FRAMES.len()]
}

fn connection_badge_and_marker(connection_state: &str, tick: usize) -> (&'static str, char) {
    match connection_state {
        "CONNECTED" => ("[+]", pulse_char(tick)),
        "CONNECTING" | "RECONNECTING" => ("[~]", spinner_char(tick)),
        _ => ("[x]", if tick % 2 == 0 { '-' } else { ' ' }),
    }
}

fn run_connect_worker(profile: ConnectionProfile) -> ConnectWorkerOutcome {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            return ConnectWorkerOutcome::Failure(format!(
                "failed to create runtime: {error}"
            ));
        }
    };

    runtime.block_on(async move {
        let mut manager = ConnectionManager::new(MysqlConnectionBackend);
        let connect_latency = match tokio::time::timeout(
            CONNECT_TIMEOUT,
            manager.connect(profile.clone()),
        )
        .await
        {
            Ok(Ok(latency)) => latency,
            Ok(Err(error)) => return ConnectWorkerOutcome::Failure(error.to_string()),
            Err(_) => {
                return ConnectWorkerOutcome::Failure(format!(
                    "connect timed out after {:.1?}",
                    CONNECT_TIMEOUT
                ));
            }
        };

        let mut warnings = Vec::new();
        match tokio::time::timeout(CONNECT_TIMEOUT, manager.disconnect()).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => warnings.push(format!("disconnect warning: {error}")),
            Err(_) => warnings.push(format!(
                "disconnect timed out after {:.1?}",
                CONNECT_TIMEOUT
            )),
        }

        let data_backend = MysqlDataBackend::from_profile(&profile);
        let mut schema_cache = SchemaCacheService::new(data_backend.clone(), Duration::from_secs(10));
        let databases = match tokio::time::timeout(CONNECT_TIMEOUT, schema_cache.list_databases()).await {
            Ok(Ok(databases)) => databases,
            Ok(Err(error)) => {
                warnings.push(format!("schema fetch failed: {error}"));
                Vec::new()
            }
            Err(_) => {
                warnings.push(format!(
                    "schema fetch timed out after {:.1?}",
                    CONNECT_TIMEOUT
                ));
                Vec::new()
            }
        };

        if let Err(error) = data_backend.disconnect().await {
            warnings.push(format!("schema backend disconnect warning: {error}"));
        }

        ConnectWorkerOutcome::Success {
            profile,
            connect_latency,
            databases,
            warning: (!warnings.is_empty()).then(|| warnings.join("; ")),
        }
    })
}

fn run_query_worker(
    backend: MysqlDataBackend,
    sql: String,
    cancellation: CancellationToken,
) -> QueryWorkerOutcome {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            return QueryWorkerOutcome::Failure(format!("failed to create runtime: {error}"));
        }
    };

    let runner = QueryRunner::new(backend);
    let mut results = ResultsRingBuffer::new(RESULT_BUFFER_CAPACITY);
    match runtime.block_on(async {
        tokio::time::timeout(
            QUERY_TIMEOUT,
            runner.execute_streaming(&sql, &mut results, &cancellation),
        )
        .await
    }) {
        Ok(Ok(summary)) => QueryWorkerOutcome::Success {
            results,
            rows_streamed: summary.rows_streamed,
            was_cancelled: summary.was_cancelled,
            elapsed: summary.elapsed,
        },
        Ok(Err(error)) => QueryWorkerOutcome::Failure(error.to_string()),
        Err(_) => {
            cancellation.cancel();
            QueryWorkerOutcome::Failure(format!("query timed out after {:.1?}", QUERY_TIMEOUT))
        }
    }
}

fn is_transient_query_error(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    [
        "timed out",
        "timeout",
        "temporary",
        "connection reset",
        "connection refused",
        "connection closed",
        "broken pipe",
        "server has gone away",
        "lost connection",
        "pool was disconnect",
        "i/o error",
        "io error",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn is_connection_lost_error(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    [
        "pool was disconnect",
        "server has gone away",
        "lost connection",
        "connection reset",
        "connection refused",
        "connection closed",
        "broken pipe",
        "not connected",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn quote_identifier(identifier: &str) -> String {
    format!("`{}`", identifier.replace('`', "``"))
}

fn candidate_key_column(columns: &[String]) -> Option<String> {
    if let Some(column) = columns
        .iter()
        .find(|column| column.eq_ignore_ascii_case("id"))
    {
        return Some(column.clone());
    }

    columns
        .iter()
        .find(|column| column.to_ascii_lowercase().ends_with("_id"))
        .cloned()
}

fn extract_key_bounds(
    results: &ResultsRingBuffer<QueryRow>,
    columns: &[String],
    key_column: &str,
) -> (Option<String>, Option<String>) {
    let Some(key_index) = columns
        .iter()
        .position(|column| column.eq_ignore_ascii_case(key_column))
    else {
        return (None, None);
    };

    let first = results
        .get(0)
        .and_then(|row| row.values.get(key_index))
        .cloned();
    let last = results
        .len()
        .checked_sub(1)
        .and_then(|index| results.get(index))
        .and_then(|row| row.values.get(key_index))
        .cloned();
    (first, last)
}

fn export_file_path(extension: &str) -> PathBuf {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    std::env::temp_dir().join(format!("myr-export-{timestamp}.{extension}"))
}

fn block_on_result<T, E, F>(future: F) -> Result<T, String>
where
    E: std::fmt::Display,
    F: std::future::Future<Output = Result<T, E>>,
{
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("failed to create runtime: {error}"))?;

    runtime.block_on(future).map_err(|error| error.to_string())
}

fn map_key_event(key: KeyEvent) -> Option<Msg> {
    if key.modifiers == KeyModifiers::CONTROL {
        return match key.code {
            KeyCode::Char('p') => Some(Msg::TogglePalette),
            KeyCode::Char('u') => Some(Msg::ClearInput),
            KeyCode::Char('c') => Some(Msg::CancelQuery),
            _ => None,
        };
    }

    if key.modifiers == KeyModifiers::ALT {
        return match key.code {
            KeyCode::Char('k') => Some(Msg::Navigate(DirectionKey::Up)),
            KeyCode::Char('j') => Some(Msg::Navigate(DirectionKey::Down)),
            KeyCode::Char('h') => Some(Msg::Navigate(DirectionKey::Left)),
            KeyCode::Char('l') => Some(Msg::Navigate(DirectionKey::Right)),
            _ => None,
        };
    }

    match key.code {
        KeyCode::Char('?') => Some(Msg::ToggleHelp),
        KeyCode::Esc => Some(Msg::TogglePalette),
        KeyCode::Tab => Some(Msg::NextPane),
        KeyCode::F(5) => Some(Msg::Connect),
        KeyCode::F(6) => Some(Msg::GoConnectionWizard),
        KeyCode::F(10) => Some(Msg::Quit),
        KeyCode::F(2) => Some(Msg::TogglePerfOverlay),
        KeyCode::F(3) => Some(Msg::ToggleSafeMode),
        KeyCode::Enter => Some(Msg::Submit),
        KeyCode::Backspace => Some(Msg::Backspace),
        KeyCode::Up => Some(Msg::Navigate(DirectionKey::Up)),
        KeyCode::Down => Some(Msg::Navigate(DirectionKey::Down)),
        KeyCode::Left => Some(Msg::Navigate(DirectionKey::Left)),
        KeyCode::Right => Some(Msg::Navigate(DirectionKey::Right)),
        KeyCode::Char('1') => Some(Msg::InvokeActionSlot(0)),
        KeyCode::Char('2') => Some(Msg::InvokeActionSlot(1)),
        KeyCode::Char('3') => Some(Msg::InvokeActionSlot(2)),
        KeyCode::Char('4') => Some(Msg::InvokeActionSlot(3)),
        KeyCode::Char('5') => Some(Msg::InvokeActionSlot(4)),
        KeyCode::Char('6') => Some(Msg::InvokeActionSlot(5)),
        KeyCode::Char('7') => Some(Msg::InvokeActionSlot(6)),
        KeyCode::Char(ch) => Some(Msg::InputChar(ch)),
        _ => None,
    }
}

#[cfg(test)]
fn suggest_limit_in_editor(query: &str) -> Option<String> {
    myr_core::actions_engine::suggest_preview_limit(query, 200)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use myr_core::actions_engine::CopyTarget;
    use myr_core::profiles::ConnectionProfile;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::{
        candidate_key_column, centered_rect, connection_badge_and_marker, extract_key_bounds,
        is_connection_lost_error, is_transient_query_error, map_key_event, quote_identifier,
        render, suggest_limit_in_editor, ActionId, ActionInvocation, AppView, ConnectIntent,
        DirectionKey, ErrorKind, Msg, MysqlDataBackend, PaginationPlan, Pane, QueryRow,
        QueryWorkerOutcome, ResultsRingBuffer, SchemaLane, TuiApp, WizardField,
        QUERY_DURATION_TICKS, QUERY_RETRY_LIMIT,
    };

    fn app_in_pane(pane: Pane) -> TuiApp {
        TuiApp {
            pane,
            ..TuiApp::default()
        }
    }

    fn drive_demo_query_to_completion(app: &mut TuiApp) {
        for _ in 0..=QUERY_DURATION_TICKS {
            app.on_tick();
        }
    }

    fn drive_connect_to_completion(app: &mut TuiApp) {
        for _ in 0..300 {
            if !app.connect_requested || app.status_line.starts_with("Connect failed:") {
                break;
            }
            app.on_tick();
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn render_once(app: &TuiApp) {
        let backend = TestBackend::new(120, 40);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal.draw(|frame| render(frame, app)).expect("render");
    }

    fn drive_query_worker_to_completion(app: &mut TuiApp) {
        for _ in 0..500 {
            if !app.query_running {
                break;
            }
            app.on_tick();
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn mysql_tui_integration_enabled() -> bool {
        matches!(
            std::env::var("MYR_RUN_TUI_MYSQL_INTEGRATION").ok().as_deref(),
            Some("1")
        )
    }

    fn mysql_integration_profile(database: Option<&str>) -> ConnectionProfile {
        let host = std::env::var("MYR_TEST_DB_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let user = std::env::var("MYR_TEST_DB_USER").unwrap_or_else(|_| "root".to_string());
        let port = std::env::var("MYR_TEST_DB_PORT")
            .ok()
            .and_then(|raw| raw.parse::<u16>().ok())
            .unwrap_or(3306);

        let mut profile = ConnectionProfile::new("tui-integration", host, user);
        profile.port = port;
        profile.database = database.map(str::to_string);
        profile
    }

    #[test]
    fn pane_cycles_in_expected_order() {
        assert_eq!(Pane::ConnectionWizard.next(), Pane::SchemaExplorer);
        assert_eq!(Pane::SchemaExplorer.next(), Pane::Results);
        assert_eq!(Pane::Results.next(), Pane::QueryEditor);
        assert_eq!(Pane::QueryEditor.next(), Pane::SchemaExplorer);
    }

    #[test]
    fn pane_tab_index_matches_current_pane() {
        assert_eq!(app_in_pane(Pane::ConnectionWizard).pane_tab_index(), 0);
        assert_eq!(app_in_pane(Pane::SchemaExplorer).pane_tab_index(), 1);
        assert_eq!(app_in_pane(Pane::Results).pane_tab_index(), 2);
        assert_eq!(app_in_pane(Pane::QueryEditor).pane_tab_index(), 3);
    }

    #[test]
    fn pane_switch_triggers_tab_flash_animation() {
        let mut app = app_in_pane(Pane::SchemaExplorer);
        assert_eq!(app.pane_flash_ticks, 0);

        app.handle(Msg::NextPane);

        assert_eq!(app.pane, Pane::Results);
        assert!(app.pane_flash_ticks > 0);

        let before_tick = app.pane_flash_ticks;
        app.on_tick();
        assert!(app.pane_flash_ticks < before_tick);
    }

    #[test]
    fn connection_badges_reflect_state() {
        assert_eq!(connection_badge_and_marker("CONNECTED", 0).0, "[+]");
        assert_eq!(connection_badge_and_marker("CONNECTING", 0).0, "[~]");
        assert_eq!(connection_badge_and_marker("RECONNECTING", 0).0, "[~]");
        assert_eq!(connection_badge_and_marker("DISCONNECTED", 0).0, "[x]");
    }

    #[test]
    fn runtime_and_connection_labels_reflect_state() {
        let mut app = TuiApp::default();
        assert_eq!(app.runtime_state_label(), "IDLE");
        assert_eq!(app.connection_state_label(), "DISCONNECTED");

        app.query_running = true;
        assert_eq!(app.runtime_state_label(), "BUSY");
        app.query_running = false;

        app.connect_requested = true;
        assert_eq!(app.connection_state_label(), "CONNECTING");
        app.connect_intent = ConnectIntent::AutoReconnect;
        assert_eq!(app.connection_state_label(), "RECONNECTING");

        app.connect_requested = false;
        let profile = ConnectionProfile::new(
            "test".to_string(),
            "127.0.0.1".to_string(),
            "root".to_string(),
        );
        app.data_backend = Some(MysqlDataBackend::from_profile(&profile));
        assert_eq!(app.connection_state_label(), "CONNECTED");
    }

    #[test]
    fn query_error_classification_detects_transient_and_disconnect_signals() {
        assert!(is_transient_query_error("query timed out after 20s"));
        assert!(is_transient_query_error("Connection reset by peer"));
        assert!(is_connection_lost_error("Pool was disconnected"));
        assert!(is_connection_lost_error("server has gone away"));
        assert!(!is_connection_lost_error("syntax error near `FROM`"));
    }

    #[test]
    fn keymap_supports_required_global_keys() {
        assert!(matches!(
            map_key_event(KeyEvent::new(KeyCode::F(10), KeyModifiers::NONE)),
            Some(Msg::Quit)
        ));
        assert!(matches!(
            map_key_event(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
            Some(Msg::InputChar('q'))
        ));
        assert!(matches!(
            map_key_event(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::NONE)),
            Some(Msg::InputChar('h'))
        ));
        assert!(matches!(
            map_key_event(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE)),
            Some(Msg::InputChar('j'))
        ));
        assert!(matches!(
            map_key_event(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::NONE)),
            Some(Msg::InputChar('k'))
        ));
        assert!(matches!(
            map_key_event(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE)),
            Some(Msg::InputChar('l'))
        ));
        assert!(matches!(
            map_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
            Some(Msg::NextPane)
        ));
        assert!(matches!(
            map_key_event(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL)),
            Some(Msg::TogglePalette)
        ));
        assert!(matches!(
            map_key_event(KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE)),
            Some(Msg::Connect)
        ));
        assert!(matches!(
            map_key_event(KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE)),
            Some(Msg::GoConnectionWizard)
        ));
        assert!(map_key_event(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)).is_none());
        assert!(matches!(
            map_key_event(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL)),
            Some(Msg::ClearInput)
        ));
        assert!(matches!(
            map_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(Msg::CancelQuery)
        ));
        assert!(matches!(
            map_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Some(Msg::Submit)
        ));
        assert!(matches!(
            map_key_event(KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE)),
            Some(Msg::TogglePerfOverlay)
        ));
        assert!(matches!(
            map_key_event(KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE)),
            Some(Msg::ToggleSafeMode)
        ));
        assert!(matches!(
            map_key_event(KeyEvent::new(KeyCode::Char('h'), KeyModifiers::ALT)),
            Some(Msg::Navigate(DirectionKey::Left))
        ));
        assert!(matches!(
            map_key_event(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::ALT)),
            Some(Msg::Navigate(DirectionKey::Down))
        ));
        assert!(matches!(
            map_key_event(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::ALT)),
            Some(Msg::Navigate(DirectionKey::Up))
        ));
        assert!(matches!(
            map_key_event(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::ALT)),
            Some(Msg::Navigate(DirectionKey::Right))
        ));
    }

    #[test]
    fn help_and_action_slot_keys_are_mapped() {
        assert!(matches!(
            map_key_event(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE)),
            Some(Msg::ToggleHelp)
        ));
        assert!(matches!(
            map_key_event(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::NONE)),
            Some(Msg::InvokeActionSlot(0))
        ));
    }

    #[test]
    fn limit_suggestion_is_applied_in_editor_helper() {
        let suggested = suggest_limit_in_editor("SELECT * FROM users");
        assert_eq!(suggested, Some("SELECT * FROM users LIMIT 200".to_string()));
    }

    #[test]
    fn unmapped_chars_can_be_used_as_palette_input() {
        assert!(matches!(
            map_key_event(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE)),
            Some(Msg::InputChar('x'))
        ));
    }

    #[test]
    fn schema_lane_switches_with_horizontal_navigation() {
        let mut app = TuiApp {
            pane: Pane::SchemaExplorer,
            schema_lane: SchemaLane::Tables,
            ..TuiApp::default()
        };

        app.navigate(DirectionKey::Right);
        assert_eq!(app.schema_lane, SchemaLane::Columns);

        app.navigate(DirectionKey::Left);
        assert_eq!(app.schema_lane, SchemaLane::Tables);
    }

    #[test]
    fn schema_table_navigation_updates_column_selection() {
        let mut app = TuiApp {
            pane: Pane::SchemaExplorer,
            schema_lane: SchemaLane::Tables,
            ..TuiApp::default()
        };
        app.selection.database = Some("app".to_string());
        app.selection.table = app.schema_tables.first().cloned();
        app.reload_columns_for_selected_table();

        app.navigate(DirectionKey::Down);

        assert_eq!(app.selection.table.as_deref(), Some("sessions"));
        assert_eq!(app.selection.column.as_deref(), Some("id"));
    }

    #[test]
    fn key_column_candidate_prefers_id_then_suffix() {
        let columns = vec![
            "created_at".to_string(),
            "id".to_string(),
            "account_id".to_string(),
        ];
        assert_eq!(candidate_key_column(&columns), Some("id".to_string()));

        let columns = vec!["created_at".to_string(), "account_id".to_string()];
        assert_eq!(
            candidate_key_column(&columns),
            Some("account_id".to_string())
        );
    }

    #[test]
    fn key_bounds_are_extracted_from_first_and_last_rows() {
        let mut results = ResultsRingBuffer::new(10);
        results.push(QueryRow::new(vec!["10".to_string(), "a".to_string()]));
        results.push(QueryRow::new(vec!["11".to_string(), "b".to_string()]));
        results.push(QueryRow::new(vec!["12".to_string(), "c".to_string()]));

        let columns = vec!["id".to_string(), "payload".to_string()];
        let bounds = extract_key_bounds(&results, &columns, "id");
        assert_eq!(bounds, (Some("10".to_string()), Some("12".to_string())));
    }

    #[test]
    fn submit_runs_query_in_demo_mode_and_populates_results() {
        let mut app = app_in_pane(Pane::QueryEditor);
        app.query_editor_text = "SELECT * FROM `app`.`users`".to_string();

        app.submit();
        assert!(app.query_running);
        assert_eq!(app.pane, Pane::Results);

        drive_demo_query_to_completion(&mut app);

        assert!(!app.query_running);
        assert!(app.has_results);
        assert!(!app.results.is_empty());
        assert_eq!(app.selection.column.as_deref(), Some("value"));
    }

    #[test]
    fn destructive_submit_requires_confirmation_then_executes() {
        let mut app = app_in_pane(Pane::QueryEditor);
        app.query_editor_text = "DELETE FROM `app`.`users` WHERE id = 1".to_string();

        app.submit();
        assert!(app.pending_confirmation.is_some());
        assert_eq!(app.pane, Pane::QueryEditor);
        assert!(app.status_line.contains("Safe mode confirmation required"));

        app.submit();
        assert!(app.query_running);
        assert_eq!(app.pane, Pane::Results);

        drive_demo_query_to_completion(&mut app);
        assert!(app.has_results);
    }

    #[test]
    fn query_failure_retries_once_when_transient() {
        let mut app = app_in_pane(Pane::QueryEditor);
        app.query_running = true;
        app.inflight_query_sql = Some("SELECT 1".to_string());

        let (tx, rx) = std::sync::mpsc::channel();
        app.query_result_rx = Some(rx);
        tx.send(QueryWorkerOutcome::Failure(
            "connection reset by peer".to_string(),
        ))
        .expect("send test query failure");

        app.poll_query_result();

        assert!(app.query_running);
        assert_eq!(app.query_retry_attempts, 1);
        assert!(app.status_line.starts_with("Running query"));
    }

    #[test]
    fn query_failure_starts_auto_reconnect_when_connection_is_lost() {
        let mut app = app_in_pane(Pane::QueryEditor);
        let profile = ConnectionProfile::new(
            "local".to_string(),
            "127.0.0.1".to_string(),
            "root".to_string(),
        );
        app.data_backend = Some(MysqlDataBackend::from_profile(&profile));
        app.active_connection_profile = Some(profile.clone());
        app.last_connect_profile = Some(profile);
        app.query_running = true;
        app.query_retry_attempts = QUERY_RETRY_LIMIT;
        app.inflight_query_sql = Some("SELECT 1".to_string());

        let (tx, rx) = std::sync::mpsc::channel();
        app.query_result_rx = Some(rx);
        tx.send(QueryWorkerOutcome::Failure("Pool was disconnected".to_string()))
            .expect("send disconnect failure");

        app.poll_query_result();

        assert!(app.connect_requested);
        assert_eq!(app.connect_intent, ConnectIntent::AutoReconnect);
        assert_eq!(app.pending_retry_query.as_deref(), Some("SELECT 1"));
    }

    #[test]
    fn error_panel_primary_action_retries_last_failed_query() {
        let mut app = app_in_pane(Pane::Results);
        app.last_failed_query = Some("SELECT 1".to_string());
        app.open_error_panel(
            ErrorKind::Query,
            "Query Error",
            "Query failed".to_string(),
            "connection lost".to_string(),
        );

        app.handle(Msg::InvokeActionSlot(0));

        assert!(app.error_panel.is_none());
        assert!(app.query_running);
        assert_eq!(app.pane, Pane::Results);
    }

    #[test]
    fn connect_from_wizard_rejects_invalid_port() {
        let mut app = app_in_pane(Pane::ConnectionWizard);
        app.wizard_form.port = "not-a-port".to_string();
        app.connect_from_wizard();

        assert_eq!(app.status_line, "Invalid port in connection wizard");
    }

    #[test]
    fn wizard_input_updates_active_field() {
        let mut app = app_in_pane(Pane::ConnectionWizard);
        app.wizard_form.active_field = WizardField::Host;
        app.wizard_form.host.clear();

        app.handle(Msg::Submit);
        app.handle(Msg::InputChar('d'));
        app.handle(Msg::InputChar('b'));
        app.handle(Msg::Submit);

        assert_eq!(app.wizard_form.host, "db");
        assert_eq!(app.status_line, "Saved Host");
    }

    #[test]
    fn wizard_backspace_updates_active_field() {
        let mut app = app_in_pane(Pane::ConnectionWizard);
        app.wizard_form.active_field = WizardField::Database;
        app.wizard_form.database = "Rfam".to_string();

        app.handle(Msg::Submit);
        app.handle(Msg::Backspace);
        app.handle(Msg::Submit);

        assert_eq!(app.wizard_form.database, "Rfa");
        assert_eq!(app.status_line, "Saved Database");
    }

    #[test]
    fn wizard_escape_cancels_edit_without_mutation() {
        let mut app = app_in_pane(Pane::ConnectionWizard);
        app.wizard_form.active_field = WizardField::Database;
        app.wizard_form.database = "Rfam".to_string();

        app.handle(Msg::Submit);
        app.handle(Msg::Backspace);
        app.handle(Msg::TogglePalette);

        assert_eq!(app.wizard_form.database, "Rfam");
        assert!(!app.wizard_form.editing);
        assert!(!app.show_palette);
        assert_eq!(app.status_line, "Canceled editing Database");
    }

    #[test]
    fn wizard_ctrl_u_clears_edit_buffer() {
        let mut app = app_in_pane(Pane::ConnectionWizard);
        app.wizard_form.active_field = WizardField::Host;
        app.wizard_form.host = "mysql.example.com".to_string();

        app.handle(Msg::Submit);
        app.handle(Msg::ClearInput);

        assert!(app.wizard_form.edit_buffer.is_empty());
        assert_eq!(app.status_line, "Cleared Host");
    }

    #[test]
    fn wizard_digit_slots_input_numbers_while_editing() {
        let mut app = app_in_pane(Pane::ConnectionWizard);
        app.wizard_form.active_field = WizardField::Port;
        app.wizard_form.port.clear();

        app.handle(Msg::Submit);
        app.handle(Msg::InvokeActionSlot(2));
        app.handle(Msg::InvokeActionSlot(0));
        app.handle(Msg::Submit);

        assert_eq!(app.wizard_form.port, "31");
    }

    #[test]
    fn export_results_handles_empty_and_non_empty_buffers() {
        let mut app = app_in_pane(Pane::Results);
        app.export_results(myr_core::actions_engine::ExportFormat::Csv);
        assert_eq!(app.status_line, "No results available to export");

        app.populate_demo_results();
        app.export_results(myr_core::actions_engine::ExportFormat::Csv);
        assert!(app.status_line.starts_with("Exported "));

        app.export_results(myr_core::actions_engine::ExportFormat::Json);
        assert!(app.status_line.starts_with("Exported "));
    }

    #[test]
    fn pagination_keyset_transitions_forward_and_backward() {
        let mut app = app_in_pane(Pane::SchemaExplorer);
        app.selection.database = Some("app".to_string());
        app.selection.table = Some("events".to_string());
        app.schema_columns = vec!["id".to_string(), "value".to_string()];

        app.start_preview_paged_query("SELECT * FROM `app`.`events` LIMIT 200".to_string());
        drive_demo_query_to_completion(&mut app);

        let state = app.pagination_state.as_ref().expect("pagination state");
        assert_eq!(state.page_index, 0);
        assert!(matches!(state.plan, PaginationPlan::Keyset { .. }));

        app.invoke_action(ActionId::NextPage);
        drive_demo_query_to_completion(&mut app);
        assert_eq!(
            app.pagination_state
                .as_ref()
                .expect("pagination state")
                .page_index,
            1
        );

        app.invoke_action(ActionId::PreviousPage);
        drive_demo_query_to_completion(&mut app);
        assert_eq!(
            app.pagination_state
                .as_ref()
                .expect("pagination state")
                .page_index,
            0
        );
    }

    #[test]
    fn pagination_falls_back_to_offset_without_key_column() {
        let mut app = app_in_pane(Pane::SchemaExplorer);
        app.selection.database = Some("app".to_string());
        app.selection.table = Some("events".to_string());
        app.schema_columns = vec!["name".to_string(), "created_at".to_string()];

        app.start_preview_paged_query("SELECT * FROM `app`.`events` LIMIT 200".to_string());
        drive_demo_query_to_completion(&mut app);

        let state = app.pagination_state.as_ref().expect("pagination state");
        assert!(matches!(state.plan, PaginationPlan::Offset));
    }

    #[test]
    fn apply_invocation_handles_non_sql_actions() {
        let mut app = app_in_pane(Pane::Results);
        app.apply_invocation(
            ActionId::ApplyLimit200,
            ActionInvocation::ReplaceQueryEditorText("SELECT 1".to_string()),
        );
        assert_eq!(app.query_editor_text, "SELECT 1");

        app.apply_invocation(
            ActionId::FocusQueryEditor,
            ActionInvocation::OpenView(AppView::SchemaExplorer),
        );
        assert_eq!(app.pane, Pane::SchemaExplorer);

        app.apply_invocation(
            ActionId::CopyCell,
            ActionInvocation::CopyToClipboard(CopyTarget::Cell),
        );
        assert!(app.status_line.contains("Copy requested"));

        app.populate_demo_results();
        app.apply_invocation(
            ActionId::SearchResults,
            ActionInvocation::SearchBufferedResults,
        );
        assert!(app.results_search_mode);
        assert!(
            app.status_line.starts_with("Search results:"),
            "unexpected search status: {}",
            app.status_line
        );
    }

    #[test]
    fn search_action_reports_empty_buffer_without_entering_mode() {
        let mut app = app_in_pane(Pane::Results);
        app.apply_invocation(
            ActionId::SearchResults,
            ActionInvocation::SearchBufferedResults,
        );

        assert!(!app.results_search_mode);
        assert_eq!(app.status_line, "No buffered rows yet");
    }

    #[test]
    fn results_search_mode_finds_and_cycles_matches() {
        let mut app = app_in_pane(Pane::Results);
        app.populate_demo_results();
        app.results_cursor = 10;

        app.apply_invocation(
            ActionId::SearchResults,
            ActionInvocation::SearchBufferedResults,
        );
        assert!(app.results_search_mode);
        assert_eq!(app.pane, Pane::Results);

        app.handle(Msg::InputChar('v'));
        app.handle(Msg::InputChar('a'));
        app.handle(Msg::InputChar('l'));

        assert_eq!(app.results_cursor, 0);
        assert!(
            app.status_line.contains("Search matched row"),
            "unexpected search status: {}",
            app.status_line
        );

        app.handle(Msg::Submit);
        assert_eq!(app.results_cursor, 1);

        app.handle(Msg::TogglePalette);
        assert!(!app.results_search_mode);
        assert_eq!(app.status_line, "Results search canceled");
    }

    #[test]
    fn palette_input_and_selection_paths_update_state() {
        let mut app = app_in_pane(Pane::QueryEditor);
        app.handle(Msg::TogglePalette);
        assert!(app.show_palette);

        app.handle(Msg::InputChar('p'));
        assert_eq!(app.palette_query, "p");

        app.handle(Msg::Navigate(DirectionKey::Down));
        assert!(app.palette_selection <= app.palette_entries().len().saturating_sub(1));

        app.handle(Msg::Backspace);
        assert!(app.palette_query.is_empty());
    }

    #[test]
    fn rendering_covers_all_panes_and_popups() {
        let mut wizard = app_in_pane(Pane::ConnectionWizard);
        wizard.show_help = true;
        render_once(&wizard);

        let mut schema = app_in_pane(Pane::SchemaExplorer);
        schema.show_palette = true;
        schema.palette_query = "prev".to_string();
        render_once(&schema);

        let mut results = app_in_pane(Pane::Results);
        results.populate_demo_results();
        results.pagination_state = Some(super::PaginationState {
            database: Some("app".to_string()),
            table: "users".to_string(),
            page_size: 200,
            page_index: 1,
            last_page_row_count: 200,
            plan: PaginationPlan::Offset,
        });
        render_once(&results);

        let editor = app_in_pane(Pane::QueryEditor);
        render_once(&editor);

        let mut exit_confirm = app_in_pane(Pane::Results);
        exit_confirm.exit_confirmation = true;
        render_once(&exit_confirm);

        let mut error_popup = app_in_pane(Pane::Results);
        error_popup.open_error_panel(
            ErrorKind::Connection,
            "Connection Error",
            "connect failed".to_string(),
            "connection refused".to_string(),
        );
        render_once(&error_popup);
    }

    #[test]
    fn helper_geometry_and_identifier_quote_are_stable() {
        let area = ratatui::layout::Rect::new(0, 0, 100, 40);
        let centered = centered_rect(70, 60, area);
        assert_eq!(centered.width, 70);
        assert_eq!(centered.height, 24);

        assert_eq!(quote_identifier("users"), "`users`");
        assert_eq!(quote_identifier("odd`name"), "`odd``name`");
    }

    #[test]
    fn handle_toggles_and_quit_paths_update_state() {
        let mut app = app_in_pane(Pane::QueryEditor);

        app.handle(Msg::ToggleHelp);
        assert!(app.show_help);

        app.handle(Msg::TogglePerfOverlay);
        assert!(app.show_perf_overlay);

        app.handle(Msg::ToggleSafeMode);
        assert!(!app.safe_mode_guard.is_enabled());

        app.handle(Msg::CancelQuery);
        assert!(!app.cancel_requested);
        assert!(app.exit_confirmation);
        assert!(app
            .status_line
            .starts_with("No active query. Exit myr?"));

        app.handle(Msg::TogglePalette);
        assert!(!app.exit_confirmation);

        app.handle(Msg::Quit);
        assert!(app.should_quit);
        assert!(!app.exit_confirmation);
    }

    #[test]
    fn record_render_updates_fps_after_window_rollover() {
        let mut app = app_in_pane(Pane::Results);
        app.fps_window_started_at = std::time::Instant::now() - std::time::Duration::from_secs(2);
        app.record_render(std::time::Duration::from_millis(12));

        assert!(app.last_render_ms > 0.0);
        assert!(app.fps > 0.0);
        assert_eq!(app.recent_render_count, 0);
    }

    #[test]
    fn submit_in_non_submittable_panes_sets_status() {
        let mut schema = app_in_pane(Pane::SchemaExplorer);
        schema.submit();
        assert_eq!(schema.status_line, "Nothing to submit in this view");

        let mut results = app_in_pane(Pane::Results);
        results.submit();
        assert_eq!(results.status_line, "Nothing to submit in this view");
    }

    #[test]
    fn navigate_results_reports_empty_and_updates_cursor() {
        let mut app = app_in_pane(Pane::Results);
        app.navigate_results(DirectionKey::Down);
        assert_eq!(app.status_line, "No buffered rows yet");

        app.populate_demo_results();
        app.navigate_results(DirectionKey::Down);
        assert!(app.status_line.starts_with("Results cursor:"));
    }

    #[test]
    fn connect_from_wizard_handles_connect_failure_path() {
        let mut app = app_in_pane(Pane::ConnectionWizard);
        app.wizard_form.host = "127.0.0.1".to_string();
        app.wizard_form.port = "1".to_string();
        app.wizard_form.user = "root".to_string();

        app.connect_from_wizard();
        drive_connect_to_completion(&mut app);
        assert!(!app.connect_requested);
        assert!(app.status_line.starts_with("Connect failed:"));
    }

    #[test]
    fn mysql_query_path_streams_rows_when_enabled() {
        if !mysql_tui_integration_enabled() {
            return;
        }

        let database =
            std::env::var("MYR_TEST_DB_DATABASE").unwrap_or_else(|_| "myr_bench".to_string());
        let profile = mysql_integration_profile(Some(&database));

        let mut app = app_in_pane(Pane::QueryEditor);
        app.data_backend = Some(MysqlDataBackend::from_profile(&profile));
        app.selection.database = Some(database.clone());
        app.selection.table = Some("events".to_string());
        app.selection.column = Some("id".to_string());
        app.query_editor_text =
            format!("SELECT id, user_id FROM `{database}`.`events` ORDER BY id LIMIT 5");

        app.submit();
        drive_query_worker_to_completion(&mut app);

        assert!(
            !app.query_running,
            "query did not complete; status was: {}",
            app.status_line
        );
        assert!(
            !app.results.is_empty(),
            "query returned no buffered rows; status was: {}",
            app.status_line
        );
        assert!(
            app.status_line.starts_with("Query returned"),
            "expected successful query status, got: {}",
            app.status_line
        );
    }

    #[test]
    fn mysql_query_path_survives_schema_cache_activity_when_enabled() {
        if !mysql_tui_integration_enabled() {
            return;
        }

        let database =
            std::env::var("MYR_TEST_DB_DATABASE").unwrap_or_else(|_| "myr_bench".to_string());
        let profile = mysql_integration_profile(Some(&database));

        let mut app = app_in_pane(Pane::SchemaExplorer);
        app.apply_connected_profile(
            profile.clone(),
            Duration::from_millis(1),
            vec![database],
            None,
        );

        app.pane = Pane::QueryEditor;
        app.query_editor_text = format!(
            "SELECT id, user_id FROM `{}`.`events` ORDER BY id LIMIT 5",
            app.selection.database.as_deref().unwrap_or("myr_bench")
        );
        app.submit();
        drive_query_worker_to_completion(&mut app);

        assert!(
            !app.query_running,
            "query did not complete; status was: {}",
            app.status_line
        );
        assert!(
            !app.results.is_empty(),
            "query returned no buffered rows; status was: {}",
            app.status_line
        );
        assert!(
            app.status_line.starts_with("Query returned"),
            "expected successful query status, got: {}",
            app.status_line
        );
    }

    #[test]
    fn connect_message_queues_request_and_tick_executes_it() {
        let mut app = app_in_pane(Pane::ConnectionWizard);
        app.wizard_form.port = "not-a-port".to_string();

        app.handle(Msg::Connect);
        assert!(!app.connect_requested);
        assert_eq!(app.status_line, "Invalid port in connection wizard");
    }

    #[test]
    fn connect_message_in_query_editor_does_not_run_query() {
        let mut app = app_in_pane(Pane::QueryEditor);
        app.query_editor_text = "SELECT 1".to_string();

        app.handle(Msg::Connect);

        assert!(!app.query_running);
        assert_eq!(
            app.status_line,
            "Connect is only available in connection wizard"
        );
    }

    #[test]
    fn go_connection_wizard_switches_from_any_pane() {
        let mut app = app_in_pane(Pane::QueryEditor);

        app.handle(Msg::GoConnectionWizard);

        assert_eq!(app.pane, Pane::ConnectionWizard);
        assert_eq!(app.status_line, "Returned to Connection Wizard");
    }

    #[test]
    fn esc_path_cancels_exit_confirmation() {
        let mut app = app_in_pane(Pane::Results);

        app.handle(Msg::CancelQuery);
        assert!(app.exit_confirmation);

        app.handle(Msg::TogglePalette);
        assert!(!app.exit_confirmation);
        assert_eq!(app.status_line, "Exit canceled");
    }

    #[test]
    fn esc_path_dismisses_error_panel() {
        let mut app = app_in_pane(Pane::Results);
        app.open_error_panel(
            ErrorKind::Query,
            "Query Error",
            "Query failed".to_string(),
            "network".to_string(),
        );

        app.handle(Msg::TogglePalette);

        assert!(app.error_panel.is_none());
        assert_eq!(app.status_line, "Error panel dismissed");
    }

    #[test]
    fn ctrl_c_path_can_confirm_exit_when_no_query_is_running() {
        let mut app = app_in_pane(Pane::Results);

        app.handle(Msg::CancelQuery);
        assert!(app.exit_confirmation);
        assert!(!app.should_quit);

        app.handle(Msg::CancelQuery);
        assert!(app.should_quit);
    }
}
