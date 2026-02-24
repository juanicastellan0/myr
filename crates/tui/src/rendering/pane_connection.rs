use super::super::*;

pub(super) fn body_lines(app: &TuiApp) -> Vec<Line<'static>> {
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
            WizardField::PasswordSource,
            "Password source (env/keyring)",
            app.wizard_form.password_source.as_str(),
        ),
        (
            WizardField::Database,
            "Database",
            app.wizard_form.database.as_str(),
        ),
        (
            WizardField::TlsMode,
            "TLS mode (disabled/prefer/require/verify_identity)",
            app.wizard_form.tls_mode.as_str(),
        ),
        (
            WizardField::ReadOnly,
            "Read-only (yes/no)",
            app.wizard_form.read_only.as_str(),
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
        let editing_active = app.wizard_form.editing && app.wizard_form.active_field == field;
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
