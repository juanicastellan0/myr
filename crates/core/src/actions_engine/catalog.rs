use super::enablement::action_enabled;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ActionId {
    PreviewTable,
    JumpToRelatedTable,
    PreviousPage,
    NextPage,
    DescribeTable,
    ShowIndexes,
    ShowCreateTable,
    CountEstimate,
    RunCurrentQuery,
    ApplyLimit200,
    ExplainQuery,
    BuildFilterSortQuery,
    InsertSelectSnippet,
    InsertJoinSnippet,
    CancelRunningQuery,
    ExportCsv,
    ExportJson,
    ExportCsvGzip,
    ExportJsonGzip,
    ExportJsonLines,
    ExportJsonLinesGzip,
    SaveBookmark,
    OpenBookmark,
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
    pub has_related_tables: bool,
    pub has_saved_bookmarks: bool,
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
            has_related_tables: false,
            has_saved_bookmarks: false,
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

const ACTIONS: [ActionDefinition; 27] = [
    ActionDefinition {
        id: ActionId::PreviewTable,
        title: "Preview table",
        description: "Run SELECT * with a safe preview LIMIT",
    },
    ActionDefinition {
        id: ActionId::JumpToRelatedTable,
        title: "Jump to related table",
        description: "Follow a foreign-key relationship to another table",
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
        id: ActionId::ExplainQuery,
        title: "Explain query",
        description: "Run EXPLAIN preflight for the current query",
    },
    ActionDefinition {
        id: ActionId::BuildFilterSortQuery,
        title: "Build filter/sort query",
        description: "Generate server-side WHERE/ORDER BY query from selected schema target",
    },
    ActionDefinition {
        id: ActionId::InsertSelectSnippet,
        title: "Insert SELECT snippet",
        description: "Insert a SELECT skeleton snippet into the editor",
    },
    ActionDefinition {
        id: ActionId::InsertJoinSnippet,
        title: "Insert JOIN snippet",
        description: "Insert a JOIN skeleton snippet into the editor",
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
        id: ActionId::ExportCsvGzip,
        title: "Export CSV (gzip)",
        description: "Export current results to compressed CSV",
    },
    ActionDefinition {
        id: ActionId::ExportJsonGzip,
        title: "Export JSON (gzip)",
        description: "Export current results to compressed JSON",
    },
    ActionDefinition {
        id: ActionId::ExportJsonLines,
        title: "Export JSONL",
        description: "Export current results as newline-delimited JSON",
    },
    ActionDefinition {
        id: ActionId::ExportJsonLinesGzip,
        title: "Export JSONL (gzip)",
        description: "Export current results as compressed newline-delimited JSON",
    },
    ActionDefinition {
        id: ActionId::SaveBookmark,
        title: "Save bookmark",
        description: "Save current schema target and query as a bookmark",
    },
    ActionDefinition {
        id: ActionId::OpenBookmark,
        title: "Open bookmark",
        description: "Open the next saved bookmark",
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
