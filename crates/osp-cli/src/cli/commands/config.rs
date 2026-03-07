use crate::app::{
    CURRENT_TERMINAL_SENTINEL, CliCommandResult, ConfigExplainContext, ReplCommandOutput,
    RuntimeConfigRequest, config_explain_json, config_explain_output, config_value_to_json,
    explain_runtime_config, format_scope, is_sensitive_key, render_config_explain_text,
};
use crate::cli::{
    ConfigArgs, ConfigCommands, ConfigGetArgs, ConfigSetArgs, ConfigShowArgs, ConfigUnsetArgs,
};
use crate::rows::RowBuilder;
use crate::rows::output::rows_to_output_result;
use crate::state::{RuntimeContext, TerminalKind, UiState};
use crate::theme_loader::ThemeCatalog;
use miette::{IntoDiagnostic, Result, WrapErr, miette};
use osp_config::{
    ConfigLayer, ConfigSchema, ResolvedConfig, ResolvedValue, RuntimeConfigPaths,
    RuntimeLoadOptions, Scope, is_bootstrap_only_key, set_scoped_value_in_toml,
    unset_scoped_value_in_toml, validate_bootstrap_value, validate_key_scope,
};
use osp_core::output::OutputFormat;
use osp_core::row::Row;
use osp_ui::messages::MessageBuffer;

pub(crate) struct ConfigCommandContext<'a> {
    pub(crate) context: &'a RuntimeContext,
    pub(crate) config: &'a ResolvedConfig,
    pub(crate) ui: &'a UiState,
    pub(crate) themes: &'a ThemeCatalog,
    pub(crate) session_overrides: &'a mut ConfigLayer,
    pub(crate) runtime_load: RuntimeLoadOptions,
}

#[derive(Clone, Copy)]
pub(crate) struct ConfigReadContext<'a> {
    pub(crate) context: &'a RuntimeContext,
    pub(crate) config: &'a ResolvedConfig,
    pub(crate) ui: &'a UiState,
    pub(crate) themes: &'a ThemeCatalog,
    pub(crate) session_layer: &'a ConfigLayer,
    pub(crate) runtime_load: RuntimeLoadOptions,
}

impl<'a> ConfigCommandContext<'a> {
    fn read(&self) -> ConfigReadContext<'_> {
        ConfigReadContext {
            context: self.context,
            config: self.config,
            ui: self.ui,
            themes: self.themes,
            session_layer: &*self.session_overrides,
            runtime_load: self.runtime_load,
        }
    }
}

pub(crate) fn run_config_command(
    context: ConfigCommandContext<'_>,
    args: ConfigArgs,
) -> Result<CliCommandResult> {
    let read = context.read();
    match args.command {
        ConfigCommands::Show(show) => Ok(CliCommandResult::output(
            rows_to_output_result(config_show_rows(read, show)),
            None,
        )),
        ConfigCommands::Get(get) => run_config_get(read, get),
        ConfigCommands::Explain(explain) => match config_explain_output(
            &ConfigExplainContext {
                context: read.context,
                config: read.config,
                ui: read.ui,
                session_layer: read.session_layer,
                runtime_load: read.runtime_load,
            },
            explain,
        )? {
            Some(output) => Ok(CliCommandResult::text(output)),
            None => Ok(CliCommandResult::exit(1)),
        },
        ConfigCommands::Set(set) => run_config_set(context, set),
        ConfigCommands::Unset(unset) => run_config_unset(context, unset),
        ConfigCommands::Doctor => Ok(CliCommandResult::output(
            rows_to_output_result(config_diagnostics_rows(read)),
            None,
        )),
    }
}

fn config_show_rows(context: ConfigReadContext<'_>, args: ConfigShowArgs) -> Vec<Row> {
    let mut entries = context
        .config
        .values()
        .iter()
        .chain(context.config.aliases().iter())
        .collect::<Vec<(&String, &ResolvedValue)>>();
    entries.sort_by(|(left, _), (right, _)| left.cmp(right));
    entries
        .into_iter()
        .map(|(key, entry)| config_entry_row(key, entry, args.sources, args.raw))
        .collect()
}

fn run_config_get(context: ConfigReadContext<'_>, args: ConfigGetArgs) -> Result<CliCommandResult> {
    let mut messages = MessageBuffer::default();
    let rows = config_get_rows(context, &args, &mut messages)?;
    match rows {
        Some(rows) => Ok(CliCommandResult {
            exit_code: 0,
            messages,
            output: Some(ReplCommandOutput::Output {
                output: rows_to_output_result(rows),
                format_hint: None,
            }),
        }),
        None => Ok(CliCommandResult {
            exit_code: 1,
            messages,
            output: None,
        }),
    }
}

fn config_get_rows(
    context: ConfigReadContext<'_>,
    args: &ConfigGetArgs,
    messages: &mut MessageBuffer,
) -> Result<Option<Vec<Row>>> {
    if let Some(entry) = context.config.get_value_entry(&args.key) {
        let row = config_entry_row(&args.key, entry, args.sources, args.raw);
        return Ok(Some(vec![row]));
    }

    if let Some(entry) = context.config.get_alias_entry(&args.key) {
        let key = if args.key.starts_with("alias.") {
            args.key.clone()
        } else {
            format!("alias.{}", args.key.trim().to_ascii_lowercase())
        };
        let row = config_entry_row(&key, entry, args.sources, args.raw);
        return Ok(Some(vec![row]));
    }

    if is_bootstrap_only_key(&args.key) {
        let explain = explain_runtime_config(
            RuntimeConfigRequest::new(
                context.context.profile_override().map(str::to_owned),
                Some(context.context.terminal_kind().as_config_terminal()),
            )
            .with_runtime_load(context.runtime_load)
            .with_session_layer(Some(context.session_layer.clone())),
            &args.key,
        )?;

        if let Some(entry) = explain.final_entry {
            let row = config_entry_row(&args.key, &entry, args.sources, args.raw);
            return Ok(Some(vec![row]));
        }
    }

    messages.error(format!("config key not found: {}", args.key));
    Ok(None)
}

pub(crate) fn config_diagnostics_rows(context: ConfigReadContext<'_>) -> Vec<Row> {
    let known_profiles = serde_json::Value::Array(
        context
            .config
            .known_profiles()
            .iter()
            .map(|value| value.clone().into())
            .collect(),
    );
    let theme_issues = serde_json::Value::Array(
        context
            .themes
            .issues
            .iter()
            .map(|issue| {
                serde_json::json!({
                    "path": issue.path.to_string_lossy().to_string(),
                    "message": issue.message,
                })
            })
            .collect(),
    );
    vec![crate::row! {
        "status" => "ok",
        "active_profile" => context.config.active_profile().to_string(),
        "known_profiles" => known_profiles,
        "resolved_keys" => (context.config.values().len()
            + context.config.aliases().len()) as i64,
        "theme_issue_count" => context.themes.issues.len() as i64,
        "theme_issues" => theme_issues,
    }]
}

fn config_entry_row(
    key: &str,
    entry: &ResolvedValue,
    include_sources: bool,
    show_raw: bool,
) -> Row {
    let mut row = RowBuilder::new();
    row.insert("key", key.to_string());
    row.insert(
        "value",
        config_value_to_json(if show_raw {
            &entry.raw_value
        } else {
            &entry.value
        }),
    );

    if include_sources {
        row.insert("source", entry.source.to_string());
        row.insert(
            "origin",
            entry
                .origin
                .clone()
                .map_or(serde_json::Value::Null, Into::into),
        );
        row.insert(
            "scope_profile",
            entry
                .scope
                .profile
                .clone()
                .map_or(serde_json::Value::Null, |v| v.into()),
        );
        row.insert(
            "scope_terminal",
            entry
                .scope
                .terminal
                .clone()
                .map_or(serde_json::Value::Null, |v| v.into()),
        );
    }

    row.build()
}

fn run_config_set(
    context: ConfigCommandContext<'_>,
    args: ConfigSetArgs,
) -> Result<CliCommandResult> {
    let key = args.key.trim().to_ascii_lowercase();
    let schema = ConfigSchema::default();
    let value = schema
        .parse_input_value(&key, &args.value)
        .into_diagnostic()
        .wrap_err("invalid value for key")?;
    validate_bootstrap_value(&key, &value)
        .into_diagnostic()
        .wrap_err("invalid bootstrap value")?;
    let target = ConfigWriteTarget::from_set_args(&args);
    let read = context.read();
    let store = resolve_config_store(read, &target);
    let scopes = resolve_config_scopes(read, &target)?;
    validate_write_scopes(&key, &scopes).into_diagnostic()?;

    let mut rows = Vec::new();
    let mut messages = MessageBuffer::default();
    if matches!(store, ConfigStore::Config) && is_sensitive_key(&key) {
        messages.warning("writing a sensitive key to config store; prefer --secrets");
    }

    let paths = RuntimeConfigPaths::discover();
    for scope in &scopes {
        let display_value = if matches!(store, ConfigStore::Secrets) {
            value.clone().into_secret()
        } else {
            value.clone()
        };
        let mut row = RowBuilder::new();
        row.insert("key", key.clone());
        row.insert("value", config_value_to_json(&display_value));
        row.insert("scope", format_scope(scope));
        row.insert("store", config_store_name(store));
        row.insert("dry_run", args.dry_run);

        match store {
            ConfigStore::Session => {
                if !args.dry_run {
                    context
                        .session_overrides
                        .insert(key.clone(), value.clone(), scope.clone());
                }
                row.insert("path", serde_json::Value::Null);
                row.insert("changed", true);
            }
            ConfigStore::Config | ConfigStore::Secrets => {
                let target_path = match store {
                    ConfigStore::Config => paths.config_file.as_deref(),
                    ConfigStore::Secrets => paths.secrets_file.as_deref(),
                    ConfigStore::Session => None,
                }
                .ok_or_else(|| {
                    miette!(
                        "unable to resolve config path for {}",
                        config_store_name(store)
                    )
                })?;

                let set_result = set_scoped_value_in_toml(
                    target_path,
                    &key,
                    &value,
                    scope,
                    args.dry_run,
                    matches!(store, ConfigStore::Secrets),
                )
                .into_diagnostic()
                .wrap_err("failed to persist config change")?;

                row.insert("path", target_path.display().to_string());
                row.insert("changed", set_result.previous.as_ref() != Some(&value));
                row.insert(
                    "previous",
                    set_result
                        .previous
                        .as_ref()
                        .map(|previous| {
                            let previous = if matches!(store, ConfigStore::Secrets) {
                                previous.clone().into_secret()
                            } else {
                                previous.clone()
                            };
                            config_value_to_json(&previous)
                        })
                        .unwrap_or(serde_json::Value::Null),
                );
            }
        }

        rows.push(row.build());
    }

    let output = if args.explain {
        let explain = explain_runtime_config(
            RuntimeConfigRequest::new(
                context.context.profile_override().map(str::to_owned),
                Some(context.context.terminal_kind().as_config_terminal()),
            )
            .with_runtime_load(context.runtime_load)
            .with_session_layer(Some(context.session_overrides.clone())),
            &key,
        )?;
        if matches!(context.ui.render_settings.format, OutputFormat::Json) {
            let payload = config_explain_json(&explain, false);
            let rendered = serde_json::to_string_pretty(&payload).into_diagnostic()?;
            ReplCommandOutput::Text(format!("{rendered}\n"))
        } else {
            ReplCommandOutput::Text(render_config_explain_text(&explain, false))
        }
    } else {
        ReplCommandOutput::Output {
            output: rows_to_output_result(rows),
            format_hint: None,
        }
    };

    messages.success(format!(
        "{} value for {} at {} scope",
        if args.dry_run { "would set" } else { "set" },
        key,
        scopes
            .first()
            .map(format_scope)
            .unwrap_or_else(|| "global".to_string())
    ));
    Ok(CliCommandResult {
        exit_code: 0,
        messages,
        output: Some(output),
    })
}

fn run_config_unset(
    context: ConfigCommandContext<'_>,
    args: ConfigUnsetArgs,
) -> Result<CliCommandResult> {
    let key = args.key.trim().to_ascii_lowercase();
    let target = ConfigWriteTarget::from_unset_args(&args);
    let read = context.read();
    let store = resolve_config_store(read, &target);
    let scopes = resolve_config_scopes(read, &target)?;
    validate_write_scopes(&key, &scopes).into_diagnostic()?;

    let mut rows = Vec::new();
    let mut messages = MessageBuffer::default();
    let paths = RuntimeConfigPaths::discover();

    for scope in &scopes {
        let mut row = RowBuilder::new();
        row.insert("key", key.clone());
        row.insert("scope", format_scope(scope));
        row.insert("store", config_store_name(store));
        row.insert("dry_run", args.dry_run);

        match store {
            ConfigStore::Session => {
                let previous = if args.dry_run {
                    session_scoped_value(context.session_overrides, &key, scope)
                } else {
                    context.session_overrides.remove_scoped(&key, scope)
                };
                row.insert("path", serde_json::Value::Null);
                row.insert("changed", previous.is_some());
                row.insert(
                    "previous",
                    previous
                        .map(|value| config_value_to_json(&value))
                        .unwrap_or(serde_json::Value::Null),
                );
            }
            ConfigStore::Config | ConfigStore::Secrets => {
                let target_path = match store {
                    ConfigStore::Config => paths.config_file.as_deref(),
                    ConfigStore::Secrets => paths.secrets_file.as_deref(),
                    ConfigStore::Session => None,
                }
                .ok_or_else(|| {
                    miette!(
                        "unable to resolve config path for {}",
                        config_store_name(store)
                    )
                })?;

                let edit_result = unset_scoped_value_in_toml(
                    target_path,
                    &key,
                    scope,
                    args.dry_run,
                    matches!(store, ConfigStore::Secrets),
                )
                .into_diagnostic()
                .wrap_err("failed to persist config change")?;

                row.insert("path", target_path.display().to_string());
                row.insert("changed", edit_result.previous.is_some());
                row.insert(
                    "previous",
                    edit_result
                        .previous
                        .as_ref()
                        .map(|previous| {
                            let previous = if matches!(store, ConfigStore::Secrets) {
                                previous.clone().into_secret()
                            } else {
                                previous.clone()
                            };
                            config_value_to_json(&previous)
                        })
                        .unwrap_or(serde_json::Value::Null),
                );
            }
        }

        rows.push(row.build());
    }

    let changed = rows
        .iter()
        .any(|row| row.get("changed").and_then(|value| value.as_bool()) == Some(true));
    if changed {
        messages.success(format!(
            "{} value for {} at {} scope",
            if args.dry_run { "would unset" } else { "unset" },
            key,
            scopes
                .first()
                .map(format_scope)
                .unwrap_or_else(|| "global".to_string())
        ));
    } else {
        messages.warning(format!(
            "no matching value for {} at {} scope",
            key,
            scopes
                .first()
                .map(format_scope)
                .unwrap_or_else(|| "global".to_string())
        ));
    }

    Ok(CliCommandResult {
        exit_code: 0,
        messages,
        output: Some(ReplCommandOutput::Output {
            output: rows_to_output_result(rows),
            format_hint: None,
        }),
    })
}

fn session_scoped_value(
    layer: &ConfigLayer,
    key: &str,
    scope: &Scope,
) -> Option<osp_config::ConfigValue> {
    layer
        .entries()
        .iter()
        .rfind(|entry| entry.key == key && &entry.scope == scope)
        .map(|entry| entry.value.clone())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigStore {
    Session,
    Config,
    Secrets,
}

#[derive(Debug, Clone)]
struct ConfigWriteTarget {
    global: bool,
    profile: Option<String>,
    profile_all: bool,
    terminal: Option<String>,
    session: bool,
    config_store: bool,
    secrets: bool,
    save: bool,
}

impl ConfigWriteTarget {
    fn from_set_args(args: &ConfigSetArgs) -> Self {
        Self {
            global: args.global,
            profile: args.profile.clone(),
            profile_all: args.profile_all,
            terminal: args.terminal.clone(),
            session: args.session,
            config_store: args.config_store,
            secrets: args.secrets,
            save: args.save,
        }
    }

    fn from_unset_args(args: &ConfigUnsetArgs) -> Self {
        Self {
            global: args.global,
            profile: args.profile.clone(),
            profile_all: args.profile_all,
            terminal: args.terminal.clone(),
            session: args.session,
            config_store: args.config_store,
            secrets: args.secrets,
            save: args.save,
        }
    }
}

fn resolve_config_store(context: ConfigReadContext<'_>, args: &ConfigWriteTarget) -> ConfigStore {
    if args.session {
        return ConfigStore::Session;
    }
    if args.config_store {
        return ConfigStore::Config;
    }
    if args.secrets {
        return ConfigStore::Secrets;
    }
    if args.save {
        return ConfigStore::Config;
    }
    if matches!(context.context.terminal_kind(), TerminalKind::Repl) {
        ConfigStore::Session
    } else {
        ConfigStore::Config
    }
}

fn config_store_name(store: ConfigStore) -> &'static str {
    match store {
        ConfigStore::Session => "session",
        ConfigStore::Config => "config",
        ConfigStore::Secrets => "secrets",
    }
}

fn resolve_config_scopes(
    context: ConfigReadContext<'_>,
    args: &ConfigWriteTarget,
) -> Result<Vec<Scope>> {
    let terminal = resolve_terminal_selector(context, args.terminal.as_deref());

    if args.profile_all {
        let profiles = if context.config.known_profiles().is_empty() {
            vec![context.config.active_profile().to_string()]
        } else {
            context
                .config
                .known_profiles()
                .iter()
                .cloned()
                .collect::<Vec<String>>()
        };

        let scopes = profiles
            .into_iter()
            .map(|profile| {
                terminal.as_deref().map_or_else(
                    || Scope::profile(&profile),
                    |current| Scope::profile_terminal(&profile, current),
                )
            })
            .collect::<Vec<Scope>>();
        return Ok(scopes);
    }

    if args.global {
        return Ok(vec![
            terminal
                .as_deref()
                .map_or_else(Scope::global, Scope::terminal),
        ]);
    }

    let profile = args
        .profile
        .as_deref()
        .unwrap_or_else(|| context.config.active_profile());
    Ok(vec![terminal.as_deref().map_or_else(
        || Scope::profile(profile),
        |current| Scope::profile_terminal(profile, current),
    )])
}

fn resolve_terminal_selector(
    context: ConfigReadContext<'_>,
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

fn validate_write_scopes(
    key: &str,
    scopes: &[Scope],
) -> std::result::Result<(), osp_config::ConfigError> {
    for scope in scopes {
        validate_key_scope(key, scope)?;
    }

    Ok(())
}
