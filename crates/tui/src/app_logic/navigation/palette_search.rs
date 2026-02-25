fn palette_aliases(action_id: ActionId) -> &'static [&'static str] {
    match action_id {
        ActionId::PreviewTable => &["preview", "peek", "sample", "pvw"],
        ActionId::JumpToRelatedTable => &["fk", "foreign key", "relationship", "related"],
        ActionId::PreviousPage => &["prev", "back", "page back"],
        ActionId::NextPage => &["next", "forward", "more"],
        ActionId::DescribeTable => &["describe", "desc", "columns", "schema"],
        ActionId::ShowIndexes => &["index", "indexes", "keys"],
        ActionId::ShowCreateTable => &["ddl", "create", "show create"],
        ActionId::CountEstimate => &["count", "estimate", "rows"],
        ActionId::RunHealthDiagnostics => &["health", "diagnostics", "doctor", "smoke"],
        ActionId::RunCurrentQuery => &["run", "execute", "query"],
        ActionId::ApplyLimit200 => &["limit", "cap rows", "preview limit"],
        ActionId::ExplainQuery => &["explain", "plan", "query plan"],
        ActionId::BuildFilterSortQuery => &["filter", "sort", "where", "order by"],
        ActionId::InsertSelectSnippet => &["snippet", "select template"],
        ActionId::InsertJoinSnippet => &["snippet", "join template"],
        ActionId::CancelRunningQuery => &["cancel", "stop", "abort"],
        ActionId::ExportCsv => &["csv", "export csv"],
        ActionId::ExportJson => &["json", "export json"],
        ActionId::ExportCsvGzip => &["csv.gz", "gzip csv", "compressed csv"],
        ActionId::ExportJsonGzip => &["json.gz", "gzip json", "compressed json"],
        ActionId::ExportJsonLines => &["jsonl", "ndjson", "json lines"],
        ActionId::ExportJsonLinesGzip => &["jsonl.gz", "gzip jsonl", "compressed jsonl"],
        ActionId::SaveBookmark => &["bookmark save", "save view", "favorite"],
        ActionId::OpenBookmark => &["bookmark open", "open view", "load bookmark"],
        ActionId::CopyCell => &["copy cell", "clipboard cell"],
        ActionId::CopyRow => &["copy row", "clipboard row"],
        ActionId::SearchResults => &["search", "find", "grep"],
        ActionId::FocusQueryEditor => &["editor", "sql", "go query editor"],
    }
}

fn palette_match_score(
    query: &str,
    title: &str,
    description: &str,
    aliases: &[&str],
) -> Option<i32> {
    let title_score = text_match_score(query, title).map(|score| score + 30);
    let description_score = text_match_score(query, description);
    let alias_score = aliases
        .iter()
        .filter_map(|alias| text_match_score(query, alias))
        .max()
        .map(|score| score + 15);
    [title_score, description_score, alias_score]
        .into_iter()
        .flatten()
        .max()
}

fn text_match_score(query: &str, text: &str) -> Option<i32> {
    if query.is_empty() || text.is_empty() {
        return None;
    }
    if text == query {
        return Some(1_000);
    }
    if text.starts_with(query) {
        return Some(900);
    }
    if text
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|word| !word.is_empty() && word.starts_with(query))
    {
        return Some(820);
    }
    if text.contains(query) {
        return Some(760);
    }
    fuzzy_subsequence_score(query, text)
}

fn fuzzy_subsequence_score(query: &str, text: &str) -> Option<i32> {
    let mut query_chars = query.chars();
    let mut current = query_chars.next()?;
    let mut matched = 0usize;
    let mut previous_index = 0usize;
    let mut gap_penalty = 0i32;

    for (index, ch) in text.chars().enumerate() {
        if ch != current {
            continue;
        }

        if matched > 0 {
            let gap = index.saturating_sub(previous_index + 1);
            gap_penalty += i32::try_from(gap.min(12)).unwrap_or(12);
        }

        matched += 1;
        previous_index = index;

        if let Some(next) = query_chars.next() {
            current = next;
            continue;
        }

        let length_bonus = i32::try_from(query.chars().count().min(12)).unwrap_or(12) * 8;
        return Some((620 + length_bonus - gap_penalty).max(500));
    }

    None
}
