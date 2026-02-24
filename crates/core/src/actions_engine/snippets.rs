use crate::sql_generator::quote_identifier;

use super::{ActionContext, PREVIEW_LIMIT};

fn qualified_selection_reference(context: &ActionContext) -> String {
    match (
        context.selection.database.as_deref(),
        context.selection.table.as_deref(),
    ) {
        (Some(database), Some(table)) => {
            format!("{}.{}", quote_identifier(database), quote_identifier(table))
        }
        (_, Some(table)) => quote_identifier(table),
        _ => "`table_name`".to_string(),
    }
}

fn selected_or_default_column(context: &ActionContext) -> String {
    context
        .selection
        .column
        .as_deref()
        .map_or_else(|| "`id`".to_string(), quote_identifier)
}

pub(super) fn select_snippet(context: &ActionContext) -> String {
    let table_ref = qualified_selection_reference(context);
    let column = selected_or_default_column(context);
    format!(
        "SELECT *\nFROM {table_ref}\nWHERE {column} = 'value'\nORDER BY {column} DESC\nLIMIT {PREVIEW_LIMIT};"
    )
}

pub(super) fn join_snippet(context: &ActionContext) -> String {
    let left_table = qualified_selection_reference(context);
    format!(
        "SELECT t1.*, t2.*\nFROM {left_table} AS t1\nJOIN `app`.`table_two` AS t2 ON t1.`id` = t2.`table_one_id`\nLIMIT {PREVIEW_LIMIT};"
    )
}
