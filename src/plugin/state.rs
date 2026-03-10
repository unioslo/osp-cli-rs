use super::conversion::{collect_completion_words, direct_subcommand_names};
use super::manager::{
    CommandCatalogEntry, CommandConflict, DiscoveredPlugin, DoctorReport, PluginManager,
    PluginSummary,
};
use crate::completion::CommandSpec;
use crate::config::{ConfigValue, ResolvedConfig};
use crate::core::command_policy::{CommandPath, CommandPolicyRegistry};
use crate::plugin::PluginDispatchError;
use anyhow::{Result, anyhow};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PluginCommandState {
    Enabled,
    Disabled,
}

impl PluginCommandState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
        }
    }

    fn from_config_value(value: &ConfigValue) -> Option<Self> {
        match value.reveal() {
            ConfigValue::String(raw) if raw.eq_ignore_ascii_case("enabled") => Some(Self::Enabled),
            ConfigValue::String(raw) if raw.eq_ignore_ascii_case("disabled") => {
                Some(Self::Disabled)
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PluginCommandPreferences {
    pub(crate) command_states: BTreeMap<String, PluginCommandState>,
    pub(crate) preferred_providers: BTreeMap<String, String>,
}

impl PluginCommandPreferences {
    pub(crate) fn from_resolved(config: &ResolvedConfig) -> Self {
        let mut preferences = Self::default();
        for (key, entry) in config.values() {
            let Some((command, field)) = plugin_command_config_field(key) else {
                continue;
            };
            match field {
                PluginCommandConfigField::State => {
                    if let Some(state) = PluginCommandState::from_config_value(&entry.value) {
                        preferences.command_states.insert(command, state);
                    }
                }
                PluginCommandConfigField::Provider => {
                    if let ConfigValue::String(provider) = entry.value.reveal() {
                        let provider = provider.trim();
                        if !provider.is_empty() {
                            preferences
                                .preferred_providers
                                .insert(command, provider.to_string());
                        }
                    }
                }
            }
        }
        preferences
    }

    fn state_for(&self, command: &str) -> Option<PluginCommandState> {
        self.command_states.get(command).copied()
    }

    fn preferred_provider_for(&self, command: &str) -> Option<&str> {
        self.preferred_providers.get(command).map(String::as_str)
    }

    #[cfg(test)]
    fn set_state(&mut self, command: &str, state: PluginCommandState) {
        self.command_states.insert(command.to_string(), state);
    }

    fn set_provider(&mut self, command: &str, plugin_id: &str) {
        self.preferred_providers
            .insert(command.to_string(), plugin_id.to_string());
    }

    fn clear_provider(&mut self, command: &str) -> bool {
        self.preferred_providers.remove(command).is_some()
    }
}

enum PluginCommandConfigField {
    State,
    Provider,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderSelectionMode {
    Override,
    Preference,
    Unique,
}

#[derive(Debug, Clone, Copy)]
struct ProviderSelection<'a> {
    plugin: &'a DiscoveredPlugin,
    mode: ProviderSelectionMode,
}

enum ProviderResolution<'a> {
    Selected(ProviderSelection<'a>),
    Ambiguous(Vec<&'a DiscoveredPlugin>),
}

#[derive(Debug)]
enum ProviderResolutionError<'a> {
    CommandNotFound,
    RequestedProviderUnavailable {
        requested_provider: String,
        providers: Vec<&'a DiscoveredPlugin>,
    },
}

impl PluginManager {
    /// Lists discovered plugins with health, command, and enablement status.
    pub fn list_plugins(&self) -> Result<Vec<PluginSummary>> {
        let discovered = self.discover();
        let preferences = self.command_preferences();

        Ok(discovered
            .iter()
            .map(|plugin| PluginSummary {
                enabled: plugin_enabled(plugin, &preferences),
                healthy: plugin.issue.is_none(),
                issue: plugin.issue.clone(),
                plugin_id: plugin.plugin_id.clone(),
                plugin_version: plugin.plugin_version.clone(),
                executable: plugin.executable.clone(),
                source: plugin.source,
                commands: plugin.commands.clone(),
            })
            .collect())
    }

    /// Builds the effective command catalog after provider resolution and health filtering.
    pub fn command_catalog(&self) -> Result<Vec<CommandCatalogEntry>> {
        let preferences = self.command_preferences();
        let discovered = self.discover();
        let healthy = healthy_plugins(discovered.as_ref()).collect::<Vec<_>>();
        let provider_index = provider_labels_by_command(&healthy, &preferences);
        let command_names = healthy
            .iter()
            .flat_map(|plugin| plugin.command_specs.iter().map(|spec| spec.name.clone()))
            .filter(|command| command_has_available_provider(command, &healthy, &preferences))
            .collect::<BTreeSet<_>>();
        let mut out = Vec::new();

        for command_name in command_names {
            let providers = provider_index
                .get(&command_name)
                .cloned()
                .unwrap_or_default();
            match resolve_provider_for_command(&command_name, &healthy, &preferences, None)
                .expect("enabled command name should resolve to one or more providers")
            {
                ProviderResolution::Selected(selection) => {
                    let spec = selection
                        .plugin
                        .command_specs
                        .iter()
                        .find(|spec| spec.name == command_name)
                        .expect("selected provider should include command spec");
                    out.push(CommandCatalogEntry {
                        name: command_name,
                        about: spec.tooltip.clone().unwrap_or_default(),
                        auth: selection
                            .plugin
                            .describe_commands
                            .iter()
                            .find(|candidate| candidate.name == spec.name)
                            .and_then(|candidate| candidate.auth.clone()),
                        subcommands: direct_subcommand_names(spec),
                        completion: spec.clone(),
                        provider: Some(selection.plugin.plugin_id.clone()),
                        providers: providers.clone(),
                        conflicted: providers.len() > 1,
                        requires_selection: false,
                        selected_explicitly: matches!(
                            selection.mode,
                            ProviderSelectionMode::Override | ProviderSelectionMode::Preference
                        ),
                        source: Some(selection.plugin.source),
                    });
                }
                ProviderResolution::Ambiguous(_) => {
                    let about = format!(
                        "provider selection required; use --plugin-provider <plugin-id> or `osp plugins select-provider {command_name} <plugin-id>`"
                    );
                    out.push(CommandCatalogEntry {
                        name: command_name.clone(),
                        about: about.clone(),
                        auth: None,
                        subcommands: Vec::new(),
                        completion: CommandSpec::new(command_name),
                        provider: None,
                        providers: providers.clone(),
                        conflicted: true,
                        requires_selection: true,
                        selected_explicitly: false,
                        source: None,
                    });
                }
            }
        }

        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Builds a command policy registry from active plugin describe metadata.
    pub fn command_policy_registry(&self) -> Result<CommandPolicyRegistry> {
        let preferences = self.command_preferences();
        let discovered = self.discover();
        let healthy = healthy_plugins(discovered.as_ref()).collect::<Vec<_>>();
        let command_names = healthy
            .iter()
            .flat_map(|plugin| plugin.command_specs.iter().map(|spec| spec.name.clone()))
            .filter(|command| command_has_available_provider(command, &healthy, &preferences))
            .collect::<BTreeSet<_>>();
        let mut registry = CommandPolicyRegistry::new();

        for command_name in command_names {
            let resolution =
                resolve_provider_for_command(&command_name, &healthy, &preferences, None).map_err(
                    |err| anyhow!("failed to resolve provider for `{command_name}`: {err:?}"),
                )?;
            let ProviderResolution::Selected(selection) = resolution else {
                continue;
            };

            if let Some(command) = selection
                .plugin
                .describe_commands
                .iter()
                .find(|candidate| candidate.name == command_name)
            {
                register_describe_command_policies(&mut registry, command, &[]);
            }
        }

        Ok(registry)
    }

    /// Returns completion words derived from the current plugin command catalog.
    pub fn completion_words(&self) -> Result<Vec<String>> {
        let catalog = self.command_catalog()?;
        let mut words = vec![
            "help".to_string(),
            "exit".to_string(),
            "quit".to_string(),
            "P".to_string(),
            "F".to_string(),
            "V".to_string(),
            "|".to_string(),
        ];

        for command in catalog {
            words.push(command.name);
            words.extend(collect_completion_words(&command.completion));
        }

        words.sort();
        words.dedup();
        Ok(words)
    }

    /// Renders a plain-text help view for plugin commands in the REPL.
    pub fn repl_help_text(&self) -> Result<String> {
        let catalog = self.command_catalog()?;
        let mut out = String::new();

        out.push_str("Backbone commands: help, exit, quit\n");
        if catalog.is_empty() {
            out.push_str("No plugin commands available.\n");
            return Ok(out);
        }

        out.push_str("Plugin commands:\n");
        for command in catalog {
            let subs = if command.subcommands.is_empty() {
                "".to_string()
            } else {
                format!(" [{}]", command.subcommands.join(", "))
            };
            let auth_hint = command
                .auth_hint()
                .map(|hint| format!(" [{hint}]"))
                .unwrap_or_default();
            let about = if command.about.trim().is_empty() {
                "-".to_string()
            } else {
                command.about.clone()
            };
            if command.requires_selection {
                out.push_str(&format!(
                    "  {name}{subs} - {about}{auth_hint} (providers: {providers})\n",
                    name = command.name,
                    auth_hint = auth_hint,
                    providers = command.providers.join(", "),
                ));
            } else {
                let conflict = if command.conflicted {
                    format!(" conflicts: {}", command.providers.join(", "))
                } else {
                    String::new()
                };
                out.push_str(&format!(
                    "  {name}{subs} - {about}{auth_hint} ({provider}/{source}){conflict}\n",
                    name = command.name,
                    auth_hint = auth_hint,
                    provider = command.provider.as_deref().unwrap_or("-"),
                    source = command
                        .source
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    conflict = conflict,
                ));
            }
        }

        Ok(out)
    }

    /// Returns the available provider labels for a command.
    pub fn command_providers(&self, command: &str) -> Result<Vec<String>> {
        let preferences = self.command_preferences();
        let discovered = self.discover();
        Ok(healthy_plugins(discovered.as_ref())
            .filter(|plugin| provider_available(plugin, command, &preferences))
            .map(plugin_label)
            .collect())
    }

    /// Returns the selected provider label when command resolution is unambiguous.
    pub fn selected_provider_label(&self, command: &str) -> Result<Option<String>> {
        let preferences = self.command_preferences();
        let discovered = self.discover();
        let healthy = healthy_plugins(discovered.as_ref()).collect::<Vec<_>>();
        Ok(
            match resolve_provider_for_command(command, &healthy, &preferences, None).ok() {
                Some(ProviderResolution::Selected(selection)) => {
                    Some(plugin_label(selection.plugin))
                }
                Some(ProviderResolution::Ambiguous(_)) | None => None,
            },
        )
    }

    /// Produces a doctor report with plugin health summaries and command conflicts.
    pub fn doctor(&self) -> Result<DoctorReport> {
        let preferences = self.command_preferences();
        let plugins = self.list_plugins()?;
        let discovered = self.discover();
        let mut conflicts_index: HashMap<String, Vec<String>> = HashMap::new();

        for plugin in healthy_plugins(discovered.as_ref()) {
            if !plugin_enabled(plugin, &preferences) {
                continue;
            }
            for command in &plugin.commands {
                if !provider_available(plugin, command, &preferences) {
                    continue;
                }
                conflicts_index
                    .entry(command.clone())
                    .or_default()
                    .push(plugin_label(plugin));
            }
        }

        let mut conflicts = conflicts_index
            .into_iter()
            .filter_map(|(command, providers)| {
                if providers.len() > 1 {
                    Some(CommandConflict { command, providers })
                } else {
                    None
                }
            })
            .collect::<Vec<CommandConflict>>();
        conflicts.sort_by(|a, b| a.command.cmp(&b.command));

        Ok(DoctorReport { plugins, conflicts })
    }

    pub(crate) fn validate_command(&self, command: &str) -> Result<()> {
        let command = command.trim();
        if command.is_empty() {
            return Err(anyhow!("command must not be empty"));
        }

        let discovered = self.discover_for_dispatch();
        let healthy = healthy_plugins(discovered.as_ref()).collect::<Vec<_>>();
        if providers_for_command(command, &healthy).is_empty() {
            return Err(anyhow!("no healthy plugin provides command `{command}`"));
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn set_command_state(&self, command: &str, state: PluginCommandState) -> Result<()> {
        self.validate_command(command)?;
        self.update_command_preferences(|preferences| {
            preferences.set_state(command, state);
        });
        Ok(())
    }

    /// Persists an explicit provider preference for a command.
    pub fn set_preferred_provider(&self, command: &str, plugin_id: &str) -> Result<()> {
        let command = command.trim();
        let plugin_id = plugin_id.trim();
        if command.is_empty() {
            return Err(anyhow!("command must not be empty"));
        }
        if plugin_id.is_empty() {
            return Err(anyhow!("plugin id must not be empty"));
        }

        self.validate_preferred_provider(command, plugin_id)?;
        self.update_command_preferences(|preferences| preferences.set_provider(command, plugin_id));
        Ok(())
    }

    /// Clears any stored provider preference for a command.
    pub fn clear_preferred_provider(&self, command: &str) -> Result<bool> {
        let command = command.trim();
        if command.is_empty() {
            return Err(anyhow!("command must not be empty"));
        }

        let mut removed = false;
        self.update_command_preferences(|preferences| {
            removed = preferences.clear_provider(command);
        });
        Ok(removed)
    }

    /// Verifies that a plugin is a healthy provider for a command before storing it.
    pub fn validate_preferred_provider(&self, command: &str, plugin_id: &str) -> Result<()> {
        let discovered = self.discover_for_dispatch();
        let healthy = healthy_plugins(discovered.as_ref()).collect::<Vec<_>>();
        let available = providers_for_command(command, &healthy);
        if available.is_empty() {
            return Err(anyhow!("no healthy plugin provides command `{command}`"));
        }
        if !available.iter().any(|plugin| plugin.plugin_id == plugin_id) {
            return Err(anyhow!(
                "plugin `{plugin_id}` does not provide healthy command `{command}`; available providers: {}",
                available
                    .iter()
                    .map(|plugin| plugin_label(plugin))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        Ok(())
    }

    pub(super) fn resolve_provider(
        &self,
        command: &str,
        provider_override: Option<&str>,
    ) -> std::result::Result<DiscoveredPlugin, PluginDispatchError> {
        let preferences = self.command_preferences();
        let discovered = self.discover_for_dispatch();
        let healthy = healthy_plugins(discovered.as_ref()).collect::<Vec<_>>();
        match resolve_provider_for_command(command, &healthy, &preferences, provider_override) {
            Ok(ProviderResolution::Selected(selection)) => {
                tracing::debug!(
                    command = %command,
                    active_providers = providers_for_command(command, &healthy).len(),
                    selected_provider = %selection.plugin.plugin_id,
                    selection_mode = ?selection.mode,
                    "resolved plugin provider"
                );
                Ok(selection.plugin.clone())
            }
            Ok(ProviderResolution::Ambiguous(providers)) => {
                let provider_labels = providers
                    .iter()
                    .copied()
                    .map(plugin_label)
                    .collect::<Vec<_>>();
                tracing::warn!(
                    command = %command,
                    providers = provider_labels.join(", "),
                    "plugin command requires explicit provider selection"
                );
                Err(PluginDispatchError::CommandAmbiguous {
                    command: command.to_string(),
                    providers: provider_labels,
                })
            }
            Err(ProviderResolutionError::RequestedProviderUnavailable {
                requested_provider,
                providers,
            }) => {
                let provider_labels = providers
                    .iter()
                    .copied()
                    .map(plugin_label)
                    .collect::<Vec<_>>();
                tracing::warn!(
                    command = %command,
                    requested_provider = %requested_provider,
                    providers = provider_labels.join(", "),
                    "requested plugin provider is not available for command"
                );
                Err(PluginDispatchError::ProviderNotFound {
                    command: command.to_string(),
                    requested_provider,
                    providers: provider_labels,
                })
            }
            Err(ProviderResolutionError::CommandNotFound) => {
                tracing::warn!(
                    command = %command,
                    active_plugins = healthy.len(),
                    "no plugin provider found for command"
                );
                Err(PluginDispatchError::CommandNotFound {
                    command: command.to_string(),
                })
            }
        }
    }

    fn command_preferences(&self) -> PluginCommandPreferences {
        self.command_preferences
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    pub(crate) fn command_preferences_snapshot(&self) -> PluginCommandPreferences {
        self.command_preferences()
    }

    pub(crate) fn replace_command_preferences(&self, preferences: PluginCommandPreferences) {
        let mut current = self
            .command_preferences
            .write()
            .unwrap_or_else(|err| err.into_inner());
        *current = preferences;
    }

    fn update_command_preferences<F>(&self, update: F)
    where
        F: FnOnce(&mut PluginCommandPreferences),
    {
        let mut preferences = self
            .command_preferences
            .write()
            .unwrap_or_else(|err| err.into_inner());
        update(&mut preferences);
    }
}

fn plugin_command_config_field(key: &str) -> Option<(String, PluginCommandConfigField)> {
    let normalized = key.trim().to_ascii_lowercase();
    let remainder = normalized.strip_prefix("plugins.")?;
    let (command, field) = remainder.rsplit_once('.')?;
    if command.trim().is_empty() {
        return None;
    }
    let field = match field {
        "state" => PluginCommandConfigField::State,
        "provider" => PluginCommandConfigField::Provider,
        _ => return None,
    };
    Some((command.to_string(), field))
}

fn register_describe_command_policies(
    registry: &mut CommandPolicyRegistry,
    command: &crate::core::plugin::DescribeCommandV1,
    prefix: &[String],
) {
    let mut segments = prefix.to_vec();
    segments.push(command.name.clone());
    let path = CommandPath::new(segments.clone());
    if let Some(policy) = command.command_policy(path) {
        registry.register(policy);
    }
    for subcommand in &command.subcommands {
        register_describe_command_policies(registry, subcommand, &segments);
    }
}

fn healthy_plugins<'a>(
    discovered: &'a [DiscoveredPlugin],
) -> impl Iterator<Item = &'a DiscoveredPlugin> + 'a {
    discovered.iter().filter(|plugin| plugin.issue.is_none())
}

fn plugin_enabled(plugin: &DiscoveredPlugin, preferences: &PluginCommandPreferences) -> bool {
    if plugin.commands.is_empty() {
        return plugin.default_enabled;
    }
    plugin
        .commands
        .iter()
        .any(|command| provider_available(plugin, command, preferences))
}

fn plugin_label(plugin: &DiscoveredPlugin) -> String {
    format!("{} ({})", plugin.plugin_id, plugin.source)
}

fn plugin_provides_command(plugin: &DiscoveredPlugin, command: &str) -> bool {
    plugin.commands.iter().any(|name| name == command)
}

fn providers_for_command<'a>(
    command: &str,
    plugins: &[&'a DiscoveredPlugin],
) -> Vec<&'a DiscoveredPlugin> {
    plugins
        .iter()
        .copied()
        .filter(|plugin| plugin_provides_command(plugin, command))
        .collect()
}

fn available_providers_for_command<'a>(
    command: &str,
    plugins: &[&'a DiscoveredPlugin],
    preferences: &PluginCommandPreferences,
) -> Vec<&'a DiscoveredPlugin> {
    providers_for_command(command, plugins)
        .into_iter()
        .filter(|plugin| provider_available(plugin, command, preferences))
        .collect()
}

fn resolve_provider_for_command<'a>(
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

fn provider_labels_by_command(
    plugins: &[&DiscoveredPlugin],
    preferences: &PluginCommandPreferences,
) -> HashMap<String, Vec<String>> {
    let mut index = HashMap::new();
    for plugin in plugins {
        let label = plugin_label(plugin);
        for command in &plugin.commands {
            if !provider_available(plugin, command, preferences) {
                continue;
            }
            index
                .entry(command.clone())
                .or_insert_with(Vec::new)
                .push(label.clone());
        }
    }
    index
}

fn command_has_available_provider(
    command: &str,
    plugins: &[&DiscoveredPlugin],
    preferences: &PluginCommandPreferences,
) -> bool {
    plugins
        .iter()
        .copied()
        .any(|plugin| provider_available(plugin, command, preferences))
}

fn provider_available(
    plugin: &DiscoveredPlugin,
    command: &str,
    preferences: &PluginCommandPreferences,
) -> bool {
    if !plugin_provides_command(plugin, command) {
        return false;
    }
    match preferences.state_for(command) {
        Some(PluginCommandState::Enabled) => true,
        Some(PluginCommandState::Disabled) => false,
        None => plugin.default_enabled,
    }
}

pub(super) fn write_text_atomic(path: &std::path::Path, payload: &str) -> Result<()> {
    crate::config::write_text_atomic(path, payload.as_bytes(), false).map_err(Into::into)
}

pub(super) fn merge_issue(target: &mut Option<String>, message: String) {
    if message.trim().is_empty() {
        return;
    }

    match target {
        Some(existing) => {
            existing.push_str("; ");
            existing.push_str(&message);
        }
        None => *target = Some(message),
    }
}
