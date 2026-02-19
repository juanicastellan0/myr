use std::io::{self, Stdout};
use std::path::PathBuf;
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
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::{Frame, Terminal};
use thiserror::Error;

const TICK_RATE: Duration = Duration::from_millis(120);
const QUERY_DURATION_TICKS: u8 = 10;
const FOOTER_ACTIONS_LIMIT: usize = 7;
const RESULT_BUFFER_CAPACITY: usize = 2_000;

const DEMO_SCHEMA_TABLES: [&str; 4] = ["users", "sessions", "playlists", "events"];

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
    Tick,
}

#[derive(Debug)]
struct TuiApp {
    actions: ActionsEngine,
    pane: Pane,
    wizard_form: ConnectionWizardForm,
    connected_profile: Option<String>,
    connection_manager: Option<ConnectionManager<MysqlConnectionBackend>>,
    data_backend: Option<MysqlDataBackend>,
    schema_cache: Option<SchemaCacheService<MysqlDataBackend>>,
    schema_databases: Vec<String>,
    active_database: Option<String>,
    schema_tables: Vec<String>,
    selected_table_index: usize,
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
    results: ResultsRingBuffer<QueryRow>,
    cancel_requested: bool,
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
            connection_manager: None,
            data_backend: None,
            schema_cache: None,
            schema_databases: Vec::new(),
            active_database: None,
            schema_tables: DEMO_SCHEMA_TABLES
                .iter()
                .map(|table| (*table).to_string())
                .collect(),
            selected_table_index: 0,
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
            results: ResultsRingBuffer::new(RESULT_BUFFER_CAPACITY),
            cancel_requested: false,
            status_line: "Fill connection details and press Enter to connect".to_string(),
            query_editor_text: "SELECT * FROM users".to_string(),
            selection: SchemaSelection {
                database: None,
                table: None,
                column: Some("email".to_string()),
            },
        }
    }
}

impl TuiApp {
    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Quit => self.should_quit = true,
            Msg::ToggleHelp => self.show_help = !self.show_help,
            Msg::NextPane => {
                if self.pane == Pane::ConnectionWizard {
                    self.wizard_form.active_field = self.wizard_form.active_field.next();
                    self.status_line =
                        format!("Wizard field: {}", self.wizard_form.active_field.label());
                } else {
                    self.pane = self.pane.next();
                    self.status_line = format!("Switched pane to {}", self.pane_name());
                }
            }
            Msg::TogglePalette => {
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
            Msg::CancelQuery => {
                self.cancel_requested = true;
                self.query_running = false;
                self.query_ticks_remaining = 0;
                self.status_line = "Cancel requested".to_string();
            }
            Msg::Navigate(direction) => self.navigate(direction),
            Msg::InvokeActionSlot(index) => self.invoke_ranked_action(index),
            Msg::InputChar(ch) => self.handle_input_char(ch),
            Msg::Backspace => self.handle_backspace(),
            Msg::Tick => self.on_tick(),
        }
    }

    fn on_tick(&mut self) {
        if self.query_running && self.data_backend.is_none() {
            if self.query_ticks_remaining == 0 {
                self.query_running = false;
                self.populate_demo_results();
                self.status_line = "Query completed".to_string();
            } else {
                self.query_ticks_remaining = self.query_ticks_remaining.saturating_sub(1);
            }
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
            Pane::ConnectionWizard => self.connect_from_wizard(),
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

    fn connect_from_wizard(&mut self) {
        let port = match self.wizard_form.port.parse::<u16>() {
            Ok(port) => port,
            Err(_) => {
                self.status_line = "Invalid port in connection wizard".to_string();
                return;
            }
        };

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

        let connection_backend = MysqlConnectionBackend;
        let mut manager = ConnectionManager::new(connection_backend);
        let connect_latency = match block_on_result(manager.connect(profile.clone())) {
            Ok(latency) => latency,
            Err(error) => {
                self.status_line = format!("Connect failed: {error}");
                return;
            }
        };

        let data_backend = MysqlDataBackend::from_profile(&profile);
        let mut schema_cache =
            SchemaCacheService::new(data_backend.clone(), Duration::from_secs(10));
        let databases = match block_on_result(schema_cache.list_databases()) {
            Ok(databases) => databases,
            Err(error) => {
                self.status_line = format!("Connected, but schema fetch failed: {error}");
                Vec::new()
            }
        };

        let mut active_database = profile.database.clone();
        if active_database.is_none() {
            active_database = databases.first().cloned();
        }

        let tables = if let Some(database_name) = active_database.as_deref() {
            match block_on_result(schema_cache.list_tables(database_name)) {
                Ok(tables) => tables,
                Err(error) => {
                    self.status_line = format!("Connected, but table fetch failed: {error}");
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

        match FileProfilesStore::load_default() {
            Ok(mut store) => {
                store.upsert_profile(profile.clone());
                if let Err(error) = store.persist() {
                    self.status_line = format!(
                        "Connected in {:.1?} (profile save failed: {error})",
                        connect_latency
                    );
                } else {
                    self.status_line =
                        format!("Connected as `{}` in {:.1?}", profile.name, connect_latency);
                }
            }
            Err(error) => {
                self.status_line = format!(
                    "Connected in {:.1?} (profile load failed: {error})",
                    connect_latency
                );
            }
        }

        self.connection_manager = Some(manager);
        self.data_backend = Some(data_backend);
        self.schema_cache = Some(schema_cache);
        self.schema_databases = databases;
        self.active_database = active_database.clone();
        self.connected_profile = Some(profile.name.clone());
        self.selection.database = active_database;
        self.schema_tables = tables;
        self.selected_table_index = 0;
        self.selection.table = self.schema_tables.first().cloned();
        self.selection.column = Some("id".to_string());
        if let Some(table) = &self.selection.table {
            self.query_editor_text = format!("SELECT * FROM `{}`", table.replace('`', "``"));
        }
        self.pane = Pane::SchemaExplorer;
    }

    fn navigate(&mut self, direction: DirectionKey) {
        if self.show_palette {
            self.navigate_palette(direction);
            return;
        }

        match self.pane {
            Pane::ConnectionWizard => {
                if matches!(direction, DirectionKey::Left | DirectionKey::Up) {
                    self.wizard_form.active_field = self.previous_wizard_field();
                } else {
                    self.wizard_form.active_field = self.wizard_form.active_field.next();
                }
                self.status_line =
                    format!("Wizard field: {}", self.wizard_form.active_field.label());
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
        if self.schema_tables.is_empty() {
            self.status_line = "No tables available".to_string();
            return;
        }

        match direction {
            DirectionKey::Up | DirectionKey::Left => {
                self.selected_table_index = self.selected_table_index.saturating_sub(1);
            }
            DirectionKey::Down | DirectionKey::Right => {
                let max_index = self.schema_tables.len() - 1;
                self.selected_table_index = (self.selected_table_index + 1).min(max_index);
            }
        }

        self.selection.table = self.schema_tables.get(self.selected_table_index).cloned();
        if let Some(table) = &self.selection.table {
            self.status_line = format!("Selected table `{table}`");
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

    fn populate_demo_results(&mut self) {
        self.results = ResultsRingBuffer::new(RESULT_BUFFER_CAPACITY);
        self.results_cursor = 0;
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
        } else if self.pane == Pane::QueryEditor {
            self.query_editor_text.pop();
            self.status_line = "Query text updated".to_string();
        }
    }

    fn invoke_ranked_action(&mut self, index: usize) {
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
            Ok(invocation) => self.apply_invocation(invocation),
            Err(error) => self.status_line = format!("Action error: {error}"),
        }
    }

    fn start_query(&mut self, sql: String) {
        self.query_editor_text = sql;
        self.pane = Pane::Results;
        self.cancel_requested = false;
        self.has_results = false;

        if let Some(data_backend) = &self.data_backend {
            self.query_running = true;
            self.query_ticks_remaining = 0;
            self.results = ResultsRingBuffer::new(RESULT_BUFFER_CAPACITY);
            let runner = QueryRunner::new(data_backend.clone());
            let cancellation = CancellationToken::new();

            match block_on_result(runner.execute_streaming(
                &self.query_editor_text,
                &mut self.results,
                &cancellation,
            )) {
                Ok(summary) => {
                    self.query_running = false;
                    self.has_results = !self.results.is_empty();
                    self.results_cursor = 0;
                    self.status_line = format!(
                        "Query returned {} rows in {:.1?}",
                        summary.rows_streamed, summary.elapsed
                    );
                }
                Err(error) => {
                    self.query_running = false;
                    self.status_line = format!("Query failed: {error}");
                }
            }
            return;
        }

        self.query_running = true;
        self.query_ticks_remaining = QUERY_DURATION_TICKS;
        self.status_line = "Running query...".to_string();
    }

    fn apply_invocation(&mut self, invocation: ActionInvocation) {
        match invocation {
            ActionInvocation::RunSql(sql) => match self.safe_mode_guard.evaluate(&sql) {
                GuardDecision::Allow { .. } => {
                    self.pending_confirmation = None;
                    self.start_query(sql);
                }
                GuardDecision::RequireConfirmation { token, assessment } => {
                    self.pending_confirmation = Some((token, sql.clone()));
                    self.query_editor_text = sql;
                    self.pane = Pane::QueryEditor;
                    self.status_line = format!(
                        "Safe mode confirmation required: {:?}. Press Enter again to confirm.",
                        assessment.reasons
                    );
                }
            },
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
                self.pane = match view {
                    AppView::ConnectionWizard => Pane::ConnectionWizard,
                    AppView::SchemaExplorer => Pane::SchemaExplorer,
                    AppView::Results => Pane::Results,
                    AppView::QueryEditor => Pane::QueryEditor,
                    AppView::CommandPalette => self.pane,
                };
                self.status_line = format!("Switched view to {}", self.pane_name());
            }
            ActionInvocation::SearchBufferedResults => {
                self.status_line = "Search requested (placeholder)".to_string();
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

        ActionContext {
            view,
            selection: self.selection.clone(),
            query_text,
            query_running: self.query_running,
            has_results: self.has_results,
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
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let latency_text = app
        .connection_manager
        .as_ref()
        .and_then(|manager| manager.status().last_latency)
        .map_or("n/a".to_string(), |latency| format!("{latency:.1?}"));
    let cache_ttl_text = app
        .schema_cache
        .as_ref()
        .map_or("n/a".to_string(), |cache| format!("{:.1?}", cache.ttl()));

    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            format!(" Pane: {} ", app.pane_name()),
            Style::default()
                .fg(Color::Yellow)
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
        Span::raw(format!("Schema TTL: {cache_ttl_text}")),
        Span::raw(" | "),
        Span::raw(format!(
            "SAFE mode: {}",
            if app.safe_mode_guard.is_enabled() {
                if app.pending_confirmation.is_some() {
                    "confirming"
                } else {
                    "on"
                }
            } else {
                "off"
            }
        )),
        Span::raw(" | "),
        Span::raw(format!(
            "Query: {}",
            if app.query_running { "running" } else { "idle" }
        )),
        Span::raw(" | "),
        Span::raw(format!(
            "Palette: {}",
            if app.show_palette { "open" } else { "closed" }
        )),
        Span::raw(" | "),
        Span::raw(if app.show_perf_overlay {
            format!(
                "Perf: {:.1}ms {:.1}fps rows:{}",
                app.last_render_ms,
                app.fps,
                app.results.len()
            )
        } else {
            "Perf: off (F2)".to_string()
        }),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Fast MySQL TUI"),
    );
    frame.render_widget(header, chunks[0]);

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
                Line::from("Enter: connect and save profile"),
                Line::from("Tab / arrows: switch field"),
                Line::from(""),
            ];
            for (field, label, value) in fields {
                let marker = if app.wizard_form.active_field == field {
                    ">"
                } else {
                    " "
                };
                lines.push(Line::from(format!("{marker} {label}: {value}")));
            }
            lines
        }
        Pane::SchemaExplorer => {
            let mut lines = vec![
                Line::from("Schema Explorer"),
                Line::from("Use arrows / hjkl to select table."),
                Line::from("Press 1 for preview action."),
                Line::from(""),
            ];

            if app.schema_databases.is_empty() {
                lines.push(Line::from("Databases: (none loaded)"));
            } else {
                lines.push(Line::from(format!(
                    "Databases: {}",
                    app.schema_databases.join(", ")
                )));
            }
            lines.push(Line::from(format!(
                "Active DB: {}",
                app.active_database.as_deref().unwrap_or("-")
            )));
            lines.push(Line::from(""));

            for (index, table) in app.schema_tables.iter().enumerate() {
                let marker = if index == app.selected_table_index {
                    ">"
                } else {
                    " "
                };
                lines.push(Line::from(format!("{marker} {table}")));
            }
            lines
        }
        Pane::Results => {
            let visible_limit = usize::from(chunks[1].height.saturating_sub(3)).max(1);
            let window_start = app.results_cursor.saturating_sub(visible_limit / 2);
            let rows = app.results.visible_rows(window_start, visible_limit);

            let mut lines = vec![
                Line::from("Results View (virtualized)"),
                Line::from("Use arrows / hjkl to move cursor."),
            ];

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

            if lines.len() == 2 {
                lines.push(Line::from("No rows buffered"));
            }
            lines
        }
        Pane::QueryEditor => vec![
            Line::from("Query Editor"),
            Line::from(app.query_editor_text.as_str()),
            Line::from("Enter to run query, 1..7 for ranked actions."),
            Line::from("Ctrl+P opens palette placeholder."),
        ],
    };

    let body = Paragraph::new(body_text)
        .block(Block::default().borders(Borders::ALL).title("Workspace"))
        .alignment(Alignment::Left);
    frame.render_widget(body, chunks[1]);

    let actions = app
        .actions
        .rank_top_n(&app.action_context(), FOOTER_ACTIONS_LIMIT);
    let footer_line = if actions.is_empty() {
        "No available actions in this context".to_string()
    } else {
        actions
            .iter()
            .enumerate()
            .map(|(index, action)| format!("{}:{} ", index + 1, action.title))
            .collect::<Vec<_>>()
            .join("| ")
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
}

fn render_help_popup(frame: &mut Frame<'_>) {
    let area = centered_rect(70, 60, frame.area());
    frame.render_widget(Clear, area);
    let help = Paragraph::new(vec![
        Line::from("Global keymap"),
        Line::from("q: quit"),
        Line::from("?: toggle help"),
        Line::from("Tab: cycle panes"),
        Line::from("Enter: connect or run query (by view)"),
        Line::from("F2: toggle perf overlay"),
        Line::from("F3: toggle safe mode"),
        Line::from("Ctrl+P: command palette"),
        Line::from("Ctrl+C: cancel active query"),
        Line::from("Arrows or hjkl: navigation"),
        Line::from("1..7: invoke ranked action slot"),
    ])
    .block(Block::default().borders(Borders::ALL).title("Help"));
    frame.render_widget(help, area);
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
            KeyCode::Char('c') => Some(Msg::CancelQuery),
            _ => None,
        };
    }

    match key.code {
        KeyCode::Char('q') => Some(Msg::Quit),
        KeyCode::Char('?') => Some(Msg::ToggleHelp),
        KeyCode::Esc => Some(Msg::TogglePalette),
        KeyCode::Tab => Some(Msg::NextPane),
        KeyCode::F(2) => Some(Msg::TogglePerfOverlay),
        KeyCode::F(3) => Some(Msg::ToggleSafeMode),
        KeyCode::Enter => Some(Msg::Submit),
        KeyCode::Backspace => Some(Msg::Backspace),
        KeyCode::Up | KeyCode::Char('k') => Some(Msg::Navigate(DirectionKey::Up)),
        KeyCode::Down | KeyCode::Char('j') => Some(Msg::Navigate(DirectionKey::Down)),
        KeyCode::Left | KeyCode::Char('h') => Some(Msg::Navigate(DirectionKey::Left)),
        KeyCode::Right | KeyCode::Char('l') => Some(Msg::Navigate(DirectionKey::Right)),
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
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::{map_key_event, suggest_limit_in_editor, Msg, Pane};

    #[test]
    fn pane_cycles_in_expected_order() {
        assert_eq!(Pane::ConnectionWizard.next(), Pane::SchemaExplorer);
        assert_eq!(Pane::SchemaExplorer.next(), Pane::Results);
        assert_eq!(Pane::Results.next(), Pane::QueryEditor);
        assert_eq!(Pane::QueryEditor.next(), Pane::SchemaExplorer);
    }

    #[test]
    fn keymap_supports_required_global_keys() {
        assert!(matches!(
            map_key_event(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
            Some(Msg::Quit)
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
}
