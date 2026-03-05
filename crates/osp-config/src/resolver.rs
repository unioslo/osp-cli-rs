use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::{
    ConfigError, ConfigExplain, ConfigLayer, ConfigSchema, ConfigSource, ConfigValue,
    ExplainCandidate, ExplainInterpolation, ExplainInterpolationStep, ExplainLayer, LayerEntry,
    LoadedLayers, ResolveOptions, ResolvedConfig, ResolvedValue, Scope, normalize_identifier,
};

#[derive(Debug, Clone)]
pub struct ConfigResolver {
    defaults: ConfigLayer,
    file: ConfigLayer,
    secrets: ConfigLayer,
    env: ConfigLayer,
    cli: ConfigLayer,
    session: ConfigLayer,
    schema: ConfigSchema,
}

impl Default for ConfigResolver {
    fn default() -> Self {
        Self {
            defaults: ConfigLayer::default(),
            file: ConfigLayer::default(),
            secrets: ConfigLayer::default(),
            env: ConfigLayer::default(),
            cli: ConfigLayer::default(),
            session: ConfigLayer::default(),
            schema: ConfigSchema::default(),
        }
    }
}

impl ConfigResolver {
    pub fn from_loaded_layers(layers: LoadedLayers) -> Self {
        Self {
            defaults: layers.defaults,
            file: layers.file,
            secrets: layers.secrets,
            env: layers.env,
            cli: layers.cli,
            session: layers.session,
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
        &mut self.defaults
    }

    pub fn file_mut(&mut self) -> &mut ConfigLayer {
        &mut self.file
    }

    pub fn secrets_mut(&mut self) -> &mut ConfigLayer {
        &mut self.secrets
    }

    pub fn env_mut(&mut self) -> &mut ConfigLayer {
        &mut self.env
    }

    pub fn cli_mut(&mut self) -> &mut ConfigLayer {
        &mut self.cli
    }

    pub fn session_mut(&mut self) -> &mut ConfigLayer {
        &mut self.session
    }

    pub fn set_defaults(&mut self, layer: ConfigLayer) {
        self.defaults = layer;
    }

    pub fn set_file(&mut self, layer: ConfigLayer) {
        self.file = layer;
    }

    pub fn set_secrets(&mut self, layer: ConfigLayer) {
        self.secrets = layer;
    }

    pub fn set_env(&mut self, layer: ConfigLayer) {
        self.env = layer;
    }

    pub fn set_cli(&mut self, layer: ConfigLayer) {
        self.cli = layer;
    }

    pub fn set_session(&mut self, layer: ConfigLayer) {
        self.session = layer;
    }

    pub fn resolve(&self, options: ResolveOptions) -> Result<ResolvedConfig, ConfigError> {
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
        let mut values = self.collect_selected_values(&active_profile, terminal.as_deref());

        interpolate_all(&mut values)?;
        self.schema.validate_and_adapt(&mut values)?;

        Ok(ResolvedConfig {
            active_profile,
            terminal,
            known_profiles,
            values,
        })
    }

    pub fn explain_key(
        &self,
        key: &str,
        options: ResolveOptions,
    ) -> Result<ConfigExplain, ConfigError> {
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

        let mut layers = Vec::new();
        for (source, layer) in self.layers() {
            let selected_entry =
                select_scoped_entry(layer, key, &active_profile, terminal.as_deref());
            let selected_entry_index = selected_entry.and_then(|entry| {
                layer
                    .entries
                    .iter()
                    .position(|candidate| std::ptr::eq(candidate, entry))
            });

            let mut candidates = Vec::new();
            for (entry_index, entry) in layer.entries.iter().enumerate() {
                if entry.key != key {
                    continue;
                }

                let rank = scope_rank(&entry.scope, &active_profile, terminal.as_deref());
                candidates.push(ExplainCandidate {
                    entry_index,
                    value: entry.value.clone(),
                    scope: entry.scope.clone(),
                    origin: entry.origin.clone(),
                    rank,
                    selected_in_layer: selected_entry_index == Some(entry_index),
                });
            }

            if !candidates.is_empty() {
                layers.push(ExplainLayer {
                    source,
                    selected_entry_index,
                    candidates,
                });
            }
        }

        let pre_interpolated = self.collect_selected_values(&active_profile, terminal.as_deref());
        let mut final_values = pre_interpolated.clone();
        interpolate_all(&mut final_values)?;
        self.schema.validate_and_adapt(&mut final_values)?;
        let final_entry = final_values.get(key).cloned();
        let interpolation = explain_interpolation(key, &pre_interpolated, &final_values)?;

        Ok(ConfigExplain {
            key: key.to_string(),
            active_profile,
            terminal,
            known_profiles,
            layers,
            final_entry,
            interpolation,
        })
    }

    fn collect_selected_values(
        &self,
        active_profile: &str,
        terminal: Option<&str>,
    ) -> BTreeMap<String, ResolvedValue> {
        let mut keys = self.collect_keys();
        keys.insert("profile.default".to_string());

        let mut values = BTreeMap::new();
        for key in keys {
            if let Some((source, entry)) = self.select_across_layers(&key, active_profile, terminal)
            {
                values.insert(
                    key,
                    ResolvedValue {
                        raw_value: entry.value.clone(),
                        value: entry.value.clone(),
                        source,
                        scope: entry.scope.clone(),
                        origin: entry.origin.clone(),
                    },
                );
            }
        }

        values.insert(
            "profile.active".to_string(),
            ResolvedValue {
                raw_value: ConfigValue::String(active_profile.to_string()),
                value: ConfigValue::String(active_profile.to_string()),
                source: ConfigSource::Derived,
                scope: Scope::global(),
                origin: None,
            },
        );
        values.insert(
            "context".to_string(),
            ResolvedValue {
                raw_value: ConfigValue::String(active_profile.to_string()),
                value: ConfigValue::String(active_profile.to_string()),
                source: ConfigSource::Derived,
                scope: Scope::global(),
                origin: None,
            },
        );

        values
    }

    fn collect_known_profiles(&self) -> BTreeSet<String> {
        let mut known = BTreeSet::new();

        for (_, layer) in self.layers() {
            for entry in &layer.entries {
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

        for (_, layer) in self.layers() {
            if let Some(entry) = select_global_entry(layer, "profile.default", terminal) {
                picked = Some(entry.value.clone());
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

        for (_, layer) in self.layers() {
            for entry in &layer.entries {
                keys.insert(entry.key.clone());
            }
        }

        keys
    }

    fn select_across_layers<'a>(
        &'a self,
        key: &str,
        profile: &str,
        terminal: Option<&str>,
    ) -> Option<(ConfigSource, &'a LayerEntry)> {
        let mut selected: Option<(ConfigSource, &'a LayerEntry)> = None;

        for (source, layer) in self.layers() {
            if let Some(entry) = select_scoped_entry(layer, key, profile, terminal) {
                selected = Some((source, entry));
            }
        }

        selected
    }

    fn layers(&self) -> [(ConfigSource, &ConfigLayer); 6] {
        [
            (ConfigSource::BuiltinDefaults, &self.defaults),
            (ConfigSource::ConfigFile, &self.file),
            (ConfigSource::Secrets, &self.secrets),
            (ConfigSource::Environment, &self.env),
            (ConfigSource::Cli, &self.cli),
            (ConfigSource::Session, &self.session),
        ]
    }
}

fn select_scoped_entry<'a>(
    layer: &'a ConfigLayer,
    key: &str,
    profile: &str,
    terminal: Option<&str>,
) -> Option<&'a LayerEntry> {
    select_entry(layer, key, |scope| scope_rank(scope, profile, terminal))
}

fn select_global_entry<'a>(
    layer: &'a ConfigLayer,
    key: &str,
    terminal: Option<&str>,
) -> Option<&'a LayerEntry> {
    select_entry(layer, key, |scope| global_rank(scope, terminal))
}

fn select_entry<'a, F>(layer: &'a ConfigLayer, key: &str, ranker: F) -> Option<&'a LayerEntry>
where
    F: Fn(&Scope) -> Option<u8>,
{
    let mut best: Option<(usize, u8, &'a LayerEntry)> = None;

    for (index, entry) in layer.entries.iter().enumerate() {
        if entry.key != key {
            continue;
        }

        let Some(rank) = ranker(&entry.scope) else {
            continue;
        };

        let replace = match best {
            None => true,
            Some((best_index, best_rank, _)) => {
                rank < best_rank || (rank == best_rank && index > best_index)
            }
        };

        if replace {
            best = Some((index, rank, entry));
        }
    }

    best.map(|(_, _, entry)| entry)
}

fn scope_rank(scope: &Scope, profile: &str, terminal: Option<&str>) -> Option<u8> {
    match (
        scope.profile.as_deref(),
        scope.terminal.as_deref(),
        terminal,
    ) {
        (Some(p), Some(t), Some(active_t)) if p == profile && t == active_t => Some(0),
        (Some(p), None, _) if p == profile => Some(1),
        (None, Some(t), Some(active_t)) if t == active_t => Some(2),
        (None, None, _) => Some(3),
        _ => None,
    }
}

fn global_rank(scope: &Scope, terminal: Option<&str>) -> Option<u8> {
    match (
        scope.profile.as_deref(),
        scope.terminal.as_deref(),
        terminal,
    ) {
        (None, Some(t), Some(active_t)) if t == active_t => Some(0),
        (None, None, _) => Some(1),
        _ => None,
    }
}

fn interpolate_all(values: &mut BTreeMap<String, ResolvedValue>) -> Result<(), ConfigError> {
    let raw = values
        .iter()
        .map(|(key, value)| (key.clone(), value.value.clone()))
        .collect::<HashMap<String, ConfigValue>>();

    let keys = values.keys().cloned().collect::<Vec<String>>();
    let mut cache: HashMap<String, ConfigValue> = HashMap::new();
    let mut stack: Vec<String> = Vec::new();

    for key in keys {
        let value = resolve_interpolated_value(&key, &raw, &mut cache, &mut stack)?;
        if let Some(entry) = values.get_mut(&key) {
            entry.value = value;
        }
    }

    Ok(())
}

fn resolve_interpolated_value(
    key: &str,
    raw: &HashMap<String, ConfigValue>,
    cache: &mut HashMap<String, ConfigValue>,
    stack: &mut Vec<String>,
) -> Result<ConfigValue, ConfigError> {
    if let Some(value) = cache.get(key) {
        return Ok(value.clone());
    }

    if let Some(index) = stack.iter().position(|item| item == key) {
        let mut cycle = stack[index..].to_vec();
        cycle.push(key.to_string());
        return Err(ConfigError::PlaceholderCycle { cycle });
    }

    let value = raw
        .get(key)
        .cloned()
        .ok_or_else(|| ConfigError::UnresolvedPlaceholder {
            key: key.to_string(),
            placeholder: key.to_string(),
        })?;

    if key.starts_with("alias.") {
        cache.insert(key.to_string(), value.clone());
        return Ok(value);
    }

    stack.push(key.to_string());

    let resolved = match value {
        ConfigValue::Secret(secret) => match secret.into_inner() {
            ConfigValue::String(template) => {
                let (interpolated, _contains_secret) =
                    interpolate_string(key, &template, raw, cache, stack)?;
                ConfigValue::String(interpolated).into_secret()
            }
            other => other.into_secret(),
        },
        ConfigValue::String(template) => {
            let (interpolated, contains_secret) =
                interpolate_string(key, &template, raw, cache, stack)?;
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
    cache.insert(key.to_string(), resolved.clone());

    Ok(resolved)
}

fn interpolate_string(
    key: &str,
    template: &str,
    raw: &HashMap<String, ConfigValue>,
    cache: &mut HashMap<String, ConfigValue>,
    stack: &mut Vec<String>,
) -> Result<(String, bool), ConfigError> {
    let mut out = String::new();
    let mut cursor = 0usize;
    let mut contains_secret = false;

    while let Some(rel_start) = template[cursor..].find("${") {
        let start = cursor + rel_start;
        out.push_str(&template[cursor..start]);

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

        if !raw.contains_key(placeholder) {
            return Err(ConfigError::UnresolvedPlaceholder {
                key: key.to_string(),
                placeholder: placeholder.to_string(),
            });
        }

        let resolved = resolve_interpolated_value(placeholder, raw, cache, stack)?;
        if resolved.is_secret() {
            contains_secret = true;
        }
        let interpolated = resolved.as_interpolation_string(key, placeholder)?;
        out.push_str(&interpolated);

        cursor = end + 1;
    }

    out.push_str(&template[cursor..]);
    Ok((out, contains_secret))
}

fn explain_interpolation(
    key: &str,
    pre_interpolated: &BTreeMap<String, ResolvedValue>,
    final_values: &BTreeMap<String, ResolvedValue>,
) -> Result<Option<ExplainInterpolation>, ConfigError> {
    if key.starts_with("alias.") {
        return Ok(None);
    }
    let Some(entry) = pre_interpolated.get(key) else {
        return Ok(None);
    };
    let ConfigValue::String(template) = entry.raw_value.reveal() else {
        return Ok(None);
    };
    if !template.contains("${") {
        return Ok(None);
    }

    let raw = pre_interpolated
        .iter()
        .map(|(entry_key, value)| (entry_key.clone(), value.raw_value.clone()))
        .collect::<HashMap<String, ConfigValue>>();
    let mut steps = Vec::new();
    let mut seen = BTreeSet::new();
    let mut stack = Vec::new();
    collect_interpolation_steps_recursive(
        key,
        &raw,
        final_values,
        &mut steps,
        &mut seen,
        &mut stack,
    )?;

    Ok(Some(ExplainInterpolation {
        template: template.clone(),
        steps,
    }))
}

fn collect_interpolation_steps_recursive(
    key: &str,
    raw: &HashMap<String, ConfigValue>,
    final_values: &BTreeMap<String, ResolvedValue>,
    steps: &mut Vec<ExplainInterpolationStep>,
    seen: &mut BTreeSet<String>,
    stack: &mut Vec<String>,
) -> Result<(), ConfigError> {
    if key.starts_with("alias.") {
        return Ok(());
    }
    if let Some(index) = stack.iter().position(|item| item == key) {
        let mut cycle = stack[index..].to_vec();
        cycle.push(key.to_string());
        return Err(ConfigError::PlaceholderCycle { cycle });
    }

    let Some(ConfigValue::String(template)) = raw.get(key).map(ConfigValue::reveal) else {
        return Ok(());
    };
    if !template.contains("${") {
        return Ok(());
    }

    stack.push(key.to_string());
    let placeholders = extract_placeholders(key, template)?;

    for placeholder in placeholders {
        if !raw.contains_key(&placeholder) {
            return Err(ConfigError::UnresolvedPlaceholder {
                key: key.to_string(),
                placeholder,
            });
        }

        if seen.insert(placeholder.clone())
            && let Some(value_entry) = final_values.get(&placeholder)
        {
            steps.push(ExplainInterpolationStep {
                placeholder: placeholder.clone(),
                value: value_entry.value.clone(),
                source: value_entry.source,
                scope: value_entry.scope.clone(),
                origin: value_entry.origin.clone(),
            });
        }

        collect_interpolation_steps_recursive(&placeholder, raw, final_values, steps, seen, stack)?;
    }

    stack.pop();
    Ok(())
}

fn extract_placeholders(key: &str, template: &str) -> Result<Vec<String>, ConfigError> {
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

        placeholders.push(placeholder.to_string());
        cursor = end + 1;
    }

    Ok(placeholders)
}
