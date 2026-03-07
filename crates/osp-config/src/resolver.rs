use std::collections::{BTreeMap, BTreeSet};

use crate::bootstrap::{ResolutionFrame, explain_default_profile_key, prepare_resolution};
use crate::interpolate::{explain_interpolation, interpolate_all};
use crate::selector::{LayerRef, ScopeSelector, SelectedLayerEntry};
use crate::{
    ConfigError, ConfigExplain, ConfigLayer, ConfigSchema, ConfigSource, ConfigValue, ExplainLayer,
    LoadedLayers, ResolveOptions, ResolvedConfig, ResolvedValue, Scope, is_bootstrap_only_key,
};

#[derive(Debug, Clone, Default)]
pub struct ConfigResolver {
    layers: LoadedLayers,
    schema: ConfigSchema,
}

/// Resolution happens in two steps:
/// 1. pick the winning raw value for each key using source + scope precedence
/// 2. interpolate placeholders and then run schema adaptation on those winners
#[derive(Debug, Clone)]
struct ResolvedMaps {
    pre_interpolated: BTreeMap<String, ResolvedValue>,
    final_values: BTreeMap<String, ResolvedValue>,
}

impl ConfigResolver {
    pub fn from_loaded_layers(layers: LoadedLayers) -> Self {
        Self {
            layers,
            schema: ConfigSchema::default(),
        }
    }

    pub fn set_schema(&mut self, schema: ConfigSchema) {
        self.schema = schema;
    }

    pub fn schema_mut(&mut self) -> &mut ConfigSchema {
        &mut self.schema
    }

    pub fn defaults_mut(&mut self) -> &mut ConfigLayer {
        &mut self.layers.defaults
    }

    pub fn file_mut(&mut self) -> &mut ConfigLayer {
        &mut self.layers.file
    }

    pub fn secrets_mut(&mut self) -> &mut ConfigLayer {
        &mut self.layers.secrets
    }

    pub fn env_mut(&mut self) -> &mut ConfigLayer {
        &mut self.layers.env
    }

    pub fn cli_mut(&mut self) -> &mut ConfigLayer {
        &mut self.layers.cli
    }

    pub fn session_mut(&mut self) -> &mut ConfigLayer {
        &mut self.layers.session
    }

    pub fn set_defaults(&mut self, layer: ConfigLayer) {
        self.layers.defaults = layer;
    }

    pub fn set_file(&mut self, layer: ConfigLayer) {
        self.layers.file = layer;
    }

    pub fn set_secrets(&mut self, layer: ConfigLayer) {
        self.layers.secrets = layer;
    }

    pub fn set_env(&mut self, layer: ConfigLayer) {
        self.layers.env = layer;
    }

    pub fn set_cli(&mut self, layer: ConfigLayer) {
        self.layers.cli = layer;
    }

    pub fn set_session(&mut self, layer: ConfigLayer) {
        self.layers.session = layer;
    }

    pub fn resolve(&self, options: ResolveOptions) -> Result<ResolvedConfig, ConfigError> {
        let frame = prepare_resolution(self.layers(), options)?;
        let values = self.resolve_values_for_frame(&frame)?;

        Ok(ResolvedConfig {
            active_profile: frame.active_profile,
            terminal: frame.terminal,
            known_profiles: frame.known_profiles,
            values,
        })
    }

    pub fn explain_key(
        &self,
        key: &str,
        options: ResolveOptions,
    ) -> Result<ConfigExplain, ConfigError> {
        if key.eq_ignore_ascii_case("profile.default") {
            return explain_default_profile_key(self.layers(), options);
        }

        let frame = prepare_resolution(self.layers(), options)?;
        let layers = self.explain_layers_for_key(key, &frame);
        let resolved = self.resolve_maps_for_frame(&frame)?;
        let final_entry = resolved.final_values.get(key).cloned();
        // Explaining interpolation intentionally re-reads the pre-interpolated
        // values so the trace shows the original placeholder chain rather than
        // the already-expanded end state.
        let interpolation =
            explain_interpolation(key, &resolved.pre_interpolated, &resolved.final_values)?;

        Ok(ConfigExplain {
            key: key.to_string(),
            active_profile: frame.active_profile,
            terminal: frame.terminal,
            known_profiles: frame.known_profiles,
            layers,
            final_entry,
            interpolation,
        })
    }

    fn resolve_values_for_frame(
        &self,
        frame: &ResolutionFrame,
    ) -> Result<BTreeMap<String, ResolvedValue>, ConfigError> {
        Ok(self.resolve_maps_for_frame(frame)?.final_values)
    }

    /// Run the resolver's two actual phases:
    /// 1. select the winning raw value for each key
    /// 2. interpolate/adapt those winners into final values
    fn resolve_maps_for_frame(&self, frame: &ResolutionFrame) -> Result<ResolvedMaps, ConfigError> {
        let pre_interpolated = self.collect_selected_values_for_frame(frame);
        // Keep both snapshots: normal resolution only needs `final_values`, but
        // `config explain` needs the selected raw winners alongside the final
        // interpolated/adapted view.
        let mut final_values = pre_interpolated.clone();
        interpolate_all(&mut final_values)?;
        self.schema.validate_and_adapt(&mut final_values)?;

        Ok(ResolvedMaps {
            pre_interpolated,
            final_values,
        })
    }

    /// Pick one raw winner per key using source precedence + scope precedence.
    ///
    /// Interpolation is intentionally excluded here; this map is the exact
    /// input to the later placeholder-expansion pass.
    fn collect_selected_values_for_frame(
        &self,
        frame: &ResolutionFrame,
    ) -> BTreeMap<String, ResolvedValue> {
        let selector = ScopeSelector::scoped(&frame.active_profile, frame.terminal.as_deref());
        let keys = self.collect_keys();

        let mut values = BTreeMap::new();
        for key in keys {
            if is_bootstrap_only_key(&key) {
                continue;
            }
            if let Some(selected) = self.select_across_layers(&key, selector) {
                values.insert(key, Self::selected_value(&selected));
            }
        }

        values.insert(
            "profile.active".to_string(),
            Self::derived_active_profile_value(frame),
        );

        values
    }

    fn selected_value(selected: &SelectedLayerEntry<'_>) -> ResolvedValue {
        ResolvedValue {
            raw_value: selected.entry.value.clone(),
            value: selected.entry.value.clone(),
            source: selected.source,
            scope: selected.entry.scope.clone(),
            origin: selected.entry.origin.clone(),
        }
    }

    /// Expose the chosen profile as a normal resolved value so later schema
    /// defaults/interpolation can refer to it without special-case APIs.
    fn derived_active_profile_value(frame: &ResolutionFrame) -> ResolvedValue {
        ResolvedValue {
            raw_value: ConfigValue::String(frame.active_profile.to_string()),
            value: ConfigValue::String(frame.active_profile.to_string()),
            source: ConfigSource::Derived,
            scope: Scope::global(),
            origin: None,
        }
    }

    fn collect_keys(&self) -> BTreeSet<String> {
        let mut keys = BTreeSet::new();

        for layer in self.layers() {
            for entry in &layer.layer.entries {
                keys.insert(entry.key.clone());
            }
        }

        keys
    }

    fn select_across_layers<'a>(
        &'a self,
        key: &str,
        selector: ScopeSelector<'a>,
    ) -> Option<SelectedLayerEntry<'a>> {
        let mut selected: Option<SelectedLayerEntry<'a>> = None;

        // Layers are returned in ascending priority order, so later matches
        // intentionally overwrite earlier ones.
        for layer in self.layers() {
            if let Some(entry) = selector.select(layer, key) {
                selected = Some(entry);
            }
        }

        selected
    }

    fn explain_layers_for_key(&self, key: &str, frame: &ResolutionFrame) -> Vec<ExplainLayer> {
        let selector = ScopeSelector::scoped(&frame.active_profile, frame.terminal.as_deref());
        self.layers()
            .into_iter()
            .filter_map(|layer| selector.explain_layer(layer, key))
            .collect()
    }

    fn layers(&self) -> [LayerRef<'_>; 6] {
        // Keep this order in ascending priority so later layers can override
        // earlier ones in `select_across_layers()`.
        [
            LayerRef {
                source: ConfigSource::BuiltinDefaults,
                layer: &self.layers.defaults,
            },
            LayerRef {
                source: ConfigSource::ConfigFile,
                layer: &self.layers.file,
            },
            LayerRef {
                source: ConfigSource::Secrets,
                layer: &self.layers.secrets,
            },
            LayerRef {
                source: ConfigSource::Environment,
                layer: &self.layers.env,
            },
            LayerRef {
                source: ConfigSource::Cli,
                layer: &self.layers.cli,
            },
            LayerRef {
                source: ConfigSource::Session,
                layer: &self.layers.session,
            },
        ]
    }
}
