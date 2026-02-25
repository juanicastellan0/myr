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
use myr_core::schema_cache::{
    ColumnSchema, RelationshipDirection, SchemaCacheService, TableRelationship,
};
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

#[derive(Debug, Error)]
pub enum TuiError {
    #[error("i/o error: {0}")]
    Io(#[from] io::Error),
}

mod app_logic;
mod lib_helpers;
mod rendering;
mod state;

pub(crate) use state::*;

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
