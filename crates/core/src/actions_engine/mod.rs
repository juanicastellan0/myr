mod catalog;
mod context;
mod enablement;
mod engine;
mod invocation;
mod ranking;
mod snippets;
mod suggestions;

pub use catalog::{
    ActionContext, ActionDefinition, ActionId, ActionRegistry, AppView, SchemaSelection,
};
pub use engine::ActionsEngine;
pub use invocation::{ActionEngineError, ActionInvocation, CopyTarget, ExportFormat, RankedAction};
pub use suggestions::{suggest_explain_query, suggest_preview_limit};

pub(super) const PREVIEW_LIMIT: usize = 200;

#[cfg(test)]
mod tests;
