use super::super::*;
use super::centered_rect;

pub(super) fn render_help_popup(frame: &mut Frame<'_>) {
    let area = centered_rect(70, 60, frame.area());
    frame.render_widget(Clear, area);
    let help = Paragraph::new(vec![
        Line::from("Global keymap"),
        Line::from("F10: quit immediately"),
        Line::from("F6: go to connection wizard"),
        Line::from("F7: open profiles/bookmarks manager"),
        Line::from("?: toggle help"),
        Line::from("Tab: cycle panes"),
        Line::from("Connection wizard: E/Enter edit, F5 connect"),
        Line::from("Profiles manager: F5 connect, r rename, d default, q quick reconnect"),
        Line::from("Query editor: Enter run, Ctrl+Enter newline"),
        Line::from("Query editor: Left/Right cursor, Up/Down history"),
        Line::from("F2: toggle perf overlay"),
        Line::from("F3: toggle safe mode"),
        Line::from("F4: toggle schema column compact/full view"),
        Line::from("Ctrl+P: command palette"),
        Line::from("Palette actions include bookmark save/open + related-table jumps"),
        Line::from("Ctrl+U: clear current input"),
        Line::from("Ctrl+C: cancel query (or request exit if idle)"),
        Line::from("Arrows (or Alt+h/j/k/l): navigation"),
        Line::from("Del: delete selected manager entry"),
        Line::from("1..7: invoke ranked action slot"),
    ])
    .block(Block::default().borders(Borders::ALL).title("Help"));
    frame.render_widget(help, area);
}

pub(super) fn render_exit_popup(frame: &mut Frame<'_>) {
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

pub(super) fn render_error_popup(frame: &mut Frame<'_>, app: &TuiApp) {
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

pub(super) fn render_palette_popup(frame: &mut Frame<'_>, app: &TuiApp) {
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
