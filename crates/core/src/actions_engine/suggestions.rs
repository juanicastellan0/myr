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

#[must_use]
pub fn suggest_explain_query(query_text: &str) -> Option<String> {
    let trimmed = query_text.trim();
    if trimmed.is_empty() {
        return None;
    }

    let without_trailing_semicolon = trimmed.trim_end_matches(';').trim();
    if without_trailing_semicolon.is_empty() || starts_with_explain(without_trailing_semicolon) {
        return None;
    }

    Some(format!("EXPLAIN {without_trailing_semicolon}"))
}

fn starts_with_explain(query: &str) -> bool {
    let mut words = query.split_whitespace();
    matches!(words.next(), Some(keyword) if keyword.eq_ignore_ascii_case("EXPLAIN"))
}
