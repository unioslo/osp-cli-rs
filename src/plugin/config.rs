use std::collections::{BTreeMap, HashMap};
use std::sync::RwLock;

use crate::config::{ConfigValue, ResolvedConfig};

use crate::app::ConfigState;

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
        if let Some(cached) = read_cached_env(&self.cached)
            .as_ref()
            .filter(|cached| cached.revision == revision)
            .cloned()
        {
            return cached.env;
        }

        let env = collect_plugin_config_env(config.resolved());
        let mut guard = write_cached_env(&self.cached);
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

fn read_cached_env(
    cache: &RwLock<Option<CachedPluginConfigEnv>>,
) -> std::sync::RwLockReadGuard<'_, Option<CachedPluginConfigEnv>> {
    match cache.read() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn write_cached_env(
    cache: &RwLock<Option<CachedPluginConfigEnv>>,
) -> std::sync::RwLockWriteGuard<'_, Option<CachedPluginConfigEnv>> {
    match cache.write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
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

pub(crate) fn plugin_config_entries(
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

#[cfg(test)]
mod tests {
    use crate::config::{ConfigLayer, ConfigResolver, ConfigValue, ResolveOptions};

    use super::{
        PluginConfigEnvCache, PluginConfigScope, collect_plugin_config_env,
        config_value_to_plugin_env, plugin_config_entries, plugin_config_env_name,
    };
    use crate::app::ConfigState;

    fn resolved_config(entries: &[(&str, &str)]) -> crate::config::ResolvedConfig {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        for (key, value) in entries {
            defaults.set(*key, *value);
        }
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        resolver
            .resolve(ResolveOptions::default())
            .expect("test config should resolve")
    }

    #[test]
    fn plugin_entries_merge_shared_and_plugin_specific_values() {
        let config = resolved_config(&[
            ("extensions.plugins.env.endpoint", "shared"),
            ("extensions.plugins.demo.env.endpoint", "plugin"),
            ("extensions.plugins.demo.env.api.token", "token-123"),
        ]);

        let entries = plugin_config_entries(&config, "demo");
        let endpoint = entries
            .iter()
            .find(|entry| entry.env_key == "OSP_PLUGIN_CFG_ENDPOINT")
            .expect("endpoint entry should exist");
        assert_eq!(endpoint.value, "plugin");
        assert_eq!(endpoint.scope, PluginConfigScope::Plugin);

        let token = entries
            .iter()
            .find(|entry| entry.env_key == "OSP_PLUGIN_CFG_API_TOKEN")
            .expect("plugin token should exist");
        assert_eq!(token.value, "token-123");
    }

    #[test]
    fn collect_plugin_config_env_ignores_incomplete_plugin_keys() {
        let config = resolved_config(&[
            ("extensions.plugins..env.endpoint", "skip-empty-plugin"),
            ("extensions.plugins.demo.value", "skip-non-env"),
            (
                "extensions.plugins.env.shared.url",
                "https://shared.example",
            ),
        ]);

        let env = collect_plugin_config_env(&config);
        assert!(env.by_plugin_id.is_empty());
        assert_eq!(env.shared.len(), 1);
        assert_eq!(env.shared[0].env_key, "OSP_PLUGIN_CFG_SHARED_URL");
    }

    #[test]
    fn plugin_config_env_cache_reuses_revision_and_refreshes_after_replace() {
        let cache = PluginConfigEnvCache::default();
        let first = resolved_config(&[("extensions.plugins.env.endpoint", "shared")]);
        let mut state = ConfigState::new(first);

        let shared = cache.collect(&state);
        assert_eq!(shared.shared[0].value, "shared");

        let changed = state.replace_resolved(resolved_config(&[(
            "extensions.plugins.env.endpoint",
            "updated",
        )]));
        assert!(changed);

        let updated = cache.collect(&state);
        assert_eq!(updated.shared[0].value, "updated");
    }

    #[test]
    fn plugin_config_env_name_normalizes_mixed_separators() {
        assert_eq!(
            plugin_config_env_name("api.token-url"),
            Some("OSP_PLUGIN_CFG_API_TOKEN_URL".to_string())
        );
        assert_eq!(plugin_config_env_name("..."), None);
    }

    #[test]
    fn config_value_to_plugin_env_serializes_scalars_lists_and_nans() {
        assert_eq!(
            config_value_to_plugin_env(&ConfigValue::Bool(true)),
            "true".to_string()
        );
        assert_eq!(
            config_value_to_plugin_env(&ConfigValue::List(vec![
                ConfigValue::Integer(7),
                ConfigValue::String("alpha".to_string()),
                ConfigValue::Float(f64::NAN),
            ])),
            r#"[7,"alpha",null]"#.to_string()
        );
    }
}
