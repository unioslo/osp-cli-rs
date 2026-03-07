use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::{
    ConfigError, ConfigExplain, ConfigLayer, ConfigSchema, ConfigSource, ConfigValue,
    ExplainCandidate, ExplainInterpolation, ExplainInterpolationStep, ExplainLayer, LayerEntry,
    LoadedLayers, ResolveOptions, ResolvedConfig, ResolvedValue, Scope, normalize_identifier,
};

#[derive(Debug, Clone, Default)]
pub struct ConfigResolver {
    layers: LoadedLayers,
    schema: ConfigSchema,
}

/// One resolution request always runs against a single active profile/terminal pair.
///
/// The resolver computes this frame once up front so value selection, explain output,
/// and interpolation all describe the same view of the world.
#[derive(Debug, Clone)]
struct ResolutionFrame {
    active_profile: String,
    terminal: Option<String>,
    known_profiles: BTreeSet<String>,
}

/// Resolution happens in two steps:
/// 1. pick the winning raw value for each key using source + scope precedence
/// 2. interpolate placeholders and then run schema adaptation on those winners
#[derive(Debug, Clone)]
struct ResolvedMaps {
    pre_interpolated: BTreeMap<String, ResolvedValue>,
    final_values: BTreeMap<String, ResolvedValue>,
}

#[derive(Debug, Clone, Copy)]
struct LayerRef<'a> {
    source: ConfigSource,
    layer: &'a ConfigLayer,
}

#[derive(Debug, Clone)]
struct SelectedLayerEntry<'a> {
    source: ConfigSource,
    entry_index: usize,
    entry: &'a LayerEntry,
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

/// Scope precedence is small but subtle:
/// profile+terminal > profile > terminal > global.
///
/// Keeping that policy in one selector object makes `resolve()`, default-profile
/// lookup, and `explain_key()` share the same matching rules instead of each
/// rebuilding them slightly differently.
#[derive(Debug, Clone, Copy)]
struct ScopeSelector<'a> {
    profile: Option<&'a str>,
    terminal: Option<&'a str>,
}

impl<'a> ScopeSelector<'a> {
    fn scoped(profile: &'a str, terminal: Option<&'a str>) -> Self {
        Self {
            profile: Some(profile),
            terminal,
        }
    }

    fn global(terminal: Option<&'a str>) -> Self {
        Self {
            profile: None,
            terminal,
        }
    }

    fn rank(self, scope: &Scope) -> Option<u8> {
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

    fn select(self, layer: LayerRef<'a>, key: &str) -> Option<SelectedLayerEntry<'a>> {
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

    fn explain_layer(self, layer: LayerRef<'a>, key: &str) -> Option<ExplainLayer> {
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

impl Interpolator {
    fn from_resolved_values(values: &BTreeMap<String, ResolvedValue>) -> Self {
        Self {
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
        let frame = self.prepare_resolution(options)?;
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
        let frame = self.prepare_resolution(options)?;
        let layers = self.explain_layers_for_key(key, &frame);
        let resolved = self.resolve_maps_for_frame(&frame)?;
        let final_entry = resolved.final_values.get(key).cloned();
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

    /// Build the single resolution frame shared by normal resolution and
    /// `config explain`.
    fn prepare_resolution(&self, options: ResolveOptions) -> Result<ResolutionFrame, ConfigError> {
        let terminal = options.terminal.map(|value| normalize_identifier(&value));
        let profile_override = options
            .profile_override
            .map(|value| normalize_identifier(&value));
        let known_profiles = self.collect_known_profiles();
        let active_profile = self.resolve_active_profile(
            profile_override.as_deref(),
            terminal.as_deref(),
            &known_profiles,
        )?;

        Ok(ResolutionFrame {
            active_profile,
            terminal,
            known_profiles,
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
        let mut keys = self.collect_keys();
        keys.insert("profile.default".to_string());

        let mut values = BTreeMap::new();
        for key in keys {
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

    fn collect_known_profiles(&self) -> BTreeSet<String> {
        let mut known = BTreeSet::new();

        for layer in self.layers() {
            for entry in &layer.layer.entries {
                if let Some(profile) = entry.scope.profile.as_deref() {
                    known.insert(profile.to_string());
                }
            }
        }

        known
    }

    fn resolve_active_profile(
        &self,
        explicit: Option<&str>,
        terminal: Option<&str>,
        known_profiles: &BTreeSet<String>,
    ) -> Result<String, ConfigError> {
        // Explicit `--profile` wins. Otherwise fall back to the resolved
        // `profile.default` view for the current terminal.
        let chosen = if let Some(profile) = explicit {
            normalize_identifier(profile)
        } else {
            self.resolve_default_profile(terminal)?
        };

        if chosen.trim().is_empty() {
            return Err(ConfigError::MissingDefaultProfile);
        }

        if !known_profiles.is_empty() && !known_profiles.contains(&chosen) {
            return Err(ConfigError::UnknownProfile {
                profile: chosen,
                known: known_profiles.iter().cloned().collect::<Vec<String>>(),
            });
        }

        Ok(chosen)
    }

    fn resolve_default_profile(&self, terminal: Option<&str>) -> Result<String, ConfigError> {
        let mut picked: Option<ConfigValue> = None;
        let selector = ScopeSelector::global(terminal);

        for layer in self.layers() {
            if let Some(selected) = selector.select(layer, "profile.default") {
                picked = Some(selected.entry.value.clone());
            }
        }

        match picked {
            None => Ok("default".to_string()),
            Some(value) => match value.reveal() {
                ConfigValue::String(profile) if !profile.trim().is_empty() => {
                    Ok(normalize_identifier(profile))
                }
                other => Err(ConfigError::InvalidDefaultProfileType(format!("{other:?}"))),
            },
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
