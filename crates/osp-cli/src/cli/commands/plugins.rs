use miette::Result;

use crate::app::{
    CliCommandResult, PluginConfigScope, ReplCommandOutput, authorized_command_catalog,
    effective_plugin_config_entries, emit_messages_with_verbosity,
};
use crate::cli::{PluginConfigArgs, PluginToggleArgs, PluginsArgs, PluginsCommands};
use crate::plugin_manager::{CommandCatalogEntry, DoctorReport, PluginSummary};
use crate::rows::output::rows_to_output_result;
use crate::state::AppState;
use osp_core::row::Row;

pub(crate) fn run_plugins_command(state: &AppState, args: PluginsArgs) -> Result<CliCommandResult> {
    let plugin_manager = &state.clients.plugins;
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
            let mut commands = authorized_command_catalog(state)?;
            commands.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(CliCommandResult::output(
                rows_to_output_result(command_catalog_rows(&commands)),
                None,
            ))
        }
        PluginsCommands::Config(PluginConfigArgs { plugin_id }) => Ok(CliCommandResult::output(
            rows_to_output_result(plugin_config_rows(
                &plugin_id,
                &effective_plugin_config_entries(state.config.resolved(), &plugin_id),
            )),
            None,
        )),
        PluginsCommands::Refresh => {
            plugin_manager.refresh();
            let mut messages = osp_ui::messages::MessageBuffer::default();
            messages.success("refreshed plugin discovery cache");
            emit_messages_with_verbosity(state, &messages, state.ui.message_verbosity);
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
            emit_messages_with_verbosity(state, &messages, state.ui.message_verbosity);
            Ok(CliCommandResult::exit(0))
        }
        PluginsCommands::Disable(PluginToggleArgs { plugin_id }) => {
            plugin_manager
                .set_enabled(&plugin_id, false)
                .map_err(|err| miette::miette!("{err:#}"))?;
            let mut messages = osp_ui::messages::MessageBuffer::default();
            messages.success(format!("disabled plugin: {plugin_id}"));
            emit_messages_with_verbosity(state, &messages, state.ui.message_verbosity);
            Ok(CliCommandResult::exit(0))
        }
    }
}

pub(crate) fn run_plugins_repl_command(
    state: &AppState,
    args: PluginsArgs,
    verbosity: osp_ui::messages::MessageLevel,
) -> Result<ReplCommandOutput> {
    let plugin_manager = &state.clients.plugins;
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
            let mut commands = authorized_command_catalog(state)?;
            commands.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(ReplCommandOutput::Output {
                output: rows_to_output_result(command_catalog_rows(&commands)),
                format_hint: None,
            })
        }
        PluginsCommands::Config(PluginConfigArgs { plugin_id }) => Ok(ReplCommandOutput::Output {
            output: rows_to_output_result(plugin_config_rows(
                &plugin_id,
                &effective_plugin_config_entries(state.config.resolved(), &plugin_id),
            )),
            format_hint: None,
        }),
        PluginsCommands::Refresh => {
            plugin_manager.refresh();
            let mut messages = osp_ui::messages::MessageBuffer::default();
            messages.success("refreshed plugin discovery cache");
            emit_messages_with_verbosity(state, &messages, verbosity);
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
            emit_messages_with_verbosity(state, &messages, verbosity);
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
            emit_messages_with_verbosity(state, &messages, verbosity);
            Ok(ReplCommandOutput::Text(format!(
                "disabled plugin: {plugin_id}\n"
            )))
        }
    }
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
