use super::manager::DiscoveredPlugin;
use super::state::{PluginCommandPreferences, PluginCommandState};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderSelectionMode {
    Override,
    Preference,
    Unique,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ProviderSelection<'a> {
    pub(crate) plugin: &'a DiscoveredPlugin,
    pub(crate) mode: ProviderSelectionMode,
}

pub(crate) enum ProviderResolution<'a> {
    Selected(ProviderSelection<'a>),
    Ambiguous(Vec<&'a DiscoveredPlugin>),
}

#[derive(Debug)]
pub(crate) enum ProviderResolutionError<'a> {
    CommandNotFound,
    RequestedProviderUnavailable {
        requested_provider: String,
        providers: Vec<&'a DiscoveredPlugin>,
    },
}

pub(crate) fn healthy_plugins(
    discovered: &[DiscoveredPlugin],
) -> impl Iterator<Item = &DiscoveredPlugin> + '_ {
    discovered.iter().filter(|plugin| plugin.issue.is_none())
}

pub(crate) fn plugin_enabled(
    plugin: &DiscoveredPlugin,
    preferences: &PluginCommandPreferences,
) -> bool {
    let commands = plugin.canonical_command_names();
    if commands.is_empty() {
        return plugin.default_enabled;
    }
    commands
        .into_iter()
        .any(|command| provider_available(plugin, command, preferences))
}

pub(crate) fn plugin_label(plugin: &DiscoveredPlugin) -> String {
    format!("{} ({})", plugin.plugin_id, plugin.source)
}

pub(crate) fn provider_labels(plugins: &[&DiscoveredPlugin]) -> Vec<String> {
    plugins.iter().copied().map(plugin_label).collect()
}

pub(crate) fn providers_for_command<'a>(
    command: &str,
    plugins: &[&'a DiscoveredPlugin],
) -> Vec<&'a DiscoveredPlugin> {
    plugins
        .iter()
        .copied()
        .filter(|plugin| plugin_provides_command(plugin, command))
        .collect()
}

pub(crate) fn provider_labels_by_command(
    plugins: &[&DiscoveredPlugin],
    preferences: &PluginCommandPreferences,
) -> HashMap<String, Vec<String>> {
    let mut index = HashMap::new();
    for plugin in plugins {
        let label = plugin_label(plugin);
        for command in plugin.canonical_command_names() {
            if !provider_available(plugin, command, preferences) {
                continue;
            }
            index
                .entry(command.to_string())
                .or_insert_with(Vec::new)
                .push(label.clone());
        }
    }
    index
}

pub(crate) fn provider_available(
    plugin: &DiscoveredPlugin,
    command: &str,
    preferences: &PluginCommandPreferences,
) -> bool {
    if plugin.canonical_command(command).is_none() {
        return false;
    }
    match preferences.state_for(command) {
        Some(PluginCommandState::Enabled) => true,
        Some(PluginCommandState::Disabled) => false,
        None => plugin.default_enabled,
    }
}

pub(crate) fn resolve_provider_for_command<'a>(
    command: &str,
    plugins: &[&'a DiscoveredPlugin],
    preferences: &PluginCommandPreferences,
    provider_override: Option<&str>,
) -> std::result::Result<ProviderResolution<'a>, ProviderResolutionError<'a>> {
    let providers = available_providers_for_command(command, plugins, preferences);
    if providers.is_empty() {
        return Err(ProviderResolutionError::CommandNotFound);
    }

    if let Some(requested_provider) = provider_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Some(plugin) = providers
            .iter()
            .copied()
            .find(|plugin| plugin.plugin_id == requested_provider)
        {
            return Ok(ProviderResolution::Selected(ProviderSelection {
                plugin,
                mode: ProviderSelectionMode::Override,
            }));
        }
        return Err(ProviderResolutionError::RequestedProviderUnavailable {
            requested_provider: requested_provider.to_string(),
            providers,
        });
    }

    if let Some(preferred) = preferences.preferred_provider_for(command) {
        if let Some(plugin) = providers
            .iter()
            .copied()
            .find(|plugin| plugin.plugin_id == preferred)
        {
            return Ok(ProviderResolution::Selected(ProviderSelection {
                plugin,
                mode: ProviderSelectionMode::Preference,
            }));
        }

        tracing::trace!(
            command = %command,
            preferred_provider = %preferred,
            available_providers = providers.len(),
            "preferred provider not available; reevaluating command provider"
        );
    }

    if providers.len() == 1 {
        return Ok(ProviderResolution::Selected(ProviderSelection {
            plugin: providers[0],
            mode: ProviderSelectionMode::Unique,
        }));
    }

    Ok(ProviderResolution::Ambiguous(providers))
}

fn plugin_provides_command(plugin: &DiscoveredPlugin, command: &str) -> bool {
    plugin.canonical_command(command).is_some()
}

pub(crate) fn available_providers_for_command<'a>(
    command: &str,
    plugins: &[&'a DiscoveredPlugin],
    preferences: &PluginCommandPreferences,
) -> Vec<&'a DiscoveredPlugin> {
    providers_for_command(command, plugins)
        .into_iter()
        .filter(|plugin| provider_available(plugin, command, preferences))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::completion::CommandSpec;
    use crate::plugin::{DiscoveredPlugin, PluginSource};
    use std::path::PathBuf;

    fn plugin(plugin_id: &str, command: &str) -> DiscoveredPlugin {
        DiscoveredPlugin {
            plugin_id: plugin_id.to_string(),
            plugin_version: Some("0.1.0".to_string()),
            executable: PathBuf::from(format!("/tmp/osp-{plugin_id}")),
            source: PluginSource::Explicit,
            commands: vec![command.to_string()],
            describe_commands: Vec::new(),
            command_specs: vec![CommandSpec::new(command)],
            issue: None,
            default_enabled: true,
        }
    }

    #[test]
    fn provider_resolution_prefers_override_then_preference_then_unique_unit() {
        let alpha = plugin("alpha", "shared");
        let beta = plugin("beta", "shared");
        let unique = plugin("solo", "solo");
        let shared_plugins = vec![&alpha, &beta];
        let unique_plugins = vec![&unique];
        let mut preferences = PluginCommandPreferences::default();

        let ProviderResolution::Ambiguous(providers) =
            resolve_provider_for_command("shared", &shared_plugins, &preferences, None)
                .expect("shared command should resolve")
        else {
            panic!("shared command should be ambiguous without overrides");
        };
        assert_eq!(providers.len(), 2);

        preferences.set_provider("shared", "beta");
        let ProviderResolution::Selected(selection) =
            resolve_provider_for_command("shared", &shared_plugins, &preferences, None)
                .expect("preferred provider should resolve")
        else {
            panic!("preferred provider should select one plugin");
        };
        assert_eq!(selection.plugin.plugin_id, "beta");
        assert_eq!(selection.mode, ProviderSelectionMode::Preference);

        let ProviderResolution::Selected(selection) =
            resolve_provider_for_command("shared", &shared_plugins, &preferences, Some("alpha"))
                .expect("override should resolve")
        else {
            panic!("override should select one plugin");
        };
        assert_eq!(selection.plugin.plugin_id, "alpha");
        assert_eq!(selection.mode, ProviderSelectionMode::Override);

        let ProviderResolution::Selected(selection) =
            resolve_provider_for_command("solo", &unique_plugins, &preferences, None)
                .expect("unique provider should resolve")
        else {
            panic!("unique provider should select one plugin");
        };
        assert_eq!(selection.plugin.plugin_id, "solo");
        assert_eq!(selection.mode, ProviderSelectionMode::Unique);
    }

    #[test]
    fn provider_availability_honors_command_state_over_default_enabled_unit() {
        let plugin = plugin("alpha", "shared");
        let mut preferences = PluginCommandPreferences::default();

        assert!(provider_available(&plugin, "shared", &preferences));

        preferences.set_state("shared", PluginCommandState::Disabled);
        assert!(!provider_available(&plugin, "shared", &preferences));

        preferences.set_state("shared", PluginCommandState::Enabled);
        assert!(provider_available(&plugin, "shared", &preferences));
    }

    #[test]
    fn canonical_command_identity_drives_provider_lookup_unit() {
        let plugin = DiscoveredPlugin {
            plugin_id: "alpha".to_string(),
            plugin_version: Some("0.1.0".to_string()),
            executable: PathBuf::from("/tmp/osp-alpha"),
            source: PluginSource::Explicit,
            commands: Vec::new(),
            describe_commands: Vec::new(),
            command_specs: vec![CommandSpec::new("shared")],
            issue: None,
            default_enabled: true,
        };
        let preferences = PluginCommandPreferences::default();

        assert!(provider_available(&plugin, "shared", &preferences));
        assert_eq!(
            providers_for_command("shared", &[&plugin])
                .into_iter()
                .map(|provider| provider.plugin_id.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha"]
        );
    }
}
