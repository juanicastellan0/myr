use thiserror::Error;

use crate::sql_generator::SqlGenerationError;

use super::{ActionId, AppView};

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
    CsvGzip,
    JsonGzip,
    JsonLines,
    JsonLinesGzip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyTarget {
    Cell,
    Row,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionInvocation {
    RunSql(String),
    RunHealthDiagnostics,
    PaginatePrevious,
    PaginateNext,
    ReplaceQueryEditorText(String),
    InsertQueryEditorText(String),
    CancelQuery,
    ExportResults(ExportFormat),
    CopyToClipboard(CopyTarget),
    SaveBookmark,
    OpenBookmark,
    JumpToRelatedTable,
    OpenView(AppView),
    SearchBufferedResults,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ActionEngineError {
    #[error("action `{0:?}` is disabled in the current context")]
    ActionDisabled(ActionId),
    #[error("selected table is required")]
    MissingTableSelection,
    #[error("selected column is required")]
    MissingColumnSelection,
    #[error("selected database is required")]
    MissingDatabaseSelection,
    #[error("query text is required")]
    MissingQueryText,
    #[error("no LIMIT suggestion is available for this query")]
    NoLimitSuggestion,
    #[error("no EXPLAIN suggestion is available for this query")]
    NoExplainSuggestion,
    #[error("failed to generate SQL: {0}")]
    SqlGeneration(#[from] SqlGenerationError),
}
