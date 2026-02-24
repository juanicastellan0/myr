use crate::sql_generator::SqlTarget;

use super::{ActionContext, ActionEngineError};

pub(super) fn context_selected_target(
    context: &ActionContext,
) -> Result<SqlTarget<'_>, ActionEngineError> {
    let table = context
        .selection
        .table
        .as_deref()
        .ok_or(ActionEngineError::MissingTableSelection)?;
    if context.selection.database.is_none() {
        return Err(ActionEngineError::MissingDatabaseSelection);
    }

    SqlTarget::new(context.selection.database.as_deref(), table).map_err(ActionEngineError::from)
}

pub(super) fn context_selected_column(context: &ActionContext) -> Result<&str, ActionEngineError> {
    context
        .selection
        .column
        .as_deref()
        .ok_or(ActionEngineError::MissingColumnSelection)
}
