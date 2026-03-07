use crate::bootstrap::ResolutionFrame;
use crate::selector::{LayerRef, ScopeSelector, SelectedLayerEntry};
use crate::{ConfigExplain, ExplainInterpolation, ExplainLayer, ResolvedValue};

pub(crate) fn build_runtime_explain(
    key: &str,
    frame: ResolutionFrame,
    layers: Vec<ExplainLayer>,
    final_entry: Option<ResolvedValue>,
    interpolation: Option<ExplainInterpolation>,
) -> ConfigExplain {
    ConfigExplain {
        key: key.to_string(),
        active_profile: frame.active_profile,
        terminal: frame.terminal,
        known_profiles: frame.known_profiles,
        layers,
        final_entry,
        interpolation,
    }
}

pub(crate) fn explain_layers_for_runtime_key(
    layers: [LayerRef<'_>; 6],
    key: &str,
    frame: &ResolutionFrame,
) -> Vec<ExplainLayer> {
    // Runtime explain must use the exact same scoped selector as runtime
    // resolution, otherwise "why" answers drift from actual behavior.
    let selector = ScopeSelector::scoped(&frame.active_profile, frame.terminal.as_deref());
    layers
        .into_iter()
        .filter_map(|layer| selector.explain_layer(layer, key))
        .collect()
}

pub(crate) fn selected_value(selected: &SelectedLayerEntry<'_>) -> ResolvedValue {
    ResolvedValue {
        raw_value: selected.entry.value.clone(),
        value: selected.entry.value.clone(),
        source: selected.source,
        scope: selected.entry.scope.clone(),
        origin: selected.entry.origin.clone(),
    }
}
