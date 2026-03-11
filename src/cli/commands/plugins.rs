use crate::app::{AppClients, AuthState, ConfigState, RuntimeContext};
use crate::app::{
    CURRENT_TERMINAL_SENTINEL, CliCommandResult, PluginConfigScope, plugin_config_entries,
};
use crate::cli::rows::output::rows_to_output_result;
use crate::cli::{PluginConfigArgs, PluginsArgs, PluginsCommands};
use crate::config::{
    ConfigValue, ResolvedConfig, RuntimeConfigPaths, Scope, set_scoped_value_in_toml,
    unset_scoped_value_in_toml,
};
use crate::core::row::Row;
use crate::plugin::{
    CommandCatalogEntry, DoctorReport, PluginManager, PluginSummary, state::PluginCommandState,
};
use miette::Result;

#[derive(Clone, Copy)]
pub(crate) struct PluginsCommandContext<'a> {
    pub(crate) context: &'a RuntimeContext,
    pub(crate) config: &'a ResolvedConfig,
    pub(crate) config_state: Option<&'a ConfigState>,
    pub(crate) auth: &'a AuthState,
    pub(crate) clients: Option<&'a AppClients>,
    pub(crate) plugin_manager: &'a PluginManager,
    pub(crate) runtime_load: crate::config::RuntimeLoadOptions,
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
            let mut commands = context
                .plugin_manager
                .command_catalog()
                .map_err(|err| miette::miette!("{err:#}"))?
                .into_iter()
                .filter(|entry| context.auth.is_external_command_visible(&entry.name))
                .collect::<Vec<_>>();
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
            let mut result = CliCommandResult::exit(0);
            result.messages.success("refreshed plugin discovery cache");
            Ok(result)
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
        PluginsCommands::Enable(args) => {
            let command = normalize_command_name(&args.command)?;
            plugin_manager
                .validate_command(&command)
                .map_err(|err| miette::miette!("{err:#}"))?;
            persist_command_state(
                context,
                command.as_str(),
                PluginCommandState::Enabled,
                plugin_scope(
                    context,
                    args.global,
                    args.profile.as_deref(),
                    args.terminal.as_deref(),
                ),
            )?;
            sync_current_command_preferences(context)?;
            let mut result = CliCommandResult::exit(0);
            result
                .messages
                .success(format!("enabled command: {command}"));
            Ok(result)
        }
        PluginsCommands::Disable(args) => {
            let command = normalize_command_name(&args.command)?;
            plugin_manager
                .validate_command(&command)
                .map_err(|err| miette::miette!("{err:#}"))?;
            persist_command_state(
                context,
                command.as_str(),
                PluginCommandState::Disabled,
                plugin_scope(
                    context,
                    args.global,
                    args.profile.as_deref(),
                    args.terminal.as_deref(),
                ),
            )?;
            sync_current_command_preferences(context)?;
            let mut result = CliCommandResult::exit(0);
            result
                .messages
                .success(format!("disabled command: {command}"));
            Ok(result)
        }
        PluginsCommands::ClearState(args) => {
            let command = normalize_command_name(&args.command)?;
            plugin_manager
                .validate_command(&command)
                .map_err(|err| miette::miette!("{err:#}"))?;
            let removed = clear_command_state(
                context,
                command.as_str(),
                plugin_scope(
                    context,
                    args.global,
                    args.profile.as_deref(),
                    args.terminal.as_deref(),
                ),
            )?;
            sync_current_command_preferences(context)?;
            let mut result = CliCommandResult::exit(0);
            if removed {
                result
                    .messages
                    .success(format!("cleared command state for {command}"));
            } else {
                result
                    .messages
                    .warning(format!("no explicit command state set for {command}"));
            }
            Ok(result)
        }
        PluginsCommands::SelectProvider(args) => {
            let command = normalize_command_name(&args.command)?;
            plugin_manager
                .validate_preferred_provider(&command, &args.plugin_id)
                .map_err(|err| miette::miette!("{err:#}"))?;
            persist_provider_selection(
                context,
                &command,
                &args.plugin_id,
                plugin_scope(
                    context,
                    args.global,
                    args.profile.as_deref(),
                    args.terminal.as_deref(),
                ),
            )?;
            sync_current_command_preferences(context)?;
            let mut result = CliCommandResult::exit(0);
            result.messages.success(format!(
                "selected provider for command `{command}`: {}",
                args.plugin_id
            ));
            Ok(result)
        }
        PluginsCommands::ClearProvider(args) => {
            let command = normalize_command_name(&args.command)?;
            let removed = clear_provider_selection(
                context,
                &command,
                plugin_scope(
                    context,
                    args.global,
                    args.profile.as_deref(),
                    args.terminal.as_deref(),
                ),
            )?;
            sync_current_command_preferences(context)?;
            let mut result = CliCommandResult::exit(0);
            if removed {
                result.messages.success(format!(
                    "cleared provider selection for command `{command}`"
                ));
            } else {
                result
                    .messages
                    .warning(format!("no provider selection set for command `{command}`"));
            }
            Ok(result)
        }
    }
}

fn persist_command_state(
    context: PluginsCommandContext<'_>,
    command: &str,
    state: PluginCommandState,
    scope: Scope,
) -> Result<()> {
    let config_path = plugin_config_path(context)?;
    let value = ConfigValue::String(state.as_str().to_string());
    set_scoped_value_in_toml(
        &config_path,
        &plugin_state_key(command),
        &value,
        &scope,
        false,
        false,
    )
    .map_err(|err| miette::miette!("{err}"))?;
    Ok(())
}

fn clear_command_state(
    context: PluginsCommandContext<'_>,
    command: &str,
    scope: Scope,
) -> Result<bool> {
    let config_path = plugin_config_path(context)?;
    let edit_result = unset_scoped_value_in_toml(
        &config_path,
        &plugin_state_key(command),
        &scope,
        false,
        false,
    )
    .map_err(|err| miette::miette!("{err}"))?;
    Ok(edit_result.previous.is_some())
}

fn persist_provider_selection(
    context: PluginsCommandContext<'_>,
    command: &str,
    plugin_id: &str,
    scope: Scope,
) -> Result<()> {
    let config_path = plugin_config_path(context)?;
    set_scoped_value_in_toml(
        &config_path,
        &plugin_provider_key(command),
        &ConfigValue::String(plugin_id.trim().to_string()),
        &scope,
        false,
        false,
    )
    .map_err(|err| miette::miette!("{err}"))?;
    Ok(())
}

fn clear_provider_selection(
    context: PluginsCommandContext<'_>,
    command: &str,
    scope: Scope,
) -> Result<bool> {
    let config_path = plugin_config_path(context)?;
    let edit_result = unset_scoped_value_in_toml(
        &config_path,
        &plugin_provider_key(command),
        &scope,
        false,
        false,
    )
    .map_err(|err| miette::miette!("{err}"))?;
    Ok(edit_result.previous.is_some())
}

fn plugin_config_path(context: PluginsCommandContext<'_>) -> Result<std::path::PathBuf> {
    if !context.runtime_load.include_config_file {
        return Err(miette::miette!(
            "config file writes are disabled for this session"
        ));
    }
    RuntimeConfigPaths::discover()
        .config_file
        .ok_or_else(|| miette::miette!("failed to resolve config.toml path for plugin config"))
}

fn plugin_scope(
    context: PluginsCommandContext<'_>,
    global: bool,
    profile: Option<&str>,
    terminal: Option<&str>,
) -> Scope {
    let terminal = resolve_terminal_selector(context, terminal);
    if global {
        return terminal
            .as_deref()
            .map_or_else(Scope::global, Scope::terminal);
    }

    let profile = profile
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| context.config.active_profile());

    terminal.as_deref().map_or_else(
        || Scope::profile(profile),
        |current| Scope::profile_terminal(profile, current),
    )
}

fn resolve_terminal_selector(
    context: PluginsCommandContext<'_>,
    selector: Option<&str>,
) -> Option<String> {
    let value = selector?;
    if value == CURRENT_TERMINAL_SENTINEL {
        return Some(
            context
                .context
                .terminal_kind()
                .as_config_terminal()
                .to_string(),
        );
    }
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_ascii_lowercase())
    }
}

fn normalize_command_name(command: &str) -> Result<String> {
    let normalized = command.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return Err(miette::miette!("command must not be empty"));
    }
    Ok(normalized)
}

fn sync_current_command_preferences(context: PluginsCommandContext<'_>) -> Result<()> {
    let resolved = crate::app::resolve_runtime_config(
        crate::app::RuntimeConfigRequest::new(
            context.context.profile_override().map(ToOwned::to_owned),
            Some(context.context.terminal_kind().as_config_terminal()),
        )
        .with_runtime_load(context.runtime_load),
    )
    .map_err(|err| miette::miette!("{err:#}"))?;
    context.plugin_manager.replace_command_preferences(
        crate::plugin::state::PluginCommandPreferences::from_resolved(&resolved),
    );
    Ok(())
}

fn plugin_state_key(command: &str) -> String {
    format!("plugins.{}.state", command.trim().to_ascii_lowercase())
}

fn plugin_provider_key(command: &str) -> String {
    format!("plugins.{}.provider", command.trim().to_ascii_lowercase())
}

fn projected_plugin_config_entries(
    context: PluginsCommandContext<'_>,
    plugin_id: &str,
) -> Vec<crate::app::PluginConfigEntry> {
    if let (Some(config_state), Some(clients)) = (context.config_state, context.clients) {
        return clients.plugin_config_entries(config_state, plugin_id);
    }
    plugin_config_entries(context.config, plugin_id)
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
            let auth_hint = command
                .auth_hint()
                .map_or(serde_json::Value::Null, Into::into);
            let auth_visibility = command
                .auth
                .as_ref()
                .and_then(|auth| auth.visibility.map(|visibility| visibility.as_label()))
                .map_or(serde_json::Value::Null, Into::into);
            let required_capabilities = serde_json::Value::Array(
                command
                    .auth
                    .as_ref()
                    .map(|auth| {
                        auth.required_capabilities
                            .iter()
                            .map(|value| value.clone().into())
                            .collect()
                    })
                    .unwrap_or_default(),
            );
            let feature_flags = serde_json::Value::Array(
                command
                    .auth
                    .as_ref()
                    .map(|auth| {
                        auth.feature_flags
                            .iter()
                            .map(|value| value.clone().into())
                            .collect()
                    })
                    .unwrap_or_default(),
            );
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
                "provider" => command
                    .provider
                    .clone()
                    .map_or(serde_json::Value::Null, Into::into),
                "providers" => serde_json::Value::Array(
                    command
                        .providers
                        .iter()
                        .map(|value| value.clone().into())
                        .collect(),
                ),
                "conflicted" => command.conflicted,
                "requires_selection" => command.requires_selection,
                "selected_explicitly" => command.selected_explicitly,
                "source" => command
                    .source
                    .map(|value| value.to_string())
                    .map_or(serde_json::Value::Null, Into::into),
                "auth_hint" => auth_hint,
                "auth_visibility" => auth_visibility,
                "required_capabilities" => required_capabilities,
                "feature_flags" => feature_flags,
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

#[cfg(test)]
mod tests {
    use super::{command_catalog_rows, doctor_rows, plugin_config_rows, plugin_list_rows};
    use crate::app::PluginConfigEntry;
    use crate::core::plugin::{DescribeCommandAuthV1, DescribeVisibilityModeV1};
    use crate::core::row::Row;
    use crate::plugin::{
        CommandCatalogEntry, CommandConflict, DoctorReport, PluginSource, PluginSummary,
    };
    use std::path::PathBuf;

    fn row_str<'a>(row: &'a Row, key: &str) -> Option<&'a str> {
        row.get(key).and_then(serde_json::Value::as_str)
    }

    #[test]
    fn plugin_rows_render_empty_states_unit() {
        let list = plugin_list_rows(&[]);
        assert_eq!(row_str(&list[0], "status"), Some("empty"));
        assert_eq!(row_str(&list[0], "message"), Some("No plugins discovered."));

        let commands = command_catalog_rows(&[]);
        assert_eq!(row_str(&commands[0], "status"), Some("empty"));
        assert_eq!(
            row_str(&commands[0], "message"),
            Some("No plugin commands discovered.")
        );

        let config = plugin_config_rows("demo", &[]);
        assert_eq!(row_str(&config[0], "status"), Some("empty"));
        assert_eq!(row_str(&config[0], "plugin_id"), Some("demo"));
    }

    #[test]
    fn plugin_rows_render_real_metadata_and_scopes_unit() {
        let plugins = plugin_list_rows(&[PluginSummary {
            plugin_id: "demo".to_string(),
            enabled: true,
            healthy: false,
            source: PluginSource::Explicit,
            plugin_version: Some("1.2.3".to_string()),
            executable: PathBuf::from("/tmp/osp-demo"),
            commands: vec!["ldap".to_string(), "mreg".to_string()],
            issue: Some("broken".to_string()),
        }]);
        assert_eq!(row_str(&plugins[0], "plugin_id"), Some("demo"));
        assert_eq!(plugins[0].get("enabled"), Some(&serde_json::json!(true)));
        assert_eq!(plugins[0].get("healthy"), Some(&serde_json::json!(false)));
        assert_eq!(row_str(&plugins[0], "source"), Some("explicit"));
        assert_eq!(row_str(&plugins[0], "plugin_version"), Some("1.2.3"));

        let commands = command_catalog_rows(&[CommandCatalogEntry {
            name: "shared".to_string(),
            about: "shared command".to_string(),
            auth: Some(DescribeCommandAuthV1 {
                visibility: Some(DescribeVisibilityModeV1::CapabilityGated),
                required_capabilities: vec!["shared.run".to_string()],
                feature_flags: vec!["shared".to_string()],
            }),
            completion: crate::completion::CommandSpec::new("shared"),
            provider: Some("beta".to_string()),
            providers: vec!["alpha".to_string(), "beta".to_string()],
            conflicted: true,
            requires_selection: false,
            selected_explicitly: true,
            source: Some(PluginSource::Explicit),
            subcommands: vec!["show".to_string()],
        }]);
        assert_eq!(row_str(&commands[0], "name"), Some("shared"));
        assert_eq!(row_str(&commands[0], "provider"), Some("beta"));
        assert_eq!(
            commands[0].get("conflicted"),
            Some(&serde_json::json!(true))
        );
        assert_eq!(
            commands[0].get("providers"),
            Some(&serde_json::json!(["alpha", "beta"]))
        );
        assert_eq!(
            row_str(&commands[0], "auth_visibility"),
            Some("capability_gated")
        );
        assert_eq!(
            row_str(&commands[0], "auth_hint"),
            Some("cap: shared.run; feature: shared")
        );
        assert_eq!(
            commands[0].get("required_capabilities"),
            Some(&serde_json::json!(["shared.run"]))
        );

        let config = plugin_config_rows(
            "demo",
            &[
                PluginConfigEntry {
                    env_key: "OSP_SHARED_TOKEN".to_string(),
                    value: "1".to_string(),
                    config_key: "extensions.demo.token".to_string(),
                    scope: crate::app::PluginConfigScope::Shared,
                },
                PluginConfigEntry {
                    env_key: "OSP_PLUGIN_FLAG".to_string(),
                    value: "2".to_string(),
                    config_key: "extensions.plugins.demo.flag".to_string(),
                    scope: crate::app::PluginConfigScope::Plugin,
                },
            ],
        );
        assert_eq!(row_str(&config[0], "scope"), Some("shared"));
        assert_eq!(row_str(&config[1], "scope"), Some("plugin"));
    }

    #[test]
    fn doctor_rows_include_summary_and_conflicts_unit() {
        let rows = doctor_rows(&DoctorReport {
            plugins: vec![PluginSummary {
                plugin_id: "demo".to_string(),
                enabled: true,
                healthy: false,
                source: PluginSource::Explicit,
                plugin_version: None,
                executable: PathBuf::from("/tmp/osp-demo"),
                commands: vec!["shared".to_string()],
                issue: Some("broken".to_string()),
            }],
            conflicts: vec![CommandConflict {
                command: "shared".to_string(),
                providers: vec!["alpha".to_string(), "beta".to_string()],
            }],
        });

        assert_eq!(row_str(&rows[0], "kind"), Some("summary"));
        assert_eq!(rows[0].get("broken_enabled"), Some(&serde_json::json!(1)));
        assert_eq!(row_str(&rows[1], "kind"), Some("conflict"));
        assert_eq!(row_str(&rows[1], "command"), Some("shared"));
    }
}
