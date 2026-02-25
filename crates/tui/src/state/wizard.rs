#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WizardField {
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
    pub(crate) fn next(self) -> Self {
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

    pub(crate) fn label(self) -> &'static str {
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
pub(crate) struct ConnectionWizardForm {
    pub(crate) profile_name: String,
    pub(crate) host: String,
    pub(crate) port: String,
    pub(crate) user: String,
    pub(crate) password_source: String,
    pub(crate) database: String,
    pub(crate) tls_mode: String,
    pub(crate) read_only: String,
    pub(crate) active_field: WizardField,
    pub(crate) editing: bool,
    pub(crate) edit_buffer: String,
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
