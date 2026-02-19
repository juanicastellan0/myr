use std::io::{self, Stdout};
use std::time::{Duration, Instant};

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use myr_core::actions_engine::{
    ActionContext, ActionInvocation, ActionsEngine, AppView, SchemaSelection,
};
use myr_core::profiles::{ConnectionProfile, FileProfilesStore};
use myr_core::query_runner::QueryRow;
use myr_core::results_buffer::ResultsRingBuffer;
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
    Submit,
    CancelQuery,
    Navigate(DirectionKey),
    InvokeActionSlot(usize),
    Tick,
}

#[derive(Debug)]
struct TuiApp {
    actions: ActionsEngine,
    pane: Pane,
    wizard_form: ConnectionWizardForm,
    connected_profile: Option<String>,
    schema_tables: Vec<String>,
    selected_table_index: usize,
    show_help: bool,
    show_palette: bool,
    should_quit: bool,
    query_running: bool,
    query_ticks_remaining: u8,
    has_results: bool,
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
            schema_tables: DEMO_SCHEMA_TABLES
                .iter()
                .map(|table| (*table).to_string())
                .collect(),
            selected_table_index: 0,
            show_help: false,
            show_palette: false,
            should_quit: false,
            query_running: false,
            query_ticks_remaining: 0,
            has_results: false,
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
                self.status_line = if self.show_palette {
                    "Command palette opened (placeholder)".to_string()
                } else {
                    "Command palette closed".to_string()
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
            Msg::Tick => self.on_tick(),
        }
    }

    fn on_tick(&mut self) {
        if self.query_running {
            if self.query_ticks_remaining == 0 {
                self.query_running = false;
                self.populate_demo_results();
                self.status_line = "Query completed".to_string();
            } else {
                self.query_ticks_remaining = self.query_ticks_remaining.saturating_sub(1);
            }
        }
    }

    fn submit(&mut self) {
        match self.pane {
            Pane::ConnectionWizard => self.connect_from_wizard(),
            Pane::QueryEditor => {
                let context = self.action_context();
                match self.actions.invoke(
                    myr_core::actions_engine::ActionId::RunCurrentQuery,
                    &context,
                ) {
                    Ok(invocation) => self.apply_invocation(invocation),
                    Err(error) => self.status_line = format!("Run query failed: {error}"),
                }
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
        profile.database = Some(self.wizard_form.database.clone());

        match FileProfilesStore::load_default() {
            Ok(mut store) => {
                store.upsert_profile(profile.clone());
                if let Err(error) = store.persist() {
                    self.status_line = format!("Connected (profile save failed: {error})");
                } else {
                    self.status_line = format!("Connected as profile `{}`", profile.name);
                }
            }
            Err(error) => {
                self.status_line = format!("Connected (profile load failed: {error})");
            }
        }

        self.connected_profile = Some(profile.name.clone());
        self.selection.database = profile.database;
        self.selected_table_index = 0;
        self.selection.table = self.schema_tables.first().cloned();
        self.selection.column = Some("id".to_string());
        if let Some(table) = &self.selection.table {
            self.query_editor_text = format!("SELECT * FROM `{}`", table.replace('`', "``"));
        }
        self.pane = Pane::SchemaExplorer;
    }

    fn navigate(&mut self, direction: DirectionKey) {
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

    fn invoke_ranked_action(&mut self, index: usize) {
        let context = self.action_context();
        let ranked = self.actions.rank_top_n(&context, FOOTER_ACTIONS_LIMIT);
        let Some(action) = ranked.get(index) else {
            self.status_line = format!("No action bound to slot {}", index + 1);
            return;
        };

        match self.actions.invoke(action.id, &context) {
            Ok(invocation) => self.apply_invocation(invocation),
            Err(error) => self.status_line = format!("Action error: {error}"),
        }
    }

    fn apply_invocation(&mut self, invocation: ActionInvocation) {
        match invocation {
            ActionInvocation::RunSql(sql) => {
                self.query_running = true;
                self.query_ticks_remaining = QUERY_DURATION_TICKS;
                self.cancel_requested = false;
                self.has_results = false;
                self.query_editor_text = sql;
                self.pane = Pane::Results;
                self.status_line = "Running query...".to_string();
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
                self.status_line = format!("Export requested: {format:?}");
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
            has_results: self.has_results || matches!(self.pane, Pane::Results),
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
        terminal.draw(|frame| render(frame, &app))?;

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
        Span::raw(format!(
            "SAFE mode: {}",
            if app.cancel_requested {
                "cancel requested"
            } else {
                "on"
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
        Line::from("Ctrl+P: command palette placeholder"),
        Line::from("Ctrl+C: cancel active query"),
        Line::from("Arrows or hjkl: navigation"),
        Line::from("1..7: invoke ranked action slot"),
    ])
    .block(Block::default().borders(Borders::ALL).title("Help"));
    frame.render_widget(help, area);
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

fn map_key_event(key: KeyEvent) -> Option<Msg> {
    match (key.modifiers, key.code) {
        (_, KeyCode::Char('q')) => Some(Msg::Quit),
        (_, KeyCode::Char('?')) => Some(Msg::ToggleHelp),
        (_, KeyCode::Tab) => Some(Msg::NextPane),
        (_, KeyCode::Enter) => Some(Msg::Submit),
        (KeyModifiers::CONTROL, KeyCode::Char('p')) => Some(Msg::TogglePalette),
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => Some(Msg::CancelQuery),
        (_, KeyCode::Up | KeyCode::Char('k')) => Some(Msg::Navigate(DirectionKey::Up)),
        (_, KeyCode::Down | KeyCode::Char('j')) => Some(Msg::Navigate(DirectionKey::Down)),
        (_, KeyCode::Left | KeyCode::Char('h')) => Some(Msg::Navigate(DirectionKey::Left)),
        (_, KeyCode::Right | KeyCode::Char('l')) => Some(Msg::Navigate(DirectionKey::Right)),
        (_, KeyCode::Char('1')) => Some(Msg::InvokeActionSlot(0)),
        (_, KeyCode::Char('2')) => Some(Msg::InvokeActionSlot(1)),
        (_, KeyCode::Char('3')) => Some(Msg::InvokeActionSlot(2)),
        (_, KeyCode::Char('4')) => Some(Msg::InvokeActionSlot(3)),
        (_, KeyCode::Char('5')) => Some(Msg::InvokeActionSlot(4)),
        (_, KeyCode::Char('6')) => Some(Msg::InvokeActionSlot(5)),
        (_, KeyCode::Char('7')) => Some(Msg::InvokeActionSlot(6)),
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
}
