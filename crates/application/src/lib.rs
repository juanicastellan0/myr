mod actor;
mod ports;
mod types;

pub use actor::{spawn_application, ApplicationHandle};
pub use ports::{ApplicationBackendError, ApplicationBackendFactory, ApplicationSession};
pub use types::{
    AppCommand, AppError, AppErrorKind, AppEvent, AppSnapshot, ConfirmationSnapshot,
    ConnectionSnapshot, ConnectionStatus, ExportFormat, ExportRequest, ExportScope, ExportSnapshot,
    OperationId, OperationKind, OperationProgress, QuerySnapshot, ResultsSnapshot, SchemaSnapshot,
};

#[must_use]
pub fn application_name() -> &'static str {
    "myr-application"
}
