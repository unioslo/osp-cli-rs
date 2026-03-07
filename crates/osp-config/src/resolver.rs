use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::bootstrap::{ResolutionFrame, explain_default_profile_key, prepare_resolution};
use crate::selector::{LayerRef, ScopeSelector, SelectedLayerEntry};
use crate::{
    ConfigError, ConfigExplain, ConfigLayer, ConfigSchema, ConfigSource, ConfigValue,
    ExplainInterpolation, ExplainInterpolationStep, ExplainLayer, LoadedLayers, ResolveOptions,
    ResolvedConfig, ResolvedValue, Scope, is_bootstrap_only_key,
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

#[derive(Debug, Clone)]
struct ParsedTemplate {
    raw: String,
    placeholders: Vec<PlaceholderSpan>,
}

#[derive(Debug, Clone)]
struct PlaceholderSpan {
    start: usize,
    end: usize,
    name: String,
}

/// Placeholder expansion is intentionally isolated from scope/source selection.
///
/// By the time interpolation runs, the resolver has already chosen one raw value
/// per key. That keeps interpolation deterministic and lets `config explain`
/// report the same placeholder chain the normal resolution path used.
struct Interpolator {
    raw: HashMap<String, ConfigValue>,
    cache: HashMap<String, ConfigValue>,
}

impl Interpolator {
    fn from_resolved_values(values: &BTreeMap<String, ResolvedValue>) -> Self {
        Self {
            // Interpolation always starts from the raw pre-interpolation value
            // for each key, never from another key's already-expanded value.
            raw: values
                .iter()
                .map(|(key, value)| (key.clone(), value.raw_value.clone()))
                .collect(),
            cache: HashMap::new(),
        }
    }

    fn apply_all(
        &mut self,
        values: &mut BTreeMap<String, ResolvedValue>,
    ) -> Result<(), ConfigError> {
        let keys = values.keys().cloned().collect::<Vec<String>>();
        for key in keys {
            let value = self.resolve_value(&key, &mut Vec::new())?;
            if let Some(entry) = values.get_mut(&key) {
                entry.value = value;
            }
        }

        Ok(())
    }

    fn explain(
        &self,
        key: &str,
        final_values: &BTreeMap<String, ResolvedValue>,
    ) -> Result<Option<ExplainInterpolation>, ConfigError> {
        let Some(template) = self.parsed_template(key)? else {
            return Ok(None);
        };

        let mut steps = Vec::new();
        let mut seen = BTreeSet::new();
        self.collect_steps_recursive(key, final_values, &mut steps, &mut seen, &mut Vec::new())?;

        Ok(Some(ExplainInterpolation {
            template: template.raw,
            steps,
        }))
    }

    fn resolve_value(
        &mut self,
        key: &str,
        stack: &mut Vec<String>,
    ) -> Result<ConfigValue, ConfigError> {
        if let Some(value) = self.cache.get(key) {
            return Ok(value.clone());
        }

        if let Some(index) = stack.iter().position(|item| item == key) {
            let mut cycle = stack[index..].to_vec();
            cycle.push(key.to_string());
            return Err(ConfigError::PlaceholderCycle { cycle });
        }

        let value =
            self.raw
                .get(key)
                .cloned()
                .ok_or_else(|| ConfigError::UnresolvedPlaceholder {
                    key: key.to_string(),
                    placeholder: key.to_string(),
                })?;

        if key.starts_with("alias.") {
            self.cache.insert(key.to_string(), value.clone());
            return Ok(value);
        }

        stack.push(key.to_string());

        let resolved = match value {
            ConfigValue::Secret(secret) => match secret.into_inner() {
                ConfigValue::String(template) => {
                    let (interpolated, _contains_secret) =
                        self.interpolate_template(key, parse_template(key, &template)?, stack)?;
                    ConfigValue::String(interpolated).into_secret()
                }
                other => other.into_secret(),
            },
            ConfigValue::String(template) => {
                let (interpolated, contains_secret) =
                    self.interpolate_template(key, parse_template(key, &template)?, stack)?;
                let value = ConfigValue::String(interpolated);
                if contains_secret {
                    value.into_secret()
                } else {
                    value
                }
            }
            other => other,
        };

        stack.pop();
        self.cache.insert(key.to_string(), resolved.clone());

        Ok(resolved)
    }

    fn interpolate_template(
        &mut self,
        key: &str,
        template: ParsedTemplate,
        stack: &mut Vec<String>,
    ) -> Result<(String, bool), ConfigError> {
        if template.placeholders.is_empty() {
            return Ok((template.raw, false));
        }

        let mut out = String::new();
        let mut cursor = 0usize;
        let mut contains_secret = false;

        for placeholder in &template.placeholders {
            out.push_str(&template.raw[cursor..placeholder.start]);
            let resolved = self.resolve_placeholder(key, &placeholder.name, stack)?;
            if resolved.is_secret() {
                contains_secret = true;
            }
            out.push_str(&resolved.as_interpolation_string(key, &placeholder.name)?);
            cursor = placeholder.end;
        }

        out.push_str(&template.raw[cursor..]);
        Ok((out, contains_secret))
    }

    fn parsed_template(&self, key: &str) -> Result<Option<ParsedTemplate>, ConfigError> {
        if key.starts_with("alias.") {
            return Ok(None);
        }

        let Some(ConfigValue::String(template)) = self.raw.get(key).map(ConfigValue::reveal) else {
            return Ok(None);
        };
        let parsed = parse_template(key, template)?;
        Ok((!parsed.placeholders.is_empty()).then_some(parsed))
    }

    fn resolve_placeholder(
        &mut self,
        key: &str,
        placeholder: &str,
        stack: &mut Vec<String>,
    ) -> Result<ConfigValue, ConfigError> {
        if !self.raw.contains_key(placeholder) {
            return Err(ConfigError::UnresolvedPlaceholder {
                key: key.to_string(),
                placeholder: placeholder.to_string(),
            });
        }

        self.resolve_value(placeholder, stack)
    }

    fn collect_steps_recursive(
        &self,
        key: &str,
        final_values: &BTreeMap<String, ResolvedValue>,
        steps: &mut Vec<ExplainInterpolationStep>,
        seen: &mut BTreeSet<String>,
        stack: &mut Vec<String>,
    ) -> Result<(), ConfigError> {
        let Some(template) = self.parsed_template(key)? else {
            return Ok(());
        };

        if let Some(index) = stack.iter().position(|item| item == key) {
            let mut cycle = stack[index..].to_vec();
            cycle.push(key.to_string());
            return Err(ConfigError::PlaceholderCycle { cycle });
        }

        stack.push(key.to_string());
        for placeholder in &template.placeholders {
            if !self.raw.contains_key(&placeholder.name) {
                return Err(ConfigError::UnresolvedPlaceholder {
                    key: key.to_string(),
                    placeholder: placeholder.name.clone(),
                });
            }

            if seen.insert(placeholder.name.clone())
                && let Some(value_entry) = final_values.get(&placeholder.name)
            {
                steps.push(ExplainInterpolationStep {
                    placeholder: placeholder.name.clone(),
                    value: value_entry.value.clone(),
                    source: value_entry.source,
                    scope: value_entry.scope.clone(),
                    origin: value_entry.origin.clone(),
                });
            }

            self.collect_steps_recursive(&placeholder.name, final_values, steps, seen, stack)?;
        }
        stack.pop();

        Ok(())
    }
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

fn interpolate_all(values: &mut BTreeMap<String, ResolvedValue>) -> Result<(), ConfigError> {
    // Build the interpolator from raw selected values, then write expanded
    // results back into the mutable resolved-value map.
    Interpolator::from_resolved_values(values).apply_all(values)
}

fn explain_interpolation(
    key: &str,
    pre_interpolated: &BTreeMap<String, ResolvedValue>,
    final_values: &BTreeMap<String, ResolvedValue>,
) -> Result<Option<ExplainInterpolation>, ConfigError> {
    Interpolator::from_resolved_values(pre_interpolated).explain(key, final_values)
}

/// Parse `${key}` segments once so interpolation and explain tracing can share
/// the same validated template shape.
fn parse_template(key: &str, template: &str) -> Result<ParsedTemplate, ConfigError> {
    let mut placeholders = Vec::new();
    let mut cursor = 0usize;

    while let Some(rel_start) = template[cursor..].find("${") {
        let start = cursor + rel_start;
        let after_open = start + 2;
        let Some(rel_end) = template[after_open..].find('}') else {
            return Err(ConfigError::InvalidPlaceholderSyntax {
                key: key.to_string(),
                template: template.to_string(),
            });
        };
        let end = after_open + rel_end;

        let placeholder = template[after_open..end].trim();
        if placeholder.is_empty() {
            return Err(ConfigError::InvalidPlaceholderSyntax {
                key: key.to_string(),
                template: template.to_string(),
            });
        }

        placeholders.push(PlaceholderSpan {
            start,
            end: end + 1,
            name: placeholder.to_string(),
        });
        cursor = end + 1;
    }

    Ok(ParsedTemplate {
        raw: template.to_string(),
        placeholders,
    })
}
