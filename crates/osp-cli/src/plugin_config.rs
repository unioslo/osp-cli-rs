use std::collections::{BTreeMap, HashMap};
use std::sync::RwLock;

use osp_config::{ConfigValue, ResolvedConfig};

use crate::state::ConfigState;

const SHARED_PLUGIN_ENV_PREFIX: &str = "extensions.plugins.env.";
const PLUGIN_ENV_ROOT_PREFIX: &str = "extensions.plugins.";
const PLUGIN_ENV_SEPARATOR: &str = ".env.";
const PLUGIN_CONFIG_ENV_PREFIX: &str = "OSP_PLUGIN_CFG_";

#[derive(Debug, Clone, Default)]
pub(crate) struct PluginConfigEnv {
    pub(crate) shared: Vec<PluginConfigEntry>,
    pub(crate) by_plugin_id: HashMap<String, Vec<PluginConfigEntry>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PluginConfigScope {
    Shared,
    Plugin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PluginConfigEntry {
    pub(crate) env_key: String,
    pub(crate) value: String,
    pub(crate) config_key: String,
    pub(crate) scope: PluginConfigScope,
}

#[derive(Debug, Default)]
pub(crate) struct PluginConfigEnvCache {
    cached: RwLock<Option<CachedPluginConfigEnv>>,
}

#[derive(Debug, Clone)]
struct CachedPluginConfigEnv {
    revision: u64,
    env: PluginConfigEnv,
}

impl PluginConfigEnvCache {
    pub(crate) fn collect(&self, config: &ConfigState) -> PluginConfigEnv {
        let revision = config.revision();
        if let Some(cached) = self
            .cached
            .read()
            .expect("plugin config cache poisoned")
            .as_ref()
            .filter(|cached| cached.revision == revision)
            .cloned()
        {
            return cached.env;
        }

        let env = collect_plugin_config_env(config.resolved());
        let mut guard = self.cached.write().expect("plugin config cache poisoned");
        if let Some(cached) = guard.as_ref()
            && cached.revision == revision
        {
            return cached.env.clone();
        }
        *guard = Some(CachedPluginConfigEnv {
            revision,
            env: env.clone(),
        });
        env
    }
}

pub(crate) fn collect_plugin_config_env(config: &ResolvedConfig) -> PluginConfigEnv {
    let mut shared: BTreeMap<String, PluginConfigEntry> = BTreeMap::new();
    let mut by_plugin_id: HashMap<String, BTreeMap<String, PluginConfigEntry>> = HashMap::new();

    for (key, entry) in config.values() {
        if let Some(name) = key.strip_prefix(SHARED_PLUGIN_ENV_PREFIX) {
            if let Some(env_entry) =
                plugin_env_mapping(key, name, &entry.value, PluginConfigScope::Shared)
            {
                shared.insert(env_entry.env_key.clone(), env_entry);
            }
            continue;
        }

        let Some(plugin_key) = key.strip_prefix(PLUGIN_ENV_ROOT_PREFIX) else {
            continue;
        };
        let Some((plugin_id, name)) = plugin_key.split_once(PLUGIN_ENV_SEPARATOR) else {
            continue;
        };
        if plugin_id.is_empty() {
            continue;
        }
        if let Some(env_entry) =
            plugin_env_mapping(key, name, &entry.value, PluginConfigScope::Plugin)
        {
            by_plugin_id
                .entry(plugin_id.to_string())
                .or_default()
                .insert(env_entry.env_key.clone(), env_entry);
        }
    }

    PluginConfigEnv {
        shared: shared.into_values().collect(),
        by_plugin_id: by_plugin_id
            .into_iter()
            .map(|(plugin_id, env)| (plugin_id, env.into_values().collect()))
            .collect(),
    }
}

pub(crate) fn effective_plugin_config_entries(
    config: &ResolvedConfig,
    plugin_id: &str,
) -> Vec<PluginConfigEntry> {
    let config_env = collect_plugin_config_env(config);
    let mut effective = BTreeMap::new();
    for entry in config_env.shared {
        effective.insert(entry.env_key.clone(), entry);
    }
    if let Some(entries) = config_env.by_plugin_id.get(plugin_id) {
        for entry in entries {
            effective.insert(entry.env_key.clone(), entry.clone());
        }
    }
    effective.into_values().collect()
}

fn plugin_env_mapping(
    config_key: &str,
    name: &str,
    value: &ConfigValue,
    scope: PluginConfigScope,
) -> Option<PluginConfigEntry> {
    Some(PluginConfigEntry {
        env_key: plugin_config_env_name(name)?,
        value: config_value_to_plugin_env(value),
        config_key: config_key.to_string(),
        scope,
    })
}

pub(crate) fn plugin_config_env_name(name: &str) -> Option<String> {
    let mut normalized = String::new();
    let mut last_was_separator = true;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch.to_ascii_uppercase());
            last_was_separator = false;
        } else if !last_was_separator {
            normalized.push('_');
            last_was_separator = true;
        }
    }
    while normalized.ends_with('_') {
        normalized.pop();
    }
    if normalized.is_empty() {
        return None;
    }
    Some(format!("{PLUGIN_CONFIG_ENV_PREFIX}{normalized}"))
}

pub(crate) fn config_value_to_plugin_env(value: &ConfigValue) -> String {
    match value {
        ConfigValue::Secret(secret) => config_value_to_plugin_env(secret.expose()),
        ConfigValue::String(value) => value.clone(),
        ConfigValue::Bool(value) => value.to_string(),
        ConfigValue::Integer(value) => value.to_string(),
        ConfigValue::Float(value) => value.to_string(),
        ConfigValue::List(values) => serde_json::Value::Array(
            values
                .iter()
                .map(config_value_to_plugin_env_json)
                .collect::<Vec<_>>(),
        )
        .to_string(),
    }
}

fn config_value_to_plugin_env_json(value: &ConfigValue) -> serde_json::Value {
    match value {
        ConfigValue::Secret(secret) => config_value_to_plugin_env_json(secret.expose()),
        ConfigValue::String(value) => serde_json::Value::String(value.clone()),
        ConfigValue::Bool(value) => serde_json::Value::Bool(*value),
        ConfigValue::Integer(value) => serde_json::Value::Number((*value).into()),
        ConfigValue::Float(value) => serde_json::Number::from_f64(*value)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        ConfigValue::List(values) => serde_json::Value::Array(
            values
                .iter()
                .map(config_value_to_plugin_env_json)
                .collect::<Vec<_>>(),
        ),
    }
}
