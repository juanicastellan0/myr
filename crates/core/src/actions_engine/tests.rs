use super::{
    suggest_explain_query, suggest_preview_limit, ActionContext, ActionId, ActionInvocation,
    ActionsEngine, AppView, ExportFormat, SchemaSelection,
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
        has_related_tables: false,
        has_saved_bookmarks: false,
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
fn suggest_explain_query_wraps_non_explain_sql() {
    assert_eq!(
        suggest_explain_query("SELECT * FROM users"),
        Some("EXPLAIN SELECT * FROM users".to_string())
    );
    assert_eq!(suggest_explain_query("EXPLAIN SELECT * FROM users"), None);
    assert_eq!(suggest_explain_query(""), None);
}

#[test]
fn explain_action_generates_preflight_sql() {
    let mut engine = ActionsEngine::new();
    let context = ActionContext::default()
        .with_view(AppView::QueryEditor)
        .with_query("SELECT * FROM users");

    let invocation = engine
        .invoke(ActionId::ExplainQuery, &context)
        .expect("EXPLAIN should be invokable");
    assert_eq!(
        invocation,
        ActionInvocation::RunSql("EXPLAIN SELECT * FROM users".to_string())
    );
}

#[test]
fn filter_sort_builder_action_generates_server_side_query() {
    let mut engine = ActionsEngine::new();
    let context = ActionContext {
        view: AppView::SchemaExplorer,
        selection: SchemaSelection {
            database: Some("app".to_string()),
            table: Some("users".to_string()),
            column: Some("email".to_string()),
        },
        query_text: None,
        query_running: false,
        has_results: false,
        has_related_tables: false,
        has_saved_bookmarks: false,
        pagination_enabled: false,
        can_page_next: false,
        can_page_previous: false,
    };

    let invocation = engine
        .invoke(ActionId::BuildFilterSortQuery, &context)
        .expect("filter/sort builder should be invokable");
    assert_eq!(
        invocation,
        ActionInvocation::ReplaceQueryEditorText(
            "SELECT * FROM `app`.`users` WHERE `email` LIKE '%search%' ORDER BY `email` ASC LIMIT 200"
                .to_string()
        )
    );
}

#[test]
fn snippet_actions_insert_editor_templates() {
    let mut engine = ActionsEngine::new();
    let context = ActionContext::default().with_view(AppView::QueryEditor);

    let select_snippet = engine
        .invoke(ActionId::InsertSelectSnippet, &context)
        .expect("select snippet should be invokable");
    assert!(matches!(
        select_snippet,
        ActionInvocation::InsertQueryEditorText(_)
    ));

    let join_snippet = engine
        .invoke(ActionId::InsertJoinSnippet, &context)
        .expect("join snippet should be invokable");
    assert!(matches!(
        join_snippet,
        ActionInvocation::InsertQueryEditorText(_)
    ));
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
        has_related_tables: false,
        has_saved_bookmarks: false,
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

#[test]
fn relationship_and_bookmark_actions_are_invokable_with_context_flags() {
    let mut engine = ActionsEngine::new();
    let context = ActionContext {
        view: AppView::SchemaExplorer,
        selection: SchemaSelection {
            database: Some("app".to_string()),
            table: Some("sessions".to_string()),
            column: Some("user_id".to_string()),
        },
        query_text: Some("SELECT * FROM `app`.`sessions`".to_string()),
        query_running: false,
        has_results: false,
        has_related_tables: true,
        has_saved_bookmarks: true,
        pagination_enabled: false,
        can_page_next: false,
        can_page_previous: false,
    };

    let jump = engine
        .invoke(ActionId::JumpToRelatedTable, &context)
        .expect("relationship jump should be enabled");
    assert_eq!(jump, ActionInvocation::JumpToRelatedTable);

    let save = engine
        .invoke(ActionId::SaveBookmark, &context)
        .expect("save bookmark should be enabled");
    assert_eq!(save, ActionInvocation::SaveBookmark);

    let open = engine
        .invoke(ActionId::OpenBookmark, &context)
        .expect("open bookmark should be enabled");
    assert_eq!(open, ActionInvocation::OpenBookmark);
}

#[test]
fn export_variant_actions_resolve_to_expected_formats() {
    let mut engine = ActionsEngine::new();
    let context = ActionContext {
        view: AppView::Results,
        selection: SchemaSelection {
            database: Some("app".to_string()),
            table: Some("events".to_string()),
            column: Some("id".to_string()),
        },
        query_text: None,
        query_running: false,
        has_results: true,
        has_related_tables: false,
        has_saved_bookmarks: false,
        pagination_enabled: false,
        can_page_next: false,
        can_page_previous: false,
    };

    assert_eq!(
        engine
            .invoke(ActionId::ExportCsvGzip, &context)
            .expect("csv gzip should be enabled"),
        ActionInvocation::ExportResults(ExportFormat::CsvGzip)
    );
    assert_eq!(
        engine
            .invoke(ActionId::ExportJsonGzip, &context)
            .expect("json gzip should be enabled"),
        ActionInvocation::ExportResults(ExportFormat::JsonGzip)
    );
    assert_eq!(
        engine
            .invoke(ActionId::ExportJsonLines, &context)
            .expect("jsonl should be enabled"),
        ActionInvocation::ExportResults(ExportFormat::JsonLines)
    );
    assert_eq!(
        engine
            .invoke(ActionId::ExportJsonLinesGzip, &context)
            .expect("jsonl gzip should be enabled"),
        ActionInvocation::ExportResults(ExportFormat::JsonLinesGzip)
    );
}
