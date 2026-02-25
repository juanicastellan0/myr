use super::super::*;

pub(super) fn body_lines(app: &TuiApp, body_area: Rect) -> Vec<Line<'static>> {
    let section_window = usize::from(body_area.height.saturating_sub(14)).clamp(3, 8);
    let profiles = app.manager_profiles();
    let bookmarks = app.manager_bookmarks();

    let mut lines = vec![
        Line::from("Profiles & Bookmarks"),
        Line::from("Left/Right: lane focus | Up/Down: selection | Enter: open/save | Del: delete"),
        Line::from("Shortcuts: r rename | d default profile | q quick reconnect | F5 connect"),
        Line::from(format!(
            "Focus lane: {} | Profiles: {} | Bookmarks: {}",
            app.manager_lane.label(),
            profiles.len(),
            bookmarks.len()
        )),
        Line::from(""),
    ];

    let profile_items = profiles
        .iter()
        .map(render_profile_summary)
        .collect::<Vec<_>>();
    lines.push(Line::from(Span::styled(
        "Profiles",
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )));
    append_windowed_manager_items(
        &mut lines,
        &profile_items,
        app.manager_profile_cursor,
        app.manager_lane == ManagerLane::Profiles,
        section_window,
        Color::Cyan,
    );

    lines.push(Line::from(""));
    let bookmark_items = bookmarks
        .iter()
        .map(render_bookmark_summary)
        .collect::<Vec<_>>();
    lines.push(Line::from(Span::styled(
        "Bookmarks",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )));
    append_windowed_manager_items(
        &mut lines,
        &bookmark_items,
        app.manager_bookmark_cursor,
        app.manager_lane == ManagerLane::Bookmarks,
        section_window,
        Color::Yellow,
    );

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Selected Entry",
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    )));
    append_selected_details(&mut lines, app, &profiles, &bookmarks);
    if app.manager_rename_mode {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!(
                "Rename input: `{}` (Enter save, Esc cancel)",
                app.manager_rename_buffer
            ),
            Style::default().fg(Color::Magenta),
        )));
    }

    lines
}

fn append_windowed_manager_items(
    lines: &mut Vec<Line<'static>>,
    items: &[String],
    selected_index: usize,
    active: bool,
    max_visible: usize,
    selected_color: Color,
) {
    if items.is_empty() {
        lines.push(Line::from("  (none)"));
        return;
    }

    let clamped_selected = selected_index.min(items.len().saturating_sub(1));
    let visible = max_visible.max(1).min(items.len());
    let mut start = clamped_selected.saturating_sub(visible / 2);
    if start + visible > items.len() {
        start = items.len().saturating_sub(visible);
    }
    let end = (start + visible).min(items.len());

    if start > 0 {
        lines.push(Line::from(format!("  ... {} above", start)));
    }

    for (offset, item) in items[start..end].iter().enumerate() {
        let index = start + offset;
        let marker = if index == clamped_selected {
            if active {
                ">"
            } else {
                "*"
            }
        } else {
            " "
        };
        let rendered = format!("{marker} {item}");
        if index == clamped_selected {
            lines.push(Line::from(Span::styled(
                rendered,
                Style::default()
                    .fg(selected_color)
                    .add_modifier(Modifier::BOLD),
            )));
        } else {
            lines.push(Line::from(rendered));
        }
    }

    if end < items.len() {
        lines.push(Line::from(format!("  ... {} more", items.len() - end)));
    }
}

fn append_selected_details(
    lines: &mut Vec<Line<'static>>,
    app: &TuiApp,
    profiles: &[ConnectionProfile],
    bookmarks: &[SavedBookmark],
) {
    match app.manager_lane {
        ManagerLane::Profiles => {
            let Some(profile) = profiles.get(
                app.manager_profile_cursor
                    .min(profiles.len().saturating_sub(1)),
            ) else {
                lines.push(Line::from("  No profile selected"));
                return;
            };

            lines.push(Line::from(format!("  Name: {}", profile.name)));
            lines.push(Line::from(format!(
                "  Target: {}@{}:{}",
                profile.user, profile.host, profile.port
            )));
            lines.push(Line::from(format!(
                "  Database: {} | Mode: {} | TLS: {:?}",
                profile.database.as_deref().unwrap_or("-"),
                if profile.read_only { "RO" } else { "RW" },
                profile.tls_mode
            )));
            lines.push(Line::from(format!(
                "  Default: {} | Quick reconnect: {}",
                if profile.is_default { "yes" } else { "no" },
                if profile.quick_reconnect { "yes" } else { "no" }
            )));
        }
        ManagerLane::Bookmarks => {
            let Some(bookmark) = bookmarks.get(
                app.manager_bookmark_cursor
                    .min(bookmarks.len().saturating_sub(1)),
            ) else {
                lines.push(Line::from("  No bookmark selected"));
                return;
            };

            lines.push(Line::from(format!("  Name: {}", bookmark.name)));
            lines.push(Line::from(format!(
                "  Target: {}.{} ({})",
                bookmark.database.as_deref().unwrap_or("-"),
                bookmark.table.as_deref().unwrap_or("-"),
                bookmark.column.as_deref().unwrap_or("-")
            )));
            if let Some(query) = bookmark
                .query
                .as_deref()
                .filter(|query| !query.trim().is_empty())
            {
                let trimmed = query.split_whitespace().collect::<Vec<_>>().join(" ");
                let preview = if trimmed.chars().count() > 100 {
                    format!("{}...", trimmed.chars().take(100).collect::<String>())
                } else {
                    trimmed
                };
                lines.push(Line::from(format!("  Query: {preview}")));
            } else {
                lines.push(Line::from("  Query: (none)"));
            }
        }
    }
}

fn render_profile_summary(profile: &ConnectionProfile) -> String {
    let mut markers = Vec::new();
    if profile.is_default {
        markers.push("default");
    }
    if profile.quick_reconnect {
        markers.push("quick");
    }
    let marker_text = if markers.is_empty() {
        String::new()
    } else {
        format!(" [{}]", markers.join(","))
    };
    format!(
        "{}{} | {}@{}:{} | db {} | {}",
        profile.name,
        marker_text,
        profile.user,
        profile.host,
        profile.port,
        profile.database.as_deref().unwrap_or("-"),
        if profile.read_only { "RO" } else { "RW" }
    )
}

fn render_bookmark_summary(bookmark: &SavedBookmark) -> String {
    format!(
        "{} | {}.{} ({})",
        bookmark.name,
        bookmark.database.as_deref().unwrap_or("-"),
        bookmark.table.as_deref().unwrap_or("-"),
        bookmark.column.as_deref().unwrap_or("-")
    )
}
