use std::collections::HashMap;

use crate::sql_generator::{
    count_estimate_sql, describe_table_sql, filtered_sorted_preview_sql, preview_select_sql,
    show_create_table_sql, show_index_sql,
};

use super::{
    context::{context_selected_column, context_selected_target},
    enablement::action_enabled,
    ranking::action_base_score,
    snippets::{join_snippet, select_snippet},
    suggest_explain_query, suggest_preview_limit, ActionContext, ActionEngineError, ActionId,
    ActionInvocation, ActionRegistry, AppView, CopyTarget, ExportFormat, RankedAction,
    PREVIEW_LIMIT,
};

const MAX_RECENCY_BOOST: i32 = 25;

#[derive(Debug, Default)]
pub struct ActionsEngine {
    registry: ActionRegistry,
    recency_tick: u64,
    recency: HashMap<ActionId, u64>,
}

impl ActionsEngine {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn registry(&self) -> &ActionRegistry {
        &self.registry
    }

    #[must_use]
    pub fn rank_top_n(&self, context: &ActionContext, limit: usize) -> Vec<RankedAction> {
        let mut ranked = self
            .registry
            .all()
            .iter()
            .copied()
            .filter(|action| action_enabled(action.id, context))
            .map(|action| RankedAction {
                id: action.id,
                title: action.title,
                score: action_base_score(action.id, context) + self.recency_boost(action.id),
            })
            .collect::<Vec<_>>();

        ranked.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.title.cmp(right.title))
        });
        ranked.truncate(limit);
        ranked
    }

    pub fn invoke(
        &mut self,
        action_id: ActionId,
        context: &ActionContext,
    ) -> Result<ActionInvocation, ActionEngineError> {
        if !action_enabled(action_id, context) {
            return Err(ActionEngineError::ActionDisabled(action_id));
        }

        let invocation = match action_id {
            ActionId::PreviewTable => {
                let target = context_selected_target(context)?;
                ActionInvocation::RunSql(preview_select_sql(&target, PREVIEW_LIMIT))
            }
            ActionId::JumpToRelatedTable => ActionInvocation::JumpToRelatedTable,
            ActionId::RunHealthDiagnostics => ActionInvocation::RunHealthDiagnostics,
            ActionId::PreviousPage => ActionInvocation::PaginatePrevious,
            ActionId::NextPage => ActionInvocation::PaginateNext,
            ActionId::DescribeTable => {
                let target = context_selected_target(context)?;
                ActionInvocation::RunSql(describe_table_sql(&target))
            }
            ActionId::ShowIndexes => {
                let target = context_selected_target(context)?;
                ActionInvocation::RunSql(show_index_sql(&target))
            }
            ActionId::ShowCreateTable => {
                let target = context_selected_target(context)?;
                ActionInvocation::RunSql(show_create_table_sql(&target))
            }
            ActionId::CountEstimate => {
                let target = context_selected_target(context)?;
                ActionInvocation::RunSql(count_estimate_sql(&target)?)
            }
            ActionId::RunCurrentQuery => {
                let query = context
                    .query_text
                    .as_deref()
                    .map(str::trim)
                    .filter(|query| !query.is_empty())
                    .ok_or(ActionEngineError::MissingQueryText)?;
                ActionInvocation::RunSql(query.to_string())
            }
            ActionId::ApplyLimit200 => {
                let query = context
                    .query_text
                    .as_deref()
                    .ok_or(ActionEngineError::MissingQueryText)?;
                let suggested = suggest_preview_limit(query, PREVIEW_LIMIT)
                    .ok_or(ActionEngineError::NoLimitSuggestion)?;
                ActionInvocation::ReplaceQueryEditorText(suggested)
            }
            ActionId::ExplainQuery => {
                let query = context
                    .query_text
                    .as_deref()
                    .ok_or(ActionEngineError::MissingQueryText)?;
                let explain =
                    suggest_explain_query(query).ok_or(ActionEngineError::NoExplainSuggestion)?;
                ActionInvocation::RunSql(explain)
            }
            ActionId::BuildFilterSortQuery => {
                let target = context_selected_target(context)?;
                let column = context_selected_column(context)?;
                ActionInvocation::ReplaceQueryEditorText(filtered_sorted_preview_sql(
                    &target,
                    column,
                    PREVIEW_LIMIT,
                )?)
            }
            ActionId::InsertSelectSnippet => {
                ActionInvocation::InsertQueryEditorText(select_snippet(context))
            }
            ActionId::InsertJoinSnippet => {
                ActionInvocation::InsertQueryEditorText(join_snippet(context))
            }
            ActionId::CancelRunningQuery => ActionInvocation::CancelQuery,
            ActionId::ExportCsv => ActionInvocation::ExportResults(ExportFormat::Csv),
            ActionId::ExportJson => ActionInvocation::ExportResults(ExportFormat::Json),
            ActionId::ExportCsvGzip => ActionInvocation::ExportResults(ExportFormat::CsvGzip),
            ActionId::ExportJsonGzip => ActionInvocation::ExportResults(ExportFormat::JsonGzip),
            ActionId::ExportJsonLines => ActionInvocation::ExportResults(ExportFormat::JsonLines),
            ActionId::ExportJsonLinesGzip => {
                ActionInvocation::ExportResults(ExportFormat::JsonLinesGzip)
            }
            ActionId::SaveBookmark => ActionInvocation::SaveBookmark,
            ActionId::OpenBookmark => ActionInvocation::OpenBookmark,
            ActionId::CopyCell => ActionInvocation::CopyToClipboard(CopyTarget::Cell),
            ActionId::CopyRow => ActionInvocation::CopyToClipboard(CopyTarget::Row),
            ActionId::SearchResults => ActionInvocation::SearchBufferedResults,
            ActionId::FocusQueryEditor => ActionInvocation::OpenView(AppView::QueryEditor),
        };

        self.record_use(action_id);
        Ok(invocation)
    }

    fn record_use(&mut self, action_id: ActionId) {
        self.recency_tick = self.recency_tick.saturating_add(1);
        self.recency.insert(action_id, self.recency_tick);
    }

    fn recency_boost(&self, action_id: ActionId) -> i32 {
        let Some(last_used_tick) = self.recency.get(&action_id).copied() else {
            return 0;
        };

        let age = self.recency_tick.saturating_sub(last_used_tick);
        let age_i32 = i32::try_from(age).unwrap_or(i32::MAX);
        (MAX_RECENCY_BOOST - age_i32).max(0)
    }
}
