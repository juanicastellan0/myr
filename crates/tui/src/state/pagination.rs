#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PageTransition {
    Reset,
    Next,
    Previous,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PaginationPlan {
    Keyset {
        key_column: String,
        first_key: Option<String>,
        last_key: Option<String>,
    },
    Offset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PaginationState {
    pub(crate) database: Option<String>,
    pub(crate) table: String,
    pub(crate) page_size: usize,
    pub(crate) page_index: usize,
    pub(crate) last_page_row_count: usize,
    pub(crate) plan: PaginationPlan,
}
