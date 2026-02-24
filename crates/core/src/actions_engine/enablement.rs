use super::{
    suggest_explain_query, suggest_preview_limit, ActionContext, ActionId, AppView, PREVIEW_LIMIT,
};

pub(super) fn action_enabled(action_id: ActionId, context: &ActionContext) -> bool {
    match action_id {
        ActionId::PreviewTable
        | ActionId::DescribeTable
        | ActionId::ShowIndexes
        | ActionId::ShowCreateTable => {
            context.view == AppView::SchemaExplorer
                && context.selection.table.is_some()
                && context.selection.database.is_some()
                && !context.query_running
        }
        ActionId::PreviousPage => {
            context.pagination_enabled && context.can_page_previous && !context.query_running
        }
        ActionId::NextPage => {
            context.pagination_enabled && context.can_page_next && !context.query_running
        }
        ActionId::CountEstimate => {
            context.selection.table.is_some()
                && context.selection.database.is_some()
                && !context.query_running
        }
        ActionId::RunCurrentQuery => {
            !context.query_running
                && context
                    .query_text
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|query| !query.is_empty())
        }
        ActionId::ApplyLimit200 => {
            !context.query_running
                && context
                    .query_text
                    .as_deref()
                    .is_some_and(|query| suggest_preview_limit(query, PREVIEW_LIMIT).is_some())
        }
        ActionId::ExplainQuery => {
            !context.query_running
                && context
                    .query_text
                    .as_deref()
                    .is_some_and(|query| suggest_explain_query(query).is_some())
        }
        ActionId::BuildFilterSortQuery => {
            !context.query_running
                && context.selection.table.is_some()
                && context.selection.database.is_some()
                && context.selection.column.is_some()
        }
        ActionId::InsertSelectSnippet | ActionId::InsertJoinSnippet => {
            !context.query_running && context.view == AppView::QueryEditor
        }
        ActionId::CancelRunningQuery => context.query_running,
        ActionId::ExportCsv
        | ActionId::ExportJson
        | ActionId::ExportCsvGzip
        | ActionId::ExportJsonGzip
        | ActionId::ExportJsonLines
        | ActionId::ExportJsonLinesGzip
        | ActionId::CopyRow
        | ActionId::SearchResults => context.has_results,
        ActionId::SaveBookmark => {
            !context.query_running
                && (context.selection.table.is_some()
                    || context
                        .query_text
                        .as_deref()
                        .is_some_and(|query| !query.trim().is_empty()))
        }
        ActionId::OpenBookmark => !context.query_running && context.has_saved_bookmarks,
        ActionId::CopyCell => context.has_results && context.selection.column.is_some(),
        ActionId::JumpToRelatedTable => {
            context.has_related_tables
                && context.selection.table.is_some()
                && context.selection.database.is_some()
                && !context.query_running
        }
        ActionId::FocusQueryEditor => context.view != AppView::QueryEditor,
    }
}
