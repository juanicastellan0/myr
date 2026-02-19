use std::collections::HashMap;

use thiserror::Error;

use crate::sql_generator::{
    count_estimate_sql, describe_table_sql, preview_select_sql, show_create_table_sql,
    show_index_sql, SqlGenerationError, SqlTarget,
};

const PREVIEW_LIMIT: usize = 200;
const MAX_RECENCY_BOOST: i32 = 25;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ActionId {
    PreviewTable,
    PreviousPage,
    NextPage,
    DescribeTable,
    ShowIndexes,
    ShowCreateTable,
    CountEstimate,
    RunCurrentQuery,
    ApplyLimit200,
    CancelRunningQuery,
    ExportCsv,
    ExportJson,
    CopyCell,
    CopyRow,
    SearchResults,
    FocusQueryEditor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppView {
    ConnectionWizard,
    SchemaExplorer,
    Results,
    QueryEditor,
    CommandPalette,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SchemaSelection {
    pub database: Option<String>,
    pub table: Option<String>,
    pub column: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionContext {
    pub view: AppView,
    pub selection: SchemaSelection,
    pub query_text: Option<String>,
    pub query_running: bool,
    pub has_results: bool,
    pub pagination_enabled: bool,
    pub can_page_next: bool,
    pub can_page_previous: bool,
}

impl Default for ActionContext {
    fn default() -> Self {
        Self {
            view: AppView::ConnectionWizard,
            selection: SchemaSelection::default(),
            query_text: None,
            query_running: false,
            has_results: false,
            pagination_enabled: false,
            can_page_next: false,
            can_page_previous: false,
        }
    }
}

impl ActionContext {
    #[must_use]
    pub fn with_view(mut self, view: AppView) -> Self {
        self.view = view;
        self
    }

    #[must_use]
    pub fn with_query(mut self, query: impl Into<String>) -> Self {
        self.query_text = Some(query.into());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionDefinition {
    pub id: ActionId,
    pub title: &'static str,
    pub description: &'static str,
}

const ACTIONS: [ActionDefinition; 16] = [
    ActionDefinition {
        id: ActionId::PreviewTable,
        title: "Preview table",
        description: "Run SELECT * with a safe preview LIMIT",
    },
    ActionDefinition {
        id: ActionId::PreviousPage,
        title: "Previous page",
        description: "Load previous result page (keyset/offset)",
    },
    ActionDefinition {
        id: ActionId::NextPage,
        title: "Next page",
        description: "Load next result page (keyset/offset)",
    },
    ActionDefinition {
        id: ActionId::DescribeTable,
        title: "Describe table",
        description: "Inspect table columns and metadata",
    },
    ActionDefinition {
        id: ActionId::ShowIndexes,
        title: "Show indexes",
        description: "Inspect table indexes",
    },
    ActionDefinition {
        id: ActionId::ShowCreateTable,
        title: "Show create table",
        description: "Inspect CREATE TABLE DDL",
    },
    ActionDefinition {
        id: ActionId::CountEstimate,
        title: "Estimate row count",
        description: "Read row estimate from information_schema",
    },
    ActionDefinition {
        id: ActionId::RunCurrentQuery,
        title: "Run query",
        description: "Execute the current editor query",
    },
    ActionDefinition {
        id: ActionId::ApplyLimit200,
        title: "Apply LIMIT 200",
        description: "Suggest a preview limit for broad SELECTs",
    },
    ActionDefinition {
        id: ActionId::CancelRunningQuery,
        title: "Cancel query",
        description: "Cancel active query execution",
    },
    ActionDefinition {
        id: ActionId::ExportCsv,
        title: "Export CSV",
        description: "Export current results to CSV",
    },
    ActionDefinition {
        id: ActionId::ExportJson,
        title: "Export JSON",
        description: "Export current results to JSON",
    },
    ActionDefinition {
        id: ActionId::CopyCell,
        title: "Copy cell",
        description: "Copy selected cell value",
    },
    ActionDefinition {
        id: ActionId::CopyRow,
        title: "Copy row",
        description: "Copy selected row values",
    },
    ActionDefinition {
        id: ActionId::SearchResults,
        title: "Search results",
        description: "Search within buffered results",
    },
    ActionDefinition {
        id: ActionId::FocusQueryEditor,
        title: "Go to query editor",
        description: "Switch to query editor view",
    },
];

#[derive(Debug, Default)]
pub struct ActionRegistry;

impl ActionRegistry {
    #[must_use]
    pub fn all(&self) -> &'static [ActionDefinition] {
        &ACTIONS
    }

    #[must_use]
    pub fn find(&self, action_id: ActionId) -> Option<ActionDefinition> {
        ACTIONS
            .iter()
            .copied()
            .find(|action| action.id == action_id)
    }

    #[must_use]
    pub fn enabled_actions(&self, context: &ActionContext) -> Vec<ActionDefinition> {
        ACTIONS
            .iter()
            .copied()
            .filter(|action| action_enabled(action.id, context))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedAction {
    pub id: ActionId,
    pub title: &'static str,
    pub score: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportFormat {
    Csv,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyTarget {
    Cell,
    Row,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionInvocation {
    RunSql(String),
    PaginatePrevious,
    PaginateNext,
    ReplaceQueryEditorText(String),
    CancelQuery,
    ExportResults(ExportFormat),
    CopyToClipboard(CopyTarget),
    OpenView(AppView),
    SearchBufferedResults,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ActionEngineError {
    #[error("action `{0:?}` is disabled in the current context")]
    ActionDisabled(ActionId),
    #[error("selected table is required")]
    MissingTableSelection,
    #[error("selected database is required")]
    MissingDatabaseSelection,
    #[error("query text is required")]
    MissingQueryText,
    #[error("no LIMIT suggestion is available for this query")]
    NoLimitSuggestion,
    #[error("failed to generate SQL: {0}")]
    SqlGeneration(#[from] SqlGenerationError),
}

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
            ActionId::CancelRunningQuery => ActionInvocation::CancelQuery,
            ActionId::ExportCsv => ActionInvocation::ExportResults(ExportFormat::Csv),
            ActionId::ExportJson => ActionInvocation::ExportResults(ExportFormat::Json),
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

fn context_selected_target(context: &ActionContext) -> Result<SqlTarget<'_>, ActionEngineError> {
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

fn action_enabled(action_id: ActionId, context: &ActionContext) -> bool {
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
        ActionId::CancelRunningQuery => context.query_running,
        ActionId::ExportCsv
        | ActionId::ExportJson
        | ActionId::CopyRow
        | ActionId::SearchResults => context.has_results,
        ActionId::CopyCell => context.has_results && context.selection.column.is_some(),
        ActionId::FocusQueryEditor => context.view != AppView::QueryEditor,
    }
}

fn action_base_score(action_id: ActionId, context: &ActionContext) -> i32 {
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
        ActionId::PreviewTable => {
            if context.view == AppView::SchemaExplorer && context.selection.table.is_some() {
                900
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
        ActionId::ExportCsv | ActionId::ExportJson => {
            if context.has_results {
                640
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

#[must_use]
pub fn suggest_preview_limit(query_text: &str, limit: usize) -> Option<String> {
    let trimmed = query_text.trim();
    if trimmed.is_empty() {
        return None;
    }

    let without_trailing_semicolon = trimmed.trim_end_matches(';').trim();
    if without_trailing_semicolon.is_empty() {
        return None;
    }

    if !starts_with_select(without_trailing_semicolon) {
        return None;
    }

    if contains_limit_keyword(without_trailing_semicolon) {
        return None;
    }

    Some(format!("{without_trailing_semicolon} LIMIT {limit}"))
}

fn starts_with_select(query: &str) -> bool {
    let mut words = query.split_whitespace();
    matches!(words.next(), Some(keyword) if keyword.eq_ignore_ascii_case("SELECT"))
}

fn contains_limit_keyword(query: &str) -> bool {
    query
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .any(|token| token.eq_ignore_ascii_case("LIMIT"))
}

#[cfg(test)]
mod tests {
    use super::{
        suggest_preview_limit, ActionContext, ActionId, ActionInvocation, ActionsEngine, AppView,
        SchemaSelection,
    };

    fn schema_context() -> ActionContext {
        ActionContext {
            view: AppView::SchemaExplorer,
            selection: SchemaSelection {
                database: Some("app".to_string()),
                table: Some("users".to_string()),
                column: None,
            },
            query_text: None,
            query_running: false,
            has_results: false,
            pagination_enabled: false,
            can_page_next: false,
            can_page_previous: false,
        }
    }

    #[test]
    fn registry_lists_actions_and_preview_is_invokable() {
        let mut engine = ActionsEngine::new();
        let all_actions = engine.registry().all();
        assert!(!all_actions.is_empty());
        assert!(all_actions
            .iter()
            .any(|action| action.id == ActionId::PreviewTable));

        let invocation = engine
            .invoke(ActionId::PreviewTable, &schema_context())
            .expect("preview action should be invokable");
        assert_eq!(
            invocation,
            ActionInvocation::RunSql("SELECT * FROM `app`.`users` LIMIT 200".to_string())
        );
    }

    #[test]
    fn ranking_prioritizes_contextual_actions() {
        let engine = ActionsEngine::new();
        let ranked = engine.rank_top_n(&schema_context(), 5);
        assert_eq!(
            ranked.first().map(|action| action.id),
            Some(ActionId::PreviewTable)
        );
    }

    #[test]
    fn query_context_surfaces_limit_suggestion() {
        let context = ActionContext::default()
            .with_view(AppView::QueryEditor)
            .with_query("SELECT * FROM users");
        let engine = ActionsEngine::new();
        let ranked = engine.rank_top_n(&context, 3);

        assert!(ranked
            .iter()
            .any(|action| action.id == ActionId::ApplyLimit200));
    }

    #[test]
    fn apply_limit_action_rewrites_query_without_running_it() {
        let mut engine = ActionsEngine::new();
        let context = ActionContext::default()
            .with_view(AppView::QueryEditor)
            .with_query("SELECT * FROM users");

        let invocation = engine
            .invoke(ActionId::ApplyLimit200, &context)
            .expect("limit suggestion should be invokable");
        assert_eq!(
            invocation,
            ActionInvocation::ReplaceQueryEditorText("SELECT * FROM users LIMIT 200".to_string())
        );
    }

    #[test]
    fn suggest_preview_limit_only_for_select_without_limit() {
        assert_eq!(
            suggest_preview_limit("SELECT * FROM users", 200),
            Some("SELECT * FROM users LIMIT 200".to_string())
        );
        assert_eq!(
            suggest_preview_limit("SELECT * FROM users LIMIT 20", 200),
            None
        );
        assert_eq!(suggest_preview_limit("DELETE FROM users", 200), None);
    }

    #[test]
    fn pagination_actions_are_available_in_results_context() {
        let mut engine = ActionsEngine::new();
        let context = ActionContext {
            view: AppView::Results,
            selection: SchemaSelection {
                database: Some("app".to_string()),
                table: Some("events".to_string()),
                column: Some("id".to_string()),
            },
            query_text: Some("SELECT * FROM `app`.`events` LIMIT 200".to_string()),
            query_running: false,
            has_results: true,
            pagination_enabled: true,
            can_page_next: true,
            can_page_previous: true,
        };

        let next = engine
            .invoke(ActionId::NextPage, &context)
            .expect("next page should be enabled");
        assert_eq!(next, ActionInvocation::PaginateNext);

        let previous = engine
            .invoke(ActionId::PreviousPage, &context)
            .expect("previous page should be enabled");
        assert_eq!(previous, ActionInvocation::PaginatePrevious);
    }
}
