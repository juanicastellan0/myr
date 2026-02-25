use super::super::*;
use super::support::pulse_char;
use super::{connection_badge_and_marker, spinner_char};

pub(super) fn render_runtime_bar(frame: &mut Frame<'_>, app: &TuiApp, area: Rect) {
    let latency_text = app
        .last_connection_latency
        .map_or("n/a".to_string(), |latency| format!("{latency:.1?}"));
    let profile_mode = app
        .active_connection_profile
        .as_ref()
        .map_or("-", |profile| if profile.read_only { "RO" } else { "RW" });
    let tls_mode = app
        .active_connection_profile
        .as_ref()
        .map_or("-", |profile| match profile.tls_mode {
            TlsMode::Disabled => "off",
            TlsMode::Prefer => "prefer",
            TlsMode::Require => "require",
            TlsMode::VerifyIdentity => "verify",
        });
    let heartbeat = spinner_char(app.loading_tick);
    let loading_text =
        if app.connect_requested && app.connect_intent == ConnectIntent::AutoReconnect {
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
            if app.loading_tick.is_multiple_of(2) {
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
        Span::raw(format!("Mode: {profile_mode}")),
        Span::raw(" | "),
        Span::raw(format!("TLS: {tls_mode}")),
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
    .block(Block::default().borders(Borders::ALL).title("Runtime"));

    frame.render_widget(runtime_bar, area);
}

pub(super) fn render_tabs_bar(frame: &mut Frame<'_>, app: &TuiApp, area: Rect) {
    let tab_focus_marker = pulse_char(app.loading_tick);
    let tabs_title = if app.pane_flash_ticks > 0 {
        format!(
            "Panes (Tab cycles, F6 wizard, F7 manager) | Active: {} {}",
            app.pane_name(),
            tab_focus_marker
        )
    } else {
        "Panes (Tab cycles, F6 wizard, F7 manager)".to_string()
    };

    let tab_labels = [
        (Pane::ConnectionWizard, "Connection Wizard"),
        (Pane::SchemaExplorer, "Schema Explorer"),
        (Pane::Results, "Results"),
        (Pane::QueryEditor, "Query Editor"),
        (Pane::ProfileBookmarks, "Profiles & Bookmarks"),
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
        let flash_bg = if app.loading_tick.is_multiple_of(2) {
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
        .block(Block::default().borders(Borders::ALL).title(tabs_title));

    frame.render_widget(tabs, area);
}

pub(super) fn footer_line(app: &TuiApp) -> String {
    if app.pane == Pane::ConnectionWizard {
        "F5: connect | E/Enter: edit | Enter: save edit | Esc: cancel edit | F10: quit".to_string()
    } else if app.pane == Pane::ProfileBookmarks {
        "F5: connect | Enter: open/save | Del: delete | r:rename d:default q:quick | F6/F7"
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
    }
}
