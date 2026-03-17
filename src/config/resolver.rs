use std::collections::{BTreeMap, BTreeSet};

use crate::config::bootstrap::{
    ResolutionFrame, explain_default_profile_bootstrap, explain_default_profile_key,
    prepare_resolution,
};
use crate::config::explain::{
    build_runtime_explain, explain_layers_for_runtime_key, selected_value,
};
use crate::config::interpolate::{explain_interpolation, interpolate_all};
use crate::config::selector::{LayerRef, ScopeSelector, SelectedLayerEntry};
use crate::config::{
    BootstrapConfigExplain, ConfigError, ConfigExplain, ConfigLayer, ConfigSchema, ConfigSource,
    ConfigValue, LoadedLayers, ResolveOptions, ResolvedConfig, ResolvedValue, Scope, is_alias_key,
    is_bootstrap_only_key,
};

/// Resolves layered config input into the runtime view seen by the rest of the
/// application.
///
/// Callers usually populate the individual source layers first and then ask
/// the resolver for either the final runtime view or an explanation trace.
///
/// High-level flow:
///
/// - select one winning raw value per key using source and scope precedence
/// - interpolate placeholders inside the selected winners
/// - adapt and validate the interpolated values against the schema
/// - optionally expose an explanation trace that shows why each winner won
///
/// Contract:
///
/// - precedence rules live here, not in callers
/// - schema adaptation happens after winner selection, not while scanning layers
/// - bootstrap handling stays aligned with the config bootstrap helpers rather
///   than becoming a separate merge system
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
    /// Creates a resolver from pre-loaded config layers.
    pub fn from_loaded_layers(layers: LoadedLayers) -> Self {
        Self::from_loaded_layers_with_schema(layers, ConfigSchema::default())
    }

    /// Creates a resolver from pre-loaded config layers and an explicit schema.
    pub fn from_loaded_layers_with_schema(layers: LoadedLayers, schema: ConfigSchema) -> Self {
        Self { layers, schema }
    }

    /// Replaces the schema used for validation and adaptation.
    pub fn set_schema(&mut self, schema: ConfigSchema) {
        self.schema = schema;
    }

    /// Returns mutable access to the active schema.
    pub fn schema_mut(&mut self) -> &mut ConfigSchema {
        &mut self.schema
    }

    /// Returns mutable access to the built-in defaults layer.
    pub fn defaults_mut(&mut self) -> &mut ConfigLayer {
        &mut self.layers.defaults
    }

    /// Returns mutable access to the config file layer.
    pub fn file_mut(&mut self) -> &mut ConfigLayer {
        &mut self.layers.file
    }

    /// Returns mutable access to the presentation defaults layer.
    pub fn presentation_mut(&mut self) -> &mut ConfigLayer {
        &mut self.layers.presentation
    }

    /// Returns mutable access to the secrets layer.
    pub fn secrets_mut(&mut self) -> &mut ConfigLayer {
        &mut self.layers.secrets
    }

    /// Returns mutable access to the environment layer.
    pub fn env_mut(&mut self) -> &mut ConfigLayer {
        &mut self.layers.env
    }

    /// Returns mutable access to the CLI layer.
    pub fn cli_mut(&mut self) -> &mut ConfigLayer {
        &mut self.layers.cli
    }

    /// Returns mutable access to the session layer.
    pub fn session_mut(&mut self) -> &mut ConfigLayer {
        &mut self.layers.session
    }

    /// Replaces the built-in defaults layer.
    pub fn set_defaults(&mut self, layer: ConfigLayer) {
        self.layers.defaults = layer;
    }

    /// Replaces the config file layer.
    pub fn set_file(&mut self, layer: ConfigLayer) {
        self.layers.file = layer;
    }

    /// Replaces the presentation defaults layer.
    pub fn set_presentation(&mut self, layer: ConfigLayer) {
        self.layers.presentation = layer;
    }

    /// Replaces the secrets layer.
    pub fn set_secrets(&mut self, layer: ConfigLayer) {
        self.layers.secrets = layer;
    }

    /// Replaces the environment layer.
    pub fn set_env(&mut self, layer: ConfigLayer) {
        self.layers.env = layer;
    }

    /// Replaces the CLI layer.
    pub fn set_cli(&mut self, layer: ConfigLayer) {
        self.layers.cli = layer;
    }

    /// Replaces the session layer.
    pub fn set_session(&mut self, layer: ConfigLayer) {
        self.layers.session = layer;
    }

    /// Resolves all configured layers into the final runtime config.
    ///
    /// Source precedence still applies inside this API, so later layers like
    /// session or CLI overrides can replace lower-priority defaults.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::config::{ConfigResolver, LoadedLayers, ResolveOptions};
    ///
    /// let mut layers = LoadedLayers::default();
    /// layers.defaults.set("profile.default", "default");
    /// layers.defaults.set("theme.name", "plain");
    /// layers.session.set("theme.name", "dracula");
    ///
    /// let resolved = ConfigResolver::from_loaded_layers(layers)
    ///     .resolve(ResolveOptions::default())
    ///     .unwrap();
    ///
    /// assert_eq!(resolved.get_string("theme.name"), Some("dracula"));
    /// ```
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

    /// Explains how a runtime key was selected, interpolated, and adapted.
    ///
    /// The explanation keeps the raw winning value as well as the final
    /// adapted value so callers can see where interpolation or type coercion
    /// changed the original input.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::config::{ConfigResolver, LoadedLayers, ResolveOptions};
    ///
    /// let mut layers = LoadedLayers::default();
    /// layers.defaults.set("profile.default", "default");
    /// layers.defaults.set("theme.name", "plain");
    /// layers.cli.set("theme.name", "dracula");
    ///
    /// let explain = ConfigResolver::from_loaded_layers(layers)
    ///     .explain_key("theme.name", ResolveOptions::default())
    ///     .unwrap();
    ///
    /// assert_eq!(explain.key, "theme.name");
    /// assert_eq!(
    ///     explain.final_entry.unwrap().value.reveal(),
    ///     &osp_cli::config::ConfigValue::String("dracula".to_string())
    /// );
    /// ```
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

    /// Explains bootstrap resolution for a bootstrap-only key.
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
                    if should_preserve_selected_secret(previous, &entry) {
                        tracing::trace!(
                            key = %key,
                            secret_origin = ?previous.entry.origin,
                            env_origin = ?entry.entry.origin,
                            "preserving secret env override over plain env value"
                        );
                        continue;
                    }
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

    fn layers(&self) -> [LayerRef<'_>; 7] {
        // Keep this order in ascending priority so later layers can override
        // earlier ones in `select_across_layers()`.
        [
            LayerRef {
                source: ConfigSource::BuiltinDefaults,
                layer: &self.layers.defaults,
            },
            LayerRef {
                source: ConfigSource::PresentationDefaults,
                layer: &self.layers.presentation,
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

fn should_preserve_selected_secret(
    previous: &SelectedLayerEntry<'_>,
    next: &SelectedLayerEntry<'_>,
) -> bool {
    previous.source == ConfigSource::Secrets
        && next.source == ConfigSource::Environment
        && previous.entry.value.is_secret()
        && previous
            .entry
            .origin
            .as_deref()
            .is_some_and(|origin| origin.starts_with("OSP_SECRET__"))
}

#[cfg(test)]
mod tests {
    use super::ConfigResolver;
    use crate::config::{
        ConfigError, ConfigLayer, ConfigSource, ConfigValue, ResolveOptions, Scope,
    };

    #[test]
    fn resolver_layer_mutators_and_setters_are_callable_unit() {
        let mut resolver = ConfigResolver::default();
        resolver.defaults_mut().set("profile.default", "default");
        resolver.file_mut().set("theme.name", "file");
        resolver.secrets_mut().set("profile.default", "default");
        resolver.env_mut().set("theme.name", "env");
        resolver.cli_mut().set("theme.name", "cli");
        resolver.session_mut().set("theme.name", "session");

        let resolved = resolver
            .resolve(ResolveOptions::default().with_terminal("cli"))
            .expect("resolver should resolve");
        assert_eq!(resolved.get_string("theme.name"), Some("session"));
        assert_eq!(resolved.active_profile(), "default");

        let mut replacement = ConfigLayer::default();
        replacement.set("profile.default", "default");
        replacement.set("theme.name", "replaced");
        resolver.set_defaults(replacement);
        resolver.set_file(ConfigLayer::default());
        resolver.set_secrets(ConfigLayer::default());
        resolver.set_env(ConfigLayer::default());
        resolver.set_cli(ConfigLayer::default());
        resolver.set_session(ConfigLayer::default());

        let replaced = resolver
            .resolve(ResolveOptions::default().with_terminal("cli"))
            .expect("replacement config should resolve");
        assert_eq!(replaced.get_string("theme.name"), Some("replaced"));

        let mut resolver = ConfigResolver::default();
        resolver.defaults_mut().set("profile.default", "default");
        resolver.secrets_mut().insert_with_origin(
            "extensions.demo.token",
            ConfigValue::String("secret-token".to_string()).into_secret(),
            Scope::global(),
            Some("OSP_SECRET__AUTH__TOKEN"),
        );
        resolver.env_mut().insert_with_origin(
            "extensions.demo.token",
            ConfigValue::String("plain-token".to_string()),
            Scope::global(),
            Some("OSP__AUTH__TOKEN"),
        );

        let resolved = resolver
            .resolve(ResolveOptions::default())
            .expect("resolver should resolve");
        let entry = resolved
            .get_value_entry("extensions.demo.token")
            .expect("extensions.demo.token should resolve");

        assert!(entry.value.is_secret());
        assert_eq!(
            entry.value.reveal(),
            &ConfigValue::String("secret-token".to_string())
        );
        assert_eq!(entry.source, ConfigSource::Secrets);

        let err = ConfigResolver::default()
            .explain_bootstrap_key("ui.theme", ResolveOptions::default())
            .expect_err("non-bootstrap key should fail");
        assert!(matches!(
            err,
            ConfigError::InvalidConfigKey { key, .. } if key == "ui.theme"
        ));

        let mut resolver = ConfigResolver::default();
        resolver.defaults_mut().set("profile.default", "ops");
        let resolved = resolver
            .resolve(ResolveOptions::default())
            .expect("selected profile without scoped entries should resolve");
        assert_eq!(resolved.active_profile(), "ops");
        assert!(resolved.known_profiles().contains("ops"));
    }
}
