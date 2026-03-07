use miette::Result;
use osp_config::ResolvedConfig;
use osp_ui::messages::MessageLevel;

use crate::app::{
    CliCommandResult, PluginConfigScope, ReplCommandOutput, authorized_command_catalog_for,
    effective_plugin_config_entries, emit_messages_for_ui,
};
use crate::cli::{
    PluginConfigArgs, PluginProviderClearArgs, PluginProviderSelectArgs, PluginToggleArgs,
    PluginsArgs, PluginsCommands,
};
use crate::plugin_manager::{CommandCatalogEntry, DoctorReport, PluginManager, PluginSummary};
use crate::rows::output::rows_to_output_result;
use crate::state::{AppClients, AuthState, ConfigState, UiState};
use osp_core::row::Row;

#[derive(Clone, Copy)]
pub(crate) struct PluginsCommandContext<'a> {
    pub(crate) config: &'a ResolvedConfig,
    pub(crate) config_state: Option<&'a ConfigState>,
    pub(crate) ui: &'a UiState,
    pub(crate) auth: &'a AuthState,
    pub(crate) clients: Option<&'a AppClients>,
    pub(crate) plugin_manager: &'a PluginManager,
}

pub(crate) fn run_plugins_command(
    context: PluginsCommandContext<'_>,
    args: PluginsArgs,
) -> Result<CliCommandResult> {
    let plugin_manager = context.plugin_manager;
    match args.command {
        PluginsCommands::List => {
            let mut plugins = plugin_manager
                .list_plugins()
                .map_err(|err| miette::miette!("{err:#}"))?;
            plugins.sort_by(|a, b| a.plugin_id.cmp(&b.plugin_id));
            Ok(CliCommandResult::output(
                rows_to_output_result(plugin_list_rows(&plugins)),
                None,
            ))
        }
        PluginsCommands::Commands => {
            let mut commands =
                authorized_command_catalog_for(context.auth, context.plugin_manager)?;
            commands.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(CliCommandResult::output(
                rows_to_output_result(command_catalog_rows(&commands)),
                None,
            ))
        }
        PluginsCommands::Config(PluginConfigArgs { plugin_id }) => Ok(CliCommandResult::output(
            rows_to_output_result(plugin_config_rows(
                &plugin_id,
                &projected_plugin_config_entries(context, &plugin_id),
            )),
            None,
        )),
        PluginsCommands::Refresh => {
            plugin_manager.refresh();
            let mut messages = osp_ui::messages::MessageBuffer::default();
            messages.success("refreshed plugin discovery cache");
            emit_messages(context, &messages, context.ui.message_verbosity);
            Ok(CliCommandResult::exit(0))
        }
        PluginsCommands::Doctor => {
            let report = plugin_manager
                .doctor()
                .map_err(|err| miette::miette!("{err:#}"))?;
            Ok(CliCommandResult::output(
                rows_to_output_result(doctor_rows(&report)),
                None,
            ))
        }
        PluginsCommands::Enable(PluginToggleArgs { plugin_id }) => {
            plugin_manager
                .set_enabled(&plugin_id, true)
                .map_err(|err| miette::miette!("{err:#}"))?;
            let mut messages = osp_ui::messages::MessageBuffer::default();
            messages.success(format!("enabled plugin: {plugin_id}"));
            emit_messages(context, &messages, context.ui.message_verbosity);
            Ok(CliCommandResult::exit(0))
        }
        PluginsCommands::Disable(PluginToggleArgs { plugin_id }) => {
            plugin_manager
                .set_enabled(&plugin_id, false)
                .map_err(|err| miette::miette!("{err:#}"))?;
            let mut messages = osp_ui::messages::MessageBuffer::default();
            messages.success(format!("disabled plugin: {plugin_id}"));
            emit_messages(context, &messages, context.ui.message_verbosity);
            Ok(CliCommandResult::exit(0))
        }
        PluginsCommands::SelectProvider(PluginProviderSelectArgs { command, plugin_id }) => {
            plugin_manager
                .set_preferred_provider(&command, &plugin_id)
                .map_err(|err| miette::miette!("{err:#}"))?;
            let mut messages = osp_ui::messages::MessageBuffer::default();
            messages.success(format!(
                "selected provider for command `{command}`: {plugin_id}"
            ));
            emit_messages(context, &messages, context.ui.message_verbosity);
            Ok(CliCommandResult::exit(0))
        }
        PluginsCommands::ClearProvider(PluginProviderClearArgs { command }) => {
            let removed = plugin_manager
                .clear_preferred_provider(&command)
                .map_err(|err| miette::miette!("{err:#}"))?;
            let mut messages = osp_ui::messages::MessageBuffer::default();
            if removed {
                messages.success(format!(
                    "cleared provider selection for command `{command}`"
                ));
            } else {
                messages.warning(format!("no provider selection set for command `{command}`"));
            }
            emit_messages(context, &messages, context.ui.message_verbosity);
            Ok(CliCommandResult::exit(0))
        }
    }
}

pub(crate) fn run_plugins_repl_command(
    context: PluginsCommandContext<'_>,
    args: PluginsArgs,
    verbosity: MessageLevel,
) -> Result<ReplCommandOutput> {
    let plugin_manager = context.plugin_manager;
    match args.command {
        PluginsCommands::List => {
            let mut plugins = plugin_manager
                .list_plugins()
                .map_err(|err| miette::miette!("{err:#}"))?;
            plugins.sort_by(|a, b| a.plugin_id.cmp(&b.plugin_id));
            Ok(ReplCommandOutput::Output {
                output: rows_to_output_result(plugin_list_rows(&plugins)),
                format_hint: None,
            })
        }
        PluginsCommands::Commands => {
            let mut commands =
                authorized_command_catalog_for(context.auth, context.plugin_manager)?;
            commands.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(ReplCommandOutput::Output {
                output: rows_to_output_result(command_catalog_rows(&commands)),
                format_hint: None,
            })
        }
        PluginsCommands::Config(PluginConfigArgs { plugin_id }) => Ok(ReplCommandOutput::Output {
            output: rows_to_output_result(plugin_config_rows(
                &plugin_id,
                &projected_plugin_config_entries(context, &plugin_id),
            )),
            format_hint: None,
        }),
        PluginsCommands::Refresh => {
            plugin_manager.refresh();
            let mut messages = osp_ui::messages::MessageBuffer::default();
            messages.success("refreshed plugin discovery cache");
            emit_messages(context, &messages, verbosity);
            Ok(ReplCommandOutput::Text(
                "refreshed plugin discovery cache\n".to_string(),
            ))
        }
        PluginsCommands::Doctor => {
            let report = plugin_manager
                .doctor()
                .map_err(|err| miette::miette!("{err:#}"))?;
            Ok(ReplCommandOutput::Output {
                output: rows_to_output_result(doctor_rows(&report)),
                format_hint: None,
            })
        }
        PluginsCommands::Enable(PluginToggleArgs { plugin_id }) => {
            plugin_manager
                .set_enabled(&plugin_id, true)
                .map_err(|err| miette::miette!("{err:#}"))?;
            let mut messages = osp_ui::messages::MessageBuffer::default();
            messages.success(format!("enabled plugin: {plugin_id}"));
            emit_messages(context, &messages, verbosity);
            Ok(ReplCommandOutput::Text(format!(
                "enabled plugin: {plugin_id}\n"
            )))
        }
        PluginsCommands::Disable(PluginToggleArgs { plugin_id }) => {
            plugin_manager
                .set_enabled(&plugin_id, false)
                .map_err(|err| miette::miette!("{err:#}"))?;
            let mut messages = osp_ui::messages::MessageBuffer::default();
            messages.success(format!("disabled plugin: {plugin_id}"));
            emit_messages(context, &messages, verbosity);
            Ok(ReplCommandOutput::Text(format!(
                "disabled plugin: {plugin_id}\n"
            )))
        }
        PluginsCommands::SelectProvider(PluginProviderSelectArgs { command, plugin_id }) => {
            plugin_manager
                .set_preferred_provider(&command, &plugin_id)
                .map_err(|err| miette::miette!("{err:#}"))?;
            let mut messages = osp_ui::messages::MessageBuffer::default();
            messages.success(format!(
                "selected provider for command `{command}`: {plugin_id}"
            ));
            emit_messages(context, &messages, verbosity);
            Ok(ReplCommandOutput::Text(format!(
                "selected provider for command `{command}`: {plugin_id}\n"
            )))
        }
        PluginsCommands::ClearProvider(PluginProviderClearArgs { command }) => {
            let removed = plugin_manager
                .clear_preferred_provider(&command)
                .map_err(|err| miette::miette!("{err:#}"))?;
            let mut messages = osp_ui::messages::MessageBuffer::default();
            let text = if removed {
                messages.success(format!(
                    "cleared provider selection for command `{command}`"
                ));
                format!("cleared provider selection for command `{command}`\n")
            } else {
                messages.warning(format!("no provider selection set for command `{command}`"));
                format!("no provider selection set for command `{command}`\n")
            };
            emit_messages(context, &messages, verbosity);
            Ok(ReplCommandOutput::Text(text))
        }
    }
}

fn emit_messages(
    context: PluginsCommandContext<'_>,
    messages: &osp_ui::messages::MessageBuffer,
    verbosity: MessageLevel,
) {
    emit_messages_for_ui(context.config, context.ui, messages, verbosity);
}

fn projected_plugin_config_entries(
    context: PluginsCommandContext<'_>,
    plugin_id: &str,
) -> Vec<crate::app::PluginConfigEntry> {
    if let (Some(config_state), Some(clients)) = (context.config_state, context.clients) {
        return clients.effective_plugin_config_entries(config_state, plugin_id);
    }
    effective_plugin_config_entries(context.config, plugin_id)
}

fn plugin_list_rows(plugins: &[PluginSummary]) -> Vec<Row> {
    if plugins.is_empty() {
        return vec![crate::row! {
            "status" => "empty",
            "message" => "No plugins discovered.",
        }];
    }

    plugins
        .iter()
        .map(|plugin| {
            let commands = serde_json::Value::Array(
                plugin
                    .commands
                    .iter()
                    .map(|command| command.clone().into())
                    .collect(),
            );
            let version = plugin
                .plugin_version
                .clone()
                .map_or(serde_json::Value::Null, Into::into);
            let issue = plugin
                .issue
                .clone()
                .map_or(serde_json::Value::Null, Into::into);
            crate::row! {
                "plugin_id" => plugin.plugin_id.clone(),
                "enabled" => plugin.enabled,
                "healthy" => plugin.healthy,
                "source" => plugin.source.to_string(),
                "plugin_version" => version,
                "path" => plugin.executable.display().to_string(),
                "commands" => commands,
                "issue" => issue,
            }
        })
        .collect()
}

fn command_catalog_rows(commands: &[CommandCatalogEntry]) -> Vec<Row> {
    if commands.is_empty() {
        return vec![crate::row! {
            "status" => "empty",
            "message" => "No plugin commands discovered.",
        }];
    }

    commands
        .iter()
        .map(|command| {
            let subcommands = serde_json::Value::Array(
                command
                    .subcommands
                    .iter()
                    .map(|value| value.clone().into())
                    .collect(),
            );
            crate::row! {
                "name" => command.name.clone(),
                "about" => command.about.clone(),
                "provider" => command.provider.clone(),
                "providers" => serde_json::Value::Array(
                    command
                        .providers
                        .iter()
                        .map(|value| value.clone().into())
                        .collect(),
                ),
                "conflicted" => command.conflicted,
                "selected_explicitly" => command.selected_explicitly,
                "source" => command.source.to_string(),
                "subcommands" => subcommands,
            }
        })
        .collect()
}

fn plugin_config_rows(plugin_id: &str, entries: &[crate::app::PluginConfigEntry]) -> Vec<Row> {
    if entries.is_empty() {
        return vec![crate::row! {
            "status" => "empty",
            "plugin_id" => plugin_id.to_string(),
            "message" => "No app-owned plugin config is projected for this plugin.",
        }];
    }

    entries
        .iter()
        .map(|entry| {
            let scope = match entry.scope {
                PluginConfigScope::Shared => "shared",
                PluginConfigScope::Plugin => "plugin",
            };
            crate::row! {
                "plugin_id" => plugin_id.to_string(),
                "env" => entry.env_key.clone(),
                "value" => entry.value.clone(),
                "config_key" => entry.config_key.clone(),
                "scope" => scope,
            }
        })
        .collect()
}

pub(crate) fn doctor_rows(report: &DoctorReport) -> Vec<Row> {
    let mut rows = Vec::new();

    let broken_enabled = report
        .plugins
        .iter()
        .filter(|plugin| plugin.enabled && !plugin.healthy)
        .count() as i64;
    rows.push(crate::row! {
        "kind" => "summary",
        "plugins" => report.plugins.len() as i64,
        "broken_enabled" => broken_enabled,
        "conflicts" => report.conflicts.len() as i64,
    });

    for conflict in &report.conflicts {
        let providers = serde_json::Value::Array(
            conflict
                .providers
                .iter()
                .map(|provider| provider.clone().into())
                .collect(),
        );
        rows.push(crate::row! {
            "kind" => "conflict",
            "command" => conflict.command.clone(),
            "providers" => providers,
        });
    }

    rows
}
