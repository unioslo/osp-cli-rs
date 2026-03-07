use std::collections::{BTreeMap, BTreeSet};

use crate::bootstrap::{
    ResolutionFrame, explain_default_profile_bootstrap, explain_default_profile_key,
    prepare_resolution,
};
use crate::explain::{build_runtime_explain, explain_layers_for_runtime_key, selected_value};
use crate::interpolate::{explain_interpolation, interpolate_all};
use crate::selector::{LayerRef, ScopeSelector, SelectedLayerEntry};
use crate::{
    BootstrapConfigExplain, ConfigError, ConfigExplain, ConfigLayer, ConfigSchema, ConfigSource,
    ConfigValue, LoadedLayers, ResolveOptions, ResolvedConfig, ResolvedValue, Scope, is_alias_key,
    is_bootstrap_only_key,
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
    alias_values: BTreeMap<String, ResolvedValue>,
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
        tracing::debug!(
            profile_override = ?options.profile_override,
            terminal = ?options.terminal,
            "resolving config"
        );
        let frame = prepare_resolution(self.layers(), options)?;
        let resolved = self.resolve_maps_for_frame(&frame)?;
        let config = ResolvedConfig {
            active_profile: frame.active_profile,
            terminal: frame.terminal,
            known_profiles: frame.known_profiles,
            values: resolved.final_values,
            aliases: resolved.alias_values,
        };
        tracing::debug!(
            active_profile = %config.active_profile(),
            terminal = ?config.terminal(),
            values = config.values().len(),
            aliases = config.aliases().len(),
            "resolved config"
        );
        Ok(config)
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
        let layers = explain_layers_for_runtime_key(self.layers(), key, &frame);
        let resolved = self.resolve_maps_for_frame(&frame)?;
        let final_entry = if is_alias_key(key) {
            resolved.alias_values.get(key).cloned()
        } else {
            resolved.final_values.get(key).cloned()
        };
        // Explaining interpolation intentionally re-reads the pre-interpolated
        // values so the trace shows the original placeholder chain rather than
        // the already-expanded end state.
        let interpolation =
            explain_interpolation(key, &resolved.pre_interpolated, &resolved.final_values)?;

        Ok(build_runtime_explain(
            key,
            frame,
            layers,
            final_entry,
            if is_alias_key(key) {
                None
            } else {
                interpolation
            },
        ))
    }

    pub fn explain_bootstrap_key(
        &self,
        key: &str,
        options: ResolveOptions,
    ) -> Result<BootstrapConfigExplain, ConfigError> {
        if key.eq_ignore_ascii_case("profile.default") {
            return explain_default_profile_bootstrap(self.layers(), options);
        }

        Err(ConfigError::InvalidConfigKey {
            key: key.to_string(),
            reason: "not a bootstrap key".to_string(),
        })
    }

    /// Run the resolver's two actual phases:
    /// 1. select the winning raw value for each key
    /// 2. interpolate/adapt those winners into final values
    fn resolve_maps_for_frame(&self, frame: &ResolutionFrame) -> Result<ResolvedMaps, ConfigError> {
        tracing::trace!(
            active_profile = %frame.active_profile,
            terminal = ?frame.terminal,
            "resolving config maps for frame"
        );
        let mut pre_interpolated = self.collect_selected_values_for_frame(frame);
        // Aliases are selected with the same precedence rules so explain can
        // still show their winning raw source, but they stay out of ordinary
        // runtime interpolation and schema validation.
        let alias_values = Self::drain_alias_values(&mut pre_interpolated);
        // Keep both snapshots: normal resolution only needs `final_values`, but
        // `config explain` needs the selected raw winners alongside the final
        // interpolated/adapted view.
        let mut final_values = pre_interpolated.clone();
        interpolate_all(&mut final_values)?;
        self.schema.validate_and_adapt(&mut final_values)?;

        tracing::trace!(
            pre_interpolated = pre_interpolated.len(),
            final_values = final_values.len(),
            aliases = alias_values.len(),
            "resolved config maps for frame"
        );
        Ok(ResolvedMaps {
            pre_interpolated,
            final_values,
            alias_values,
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
                values.insert(key, selected_value(&selected));
            }
        }

        values.insert(
            "profile.active".to_string(),
            Self::derived_active_profile_value(frame),
        );

        values
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

    fn drain_alias_values(
        values: &mut BTreeMap<String, ResolvedValue>,
    ) -> BTreeMap<String, ResolvedValue> {
        let alias_keys = values
            .keys()
            .filter(|key| is_alias_key(key))
            .cloned()
            .collect::<Vec<_>>();
        let mut aliases = BTreeMap::new();
        for key in alias_keys {
            if let Some(value) = values.remove(&key) {
                aliases.insert(key, value);
            }
        }
        aliases
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
                if let Some(previous) = &selected {
                    tracing::trace!(
                        key = %key,
                        previous_source = ?previous.source,
                        next_source = ?entry.source,
                        "config key winner changed across layers"
                    );
                }
                selected = Some(entry);
            }
        }

        selected
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
