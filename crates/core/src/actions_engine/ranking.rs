use super::{
    suggest_explain_query, suggest_preview_limit, ActionContext, ActionId, AppView, PREVIEW_LIMIT,
};

pub(super) fn action_base_score(action_id: ActionId, context: &ActionContext) -> i32 {
    match action_id {
        ActionId::CancelRunningQuery => {
            if context.query_running {
                1_000
            } else {
                0
            }
        }
        ActionId::ApplyLimit200 => {
            if context
                .query_text
                .as_deref()
                .is_some_and(|query| suggest_preview_limit(query, PREVIEW_LIMIT).is_some())
            {
                950
            } else {
                0
            }
        }
        ActionId::ExplainQuery => {
            if context
                .query_text
                .as_deref()
                .is_some_and(|query| suggest_explain_query(query).is_some())
            {
                910
            } else {
                0
            }
        }
        ActionId::BuildFilterSortQuery => {
            if context.selection.table.is_some()
                && context.selection.database.is_some()
                && context.selection.column.is_some()
            {
                880
            } else {
                0
            }
        }
        ActionId::InsertSelectSnippet => {
            if context.view == AppView::QueryEditor {
                820
            } else {
                0
            }
        }
        ActionId::InsertJoinSnippet => {
            if context.view == AppView::QueryEditor {
                780
            } else {
                0
            }
        }
        ActionId::PreviewTable => {
            if context.view == AppView::SchemaExplorer && context.selection.table.is_some() {
                900
            } else {
                0
            }
        }
        ActionId::JumpToRelatedTable => {
            if context.view == AppView::SchemaExplorer
                && context.selection.table.is_some()
                && context.has_related_tables
            {
                870
            } else {
                0
            }
        }
        ActionId::PreviousPage => {
            if context.pagination_enabled
                && context.can_page_previous
                && context.view == AppView::Results
            {
                840
            } else {
                0
            }
        }
        ActionId::NextPage => {
            if context.pagination_enabled
                && context.can_page_next
                && context.view == AppView::Results
            {
                860
            } else {
                0
            }
        }
        ActionId::DescribeTable => {
            if context.view == AppView::SchemaExplorer && context.selection.table.is_some() {
                820
            } else {
                0
            }
        }
        ActionId::ShowIndexes => {
            if context.view == AppView::SchemaExplorer && context.selection.table.is_some() {
                790
            } else {
                0
            }
        }
        ActionId::ShowCreateTable => {
            if context.view == AppView::SchemaExplorer && context.selection.table.is_some() {
                760
            } else {
                0
            }
        }
        ActionId::CountEstimate => {
            if context.selection.table.is_some() && context.selection.database.is_some() {
                700
            } else {
                0
            }
        }
        ActionId::RunCurrentQuery => {
            if context
                .query_text
                .as_deref()
                .map(str::trim)
                .is_some_and(|query| !query.is_empty())
            {
                850
            } else {
                0
            }
        }
        ActionId::ExportCsv => {
            if context.has_results {
                640
            } else {
                0
            }
        }
        ActionId::ExportJson => {
            if context.has_results {
                638
            } else {
                0
            }
        }
        ActionId::ExportCsvGzip => {
            if context.has_results {
                636
            } else {
                0
            }
        }
        ActionId::ExportJsonGzip => {
            if context.has_results {
                634
            } else {
                0
            }
        }
        ActionId::ExportJsonLines => {
            if context.has_results {
                632
            } else {
                0
            }
        }
        ActionId::ExportJsonLinesGzip => {
            if context.has_results {
                630
            } else {
                0
            }
        }
        ActionId::SaveBookmark => {
            if context.selection.table.is_some()
                || context
                    .query_text
                    .as_deref()
                    .is_some_and(|query| !query.trim().is_empty())
            {
                620
            } else {
                0
            }
        }
        ActionId::OpenBookmark => {
            if context.has_saved_bookmarks {
                610
            } else {
                0
            }
        }
        ActionId::CopyCell | ActionId::CopyRow => {
            if context.has_results {
                600
            } else {
                0
            }
        }
        ActionId::SearchResults => {
            if context.has_results {
                580
            } else {
                0
            }
        }
        ActionId::FocusQueryEditor => {
            if context.view != AppView::QueryEditor {
                500
            } else {
                0
            }
        }
    }
}
