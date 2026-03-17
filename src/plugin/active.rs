//! Internal active-plugin working set for `PluginManager` read paths.
//!
//! `PluginManager` exposes several surfaces that all need the same derived
//! context: discovered plugins, command preferences, the healthy provider set,
//! and the labels for commands that remain active after preference filtering.
//! This view computes that working set once per manager operation so catalog,
//! help, doctor, and provider-selection code stop rebuilding it ad hoc.

use super::manager::DiscoveredPlugin;
use super::selection::{
    ProviderResolution, ProviderResolutionError, available_providers_for_command, healthy_plugins,
    provider_labels_by_command, providers_for_command, resolve_provider_for_command,
};
use super::state::PluginCommandPreferences;
use std::collections::{BTreeSet, HashMap};

/// Borrowed view over the current active plugin working set.
pub(crate) struct ActivePluginView<'a> {
    discovered: &'a [DiscoveredPlugin],
    preferences: &'a PluginCommandPreferences,
    healthy: Vec<&'a DiscoveredPlugin>,
    available_provider_labels: HashMap<String, Vec<String>>,
    active_command_names: BTreeSet<String>,
}

pub(crate) struct ResolvedActiveCommand<'a> {
    command: String,
    providers: Vec<String>,
    resolution: ProviderResolution<'a>,
}

impl<'a> ResolvedActiveCommand<'a> {
    pub(crate) fn command(&self) -> &str {
        self.command.as_str()
    }

    pub(crate) fn providers(&self) -> &[String] {
        &self.providers
    }

    pub(crate) fn resolution(&self) -> &ProviderResolution<'a> {
        &self.resolution
    }
}

impl<'a> ActivePluginView<'a> {
    /// Builds the shared plugin working set from one discovery snapshot plus
    /// one command-preference snapshot.
    pub(crate) fn new(
        discovered: &'a [DiscoveredPlugin],
        preferences: &'a PluginCommandPreferences,
    ) -> Self {
        let healthy = healthy_plugins(discovered).collect::<Vec<_>>();
        let available_provider_labels = provider_labels_by_command(&healthy, preferences);
        let active_command_names = available_provider_labels.keys().cloned().collect();

        Self {
            discovered,
            preferences,
            healthy,
            available_provider_labels,
            active_command_names,
        }
    }

    pub(crate) fn discovered(&self) -> &'a [DiscoveredPlugin] {
        self.discovered
    }

    pub(crate) fn preferences(&self) -> &PluginCommandPreferences {
        self.preferences
    }

    pub(crate) fn healthy_plugins(&self) -> &[&'a DiscoveredPlugin] {
        &self.healthy
    }

    pub(crate) fn active_command_names(&self) -> impl Iterator<Item = &str> + '_ {
        self.active_command_names.iter().map(String::as_str)
    }

    pub(crate) fn provider_labels(&self, command: &str) -> Vec<String> {
        self.available_provider_labels
            .get(command)
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn provider_labels_by_command(&self) -> &HashMap<String, Vec<String>> {
        &self.available_provider_labels
    }

    pub(crate) fn healthy_providers(&self, command: &str) -> Vec<&'a DiscoveredPlugin> {
        providers_for_command(command, &self.healthy)
    }

    pub(crate) fn available_providers(&self, command: &str) -> Vec<&'a DiscoveredPlugin> {
        available_providers_for_command(command, &self.healthy, self.preferences)
    }

    pub(crate) fn resolve_provider(
        &self,
        command: &str,
        provider_override: Option<&str>,
    ) -> std::result::Result<ProviderResolution<'a>, ProviderResolutionError<'a>> {
        resolve_provider_for_command(command, &self.healthy, self.preferences, provider_override)
    }

    pub(crate) fn resolved_active_commands(&self) -> Vec<ResolvedActiveCommand<'a>> {
        let mut out = Vec::new();
        for command in self.active_command_names() {
            let providers = self.provider_labels(command);
            let resolution = match self.resolve_provider(command, None) {
                Ok(resolution) => resolution,
                Err(err) => {
                    tracing::debug!(
                        command = %command,
                        error = ?err,
                        "skipping inconsistent active command during resolved command build"
                    );
                    continue;
                }
            };
            out.push(ResolvedActiveCommand {
                command: command.to_string(),
                providers,
                resolution,
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::completion::CommandSpec;
    use crate::plugin::state::{PluginCommandPreferences, PluginCommandState};
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
    fn active_view_reuses_one_available_provider_index_unit() {
        let alpha = plugin("alpha", "shared");
        let beta = plugin("beta", "shared");
        let solo = plugin("solo", "solo");
        let discovered = [alpha, beta, solo];
        let mut preferences = PluginCommandPreferences::default();

        let ambiguous = ActivePluginView::new(&discovered, &preferences);
        assert_eq!(ambiguous.provider_labels("shared").len(), 2);
        assert!(
            ambiguous
                .active_command_names()
                .any(|command| command == "shared")
        );

        preferences.set_provider("shared", "beta");
        preferences.set_state("solo", PluginCommandState::Disabled);
        let preferred = ActivePluginView::new(&discovered, &preferences);
        assert_eq!(preferred.provider_labels("shared").len(), 2);
        assert!(
            preferred
                .active_command_names()
                .any(|command| command == "shared")
        );
        assert!(
            !preferred
                .active_command_names()
                .any(|command| command == "solo")
        );
    }
}
