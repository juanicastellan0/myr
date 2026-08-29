mod preferences;

use std::path::PathBuf;
use std::time::Duration;

use iced::widget::table::{column as table_column, table};
use iced::widget::{
    button, checkbox, column, container, pick_list, row, rule, scrollable, space, text,
    text_editor, text_input,
};
use iced::{Element, Length, Subscription, Task, Theme};
use myr_application::{
    AppCommand, AppEvent, AppSnapshot, ApplicationHandle, ConnectionStatus, ExportFormat,
    ExportRequest, ExportScope, OperationKind,
};
use myr_core::profiles::{ConnectionProfile, PasswordSource, TlsMode};

pub use preferences::{default_gui_preferences_path, ColorScheme, GuiPreferences};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Schema,
    Query,
    Results,
}

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    ProfileSelected(String),
    Connect,
    Disconnect,
    DatabaseSelected(String),
    TableSelected(String),
    TabSelected(Tab),
    QueryEdited(text_editor::Action),
    ExecuteQuery,
    ConfirmQuery,
    CancelOperation,
    PreviewTable,
    SearchChanged(String),
    PreviousResultPage,
    NextResultPage,
    PreviousPreviewPage,
    NextPreviewPage,
    SelectResultRow(usize),
    ResizeColumn { index: usize, delta: f32 },
    ToggleProfileEditor,
    NewProfile,
    EditProfile,
    ProfileNameChanged(String),
    ProfileHostChanged(String),
    ProfilePortChanged(String),
    ProfileUserChanged(String),
    ProfileDatabaseChanged(String),
    ProfileReadOnlyChanged(bool),
    CycleTlsMode,
    CyclePasswordSource,
    PickCaCertificate,
    CaCertificatePicked(Option<PathBuf>),
    PickClientCertificate,
    ClientCertificatePicked(Option<PathBuf>),
    PickClientKey,
    ClientKeyPicked(Option<PathBuf>),
    SaveProfile,
    DeleteProfile,
    SetDefaultProfile,
    SetQuickReconnectProfile,
    ChooseExport(ExportScope),
    ExportPathPicked(Option<PathBuf>, ExportScope),
    CycleExportFormat,
    ToggleTheme,
    ClearError,
}

#[derive(Debug, Clone)]
struct ProfileDraft {
    original_name: Option<String>,
    name: String,
    host: String,
    port: String,
    user: String,
    database: String,
    tls_mode: TlsMode,
    password_source: PasswordSource,
    tls_ca_cert_path: Option<String>,
    tls_client_cert_path: Option<String>,
    tls_client_key_path: Option<String>,
    read_only: bool,
}

impl Default for ProfileDraft {
    fn default() -> Self {
        Self {
            original_name: None,
            name: "local".to_string(),
            host: "127.0.0.1".to_string(),
            port: "3306".to_string(),
            user: "root".to_string(),
            database: String::new(),
            tls_mode: TlsMode::Prefer,
            password_source: PasswordSource::EnvVar,
            tls_ca_cert_path: None,
            tls_client_cert_path: None,
            tls_client_key_path: None,
            read_only: false,
        }
    }
}

impl From<&ConnectionProfile> for ProfileDraft {
    fn from(profile: &ConnectionProfile) -> Self {
        Self {
            original_name: Some(profile.name.clone()),
            name: profile.name.clone(),
            host: profile.host.clone(),
            port: profile.port.to_string(),
            user: profile.user.clone(),
            database: profile.database.clone().unwrap_or_default(),
            tls_mode: profile.tls_mode,
            password_source: profile.password_source,
            tls_ca_cert_path: profile.tls_ca_cert_path.clone(),
            tls_client_cert_path: profile.tls_client_cert_path.clone(),
            tls_client_key_path: profile.tls_client_key_path.clone(),
            read_only: profile.read_only,
        }
    }
}

pub struct Gui {
    application: Option<ApplicationHandle>,
    events: Option<tokio::sync::broadcast::Receiver<AppEvent>>,
    snapshot: AppSnapshot,
    selected_profile: Option<String>,
    active_tab: Tab,
    query_editor: text_editor::Content,
    result_search: String,
    result_page: usize,
    preview_offset: u64,
    selected_result_row: Option<usize>,
    column_widths: Vec<f32>,
    profile_editor_open: bool,
    profile_draft: ProfileDraft,
    export_format: ExportFormat,
    status: String,
    preferences: GuiPreferences,
}

impl Gui {
    #[must_use]
    pub fn new(application: ApplicationHandle, preferences: GuiPreferences) -> Self {
        let snapshot = application.snapshot();
        let events = Some(application.subscribe());
        let selected_profile = snapshot
            .profiles
            .iter()
            .find(|profile| profile.is_default)
            .or_else(|| {
                snapshot
                    .profiles
                    .iter()
                    .find(|profile| profile.quick_reconnect)
            })
            .or_else(|| snapshot.profiles.first())
            .map(|profile| profile.name.clone());
        let column_widths = vec![160.0; snapshot.results.columns.len()];
        Self {
            application: Some(application),
            events,
            snapshot,
            selected_profile,
            active_tab: Tab::Schema,
            query_editor: text_editor::Content::with_text("SELECT * FROM `users` LIMIT 200"),
            result_search: String::new(),
            result_page: 0,
            preview_offset: 0,
            selected_result_row: None,
            column_widths,
            profile_editor_open: false,
            profile_draft: ProfileDraft::default(),
            export_format: ExportFormat::JsonLines,
            status: "Select a profile and connect".to_string(),
            preferences,
        }
    }

    #[cfg(test)]
    fn preview(snapshot: AppSnapshot) -> Self {
        let column_widths = vec![160.0; snapshot.results.columns.len()];
        Self {
            application: None,
            events: None,
            snapshot,
            selected_profile: None,
            active_tab: Tab::Schema,
            query_editor: text_editor::Content::with_text("SELECT 1"),
            result_search: String::new(),
            result_page: 0,
            preview_offset: 0,
            selected_result_row: None,
            column_widths,
            profile_editor_open: false,
            profile_draft: ProfileDraft::default(),
            export_format: ExportFormat::JsonLines,
            status: "Ready".to_string(),
            preferences: GuiPreferences::default(),
        }
    }

    #[allow(clippy::too_many_lines)]
    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => self.refresh_application(),
            Message::ProfileSelected(profile) => {
                self.selected_profile = Some(profile);
            }
            Message::Connect => {
                if let Some(profile_name) = self.selected_profile.clone() {
                    self.send(AppCommand::Connect { profile_name });
                    self.status = "Connecting...".to_string();
                } else {
                    self.status = "Choose a profile first".to_string();
                }
            }
            Message::Disconnect => {
                self.send(AppCommand::Disconnect);
                self.status = "Disconnecting...".to_string();
            }
            Message::DatabaseSelected(database) => {
                self.send(AppCommand::SelectDatabase { database });
                self.status = "Loading tables...".to_string();
            }
            Message::TableSelected(table) => {
                let Some(database) = self.snapshot.schema.selected_database.clone() else {
                    return Task::none();
                };
                self.send(AppCommand::SelectTable { database, table });
                self.status = "Loading columns...".to_string();
            }
            Message::TabSelected(tab) => self.active_tab = tab,
            Message::QueryEdited(action) => self.query_editor.perform(action),
            Message::ExecuteQuery => {
                let sql = self.query_editor.text();
                self.send(AppCommand::ExecuteSql { sql });
                self.active_tab = Tab::Results;
                self.status = "Running query...".to_string();
            }
            Message::ConfirmQuery => {
                if let Some(confirmation) = &self.snapshot.query.confirmation {
                    self.send(AppCommand::ConfirmSql {
                        operation_id: confirmation.operation_id,
                    });
                    self.active_tab = Tab::Results;
                    self.status = "Running confirmed query...".to_string();
                }
            }
            Message::CancelOperation => {
                self.send(AppCommand::Cancel { operation_id: None });
                self.status = "Cancellation requested".to_string();
            }
            Message::PreviewTable => {
                self.preview_offset = 0;
                self.request_preview();
            }
            Message::SearchChanged(query) => {
                self.result_search.clone_from(&query);
                self.result_page = 0;
                self.send(AppCommand::SearchResults { query });
            }
            Message::PreviousResultPage => {
                self.result_page = self.result_page.saturating_sub(1);
            }
            Message::NextResultPage => {
                let total = self.filtered_rows().len();
                let page_size = self.preferences.result_page_size.max(1);
                if (self.result_page + 1).saturating_mul(page_size) < total {
                    self.result_page = self.result_page.saturating_add(1);
                }
            }
            Message::PreviousPreviewPage => {
                self.preview_offset = self.preview_offset.saturating_sub(200);
                self.request_preview();
            }
            Message::NextPreviewPage => {
                self.preview_offset = self.preview_offset.saturating_add(200);
                self.request_preview();
            }
            Message::SelectResultRow(index) => self.selected_result_row = Some(index),
            Message::ResizeColumn { index, delta } => {
                if let Some(width) = self.column_widths.get_mut(index) {
                    *width = (*width + delta).clamp(72.0, 640.0);
                }
            }
            Message::ToggleProfileEditor => {
                self.profile_editor_open = !self.profile_editor_open;
            }
            Message::NewProfile => {
                self.profile_draft = ProfileDraft::default();
                self.profile_editor_open = true;
            }
            Message::EditProfile => {
                if let Some(profile) = self.selected_profile_data() {
                    self.profile_draft = ProfileDraft::from(profile);
                    self.profile_editor_open = true;
                }
            }
            Message::ProfileNameChanged(value) => self.profile_draft.name = value,
            Message::ProfileHostChanged(value) => self.profile_draft.host = value,
            Message::ProfilePortChanged(value) => self.profile_draft.port = value,
            Message::ProfileUserChanged(value) => self.profile_draft.user = value,
            Message::ProfileDatabaseChanged(value) => self.profile_draft.database = value,
            Message::ProfileReadOnlyChanged(value) => self.profile_draft.read_only = value,
            Message::CycleTlsMode => {
                self.profile_draft.tls_mode = match self.profile_draft.tls_mode {
                    TlsMode::Disabled => TlsMode::Prefer,
                    TlsMode::Prefer => TlsMode::Require,
                    TlsMode::Require => TlsMode::VerifyIdentity,
                    TlsMode::VerifyIdentity => TlsMode::Disabled,
                };
            }
            Message::CyclePasswordSource => {
                self.profile_draft.password_source = match self.profile_draft.password_source {
                    PasswordSource::EnvVar => PasswordSource::Keyring,
                    PasswordSource::Keyring => PasswordSource::EnvVar,
                };
            }
            Message::PickCaCertificate => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .add_filter("Certificates", &["pem", "crt", "cer"])
                            .pick_file()
                            .await
                            .map(|file| file.path().to_path_buf())
                    },
                    Message::CaCertificatePicked,
                );
            }
            Message::CaCertificatePicked(path) => {
                self.profile_draft.tls_ca_cert_path = path.map(|path| path.display().to_string());
            }
            Message::PickClientCertificate => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .add_filter("Client certificates", &["pem", "crt", "cer"])
                            .pick_file()
                            .await
                            .map(|file| file.path().to_path_buf())
                    },
                    Message::ClientCertificatePicked,
                );
            }
            Message::ClientCertificatePicked(path) => {
                self.profile_draft.tls_client_cert_path =
                    path.map(|path| path.display().to_string());
            }
            Message::PickClientKey => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .add_filter("Private keys", &["pem", "key"])
                            .pick_file()
                            .await
                            .map(|file| file.path().to_path_buf())
                    },
                    Message::ClientKeyPicked,
                );
            }
            Message::ClientKeyPicked(path) => {
                self.profile_draft.tls_client_key_path =
                    path.map(|path| path.display().to_string());
            }
            Message::SaveProfile => self.save_profile(),
            Message::DeleteProfile => {
                if let Some(profile_name) = self.selected_profile.clone() {
                    self.send(AppCommand::DeleteProfile { profile_name });
                    self.selected_profile = None;
                    self.status = "Profile deleted".to_string();
                }
            }
            Message::SetDefaultProfile => {
                if let Some(profile_name) = self.selected_profile.clone() {
                    self.send(AppCommand::SetDefaultProfile { profile_name });
                }
            }
            Message::SetQuickReconnectProfile => {
                if let Some(profile_name) = self.selected_profile.clone() {
                    self.send(AppCommand::SetQuickReconnectProfile { profile_name });
                }
            }
            Message::ChooseExport(scope) => {
                let extension = match self.export_format {
                    ExportFormat::Csv => "csv",
                    ExportFormat::Json => "json",
                    ExportFormat::JsonLines => "jsonl",
                };
                return Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new()
                            .set_file_name(format!("myr-export.{extension}"))
                            .save_file()
                            .await
                            .map(|file| file.path().to_path_buf())
                    },
                    move |path| Message::ExportPathPicked(path, scope.clone()),
                );
            }
            Message::ExportPathPicked(path, scope) => {
                if let Some(path) = path {
                    self.send(AppCommand::Export(ExportRequest {
                        path,
                        format: self.export_format,
                        scope,
                        typed_values: true,
                        gzip: false,
                    }));
                    self.status = "Exporting...".to_string();
                }
            }
            Message::CycleExportFormat => {
                self.export_format = match self.export_format {
                    ExportFormat::Csv => ExportFormat::Json,
                    ExportFormat::Json => ExportFormat::JsonLines,
                    ExportFormat::JsonLines => ExportFormat::Csv,
                };
            }
            Message::ToggleTheme => {
                self.preferences.color_scheme = match self.preferences.color_scheme {
                    ColorScheme::Dark => ColorScheme::Light,
                    ColorScheme::Light => ColorScheme::Dark,
                };
                if let Err(error) = self.preferences.save_default() {
                    self.status = error;
                }
            }
            Message::ClearError => self.send(AppCommand::ClearError),
        }
        Task::none()
    }

    #[must_use]
    pub fn view(&self) -> Element<'_, Message> {
        let top = self.top_bar();
        let body = row![self.sidebar(), self.main_area()]
            .spacing(12)
            .height(Length::Fill);
        let bottom = self.bottom_bar();
        container(column![top, rule::horizontal(1), body, rule::horizontal(1), bottom].spacing(8))
            .padding(12)
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn top_bar(&self) -> Element<'_, Message> {
        let connected = self.snapshot.connection.status == ConnectionStatus::Connected;
        let connection_action = if connected {
            button("Disconnect").on_press(Message::Disconnect)
        } else {
            button("Connect").on_press(Message::Connect)
        };
        let status = format!("{:?}", self.snapshot.connection.status);
        row![
            text("myr").size(28),
            pick_list(
                self.profile_names(),
                self.selected_profile.clone(),
                Message::ProfileSelected
            )
            .placeholder("Connection profile"),
            connection_action,
            button("New").on_press(Message::NewProfile),
            button("Edit").on_press(Message::EditProfile),
            button("Profiles").on_press(Message::ToggleProfileEditor),
            space::horizontal(),
            text(status),
            button(match self.preferences.color_scheme {
                ColorScheme::Dark => "Light",
                ColorScheme::Light => "Dark",
            })
            .on_press(Message::ToggleTheme),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center)
        .into()
    }

    fn sidebar(&self) -> Element<'_, Message> {
        let mut databases = column![text("DATABASES").size(13)].spacing(4);
        for database in &self.snapshot.schema.databases {
            databases = databases.push(
                button(text(database.clone()))
                    .width(Length::Fill)
                    .on_press(Message::DatabaseSelected(database.clone())),
            );
        }

        let mut tables = column![text("TABLES").size(13)].spacing(4);
        for table_name in &self.snapshot.schema.tables {
            tables = tables.push(
                button(text(table_name.clone()))
                    .width(Length::Fill)
                    .on_press(Message::TableSelected(table_name.clone())),
            );
        }

        let mut columns = column![text("COLUMNS").size(13)].spacing(3);
        for schema_column in &self.snapshot.schema.columns {
            columns = columns.push(text(format!(
                "{}  {}{}",
                schema_column.name,
                schema_column.data_type,
                if schema_column.nullable { " ?" } else { "" }
            )));
        }

        let sidebar = column![
            databases,
            rule::horizontal(1),
            tables,
            rule::horizontal(1),
            columns
        ]
        .spacing(10)
        .width(260);
        container(scrollable(sidebar).height(Length::Fill))
            .padding(8)
            .width(280)
            .height(Length::Fill)
            .style(container::rounded_box)
            .into()
    }

    fn main_area(&self) -> Element<'_, Message> {
        if self.profile_editor_open {
            return self.profile_editor();
        }
        let tabs = row![
            button("Schema").on_press(Message::TabSelected(Tab::Schema)),
            button("Query").on_press(Message::TabSelected(Tab::Query)),
            button("Results").on_press(Message::TabSelected(Tab::Results)),
        ]
        .spacing(6);
        let content = match self.active_tab {
            Tab::Schema => self.schema_view(),
            Tab::Query => self.query_view(),
            Tab::Results => self.results_view(),
        };
        container(column![tabs, content].spacing(10))
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn schema_view(&self) -> Element<'_, Message> {
        let database = self
            .snapshot
            .schema
            .selected_database
            .as_deref()
            .unwrap_or("No database selected");
        let table_name = self
            .snapshot
            .schema
            .selected_table
            .as_deref()
            .unwrap_or("No table selected");
        let mut content = column![
            text("Schema explorer").size(24),
            text(format!("Database: {database}")),
            text(format!("Table: {table_name}")),
        ]
        .spacing(8);
        if self.snapshot.schema.selected_table.is_some() {
            content = content.push(button("Preview 200 rows").on_press(Message::PreviewTable));
        }
        if !self.snapshot.schema.relationships.is_empty() {
            content = content.push(text("Relationships").size(18));
            for relationship in &self.snapshot.schema.relationships {
                content = content.push(text(format!(
                    "{:?}: {}.{} ({})",
                    relationship.direction,
                    relationship.related_database,
                    relationship.related_table,
                    relationship.constraint_name
                )));
            }
        }
        container(scrollable(content))
            .padding(16)
            .width(Length::Fill)
            .into()
    }

    fn query_view(&self) -> Element<'_, Message> {
        let mut actions = row![
            button("Run").on_press(Message::ExecuteQuery),
            button("Cancel").on_press(Message::CancelOperation),
        ]
        .spacing(8);
        if self.snapshot.query.confirmation.is_some() {
            actions = actions.push(button("Confirm risky SQL").on_press(Message::ConfirmQuery));
        }
        let reasons = self.snapshot.query.confirmation.as_ref().map_or_else(
            || "Safe mode checks writes, DDL, transactions, and multi-statements.".to_string(),
            |confirmation| format!("Confirmation required: {}", confirmation.reasons.join(", ")),
        );
        column![
            text("SQL editor").size(24),
            text_editor(&self.query_editor)
                .placeholder("Enter SQL")
                .on_action(Message::QueryEdited)
                .height(Length::Fill),
            text(reasons),
            actions,
        ]
        .spacing(10)
        .height(Length::Fill)
        .into()
    }

    fn results_view(&self) -> Element<'_, Message> {
        let rows = self.visible_rows();
        let mut table_columns = Vec::new();
        for (index, column_meta) in self.snapshot.results.columns.iter().enumerate() {
            let header = column_meta.name.clone();
            let header_controls = row![
                text(header),
                button("−").on_press(Message::ResizeColumn {
                    index,
                    delta: -24.0,
                }),
                button("+").on_press(Message::ResizeColumn { index, delta: 24.0 }),
            ]
            .spacing(4)
            .align_y(iced::Alignment::Center);
            table_columns.push(
                table_column(
                    header_controls,
                    move |(row_index, row): (usize, myr_core::query_runner::QueryRow)| {
                        let value = row.values.get(index).map_or_else(
                            String::new,
                            myr_core::query_runner::QueryValue::display_text,
                        );
                        button(text(value))
                            .on_press(Message::SelectResultRow(row_index))
                            .width(Length::Fill)
                    },
                )
                .width(Length::Fixed(
                    self.column_widths.get(index).copied().unwrap_or(160.0),
                )),
            );
        }
        let result_table: Element<'_, Message> = if table_columns.is_empty() {
            container(text("No result columns yet"))
                .padding(16)
                .width(Length::Fill)
                .into()
        } else {
            scrollable(table(table_columns, rows).width(Length::Fill))
                .height(Length::Fill)
                .into()
        };
        let pagination = row![
            button("Previous buffer page").on_press(Message::PreviousResultPage),
            text(format!("Page {}", self.result_page + 1)),
            button("Next buffer page").on_press(Message::NextResultPage),
            button("Previous query page").on_press(Message::PreviousPreviewPage),
            button("Next query page").on_press(Message::NextPreviewPage),
            text(format!("OFFSET {}", self.preview_offset)),
            text(format!(
                "{} seen / {} loaded{}",
                self.snapshot.results.rows_seen,
                self.snapshot.results.rows_buffered,
                if self.snapshot.results.truncated {
                    " (buffer limit reached)"
                } else {
                    ""
                }
            )),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);
        let full_sql = self.query_editor.text();
        let export_actions = row![
            button(export_format_label(self.export_format)).on_press(Message::CycleExportFormat),
            button("Export loaded rows").on_press(Message::ChooseExport(ExportScope::LoadedRows)),
            button("Export full read query").on_press(Message::ChooseExport(
                ExportScope::FullQuery { sql: full_sql }
            )),
        ]
        .spacing(8);
        column![
            text_input("Search loaded rows", &self.result_search).on_input(Message::SearchChanged),
            pagination,
            result_table,
            export_actions,
        ]
        .spacing(8)
        .height(Length::Fill)
        .into()
    }

    fn profile_editor(&self) -> Element<'_, Message> {
        let ca_certificate = self
            .profile_draft
            .tls_ca_cert_path
            .as_deref()
            .unwrap_or("System roots");
        let client_certificate = self
            .profile_draft
            .tls_client_cert_path
            .as_deref()
            .unwrap_or("No client certificate");
        let client_key = self
            .profile_draft
            .tls_client_key_path
            .as_deref()
            .unwrap_or("No client key");
        let editor = column![
            row![
                text("Connection profile").size(24),
                space::horizontal(),
                button("Close").on_press(Message::ToggleProfileEditor)
            ],
            text_input("Profile name", &self.profile_draft.name)
                .on_input(Message::ProfileNameChanged),
            row![
                text_input("Host", &self.profile_draft.host).on_input(Message::ProfileHostChanged),
                text_input("Port", &self.profile_draft.port)
                    .on_input(Message::ProfilePortChanged)
                    .width(120),
            ]
            .spacing(8),
            text_input("User", &self.profile_draft.user).on_input(Message::ProfileUserChanged),
            text_input("Default database (optional)", &self.profile_draft.database)
                .on_input(Message::ProfileDatabaseChanged),
            row![
                button(text(format!("TLS: {:?}", self.profile_draft.tls_mode)))
                    .on_press(Message::CycleTlsMode),
                button(text(format!(
                    "Password: {:?}",
                    self.profile_draft.password_source
                )))
                .on_press(Message::CyclePasswordSource),
                checkbox(self.profile_draft.read_only)
                    .label("Read-only")
                    .on_toggle(Message::ProfileReadOnlyChanged),
            ]
            .spacing(8),
            row![
                button("Choose CA certificate").on_press(Message::PickCaCertificate),
                text(ca_certificate),
            ]
            .spacing(8),
            row![
                button("Choose client certificate").on_press(Message::PickClientCertificate),
                text(client_certificate),
            ]
            .spacing(8),
            row![
                button("Choose client key").on_press(Message::PickClientKey),
                text(client_key),
            ]
            .spacing(8),
            text("Passwords are never stored in profiles.toml; use MYR_DB_PASSWORD or keyring."),
            row![
                button("Save profile").on_press(Message::SaveProfile),
                button("Delete selected").on_press(Message::DeleteProfile),
                button("Make default").on_press(Message::SetDefaultProfile),
                button("Quick reconnect").on_press(Message::SetQuickReconnectProfile),
            ]
            .spacing(8),
        ]
        .spacing(10);
        container(editor)
            .padding(16)
            .width(Length::Fill)
            .style(container::rounded_box)
            .into()
    }

    fn bottom_bar(&self) -> Element<'_, Message> {
        let progress = if self.status == "Cancellation requested" {
            self.status.clone()
        } else if self.snapshot.query.running {
            format!("Query: {} rows", self.snapshot.results.rows_seen)
        } else if self.snapshot.export.running {
            format!(
                "Export: {} rows / {} bytes",
                self.snapshot.export.rows_written, self.snapshot.export.bytes_written
            )
        } else {
            self.status.clone()
        };
        let mut content = row![text(progress), space::horizontal()].spacing(8);
        if let Some(error) = &self.snapshot.last_error {
            content = content
                .push(text(format!("{:?}: {}", error.kind, error.message)))
                .push(button("Dismiss").on_press(Message::ClearError));
        }
        content.into()
    }

    fn refresh_application(&mut self) {
        let Some(application) = self.application.clone() else {
            return;
        };
        self.snapshot = application.snapshot();

        let mut events = Vec::new();
        if let Some(receiver) = &mut self.events {
            loop {
                match receiver.try_recv() {
                    Ok(event) => events.push(event),
                    Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {
                        self.snapshot = application.snapshot();
                    }
                    Err(
                        tokio::sync::broadcast::error::TryRecvError::Empty
                        | tokio::sync::broadcast::error::TryRecvError::Closed,
                    ) => break,
                }
            }
        }
        for event in events {
            self.handle_application_event(event);
        }

        self.column_widths
            .resize(self.snapshot.results.columns.len(), 160.0);
        if self
            .selected_profile
            .as_ref()
            .is_none_or(|selected| !self.snapshot.profiles.iter().any(|p| &p.name == selected))
        {
            self.selected_profile = self.snapshot.profiles.first().map(|p| p.name.clone());
        }
    }

    fn handle_application_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::SnapshotChanged(snapshot) => self.snapshot = *snapshot,
            AppEvent::ResultsBatch { .. } => {}
            AppEvent::Progress(progress) => match progress.kind {
                OperationKind::Query => {
                    self.status = format!("Query: {} rows", progress.rows);
                }
                OperationKind::Export => {
                    self.status =
                        format!("Export: {} rows / {} bytes", progress.rows, progress.bytes);
                }
                OperationKind::Connection | OperationKind::Schema | OperationKind::Profile => {}
            },
            AppEvent::ConfirmationRequired(_) => {
                self.active_tab = Tab::Query;
                self.status = "Confirmation required".to_string();
            }
            AppEvent::Finished { kind, .. } => match kind {
                OperationKind::Connection => {
                    self.status = if self.snapshot.connection.status == ConnectionStatus::Connected
                    {
                        "Connected; choose a database".to_string()
                    } else {
                        "Disconnected".to_string()
                    };
                }
                OperationKind::Schema => {
                    self.status = if self.snapshot.schema.selected_table.is_some() {
                        format!("Loaded {} columns", self.snapshot.schema.columns.len())
                    } else if self.snapshot.schema.selected_database.is_some() {
                        format!("Loaded {} tables", self.snapshot.schema.tables.len())
                    } else {
                        format!("Loaded {} databases", self.snapshot.schema.databases.len())
                    };
                }
                OperationKind::Query => {
                    self.active_tab = Tab::Results;
                    self.status =
                        format!("Query finished: {} rows", self.snapshot.results.rows_seen);
                }
                OperationKind::Export => {
                    self.status = format!(
                        "Export finished: {} rows, {} bytes",
                        self.snapshot.export.rows_written, self.snapshot.export.bytes_written
                    );
                }
                OperationKind::Profile => {}
            },
            AppEvent::Error(error) => {
                self.status = if error.kind == myr_application::AppErrorKind::Cancellation {
                    "Cancellation completed".to_string()
                } else {
                    "Operation failed".to_string()
                };
            }
        }
    }

    fn save_profile(&mut self) {
        let Ok(port) = self.profile_draft.port.parse::<u16>() else {
            self.status = "Port must be between 1 and 65535".to_string();
            return;
        };
        if self.profile_draft.name.trim().is_empty()
            || self.profile_draft.host.trim().is_empty()
            || self.profile_draft.user.trim().is_empty()
        {
            self.status = "Name, host, and user are required".to_string();
            return;
        }
        if let Some(original_name) = &self.profile_draft.original_name {
            if original_name != &self.profile_draft.name {
                self.send(AppCommand::DeleteProfile {
                    profile_name: original_name.clone(),
                });
            }
        }
        let profile = self.profile_from_draft(port);
        self.selected_profile = Some(profile.name.clone());
        self.send(AppCommand::UpsertProfile { profile });
        self.profile_editor_open = false;
        self.status = "Profile saved".to_string();
    }

    fn profile_from_draft(&self, port: u16) -> ConnectionProfile {
        let mut profile = self
            .profile_draft
            .original_name
            .as_deref()
            .and_then(|name| {
                self.snapshot
                    .profiles
                    .iter()
                    .find(|profile| profile.name == name)
            })
            .cloned()
            .unwrap_or_else(|| {
                ConnectionProfile::new(
                    self.profile_draft.name.trim(),
                    self.profile_draft.host.trim(),
                    self.profile_draft.user.trim(),
                )
            });
        profile.name = self.profile_draft.name.trim().to_string();
        profile.host = self.profile_draft.host.trim().to_string();
        profile.user = self.profile_draft.user.trim().to_string();
        profile.port = port;
        profile.database = (!self.profile_draft.database.trim().is_empty())
            .then(|| self.profile_draft.database.trim().to_string());
        profile.tls_mode = self.profile_draft.tls_mode;
        profile.password_source = self.profile_draft.password_source;
        profile
            .tls_ca_cert_path
            .clone_from(&self.profile_draft.tls_ca_cert_path);
        profile
            .tls_client_cert_path
            .clone_from(&self.profile_draft.tls_client_cert_path);
        profile
            .tls_client_key_path
            .clone_from(&self.profile_draft.tls_client_key_path);
        profile.read_only = self.profile_draft.read_only;
        profile
    }

    fn request_preview(&mut self) {
        if let (Some(database), Some(table)) = (
            self.snapshot.schema.selected_database.clone(),
            self.snapshot.schema.selected_table.clone(),
        ) {
            self.send(AppCommand::PreviewTable {
                database,
                table,
                limit: 200,
                offset: self.preview_offset,
            });
            self.active_tab = Tab::Results;
            self.status = format!("Loading preview at offset {}...", self.preview_offset);
        }
    }

    fn send(&mut self, command: AppCommand) {
        let Some(application) = &self.application else {
            return;
        };
        if let Err(error) = application.try_command(command) {
            self.status = format!("Application command failed: {error:?}");
        }
    }

    fn profile_names(&self) -> Vec<String> {
        self.snapshot
            .profiles
            .iter()
            .map(|profile| profile.name.clone())
            .collect()
    }

    fn selected_profile_data(&self) -> Option<&ConnectionProfile> {
        let selected = self.selected_profile.as_deref()?;
        self.snapshot
            .profiles
            .iter()
            .find(|profile| profile.name == selected)
    }

    fn filtered_rows(&self) -> Vec<(usize, myr_core::query_runner::QueryRow)> {
        let needle = self.result_search.trim().to_ascii_lowercase();
        self.snapshot
            .results
            .rows
            .iter()
            .cloned()
            .enumerate()
            .filter(|(_, row)| {
                needle.is_empty()
                    || row
                        .values
                        .iter()
                        .any(|value| value.display_text().to_ascii_lowercase().contains(&needle))
            })
            .collect()
    }

    fn visible_rows(&self) -> Vec<(usize, myr_core::query_runner::QueryRow)> {
        let page_size = self.preferences.result_page_size.max(1);
        self.filtered_rows()
            .into_iter()
            .skip(self.result_page.saturating_mul(page_size))
            .take(page_size)
            .collect()
    }
}

/// Opens the native `myr` GUI and runs it until its window closes.
///
/// # Errors
///
/// Returns an Iced runtime error if the window or renderer cannot be initialized.
pub fn run(application: ApplicationHandle) -> iced::Result {
    let preferences = GuiPreferences::load_default();
    let window = iced::window::Settings {
        size: iced::Size::new(preferences.width, preferences.height),
        min_size: Some(iced::Size::new(900.0, 620.0)),
        ..iced::window::Settings::default()
    };
    iced::application(
        move || Gui::new(application.clone(), preferences.clone()),
        Gui::update,
        Gui::view,
    )
    .title("myr — MySQL explorer")
    .window(window)
    .subscription(|_| subscription())
    .theme(gui_theme)
    .run()
}

fn gui_theme(gui: &Gui) -> Theme {
    match gui.preferences.color_scheme {
        ColorScheme::Dark => Theme::Dark,
        ColorScheme::Light => Theme::Light,
    }
}

fn subscription() -> Subscription<Message> {
    iced::time::every(Duration::from_millis(100)).map(|_| Message::Tick)
}

fn export_format_label(format: ExportFormat) -> &'static str {
    match format {
        ExportFormat::Csv => "Format: CSV",
        ExportFormat::Json => "Format: JSON",
        ExportFormat::JsonLines => "Format: JSONL",
    }
}

#[cfg(test)]
mod tests {
    use iced_test::simulator;
    use myr_application::{ConfirmationSnapshot, OperationId};
    use myr_core::query_runner::{ColumnMeta, QueryRow, QueryValue};

    use super::*;

    fn sample_gui() -> Gui {
        let mut snapshot = AppSnapshot::default();
        snapshot.schema.databases = vec!["app".to_string()];
        snapshot.schema.selected_database = Some("app".to_string());
        snapshot.schema.tables = vec!["users".to_string()];
        snapshot.schema.selected_table = Some("users".to_string());
        snapshot.results.columns = vec![ColumnMeta::new("id"), ColumnMeta::new("name")];
        snapshot.results.rows = vec![QueryRow::from_values(vec![
            QueryValue::UInt(1),
            QueryValue::Text("Ada".to_string()),
        ])];
        snapshot.results.rows_seen = 1;
        snapshot.results.rows_buffered = 1;
        Gui::preview(snapshot)
    }

    #[test]
    fn headless_navigation_exposes_schema_query_and_results() {
        let mut gui = sample_gui();
        let mut ui = simulator(gui.view());
        assert!(ui.find("Schema explorer").is_ok());

        ui.click("Query").expect("query tab should be clickable");
        for message in ui.into_messages() {
            let _ = gui.update(message);
        }
        let mut ui = simulator(gui.view());
        assert!(ui.find("SQL editor").is_ok());

        ui.click("Results")
            .expect("results tab should be clickable");
        for message in ui.into_messages() {
            let _ = gui.update(message);
        }
        let mut ui = simulator(gui.view());
        assert!(ui.find("Ada").is_ok());
        assert!(ui.find("1 seen / 1 loaded").is_ok());
    }

    #[test]
    fn profile_editor_documents_secret_handling_and_tls() {
        let mut gui = sample_gui();
        gui.profile_editor_open = true;
        let mut ui = simulator(gui.view());
        assert!(ui.find("Connection profile").is_ok());
        assert!(ui
            .find("Passwords are never stored in profiles.toml; use MYR_DB_PASSWORD or keyring.")
            .is_ok());
        assert!(ui.find("TLS: Prefer").is_ok());
        assert!(ui.find("Choose CA certificate").is_ok());
        assert!(ui.find("Choose client certificate").is_ok());
        assert!(ui.find("Choose client key").is_ok());
    }

    #[test]
    fn result_search_filters_the_loaded_buffer() {
        let mut gui = sample_gui();
        gui.result_search = "missing".to_string();
        gui.active_tab = Tab::Results;
        let mut ui = simulator(gui.view());
        assert!(ui.find("Ada").is_err());
        drop(ui);
        gui.result_search = "ada".to_string();
        let mut ui = simulator(gui.view());
        assert!(ui.find("Ada").is_ok());
    }

    #[test]
    fn headless_layout_supports_alpha_window_sizes() {
        let gui = sample_gui();
        for size in [
            iced::Size::new(1_280.0, 800.0),
            iced::Size::new(1_024.0, 768.0),
        ] {
            let mut ui =
                iced_test::Simulator::with_size(iced::Settings::default(), size, gui.view());
            assert!(ui.find("myr").is_ok());
            assert!(ui.find("Schema explorer").is_ok());
        }
    }

    #[test]
    fn headless_snapshots_render_both_viewports_at_scale_two() {
        let gui = sample_gui();
        let temp_dir = tempfile::TempDir::new().expect("snapshot directory should create");

        for (name, size, physical_size) in [
            (
                "1280x800",
                iced::Size::new(1_280.0, 800.0),
                "width: 2560, height: 1600",
            ),
            (
                "1024x768",
                iced::Size::new(1_024.0, 768.0),
                "width: 2048, height: 1536",
            ),
        ] {
            let mut ui =
                iced_test::Simulator::with_size(iced::Settings::default(), size, gui.view());
            let snapshot = ui.snapshot(&Theme::Dark).expect("snapshot should render");
            let debug = format!("{snapshot:?}");
            assert!(debug.contains("scale: 2"));
            assert!(debug.contains(physical_size));
            assert!(snapshot
                .matches_hash(temp_dir.path().join(name))
                .expect("snapshot hash should write and match"));
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn messages_drive_connection_query_profile_and_export_state() {
        let mut gui = sample_gui();

        let _ = gui.update(Message::Connect);
        assert_eq!(gui.status, "Choose a profile first");
        let _ = gui.update(Message::ProfileSelected("local".to_string()));
        let _ = gui.update(Message::Connect);
        assert_eq!(gui.status, "Connecting...");
        let _ = gui.update(Message::Disconnect);
        assert_eq!(gui.status, "Disconnecting...");
        let _ = gui.update(Message::DatabaseSelected("analytics".to_string()));
        assert_eq!(gui.status, "Loading tables...");

        gui.snapshot.schema.selected_database = None;
        let _ = gui.update(Message::TableSelected("events".to_string()));
        gui.snapshot.schema.selected_database = Some("app".to_string());
        let _ = gui.update(Message::TableSelected("users".to_string()));
        assert_eq!(gui.status, "Loading columns...");
        let _ = gui.update(Message::TabSelected(Tab::Query));
        let _ = gui.update(Message::ExecuteQuery);
        assert_eq!(gui.active_tab, Tab::Results);
        gui.snapshot.query.confirmation = Some(ConfirmationSnapshot {
            operation_id: OperationId(7),
            sql: "DELETE FROM users".to_string(),
            reasons: vec!["DestructiveStatement".to_string()],
        });
        let _ = gui.update(Message::ConfirmQuery);
        assert_eq!(gui.status, "Running confirmed query...");
        let _ = gui.update(Message::CancelOperation);
        assert_eq!(gui.status, "Cancellation requested");
        let mut ui = simulator(gui.view());
        assert!(ui.find("Cancellation requested").is_ok());
        drop(ui);
        let _ = gui.update(Message::PreviewTable);
        assert_eq!(gui.status, "Loading preview at offset 0...");
        let _ = gui.update(Message::NextPreviewPage);
        assert_eq!(gui.preview_offset, 200);
        let _ = gui.update(Message::PreviousPreviewPage);
        assert_eq!(gui.preview_offset, 0);
        let _ = gui.update(Message::ResizeColumn {
            index: 0,
            delta: 24.0,
        });
        assert!((gui.column_widths[0] - 184.0).abs() < f32::EPSILON);

        let _ = gui.update(Message::SearchChanged("ada".to_string()));
        assert_eq!(gui.result_search, "ada");
        gui.snapshot.results.rows = (0..205)
            .map(|index| QueryRow::from_values(vec![QueryValue::UInt(index)]))
            .collect();
        let _ = gui.update(Message::SearchChanged(String::new()));
        let _ = gui.update(Message::NextResultPage);
        assert_eq!(gui.result_page, 1);
        let _ = gui.update(Message::PreviousResultPage);
        assert_eq!(gui.result_page, 0);
        let _ = gui.update(Message::SelectResultRow(3));
        assert_eq!(gui.selected_result_row, Some(3));

        let _ = gui.update(Message::ToggleProfileEditor);
        assert!(gui.profile_editor_open);
        let _ = gui.update(Message::NewProfile);
        let _ = gui.update(Message::ProfileNameChanged("team".to_string()));
        let _ = gui.update(Message::ProfileHostChanged("db.internal".to_string()));
        let _ = gui.update(Message::ProfilePortChanged("invalid".to_string()));
        let _ = gui.update(Message::ProfileUserChanged("reader".to_string()));
        let _ = gui.update(Message::ProfileDatabaseChanged("analytics".to_string()));
        let _ = gui.update(Message::ProfileReadOnlyChanged(true));
        let _ = gui.update(Message::SaveProfile);
        assert!(gui.status.contains("Port"));
        let _ = gui.update(Message::ProfilePortChanged("3307".to_string()));
        let _ = gui.update(Message::CycleTlsMode);
        let _ = gui.update(Message::CycleTlsMode);
        let _ = gui.update(Message::CycleTlsMode);
        let _ = gui.update(Message::CycleTlsMode);
        let _ = gui.update(Message::CyclePasswordSource);
        let _ = gui.update(Message::CyclePasswordSource);
        let certificate = PathBuf::from("/tmp/test-ca.pem");
        let _ = gui.update(Message::CaCertificatePicked(Some(certificate.clone())));
        assert_eq!(
            gui.profile_draft.tls_ca_cert_path.as_deref(),
            certificate.to_str()
        );
        let client_certificate = PathBuf::from("/tmp/test-client.pem");
        let client_key = PathBuf::from("/tmp/test-client.key");
        let _ = gui.update(Message::ClientCertificatePicked(Some(
            client_certificate.clone(),
        )));
        let _ = gui.update(Message::ClientKeyPicked(Some(client_key.clone())));
        assert_eq!(
            gui.profile_draft.tls_client_cert_path.as_deref(),
            client_certificate.to_str()
        );
        assert_eq!(
            gui.profile_draft.tls_client_key_path.as_deref(),
            client_key.to_str()
        );
        let _ = gui.update(Message::SaveProfile);
        assert_eq!(gui.selected_profile.as_deref(), Some("team"));
        assert!(!gui.profile_editor_open);

        let mut local = ConnectionProfile::new("local", "localhost", "root");
        local.database = Some("app".to_string());
        local.keyring_service = Some("custom-service".to_string());
        local.is_default = true;
        gui.snapshot.profiles = vec![local];
        gui.selected_profile = Some("local".to_string());
        let _ = gui.update(Message::EditProfile);
        assert_eq!(gui.profile_draft.name, "local");
        let _ = gui.update(Message::ProfileNameChanged("renamed".to_string()));
        let rebuilt = gui.profile_from_draft(3306);
        assert_eq!(rebuilt.keyring_service.as_deref(), Some("custom-service"));
        assert!(rebuilt.is_default);
        let _ = gui.update(Message::SaveProfile);
        let _ = gui.update(Message::SetDefaultProfile);
        let _ = gui.update(Message::SetQuickReconnectProfile);
        let _ = gui.update(Message::DeleteProfile);
        assert!(gui.selected_profile.is_none());

        let export_path = PathBuf::from("/tmp/myr-test.jsonl");
        let _ = gui.update(Message::ExportPathPicked(
            Some(export_path),
            ExportScope::LoadedRows,
        ));
        assert_eq!(gui.status, "Exporting...");
        let _ = gui.update(Message::ExportPathPicked(None, ExportScope::LoadedRows));
        let _ = gui.update(Message::CycleExportFormat);
        assert_eq!(gui.export_format, ExportFormat::Csv);
        let _ = gui.update(Message::CycleExportFormat);
        let _ = gui.update(Message::CycleExportFormat);
        let _ = gui.update(Message::ClearError);
        let _ = gui.update(Message::Tick);
    }

    #[test]
    fn application_events_report_fast_operation_completion() {
        let mut gui = sample_gui();
        gui.snapshot.connection.status = ConnectionStatus::Connected;
        gui.handle_application_event(AppEvent::Finished {
            operation_id: OperationId(1),
            kind: OperationKind::Connection,
        });
        assert_eq!(gui.status, "Connected; choose a database");

        gui.handle_application_event(AppEvent::Finished {
            operation_id: OperationId(2),
            kind: OperationKind::Schema,
        });
        assert_eq!(gui.status, "Loaded 0 columns");
        gui.snapshot.schema.selected_table = None;
        gui.handle_application_event(AppEvent::Finished {
            operation_id: OperationId(3),
            kind: OperationKind::Schema,
        });
        assert_eq!(gui.status, "Loaded 1 tables");
        gui.snapshot.schema.selected_database = None;
        gui.handle_application_event(AppEvent::Finished {
            operation_id: OperationId(4),
            kind: OperationKind::Schema,
        });
        assert_eq!(gui.status, "Loaded 1 databases");

        gui.snapshot.results.rows_seen = 2;
        gui.handle_application_event(AppEvent::Progress(myr_application::OperationProgress {
            operation_id: OperationId(5),
            kind: OperationKind::Query,
            rows: 1,
            bytes: 0,
            message: "query streaming".to_string(),
        }));
        assert_eq!(gui.status, "Query: 1 rows");
        gui.handle_application_event(AppEvent::Finished {
            operation_id: OperationId(5),
            kind: OperationKind::Query,
        });
        assert_eq!(gui.status, "Query finished: 2 rows");
        assert_eq!(gui.active_tab, Tab::Results);

        gui.snapshot.export.rows_written = 2;
        gui.snapshot.export.bytes_written = 48;
        gui.handle_application_event(AppEvent::Progress(myr_application::OperationProgress {
            operation_id: OperationId(6),
            kind: OperationKind::Export,
            rows: 1,
            bytes: 24,
            message: "export streaming".to_string(),
        }));
        assert_eq!(gui.status, "Export: 1 rows / 24 bytes");
        gui.handle_application_event(AppEvent::Finished {
            operation_id: OperationId(6),
            kind: OperationKind::Export,
        });
        assert_eq!(gui.status, "Export finished: 2 rows, 48 bytes");

        let confirmation = ConfirmationSnapshot {
            operation_id: OperationId(7),
            sql: "UPDATE users SET name = 'Grace'".to_string(),
            reasons: vec!["WriteOperation".to_string()],
        };
        gui.handle_application_event(AppEvent::ConfirmationRequired(confirmation));
        assert_eq!(gui.status, "Confirmation required");
        assert_eq!(gui.active_tab, Tab::Query);

        gui.handle_application_event(AppEvent::Error(myr_application::AppError::new(
            myr_application::AppErrorKind::Cancellation,
            "query cancelled",
        )));
        assert_eq!(gui.status, "Cancellation completed");

        gui.handle_application_event(AppEvent::Error(myr_application::AppError::new(
            myr_application::AppErrorKind::Query,
            "query failed",
        )));
        assert_eq!(gui.status, "Operation failed");

        let mut snapshot = AppSnapshot::default();
        snapshot.schema.databases = vec!["replacement".to_string()];
        gui.handle_application_event(AppEvent::SnapshotChanged(Box::new(snapshot)));
        assert_eq!(gui.snapshot.schema.databases, ["replacement"]);
        gui.handle_application_event(AppEvent::ResultsBatch {
            operation_id: OperationId(8),
            columns: Vec::new(),
            rows: Vec::new(),
            rows_seen: 0,
        });
        gui.handle_application_event(AppEvent::Finished {
            operation_id: OperationId(9),
            kind: OperationKind::Profile,
        });
        gui.handle_application_event(AppEvent::Progress(myr_application::OperationProgress {
            operation_id: OperationId(10),
            kind: OperationKind::Schema,
            rows: 0,
            bytes: 0,
            message: "schema loading".to_string(),
        }));
    }
}
