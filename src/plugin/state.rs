use super::conversion::{collect_completion_words, direct_subcommand_names};
use super::manager::{
    CommandCatalogEntry, CommandConflict, DiscoveredPlugin, DoctorReport, PluginManager,
    PluginSummary,
};
use crate::completion::CommandSpec;
use crate::config::default_config_root_dir;
use crate::core::command_policy::{CommandPath, CommandPolicyRegistry};
use crate::plugin::PluginDispatchError;
use anyhow::{Result, anyhow};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct PluginState {
    #[serde(default)]
    pub(super) enabled: Vec<String>,
    #[serde(default)]
    pub(super) disabled: Vec<String>,
    #[serde(default)]
    pub(super) preferred_providers: BTreeMap<String, String>,
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
    pub fn list_plugins(&self) -> Result<Vec<PluginSummary>> {
        let discovered = self.discover();
        let state = self.load_state()?;

        Ok(discovered
            .iter()
            .map(|plugin| PluginSummary {
                enabled: is_enabled(&state, &plugin.plugin_id, plugin.default_enabled),
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

    pub fn command_catalog(&self) -> Result<Vec<CommandCatalogEntry>> {
        let state = self.load_state()?;
        let discovered = self.discover();
        let active = active_plugins(discovered.as_ref(), &state).collect::<Vec<_>>();
        let provider_index = provider_labels_by_command(&active);
        let command_names = active
            .iter()
            .flat_map(|plugin| plugin.command_specs.iter().map(|spec| spec.name.clone()))
            .collect::<BTreeSet<_>>();
        let mut out = Vec::new();

        for command_name in command_names {
            let providers = provider_index
                .get(&command_name)
                .cloned()
                .unwrap_or_default();
            match resolve_provider_for_command(&command_name, &active, &state, None)
                .expect("active command name should resolve to one or more providers")
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

    pub fn command_policy_registry(&self) -> Result<CommandPolicyRegistry> {
        let state = self.load_state()?;
        let discovered = self.discover();
        let active = active_plugins(discovered.as_ref(), &state).collect::<Vec<_>>();
        let command_names = active
            .iter()
            .flat_map(|plugin| plugin.command_specs.iter().map(|spec| spec.name.clone()))
            .collect::<BTreeSet<_>>();
        let mut registry = CommandPolicyRegistry::new();

        for command_name in command_names {
            let resolution = resolve_provider_for_command(&command_name, &active, &state, None)
                .map_err(|err| {
                    anyhow!("failed to resolve provider for `{command_name}`: {err:?}")
                })?;
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

    pub fn command_providers(&self, command: &str) -> Result<Vec<String>> {
        let state = self.load_state()?;
        let discovered = self.discover();
        let mut out = Vec::new();
        for plugin in active_plugins(discovered.as_ref(), &state) {
            if plugin.commands.iter().any(|name| name == command) {
                out.push(format!("{} ({})", plugin.plugin_id, plugin.source));
            }
        }
        Ok(out)
    }

    pub fn selected_provider_label(&self, command: &str) -> Result<Option<String>> {
        let state = self.load_state()?;
        let discovered = self.discover();
        let active = active_plugins(discovered.as_ref(), &state).collect::<Vec<_>>();
        Ok(
            match resolve_provider_for_command(command, &active, &state, None).ok() {
                Some(ProviderResolution::Selected(selection)) => {
                    Some(plugin_label(selection.plugin))
                }
                Some(ProviderResolution::Ambiguous(_)) | None => None,
            },
        )
    }

    pub fn doctor(&self) -> Result<DoctorReport> {
        let plugins = self.list_plugins()?;
        let mut conflicts_index: HashMap<String, Vec<String>> = HashMap::new();

        for plugin in &plugins {
            if !plugin.enabled || !plugin.healthy {
                continue;
            }
            for command in &plugin.commands {
                conflicts_index
                    .entry(command.clone())
                    .or_default()
                    .push(format!("{} ({})", plugin.plugin_id, plugin.source));
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

    pub fn set_enabled(&self, plugin_id: &str, enabled: bool) -> Result<()> {
        let mut state = self.load_state()?;
        state.enabled.retain(|id| id != plugin_id);
        state.disabled.retain(|id| id != plugin_id);

        if enabled {
            state.enabled.push(plugin_id.to_string());
        } else {
            state.disabled.push(plugin_id.to_string());
        }

        state.enabled.sort();
        state.enabled.dedup();
        state.disabled.sort();
        state.disabled.dedup();
        self.save_state(&state)
    }

    pub fn set_preferred_provider(&self, command: &str, plugin_id: &str) -> Result<()> {
        let command = command.trim();
        let plugin_id = plugin_id.trim();
        if command.is_empty() {
            return Err(anyhow!("command must not be empty"));
        }
        if plugin_id.is_empty() {
            return Err(anyhow!("plugin id must not be empty"));
        }

        let mut state = self.load_state()?;
        let discovered = self.discover();
        let active = active_plugins(discovered.as_ref(), &state).collect::<Vec<_>>();
        let available = providers_for_command(command, &active);
        if available.is_empty() {
            return Err(anyhow!("no active plugin provides command `{command}`"));
        }
        if !available.iter().any(|plugin| plugin.plugin_id == plugin_id) {
            return Err(anyhow!(
                "plugin `{plugin_id}` does not provide active command `{command}`; available providers: {}",
                available
                    .iter()
                    .map(|plugin| plugin_label(plugin))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        state
            .preferred_providers
            .insert(command.to_string(), plugin_id.to_string());
        self.save_state(&state)
    }

    pub fn clear_preferred_provider(&self, command: &str) -> Result<bool> {
        let command = command.trim();
        if command.is_empty() {
            return Err(anyhow!("command must not be empty"));
        }

        let mut state = self.load_state()?;
        let removed = state.preferred_providers.remove(command).is_some();
        if removed {
            self.save_state(&state)?;
        }
        Ok(removed)
    }

    pub(super) fn resolve_provider(
        &self,
        command: &str,
        provider_override: Option<&str>,
    ) -> std::result::Result<DiscoveredPlugin, PluginDispatchError> {
        let state = self
            .load_state()
            .map_err(|source| PluginDispatchError::StateLoadFailed { source })?;
        let discovered = self.discover();
        let active = active_plugins(discovered.as_ref(), &state).collect::<Vec<_>>();
        match resolve_provider_for_command(command, &active, &state, provider_override) {
            Ok(ProviderResolution::Selected(selection)) => {
                tracing::debug!(
                    command = %command,
                    active_providers = providers_for_command(command, &active).len(),
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
                    active_plugins = active.len(),
                    "no plugin provider found for command"
                );
                Err(PluginDispatchError::CommandNotFound {
                    command: command.to_string(),
                })
            }
        }
    }

    pub(super) fn load_state(&self) -> Result<PluginState> {
        let path = self
            .plugin_state_path()
            .ok_or_else(|| anyhow!("failed to resolve plugin state path"))?;
        if !path.exists() {
            tracing::debug!(path = %path.display(), "plugin state file missing; using defaults");
            return Ok(PluginState::default());
        }

        let raw = std::fs::read_to_string(&path)
            .map_err(|err| anyhow!("failed to read plugin state {}: {err}", path.display()))?;
        let raw = serde_json::from_str::<PluginState>(&raw).map_err(|err| {
            anyhow!(
                "failed to parse plugin state {} at line {}, column {}: {}",
                path.display(),
                err.line(),
                err.column(),
                err
            )
        })?;
        tracing::debug!(
            path = %path.display(),
            enabled = raw.enabled.len(),
            disabled = raw.disabled.len(),
            preferred = raw.preferred_providers.len(),
            "loaded plugin state"
        );
        Ok(raw)
    }

    pub(super) fn save_state(&self, state: &PluginState) -> Result<()> {
        let path = self
            .plugin_state_path()
            .ok_or_else(|| anyhow!("failed to resolve plugin state path"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let payload = serde_json::to_string_pretty(state)?;
        write_text_atomic(&path, &payload)
    }

    fn plugin_state_path(&self) -> Option<PathBuf> {
        let mut path = self.config_root.clone().or_else(default_config_root_dir)?;
        path.push("plugins.json");
        Some(path)
    }
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

pub(super) fn is_active_plugin(plugin: &DiscoveredPlugin, state: &PluginState) -> bool {
    plugin.issue.is_none() && is_enabled(state, &plugin.plugin_id, plugin.default_enabled)
}

pub(super) fn active_plugins<'a>(
    discovered: &'a [DiscoveredPlugin],
    state: &'a PluginState,
) -> impl Iterator<Item = &'a DiscoveredPlugin> + 'a {
    discovered
        .iter()
        .filter(move |plugin| is_active_plugin(plugin, state))
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

fn resolve_provider_for_command<'a>(
    command: &str,
    plugins: &[&'a DiscoveredPlugin],
    state: &PluginState,
    provider_override: Option<&str>,
) -> std::result::Result<ProviderResolution<'a>, ProviderResolutionError<'a>> {
    let providers = providers_for_command(command, plugins);
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

    if let Some(preferred) = state.preferred_providers.get(command) {
        if let Some(plugin) = providers
            .iter()
            .copied()
            .find(|plugin| plugin.plugin_id == *preferred)
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

fn provider_labels_by_command(plugins: &[&DiscoveredPlugin]) -> HashMap<String, Vec<String>> {
    let mut index = HashMap::new();
    for plugin in plugins {
        let label = plugin_label(plugin);
        for command in &plugin.commands {
            index
                .entry(command.clone())
                .or_insert_with(Vec::new)
                .push(label.clone());
        }
    }
    index
}

pub(super) fn is_enabled(state: &PluginState, plugin_id: &str, default_enabled: bool) -> bool {
    if state.enabled.iter().any(|id| id == plugin_id) {
        return true;
    }
    if state.disabled.iter().any(|id| id == plugin_id) {
        return false;
    }
    default_enabled
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
