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
use myr_adapters::export::{
    export_rows_to_csv, export_rows_to_csv_with_options, export_rows_to_json,
    export_rows_to_json_with_options, ExportCompression, JsonExportFormat,
};
use myr_adapters::mysql::{MysqlConnectionBackend, MysqlDataBackend};
use myr_core::actions_engine::{
    ActionContext, ActionId, ActionInvocation, ActionsEngine, AppView, SchemaSelection,
};
use myr_core::audit_trail::{unix_timestamp_millis, AuditOutcome, AuditRecord, FileAuditTrail};
use myr_core::bookmarks::{FileBookmarksStore, SavedBookmark};
use myr_core::connection_manager::ConnectionManager;
use myr_core::profiles::{ConnectionProfile, FileProfilesStore, PasswordSource, TlsMode};
use myr_core::query_runner::{CancellationToken, QueryRow, QueryRunner};
use myr_core::results_buffer::ResultsRingBuffer;
use myr_core::safe_mode::{assess_sql_safety, ConfirmationToken, GuardDecision, SafeModeGuard};
use myr_core::schema_cache::{RelationshipDirection, SchemaCacheService, TableRelationship};
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
const AUDIT_SQL_MAX_CHARS: usize = 1_000;
const AUDIT_ERROR_MAX_CHARS: usize = 400;
const BOOKMARK_NAME_MAX_CHARS: usize = 64;

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
    PasswordSource,
    Database,
    TlsMode,
    ReadOnly,
}

impl WizardField {
    fn next(self) -> Self {
        match self {
            Self::ProfileName => Self::Host,
            Self::Host => Self::Port,
            Self::Port => Self::User,
            Self::User => Self::PasswordSource,
            Self::PasswordSource => Self::Database,
            Self::Database => Self::TlsMode,
            Self::TlsMode => Self::ReadOnly,
            Self::ReadOnly => Self::ProfileName,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::ProfileName => "Profile",
            Self::Host => "Host",
            Self::Port => "Port",
            Self::User => "User",
            Self::PasswordSource => "Password source",
            Self::Database => "Database",
            Self::TlsMode => "TLS mode",
            Self::ReadOnly => "Read-only",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConnectionWizardForm {
    profile_name: String,
    host: String,
    port: String,
    user: String,
    password_source: String,
    database: String,
    tls_mode: String,
    read_only: String,
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
            password_source: "env".to_string(),
            database: "app".to_string(),
            tls_mode: "prefer".to_string(),
            read_only: "no".to_string(),
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
    InsertNewline,
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
    schema_relationships: Vec<TableRelationship>,
    selected_relationship_index: usize,
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
    audit_trail: Option<FileAuditTrail>,
    bookmark_store: Option<FileBookmarksStore>,
    bookmark_cycle_index: usize,
    query_editor_text: String,
    query_cursor: usize,
    query_history: Vec<String>,
    query_history_index: Option<usize>,
    query_history_draft: Option<String>,
    selection: SchemaSelection,
}

mod app_logic;
mod lib_helpers;
mod rendering;

#[cfg(test)]
use app_logic::wizard_form_from_profile;
pub(crate) use lib_helpers::*;
#[cfg(test)]
use rendering::{centered_rect, connection_badge_and_marker};
use rendering::{demo_relationships, relationship_direction_label, render, spinner_char};

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

#[cfg(test)]
mod tests;
