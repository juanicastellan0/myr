impl TuiApp {
    pub(super) fn palette_entries(&self) -> Vec<ActionId> {
        let query = self.palette_query.trim().to_ascii_lowercase();
        let ranked = self.actions.rank_top_n(&self.action_context(), 50);
        if query.is_empty() {
            return ranked.into_iter().map(|action| action.id).collect();
        }

        let mut matches = ranked
            .into_iter()
            .filter_map(|ranked_action| {
                let metadata = self.actions.registry().find(ranked_action.id);
                let title = ranked_action.title.to_ascii_lowercase();
                let description = metadata
                    .map_or("", |action| action.description)
                    .to_ascii_lowercase();
                let search_score = palette_match_score(
                    query.as_str(),
                    title.as_str(),
                    description.as_str(),
                    palette_aliases(ranked_action.id),
                )?;
                let combined_score = search_score * 10_000 + ranked_action.score;
                Some((combined_score, ranked_action.title, ranked_action.id))
            })
            .collect::<Vec<_>>();

        matches.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| left.1.cmp(right.1))
        });
        matches.into_iter().map(|(_, _, action_id)| action_id).collect()
    }

    fn selected_palette_action(&self) -> Option<ActionId> {
        let entries = self.palette_entries();
        entries.get(self.palette_selection).copied()
    }

}
