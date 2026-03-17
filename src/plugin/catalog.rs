//! Internal catalog/report builders derived from an [`ActivePluginView`].
//!
//! `PluginManager` keeps discovery and preference snapshots, but higher-level
//! read surfaces such as help text, command catalogs, policy registries, and
//! doctor reports should all see the same already-filtered active provider
//! picture. This module is the pure lowering step from that shared view into
//! host-facing summaries.

use super::active::{ActivePluginView, ResolvedActiveCommand};
use super::conversion::{collect_completion_words, direct_subcommand_names};
use super::manager::{CommandCatalogEntry, CommandConflict, DoctorReport, PluginSummary};
use super::selection::{ProviderResolution, ProviderSelectionMode, plugin_enabled, plugin_label};
use crate::completion::CommandSpec;
use crate::core::command_policy::{CommandPath, CommandPolicyRegistry};
use crate::core::plugin::DescribeCommandV1;

pub(crate) fn list_plugins(view: &ActivePluginView<'_>) -> Vec<PluginSummary> {
    view.discovered()
        .iter()
        .map(|plugin| PluginSummary {
            enabled: plugin_enabled(plugin, view.preferences()),
            healthy: plugin.issue.is_none(),
            issue: plugin.issue.clone(),
            plugin_id: plugin.plugin_id.clone(),
            plugin_version: plugin.plugin_version.clone(),
            executable: plugin.executable.clone(),
            source: plugin.source,
            commands: plugin
                .canonical_command_names()
                .into_iter()
                .map(str::to_string)
                .collect(),
        })
        .collect()
}

pub(crate) fn build_command_catalog(view: &ActivePluginView<'_>) -> Vec<CommandCatalogEntry> {
    let mut out = Vec::new();

    for resolved in view.resolved_active_commands() {
        match resolved.resolution() {
            ProviderResolution::Selected(selection) => {
                let Some(command) = selection.plugin.canonical_command(resolved.command()) else {
                    tracing::debug!(
                        command = %resolved.command(),
                        provider = %selection.plugin.plugin_id,
                        "skipping catalog entry for inconsistent canonical plugin command"
                    );
                    continue;
                };
                let completion = command.completion();
                out.push(CommandCatalogEntry {
                    name: command.name().to_string(),
                    about: completion.tooltip.clone().unwrap_or_default(),
                    auth: command.auth(),
                    subcommands: direct_subcommand_names(&completion),
                    completion,
                    provider: Some(selection.plugin.plugin_id.clone()),
                    providers: resolved.providers().to_vec(),
                    conflicted: resolved.providers().len() > 1,
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
                    "provider selection required; use --plugin-provider <plugin-id> or `osp plugins select-provider {} <plugin-id>`",
                    resolved.command()
                );
                out.push(CommandCatalogEntry {
                    name: resolved.command().to_string(),
                    about,
                    auth: None,
                    subcommands: Vec::new(),
                    completion: CommandSpec::new(resolved.command()),
                    provider: None,
                    providers: resolved.providers().to_vec(),
                    conflicted: true,
                    requires_selection: true,
                    selected_explicitly: false,
                    source: None,
                });
            }
        }
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub(crate) fn build_command_policy_registry(view: &ActivePluginView<'_>) -> CommandPolicyRegistry {
    let mut registry = CommandPolicyRegistry::new();

    for resolved in view.resolved_active_commands() {
        let ProviderResolution::Selected(selection) = resolved.resolution() else {
            continue;
        };

        if let Some(command) = selection
            .plugin
            .canonical_command(resolved.command())
            .and_then(|command| command.describe())
        {
            register_describe_command_policies(&mut registry, command, &[]);
        }
    }

    registry
}

pub(crate) fn completion_words_from_catalog(catalog: &[CommandCatalogEntry]) -> Vec<String> {
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
        words.push(command.name.clone());
        words.extend(collect_completion_words(&command.completion));
    }

    words.sort();
    words.dedup();
    words
}

pub(crate) fn render_repl_help(catalog: &[CommandCatalogEntry]) -> String {
    let mut out = String::from("Backbone commands: help, exit, quit\n");
    if catalog.is_empty() {
        out.push_str("No plugin commands available.\n");
        return out;
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
                provider = command.provider.as_deref().unwrap_or("-"),
                source = command
                    .source
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "-".to_string()),
            ));
            if !conflict.is_empty() {
                let trim_len = out.trim_end_matches('\n').len();
                out.truncate(trim_len);
                out.push_str(&conflict);
                out.push('\n');
            }
        }
    }

    out
}

pub(crate) fn command_provider_labels(command: &str, view: &ActivePluginView<'_>) -> Vec<String> {
    view.provider_labels(command)
}

pub(crate) fn selected_provider_label(
    command: &str,
    view: &ActivePluginView<'_>,
) -> Option<String> {
    resolved_command(view, command).and_then(|resolved| match resolved.resolution() {
        ProviderResolution::Selected(selection) => Some(plugin_label(selection.plugin)),
        ProviderResolution::Ambiguous(_) => None,
    })
}

fn resolved_command<'a>(
    view: &'a ActivePluginView<'a>,
    command: &str,
) -> Option<ResolvedActiveCommand<'a>> {
    view.resolved_active_commands()
        .into_iter()
        .find(|resolved| resolved.command() == command)
}

pub(crate) fn build_doctor_report(view: &ActivePluginView<'_>) -> DoctorReport {
    let plugins = list_plugins(view);
    let mut conflicts = view
        .provider_labels_by_command()
        .iter()
        .filter_map(|(command, providers)| {
            if providers.len() > 1 {
                Some(CommandConflict {
                    command: command.clone(),
                    providers: providers.clone(),
                })
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    conflicts.sort_by(|a, b| a.command.cmp(&b.command));

    DoctorReport { plugins, conflicts }
}

fn register_describe_command_policies(
    registry: &mut CommandPolicyRegistry,
    command: &DescribeCommandV1,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::completion::CommandSpec;
    use crate::core::plugin::{DescribeCommandAuthV1, DescribeCommandV1, DescribeVisibilityModeV1};
    use crate::plugin::active::ActivePluginView;
    use crate::plugin::state::PluginCommandPreferences;
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
    fn command_catalog_marks_ambiguous_commands_without_touching_dispatch_unit() {
        let alpha = plugin("alpha", "shared");
        let beta = plugin("beta", "shared");
        let preferences = PluginCommandPreferences::default();
        let discovered = [alpha, beta];
        let view = ActivePluginView::new(&discovered, &preferences);
        let catalog = build_command_catalog(&view);
        let entry = catalog
            .iter()
            .find(|entry| entry.name == "shared")
            .expect("shared command should exist");

        assert!(entry.requires_selection);
        assert!(entry.conflicted);
        assert_eq!(entry.provider, None);
        assert_eq!(entry.providers.len(), 2);
    }

    #[test]
    fn completion_words_and_help_render_from_catalog_surface_unit() {
        let alpha = plugin("alpha", "ldap");
        let preferences = PluginCommandPreferences::default();
        let discovered = [alpha];
        let view = ActivePluginView::new(&discovered, &preferences);
        let catalog = build_command_catalog(&view);

        let words = completion_words_from_catalog(&catalog);
        assert!(words.iter().any(|word| word == "ldap"));

        let help = render_repl_help(&catalog);
        assert!(help.contains("Plugin commands:"));
        assert!(help.contains("ldap"));
        assert!(help.contains("alpha/explicit"));
    }

    #[test]
    fn catalog_uses_canonical_command_metadata_when_raw_names_drift_unit() {
        let plugin = DiscoveredPlugin {
            plugin_id: "alpha".to_string(),
            plugin_version: Some("0.1.0".to_string()),
            executable: PathBuf::from("/tmp/osp-alpha"),
            source: PluginSource::Explicit,
            commands: Vec::new(),
            describe_commands: vec![DescribeCommandV1 {
                name: "ldap".to_string(),
                about: "lookup users".to_string(),
                args: Vec::new(),
                flags: Default::default(),
                subcommands: Vec::new(),
                auth: Some(DescribeCommandAuthV1 {
                    visibility: Some(DescribeVisibilityModeV1::Authenticated),
                    required_capabilities: Vec::new(),
                    feature_flags: Vec::new(),
                }),
            }],
            command_specs: Vec::new(),
            issue: None,
            default_enabled: true,
        };
        let preferences = PluginCommandPreferences::default();
        let discovered = [plugin];
        let view = ActivePluginView::new(&discovered, &preferences);
        let catalog = build_command_catalog(&view);
        let entry = catalog
            .iter()
            .find(|entry| entry.name == "ldap")
            .expect("canonical command should be cataloged");

        assert_eq!(entry.about, "lookup users");
        assert_eq!(entry.auth_hint().as_deref(), Some("auth"));
        assert_eq!(entry.provider.as_deref(), Some("alpha"));
    }
}
