use crate::osp_config::{
    ConfigLayer, ConfigSource, ExplainCandidate, ExplainLayer, LayerEntry, Scope,
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct LayerRef<'a> {
    pub(crate) source: ConfigSource,
    pub(crate) layer: &'a ConfigLayer,
}

#[derive(Debug, Clone)]
pub(crate) struct SelectedLayerEntry<'a> {
    pub(crate) source: ConfigSource,
    pub(crate) entry_index: usize,
    pub(crate) entry: &'a LayerEntry,
}

/// Scope precedence is small but subtle:
/// profile+terminal > profile > terminal > global.
///
/// Keeping that policy in one selector object makes `resolve()`, bootstrap
/// profile lookup, and `explain_key()` share the same matching rules instead of
/// each rebuilding them slightly differently.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ScopeSelector<'a> {
    profile: Option<&'a str>,
    terminal: Option<&'a str>,
}

impl<'a> ScopeSelector<'a> {
    pub(crate) fn scoped(profile: &'a str, terminal: Option<&'a str>) -> Self {
        Self {
            profile: Some(profile),
            terminal,
        }
    }

    pub(crate) fn global(terminal: Option<&'a str>) -> Self {
        Self {
            profile: None,
            terminal,
        }
    }

    pub(crate) fn rank(self, scope: &Scope) -> Option<u8> {
        // Lower rank wins:
        // 0 = exact profile+terminal match
        // 1 = profile-only match
        // 2 = terminal-only match
        // 3 = global fallback
        match (
            self.profile,
            scope.profile.as_deref(),
            scope.terminal.as_deref(),
            self.terminal,
        ) {
            (Some(active_profile), Some(profile), Some(term), Some(active_term))
                if profile == active_profile && term == active_term =>
            {
                Some(0)
            }
            (Some(active_profile), Some(profile), None, _) if profile == active_profile => Some(1),
            (_, None, Some(term), Some(active_term)) if term == active_term => Some(2),
            (_, None, None, _) => Some(3),
            _ => None,
        }
    }

    pub(crate) fn select(self, layer: LayerRef<'a>, key: &str) -> Option<SelectedLayerEntry<'a>> {
        let mut best: Option<(usize, u8, &'a LayerEntry)> = None;

        for (entry_index, entry) in layer.layer.entries.iter().enumerate() {
            if entry.key != key {
                continue;
            }

            let Some(rank) = self.rank(&entry.scope) else {
                continue;
            };

            let replace = match best {
                None => true,
                Some((best_index, best_rank, _)) => {
                    rank < best_rank || (rank == best_rank && entry_index > best_index)
                }
            };

            if replace {
                best = Some((entry_index, rank, entry));
            }
        }

        best.map(|(entry_index, _, entry)| SelectedLayerEntry {
            source: layer.source,
            entry_index,
            entry,
        })
    }

    pub(crate) fn explain_layer(self, layer: LayerRef<'a>, key: &str) -> Option<ExplainLayer> {
        let selected = self.select(layer, key);
        let selected_entry_index = selected.as_ref().map(|entry| entry.entry_index);

        let candidates = layer
            .layer
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.key == key)
            .map(|(entry_index, entry)| ExplainCandidate {
                entry_index,
                value: entry.value.clone(),
                scope: entry.scope.clone(),
                origin: entry.origin.clone(),
                rank: self.rank(&entry.scope),
                selected_in_layer: selected_entry_index == Some(entry_index),
            })
            .collect::<Vec<ExplainCandidate>>();

        if candidates.is_empty() {
            None
        } else {
            Some(ExplainLayer {
                source: layer.source,
                selected_entry_index,
                candidates,
            })
        }
    }
}
