fn first_filtered_index(items: &[String], filter: &str) -> Option<usize> {
    filtered_schema_indices(items, filter).into_iter().next()
}

fn filtered_schema_indices(items: &[String], filter: &str) -> Vec<usize> {
    let needle = filter.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return (0..items.len()).collect();
    }

    items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            item.to_ascii_lowercase()
                .contains(needle.as_str())
                .then_some(index)
        })
        .collect()
}

fn previous_filtered_index(indices: &[usize], current: usize) -> usize {
    let position = indices
        .iter()
        .position(|index| *index == current)
        .unwrap_or(0);
    indices[position.saturating_sub(1)]
}

fn next_filtered_index(indices: &[usize], current: usize) -> usize {
    let position = indices
        .iter()
        .position(|index| *index == current)
        .unwrap_or(0);
    let next_position = (position + 1).min(indices.len().saturating_sub(1));
    indices[next_position]
}
