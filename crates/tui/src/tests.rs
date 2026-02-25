use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use myr_core::actions_engine::CopyTarget;
use myr_core::bookmarks::{FileBookmarksStore, SavedBookmark};
use myr_core::profiles::{ConnectionProfile, FileProfilesStore, PasswordSource, TlsMode};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use tempfile::TempDir;

use super::{
    bookmark_base_name, candidate_key_column, centered_rect, connection_badge_and_marker,
    extract_key_bounds, is_connection_lost_error, is_transient_query_error, map_key_event,
    next_bookmark_name, parse_password_source, parse_read_only_flag, parse_tls_mode,
    quote_identifier, render, suggest_limit_in_editor, wizard_form_from_profile, ActionId,
    ActionInvocation, AppView, ConnectIntent, DirectionKey, ErrorKind, ManagerLane, Msg,
    MysqlDataBackend, PaginationPlan, Pane, QueryRow, QueryWorkerOutcome, ResultsRingBuffer,
    SchemaColumnViewMode, SchemaLane, TuiApp, WizardField, QUERY_DURATION_TICKS, QUERY_RETRY_LIMIT,
};

fn app_in_pane(pane: Pane) -> TuiApp {
    TuiApp {
        pane,
        ..TuiApp::default()
    }
}

fn app_with_bookmark_store(pane: Pane, temp_dir: &TempDir) -> TuiApp {
    let path = temp_dir.path().join("bookmarks.toml");
    let mut app = app_in_pane(pane);
    app.bookmark_store =
        Some(FileBookmarksStore::load_from_path(path).expect("failed to load test bookmark store"));
    app
}

fn app_with_manager_stores(pane: Pane, temp_dir: &TempDir) -> TuiApp {
    let bookmarks_path = temp_dir.path().join("bookmarks.toml");
    let profiles_path = temp_dir.path().join("profiles.toml");
    let mut app = app_in_pane(pane);
    app.bookmark_store = Some(
        FileBookmarksStore::load_from_path(bookmarks_path)
            .expect("failed to load test bookmark store"),
    );
    app.profile_store = Some(
        FileProfilesStore::load_from_path(profiles_path)
            .expect("failed to load test profile store"),
    );
    app
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
        std::env::var("MYR_RUN_TUI_MYSQL_INTEGRATION")
            .ok()
            .as_deref(),
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
    assert_eq!(Pane::QueryEditor.next(), Pane::ProfileBookmarks);
    assert_eq!(Pane::ProfileBookmarks.next(), Pane::SchemaExplorer);
}

#[test]
fn pane_tab_index_matches_current_pane() {
    assert_eq!(app_in_pane(Pane::ConnectionWizard).pane_tab_index(), 0);
    assert_eq!(app_in_pane(Pane::SchemaExplorer).pane_tab_index(), 1);
    assert_eq!(app_in_pane(Pane::Results).pane_tab_index(), 2);
    assert_eq!(app_in_pane(Pane::QueryEditor).pane_tab_index(), 3);
    assert_eq!(app_in_pane(Pane::ProfileBookmarks).pane_tab_index(), 4);
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
    assert!(matches!(
        map_key_event(KeyEvent::new(KeyCode::F(7), KeyModifiers::NONE)),
        Some(Msg::GoProfileBookmarkManager)
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
        map_key_event(KeyEvent::new(KeyCode::F(4), KeyModifiers::NONE)),
        Some(Msg::ToggleSchemaColumnView)
    ));
    assert!(matches!(
        map_key_event(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)),
        Some(Msg::DeleteSelection)
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
fn read_only_parser_accepts_common_values() {
    assert_eq!(parse_read_only_flag("yes"), Some(true));
    assert_eq!(parse_read_only_flag("RO"), Some(true));
    assert_eq!(parse_read_only_flag("no"), Some(false));
    assert_eq!(parse_read_only_flag("rw"), Some(false));
    assert_eq!(parse_read_only_flag("maybe"), None);
}

#[test]
fn password_source_parser_accepts_common_values() {
    assert!(matches!(
        parse_password_source("env"),
        Some(PasswordSource::EnvVar)
    ));
    assert!(matches!(
        parse_password_source("KEYRING"),
        Some(PasswordSource::Keyring)
    ));
    assert_eq!(parse_password_source("vault"), None);
}

#[test]
fn tls_mode_parser_accepts_common_values() {
    assert!(matches!(
        parse_tls_mode("disabled"),
        Some(TlsMode::Disabled)
    ));
    assert!(matches!(parse_tls_mode("prefer"), Some(TlsMode::Prefer)));
    assert!(matches!(parse_tls_mode("require"), Some(TlsMode::Require)));
    assert!(matches!(
        parse_tls_mode("verify_identity"),
        Some(TlsMode::VerifyIdentity)
    ));
    assert_eq!(parse_tls_mode("mtls"), None);
}

#[test]
fn wizard_form_conversion_uses_profile_values() {
    let mut profile =
        ConnectionProfile::new("qa-profile".to_string(), "10.0.0.8".to_string(), "appuser");
    profile.port = 4406;
    profile.database = Some("analytics".to_string());
    profile.password_source = PasswordSource::Keyring;
    profile.tls_mode = TlsMode::VerifyIdentity;
    profile.read_only = true;

    let form = wizard_form_from_profile(&profile);
    assert_eq!(form.profile_name, "qa-profile");
    assert_eq!(form.host, "10.0.0.8");
    assert_eq!(form.port, "4406");
    assert_eq!(form.user, "appuser");
    assert_eq!(form.database, "analytics");
    assert_eq!(form.password_source, "keyring");
    assert_eq!(form.tls_mode, "verify_identity");
    assert_eq!(form.read_only, "yes");
    assert_eq!(form.active_field, WizardField::ProfileName);
    assert!(!form.editing);
    assert!(form.edit_buffer.is_empty());
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
fn schema_column_view_mode_toggles_in_schema_explorer_only() {
    let mut app = app_in_pane(Pane::SchemaExplorer);
    assert_eq!(app.schema_column_view_mode, SchemaColumnViewMode::Compact);

    app.handle(Msg::ToggleSchemaColumnView);
    assert_eq!(app.schema_column_view_mode, SchemaColumnViewMode::Full);
    assert_eq!(app.status_line, "Schema columns view: full");

    app.handle(Msg::ToggleSchemaColumnView);
    assert_eq!(app.schema_column_view_mode, SchemaColumnViewMode::Compact);
    assert_eq!(app.status_line, "Schema columns view: compact");

    app.pane = Pane::QueryEditor;
    app.handle(Msg::ToggleSchemaColumnView);
    assert_eq!(app.schema_column_view_mode, SchemaColumnViewMode::Compact);
    assert_eq!(
        app.status_line,
        "Schema column view toggle is available in Schema Explorer"
    );
}

#[test]
fn schema_database_filter_updates_active_selection() {
    let mut app = TuiApp {
        pane: Pane::SchemaExplorer,
        schema_lane: SchemaLane::Databases,
        ..TuiApp::default()
    };
    app.schema_databases = vec![
        "app".to_string(),
        "analytics".to_string(),
        "myr_bench".to_string(),
    ];
    app.selected_database_index = 0;
    app.active_database = Some("app".to_string());
    app.selection.database = Some("app".to_string());

    app.handle(Msg::InputChar('b'));
    app.handle(Msg::InputChar('e'));
    app.handle(Msg::InputChar('n'));

    assert_eq!(app.schema_database_filter, "ben");
    assert_eq!(app.active_database.as_deref(), Some("myr_bench"));
    assert_eq!(app.selection.database.as_deref(), Some("myr_bench"));
    assert!(app.status_line.contains("matched 1 entries"));
}

#[test]
fn schema_table_filter_navigation_moves_within_matches() {
    let mut app = TuiApp {
        pane: Pane::SchemaExplorer,
        schema_lane: SchemaLane::Tables,
        ..TuiApp::default()
    };
    app.selection.database = Some("app".to_string());
    app.selection.table = app.schema_tables.first().cloned();
    app.reload_columns_for_selected_table();

    app.handle(Msg::InputChar('e'));
    assert_eq!(app.schema_table_filter, "e");
    assert_eq!(app.selection.table.as_deref(), Some("users"));

    app.navigate(DirectionKey::Down);
    assert_eq!(app.selection.table.as_deref(), Some("sessions"));

    app.navigate(DirectionKey::Down);
    assert_eq!(app.selection.table.as_deref(), Some("events"));
    assert_eq!(app.selected_table_index, 3);
}

#[test]
fn schema_column_filter_backspace_and_clear_updates_selection() {
    let mut app = TuiApp {
        pane: Pane::SchemaExplorer,
        schema_lane: SchemaLane::Columns,
        ..TuiApp::default()
    };
    app.selection.database = Some("app".to_string());
    app.selection.table = Some("users".to_string());
    app.reload_columns_for_selected_table();

    app.handle(Msg::InputChar('u'));
    app.handle(Msg::InputChar('p'));
    app.handle(Msg::InputChar('d'));
    assert_eq!(app.schema_column_filter, "upd");
    assert_eq!(app.selection.column.as_deref(), Some("updated_at"));

    app.handle(Msg::Backspace);
    assert_eq!(app.schema_column_filter, "up");
    assert_eq!(app.selection.column.as_deref(), Some("updated_at"));

    app.handle(Msg::ClearInput);
    assert!(app.schema_column_filter.is_empty());
    assert_eq!(app.selection.column.as_deref(), Some("id"));
}

#[test]
fn demo_relationships_are_loaded_for_selected_table() {
    let mut app = app_in_pane(Pane::SchemaExplorer);
    app.selection.database = Some("app".to_string());
    app.selection.table = Some("users".to_string());
    app.reload_columns_for_selected_table();

    assert!(!app.schema_relationships.is_empty());
    assert_eq!(
        app.schema_relationships[0].constraint_name,
        "fk_sessions_users"
    );
}

#[test]
fn jump_related_table_updates_schema_selection() {
    let mut app = app_in_pane(Pane::SchemaExplorer);
    app.selection.database = Some("app".to_string());
    app.selection.table = Some("users".to_string());
    app.reload_columns_for_selected_table();

    app.jump_to_next_related_table();

    assert_eq!(app.selection.table.as_deref(), Some("sessions"));
    assert_eq!(app.selection.column.as_deref(), Some("user_id"));
    assert!(app.status_line.contains("fk_sessions_users"));
}

#[test]
fn bookmark_name_helpers_are_stable() {
    let base = bookmark_base_name(Some("local-dev"), Some("myr bench"), Some("events"));
    assert_eq!(base, "local-dev:myr_bench.events");

    let bookmarks = vec![
        SavedBookmark::new("local-dev:myr_bench.events"),
        SavedBookmark::new("local-dev:myr_bench.events-2"),
    ];
    assert_eq!(
        next_bookmark_name(&bookmarks, "local-dev:myr_bench.events"),
        "local-dev:myr_bench.events-3"
    );
}

#[test]
fn save_and_open_bookmark_paths_round_trip() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let mut app = app_with_bookmark_store(Pane::QueryEditor, &temp_dir);
    app.connected_profile = Some("local-dev".to_string());
    app.selection.database = Some("app".to_string());
    app.selection.table = Some("events".to_string());
    app.selection.column = Some("user_id".to_string());
    app.query_editor_text = "SELECT user_id FROM `app`.`events` LIMIT 5".to_string();

    app.save_current_bookmark();
    assert!(app.status_line.starts_with("Saved bookmark"));
    assert!(app.has_saved_bookmarks());

    app.query_editor_text = "SELECT 1".to_string();
    app.selection.table = Some("users".to_string());

    app.open_next_bookmark();
    assert_eq!(app.pane, Pane::QueryEditor);
    assert_eq!(app.selection.table.as_deref(), Some("events"));
    assert_eq!(app.selection.column.as_deref(), Some("user_id"));
    assert_eq!(
        app.query_editor_text,
        "SELECT user_id FROM `app`.`events` LIMIT 5"
    );
    assert!(app.status_line.starts_with("Opened bookmark"));
}

#[test]
fn manager_can_open_and_delete_profiles_and_bookmarks() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let mut app = app_with_manager_stores(Pane::SchemaExplorer, &temp_dir);

    let mut profile = ConnectionProfile::new("qa-profile", "127.0.0.1", "root");
    profile.port = 33306;
    profile.database = Some("myr_bench".to_string());
    profile.read_only = true;
    {
        let store = app.profile_store.as_mut().expect("profile store");
        store.upsert_profile(profile.clone());
        store.persist().expect("persist profile store");
    }

    let mut bookmark = SavedBookmark::new("qa-profile:myr_bench.events");
    bookmark.profile_name = Some("qa-profile".to_string());
    bookmark.database = Some("myr_bench".to_string());
    bookmark.table = Some("events".to_string());
    bookmark.column = Some("user_id".to_string());
    bookmark.query = Some("SELECT user_id FROM `myr_bench`.`events` LIMIT 5".to_string());
    {
        let store = app.bookmark_store.as_mut().expect("bookmark store");
        store.upsert_bookmark(bookmark);
        store.persist().expect("persist bookmark store");
    }

    app.handle(Msg::GoProfileBookmarkManager);
    assert_eq!(app.pane, Pane::ProfileBookmarks);

    app.submit();
    assert_eq!(app.pane, Pane::ConnectionWizard);
    assert_eq!(app.wizard_form.profile_name, "qa-profile");

    app.handle(Msg::GoProfileBookmarkManager);
    app.navigate(DirectionKey::Right);
    assert_eq!(app.manager_lane, ManagerLane::Bookmarks);

    app.submit();
    assert_eq!(app.pane, Pane::QueryEditor);
    assert_eq!(app.selection.table.as_deref(), Some("events"));

    app.handle(Msg::GoProfileBookmarkManager);
    app.handle(Msg::DeleteSelection);
    assert!(app.status_line.starts_with("Deleted bookmark"));
    assert!(app
        .bookmark_store
        .as_ref()
        .expect("bookmark store")
        .bookmarks()
        .is_empty());

    app.navigate(DirectionKey::Left);
    app.handle(Msg::DeleteSelection);
    assert!(app.status_line.starts_with("Deleted profile"));
    assert!(app
        .profile_store
        .as_ref()
        .expect("profile store")
        .profiles()
        .is_empty());
}

#[test]
fn manager_can_rename_profile_and_update_connection_references() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let mut app = app_with_manager_stores(Pane::SchemaExplorer, &temp_dir);

    let mut profile = ConnectionProfile::new("qa-profile", "127.0.0.1", "root");
    profile.port = 33306;
    {
        let store = app.profile_store.as_mut().expect("profile store");
        store.upsert_profile(profile.clone());
        store.persist().expect("persist profile store");
    }

    app.connected_profile = Some("qa-profile".to_string());
    app.last_connect_profile = Some(profile.clone());
    app.active_connection_profile = Some(profile);
    app.wizard_form.profile_name = "qa-profile".to_string();

    app.handle(Msg::GoProfileBookmarkManager);
    app.handle(Msg::InputChar('r'));
    assert!(app.manager_rename_mode);

    app.handle(Msg::ClearInput);
    for ch in "qa-renamed".chars() {
        app.handle(Msg::InputChar(ch));
    }
    app.handle(Msg::Submit);

    assert!(!app.manager_rename_mode);
    assert!(app.status_line.starts_with("Renamed profile"));

    let store = app.profile_store.as_ref().expect("profile store");
    assert!(store.profile("qa-profile").is_none());
    assert!(store.profile("qa-renamed").is_some());
    assert_eq!(app.connected_profile.as_deref(), Some("qa-renamed"));
    assert_eq!(
        app.last_connect_profile
            .as_ref()
            .map(|saved| saved.name.as_str()),
        Some("qa-renamed")
    );
    assert_eq!(
        app.active_connection_profile
            .as_ref()
            .map(|saved| saved.name.as_str()),
        Some("qa-renamed")
    );
    assert_eq!(app.wizard_form.profile_name, "qa-renamed");
}

#[test]
fn manager_shortcuts_mark_default_and_quick_reconnect_profiles() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let mut app = app_with_manager_stores(Pane::SchemaExplorer, &temp_dir);

    let alpha = ConnectionProfile::new("alpha", "127.0.0.1", "root");
    let beta = ConnectionProfile::new("beta", "127.0.0.1", "root");
    {
        let store = app.profile_store.as_mut().expect("profile store");
        store.upsert_profile(alpha);
        store.upsert_profile(beta);
        store.persist().expect("persist profile store");
    }

    app.handle(Msg::GoProfileBookmarkManager);
    app.navigate(DirectionKey::Down);
    app.handle(Msg::InputChar('d'));
    app.handle(Msg::InputChar('q'));

    let store = app.profile_store.as_ref().expect("profile store");
    assert_eq!(
        store.default_profile().map(|profile| profile.name.as_str()),
        Some("beta")
    );
    assert_eq!(
        store
            .quick_reconnect_profile()
            .map(|profile| profile.name.as_str()),
        Some("beta")
    );
}

#[test]
fn manager_connect_prefers_quick_reconnect_profile_from_bookmarks_lane() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let mut app = app_with_manager_stores(Pane::SchemaExplorer, &temp_dir);

    let mut alpha = ConnectionProfile::new("alpha", "127.0.0.1", "root");
    alpha.port = 1;
    let mut beta = ConnectionProfile::new("beta", "127.0.0.1", "root");
    beta.port = 2;

    let mut bookmark = SavedBookmark::new("alpha:myr_bench.events");
    bookmark.profile_name = Some("alpha".to_string());
    bookmark.database = Some("myr_bench".to_string());
    bookmark.table = Some("events".to_string());

    {
        let store = app.profile_store.as_mut().expect("profile store");
        store.upsert_profile(alpha);
        store.upsert_profile(beta);
        assert!(store.set_quick_reconnect_profile("beta"));
        store.persist().expect("persist profile store");
    }
    {
        let store = app.bookmark_store.as_mut().expect("bookmark store");
        store.upsert_bookmark(bookmark);
        store.persist().expect("persist bookmark store");
    }

    app.handle(Msg::GoProfileBookmarkManager);
    app.navigate(DirectionKey::Right);
    app.handle(Msg::Connect);

    assert_eq!(
        app.last_connect_profile
            .as_ref()
            .map(|profile| profile.name.as_str()),
        Some("beta")
    );
    drive_connect_to_completion(&mut app);
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
fn read_only_profile_blocks_destructive_submit() {
    let mut app = app_in_pane(Pane::QueryEditor);
    let mut profile = ConnectionProfile::new(
        "local-ro".to_string(),
        "127.0.0.1".to_string(),
        "root".to_string(),
    );
    profile.read_only = true;
    app.active_connection_profile = Some(profile.clone());
    app.last_connect_profile = Some(profile);
    app.query_editor_text = "DELETE FROM `app`.`users` WHERE id = 1".to_string();

    app.submit();

    assert!(!app.query_running);
    assert!(app.pending_confirmation.is_none());
    assert_eq!(
        app.status_line,
        "Blocked by read-only profile mode: write/DDL SQL is disabled"
    );
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
    tx.send(QueryWorkerOutcome::Failure(
        "Pool was disconnected".to_string(),
    ))
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
fn connect_from_wizard_rejects_invalid_password_source() {
    let mut app = app_in_pane(Pane::ConnectionWizard);
    app.wizard_form.password_source = "vault".to_string();
    app.connect_from_wizard();

    assert_eq!(
        app.status_line,
        "Invalid password source in connection wizard (use env/keyring)"
    );
}

#[test]
fn connect_from_wizard_rejects_invalid_tls_mode() {
    let mut app = app_in_pane(Pane::ConnectionWizard);
    app.wizard_form.tls_mode = "mtls".to_string();
    app.connect_from_wizard();

    assert_eq!(
        app.status_line,
        "Invalid TLS mode in connection wizard (use disabled/prefer/require/verify_identity)"
    );
}

#[test]
fn connect_from_wizard_rejects_invalid_read_only_mode() {
    let mut app = app_in_pane(Pane::ConnectionWizard);
    app.wizard_form.read_only = "sometimes".to_string();
    app.connect_from_wizard();

    assert_eq!(
        app.status_line,
        "Invalid read-only mode in connection wizard (use yes/no)"
    );
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

    app.export_results(myr_core::actions_engine::ExportFormat::CsvGzip);
    assert!(app.status_line.starts_with("Exported "));

    app.export_results(myr_core::actions_engine::ExportFormat::JsonGzip);
    assert!(app.status_line.starts_with("Exported "));

    app.export_results(myr_core::actions_engine::ExportFormat::JsonLines);
    assert!(app.status_line.starts_with("Exported "));

    app.export_results(myr_core::actions_engine::ExportFormat::JsonLinesGzip);
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
fn apply_invocation_handles_bookmark_and_relationship_actions() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let mut app = app_with_bookmark_store(Pane::QueryEditor, &temp_dir);
    app.selection.database = Some("app".to_string());
    app.selection.table = Some("users".to_string());
    app.selection.column = Some("id".to_string());
    app.query_editor_text = "SELECT * FROM `app`.`users` LIMIT 5".to_string();

    app.apply_invocation(ActionId::SaveBookmark, ActionInvocation::SaveBookmark);
    assert!(app.status_line.starts_with("Saved bookmark"));

    app.selection.table = Some("events".to_string());
    app.query_editor_text = "SELECT 1".to_string();
    app.apply_invocation(ActionId::OpenBookmark, ActionInvocation::OpenBookmark);
    assert_eq!(app.selection.table.as_deref(), Some("users"));
    assert!(app.status_line.starts_with("Opened bookmark"));

    app.pane = Pane::SchemaExplorer;
    app.selection.table = Some("users".to_string());
    app.reload_columns_for_selected_table();
    app.apply_invocation(
        ActionId::JumpToRelatedTable,
        ActionInvocation::JumpToRelatedTable,
    );
    assert_eq!(app.selection.table.as_deref(), Some("sessions"));
    assert!(app.status_line.contains("Jumped"));
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
fn palette_supports_fuzzy_and_alias_queries() {
    let mut app = app_in_pane(Pane::SchemaExplorer);
    app.show_palette = true;

    app.palette_query = "pvw".to_string();
    let entries = app.palette_entries();
    assert_eq!(entries.first().copied(), Some(ActionId::PreviewTable));

    app.palette_query = "ddl".to_string();
    let entries = app.palette_entries();
    assert_eq!(entries.first().copied(), Some(ActionId::ShowCreateTable));

    app.palette_query = "fk".to_string();
    let entries = app.palette_entries();
    assert_eq!(entries.first().copied(), Some(ActionId::JumpToRelatedTable));
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

    let manager = app_in_pane(Pane::ProfileBookmarks);
    render_once(&manager);

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
    assert!(app.status_line.starts_with("No active query. Exit myr?"));

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
fn delete_selection_requires_manager_view() {
    let mut app = app_in_pane(Pane::Results);
    app.handle(Msg::DeleteSelection);
    assert_eq!(
        app.status_line,
        "Delete selection is only available in manager view"
    );
}

#[test]
fn navigate_results_reports_empty_and_updates_cursor() {
    let mut app = app_in_pane(Pane::Results);
    app.navigate_results(DirectionKey::Down);
    assert_eq!(app.status_line, "No buffered rows yet");

    app.populate_demo_results();
    app.navigate_results(DirectionKey::Down);
    assert!(app.status_line.starts_with("Results cursor: row"));
}

#[test]
fn navigate_results_horizontal_changes_active_column() {
    let mut app = app_in_pane(Pane::Results);
    app.populate_demo_results();

    assert_eq!(app.selection.column.as_deref(), Some("value"));

    app.navigate_results(DirectionKey::Right);
    assert_eq!(app.selection.column.as_deref(), Some("observed_at"));
    assert!(app.status_line.contains("col 3 / 3"));

    app.navigate_results(DirectionKey::Left);
    assert_eq!(app.selection.column.as_deref(), Some("value"));
    assert!(app.status_line.contains("col 2 / 3"));
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
fn apply_connected_profile_resets_schema_filters() {
    let profile = ConnectionProfile::new("local-dev", "127.0.0.1", "root");
    let mut app = app_in_pane(Pane::SchemaExplorer);
    app.schema_database_filter = "bench".to_string();
    app.schema_table_filter = "event".to_string();
    app.schema_column_filter = "created".to_string();

    app.apply_connected_profile(
        profile,
        Duration::from_millis(1),
        vec!["myr_bench".to_string()],
        None,
    );

    assert!(app.schema_database_filter.is_empty());
    assert!(app.schema_table_filter.is_empty());
    assert!(app.schema_column_filter.is_empty());
}

#[test]
fn apply_connected_profile_preserves_default_and_quick_markers() {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let mut app = app_with_manager_stores(Pane::SchemaExplorer, &temp_dir);

    let mut stored = ConnectionProfile::new("local-dev", "127.0.0.1", "root");
    stored.is_default = true;
    stored.quick_reconnect = true;
    {
        let store = app.profile_store.as_mut().expect("profile store");
        store.upsert_profile(stored);
        store.persist().expect("persist profile store");
    }

    let incoming = ConnectionProfile::new("local-dev", "127.0.0.1", "root");
    app.apply_connected_profile(
        incoming,
        Duration::from_millis(1),
        vec!["myr_bench".to_string()],
        None,
    );

    let persisted = app
        .profile_store
        .as_ref()
        .expect("profile store")
        .profile("local-dev")
        .expect("persisted profile");
    assert!(persisted.is_default);
    assert!(persisted.quick_reconnect);
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
        "Connect is available in wizard or profiles manager"
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
