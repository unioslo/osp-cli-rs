//! `config` builtin command implementation.
//!
//! This module owns the operator-facing config command surface: reading,
//! explaining, validating, and writing config values through the same schema
//! and scope rules used at runtime.

use super::resolve_terminal_selector;
use crate::app::{AppRuntime, AppSession, UiState};
use crate::app::{
    CommandOutput, RuntimeConfigRequest, config_explain_json, config_explain_result,
    config_value_to_json_for_key, explain_runtime_config, format_scope, is_sensitive_key,
    prepare_runtime_config, push_missing_config_key_messages, render_config_explain_text,
    resolve_runtime_config,
};
use crate::cli::pipeline::validate_alias_template;
use crate::cli::rows::output::rows_to_output_result;
use crate::cli::rows::row::RowBuilder;
use crate::cli::{
    AliasAddArgs, AliasArgs, AliasCommands, AliasListArgs, AliasRemoveArgs, ConfigArgs,
    ConfigCommands, ConfigGetArgs, ConfigScopeArgs, ConfigSetArgs, ConfigShowArgs, ConfigStoreArgs,
    ConfigUnsetArgs,
};
#[cfg(unix)]
use crate::config::secret_file_mode;
use crate::config::{
    ConfigLayer, ConfigSchema, ConfigValue, ResolvedConfig, ResolvedValue, RuntimeConfigPaths,
    RuntimeLoadOptions, RuntimeSecretStore, Scope, SecretBackendKind, TomlStoreEditOptions,
    is_bootstrap_only_key, set_scoped_value_in_toml, unset_scoped_value_in_toml,
    validate_bootstrap_value, validate_key_scope,
};
use crate::core::output::OutputFormat;
use crate::core::row::Row;
use crate::core::shell_words::escape_for_shell;
use crate::ui::messages::MessageBuffer;
use miette::{IntoDiagnostic, Result, WrapErr, miette};

fn runtime_request(runtime: &AppRuntime, session: &AppSession) -> RuntimeConfigRequest {
    RuntimeConfigRequest::new(
        runtime.context.profile_override().map(str::to_owned),
        Some(runtime.context.terminal_kind().as_config_terminal()),
    )
    .with_runtime_load(runtime.launch.runtime_load)
    .with_product_defaults(runtime.product_defaults().clone())
    .with_session_layer(Some(session.config_overrides.clone()))
}

pub(crate) fn run_config_command(
    runtime: &AppRuntime,
    session: &mut AppSession,
    ui: &UiState,
    args: ConfigArgs,
) -> Result<CommandOutput> {
    match args.command {
        ConfigCommands::Edit(args) => picker::run(runtime, session, ui, args.key),
        ConfigCommands::Show(show) => Ok(CommandOutput::output(
            rows_to_output_result(config_show_rows(runtime, show)),
            None,
        )),
        ConfigCommands::Get(get) => run_config_get(runtime, session, get),
        ConfigCommands::Explain(explain) => config_explain_result(
            runtime.config.resolved(),
            ui,
            runtime_request(runtime, session),
            explain,
        ),
        ConfigCommands::Set(set) => run_config_set(runtime, session, ui, set),
        ConfigCommands::Unset(unset) => run_config_unset(runtime, session, unset),
        ConfigCommands::Doctor => Ok(CommandOutput::output(
            rows_to_output_result(config_diagnostics_rows(runtime, session, ui)),
            None,
        )),
    }
}

pub(crate) fn run_alias_command(
    runtime: &AppRuntime,
    session: &mut AppSession,
    ui: &UiState,
    args: AliasArgs,
) -> Result<CommandOutput> {
    match args.command {
        AliasCommands::List(list) => Ok(super::rows_with_empty_info(
            alias_list_rows(runtime, list),
            &format!(
                "No aliases configured. Add one with `{}`.",
                ui.render_settings
                    .runtime
                    .command_scope
                    .command("alias add NAME TEMPLATE")
            ),
        )),
        AliasCommands::Add(add) => run_alias_add(runtime, session, ui, add),
        AliasCommands::Remove(remove) => run_alias_remove(runtime, session, remove),
    }
}

fn alias_list_rows(runtime: &AppRuntime, args: AliasListArgs) -> Vec<Row> {
    runtime
        .config
        .resolved()
        .aliases()
        .iter()
        .map(|(key, entry)| {
            let mut row = RowBuilder::new();
            row.insert(
                "name",
                key.strip_prefix("alias.").unwrap_or(key).to_string(),
            );
            row.insert(
                "template",
                config_value_to_json_for_key(key, &entry.raw_value),
            );
            if args.sources {
                row.insert("source", entry.source.to_string());
                row.insert("scope", format_scope(&entry.scope));
                row.insert(
                    "origin",
                    entry
                        .origin
                        .clone()
                        .map_or(serde_json::Value::Null, Into::into),
                );
            }
            row.build()
        })
        .collect()
}

fn run_alias_add(
    runtime: &AppRuntime,
    session: &mut AppSession,
    ui: &UiState,
    args: AliasAddArgs,
) -> Result<CommandOutput> {
    let key = alias_key(&args.name)?;
    validate_alias_template(&args.name, &args.template, runtime.config.resolved())?;
    run_config_set(
        runtime,
        session,
        ui,
        ConfigSetArgs {
            key,
            value: vec![args.template],
            display_user: None,
            from_file: None,
            scope: args.scope,
            store: args.store,
            dry_run: args.dry_run,
            yes: true,
            explain: false,
        },
    )
}

fn run_alias_remove(
    runtime: &AppRuntime,
    session: &mut AppSession,
    args: AliasRemoveArgs,
) -> Result<CommandOutput> {
    run_config_unset(
        runtime,
        session,
        ConfigUnsetArgs {
            key: alias_key(&args.name)?,
            scope: args.scope,
            store: args.store,
            dry_run: args.dry_run,
        },
    )
}

fn alias_key(name: &str) -> Result<String> {
    let name = name
        .trim()
        .strip_prefix("alias.")
        .unwrap_or(name.trim())
        .to_ascii_lowercase();
    if name.is_empty()
        || name.starts_with('-')
        || name.contains('|')
        || name.chars().any(char::is_whitespace)
    {
        return Err(miette!(
            "invalid alias name `{name}`; use one command word such as `lsng`"
        ));
    }
    Ok(format!("alias.{name}"))
}

fn config_show_rows(runtime: &AppRuntime, args: ConfigShowArgs) -> Vec<Row> {
    let mut entries = runtime
        .config
        .resolved()
        .values()
        .iter()
        .chain(runtime.config.resolved().aliases().iter())
        .collect::<Vec<(&String, &ResolvedValue)>>();
    entries.sort_by_key(|(key, _)| *key);
    let mut rows = entries
        .into_iter()
        .map(|(key, entry)| {
            config_entry_row(
                &runtime.config.resolved().display_key(key),
                entry,
                args.output.sources,
                args.output.raw,
            )
        })
        .collect::<Vec<_>>();
    for role in crate::core::style::ColorRole::ALL {
        if runtime.config.resolved().get(role.key()).is_none() {
            rows.push(crate::row! { "key" => role.key(), "value" => serde_json::Value::Null });
        }
    }
    rows.sort_by(|left, right| left["key"].as_str().cmp(&right["key"].as_str()));
    rows
}

fn run_config_get(
    runtime: &AppRuntime,
    session: &AppSession,
    mut args: ConfigGetArgs,
) -> Result<CommandOutput> {
    args.key = runtime.config.resolved().display_key(&args.key);
    if runtime.launch.runtime_load.include_env
        && let Some(path) = std::env::var_os("OSP_CONFIG_FILE")
        && !std::path::Path::new(&path).exists()
    {
        return Err(miette!(
            "Config file {} does not exist.",
            std::path::Path::new(&path).display()
        ));
    }
    let mut messages = MessageBuffer::default();
    let rows = config_get_rows(runtime, session, &args)?;
    match rows {
        Some(rows) => Ok(CommandOutput {
            messages,
            ..CommandOutput::output(rows_to_output_result(rows), None)
        }),
        None => {
            let error = push_missing_config_key_messages(
                &mut messages,
                runtime.config.resolved(),
                &args.key,
            );
            Ok(CommandOutput {
                exit_code: 1,
                messages,
                failure_report: Some(miette::Report::new(error)),
                ..CommandOutput::exit(0)
            })
        }
    }
}

fn config_get_rows(
    runtime: &AppRuntime,
    session: &AppSession,
    args: &ConfigGetArgs,
) -> Result<Option<Vec<Row>>> {
    if let Some(entry) = runtime.config.resolved().get_value_entry(&args.key) {
        let row = config_entry_row(&args.key, entry, args.output.sources, args.output.raw);
        return Ok(Some(vec![row]));
    }

    if let Some(entry) = runtime.config.resolved().get_alias_entry(&args.key) {
        let key = if args.key.starts_with("alias.") {
            args.key.clone()
        } else {
            format!("alias.{}", args.key.trim().to_ascii_lowercase())
        };
        let row = config_entry_row(&key, entry, args.output.sources, args.output.raw);
        return Ok(Some(vec![row]));
    }

    if is_bootstrap_only_key(&args.key) {
        let explain = explain_runtime_config(runtime_request(runtime, session), &args.key)
            .wrap_err_with(|| format!("failed to resolve bootstrap key `{}`", args.key))?;

        if let Some(entry) = explain.final_entry {
            let row = config_entry_row(&args.key, &entry, args.output.sources, args.output.raw);
            return Ok(Some(vec![row]));
        }
    }

    Ok(None)
}

pub(crate) fn config_diagnostics_rows(
    runtime: &AppRuntime,
    session: &AppSession,
    ui: &UiState,
) -> Vec<Row> {
    let paths = RuntimeConfigPaths::discover_with(runtime.launch.runtime_load);
    let backend = runtime
        .config
        .resolved()
        .get_string("secrets.backend")
        .unwrap_or("toml");
    let selected_path = if backend.eq_ignore_ascii_case("keyring") {
        paths.secrets_index_file.clone()
    } else {
        paths.secrets_file.clone()
    };
    let secrets =
        secrets_permissions_diagnostic(selected_path, &ui.render_settings.runtime.command_scope);
    let known_profiles = serde_json::Value::Array(
        runtime
            .config
            .resolved()
            .known_profiles()
            .iter()
            .map(|value| value.clone().into())
            .collect(),
    );
    let theme_issues = serde_json::Value::Array(
        runtime
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
    let resolution_error = prepare_runtime_config(runtime_request(runtime, session), false)
        .and_then(crate::app::PreparedRuntimeConfig::resolve)
        .err();
    let mut row = crate::row! {
        "status" => if resolution_error.is_some() { "error" } else { "ok" },
        "active_profile" => runtime.config.resolved().active_profile().to_string(),
        "known_profiles" => known_profiles,
        "resolved_keys" => (runtime.config.resolved().values().len()
            + runtime.config.resolved().aliases().len()) as i64,
        "theme_issue_count" => runtime.themes.issues.len() as i64,
        "theme_issues" => theme_issues,
        "secrets_backend" => backend,
        "secrets_file" => paths.secrets_file
            .map_or(serde_json::Value::Null, |path| path.display().to_string().into()),
        "secrets_index_file" => paths.secrets_index_file
            .map_or(serde_json::Value::Null, |path| path.display().to_string().into()),
        "secrets_store_path" => secrets.path,
        "secrets_permissions_status" => secrets.status,
        "secrets_permissions_mode" => secrets.mode,
        "secrets_permissions_message" => secrets.message,
    };
    if let Some(error) = resolution_error {
        row.insert("resolution_error".to_string(), error.to_string().into());
    }
    vec![row]
}

struct SecretsPermissionsDiagnostic {
    path: serde_json::Value,
    status: &'static str,
    mode: serde_json::Value,
    message: String,
}

fn secrets_permissions_diagnostic(
    path: Option<std::path::PathBuf>,
    scope: &crate::core::CommandScope,
) -> SecretsPermissionsDiagnostic {
    let Some(path) = path else {
        return SecretsPermissionsDiagnostic {
            path: serde_json::Value::Null,
            status: "unavailable",
            mode: serde_json::Value::Null,
            message: "secrets path unavailable".to_string(),
        };
    };

    let display = serde_json::Value::String(path.display().to_string());
    if !path.exists() {
        return SecretsPermissionsDiagnostic {
            path: display,
            status: "missing",
            mode: serde_json::Value::Null,
            message: "secrets file does not exist".to_string(),
        };
    }

    #[cfg(unix)]
    {
        match secret_file_mode(&path) {
            Ok(mode) => {
                let status = if mode == 0o600 {
                    "ok"
                } else if mode & 0o077 == 0 {
                    "warning"
                } else {
                    "issue"
                };
                let message = match status {
                    "ok" => "secrets file permissions are 0600".to_string(),
                    "warning" => format!(
                        "secrets file mode is {:o}; 0600 is recommended so `{}` stays predictable",
                        mode,
                        scope.command("config set --secrets")
                    ),
                    _ => format!(
                        "secrets file mode is {:o}; owner-only permissions are required",
                        mode
                    ),
                };
                SecretsPermissionsDiagnostic {
                    path: display,
                    status,
                    mode: serde_json::Value::String(format!("{mode:o}")),
                    message,
                }
            }
            Err(err) => SecretsPermissionsDiagnostic {
                path: display,
                status: "error",
                mode: serde_json::Value::Null,
                message: err.to_string(),
            },
        }
    }

    #[cfg(not(unix))]
    {
        SecretsPermissionsDiagnostic {
            path: display,
            status: "unavailable",
            mode: serde_json::Value::Null,
            message: "secrets permission diagnostics are unavailable on this platform".to_string(),
        }
    }
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
        config_value_to_json_for_key(
            key,
            if show_raw {
                &entry.raw_value
            } else {
                &entry.value
            },
        ),
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

pub(super) fn clear_theme_values(
    runtime: &AppRuntime,
    session: &mut AppSession,
    keys: &[String],
) -> Result<Vec<String>> {
    let paths = config_write_paths(runtime.launch.runtime_load, ConfigStore::Config)?;
    let path = paths
        .as_ref()
        .and_then(|paths| paths.config_file.as_deref())
        .ok_or_else(|| miette!("unable to resolve config path"))?;
    let layer = match std::fs::read_to_string(path) {
        Ok(text) => ConfigLayer::from_toml_str(&text).into_diagnostic()?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => ConfigLayer::default(),
        Err(error) => return Err(error).into_diagnostic(),
    };
    let mut changed = Vec::new();
    for entry in layer
        .entries()
        .iter()
        .filter(|entry| keys.contains(&entry.key) && scope_applies(&entry.scope, runtime))
    {
        let output = run_config_unset(
            runtime,
            session,
            ConfigUnsetArgs {
                key: entry.key.clone(),
                scope: ConfigScopeArgs {
                    global: entry.scope.profile.is_none(),
                    profile: entry.scope.profile.clone(),
                    terminal: entry.scope.terminal.clone(),
                    ..ConfigScopeArgs::default()
                },
                store: ConfigStoreArgs {
                    save: true,
                    ..ConfigStoreArgs::default()
                },
                dry_run: false,
            },
        )?;
        if let crate::app::CommandBody::Output(output) = output.body
            && output
                .as_rows()
                .unwrap_or_default()
                .iter()
                .any(|row| row["changed"] == true)
            && !changed.contains(&entry.key)
        {
            changed.push(entry.key.clone());
        }
    }
    clear_session_values(runtime, session, keys, &mut changed);
    Ok(changed)
}

fn scope_applies(scope: &Scope, runtime: &AppRuntime) -> bool {
    scope
        .profile
        .as_deref()
        .is_none_or(|profile| profile == runtime.config.resolved().active_profile())
        && scope
            .terminal
            .as_deref()
            .is_none_or(|terminal| terminal == runtime.context.terminal_kind().as_config_terminal())
}

fn clear_session_values(
    runtime: &AppRuntime,
    session: &mut AppSession,
    keys: &[String],
    changed: &mut Vec<String>,
) {
    let applicable = session
        .config_overrides
        .entries()
        .iter()
        .filter(|entry| keys.contains(&entry.key) && scope_applies(&entry.scope, runtime))
        .map(|entry| (entry.key.clone(), entry.scope.clone()))
        .collect::<Vec<_>>();
    for (key, scope) in applicable {
        session.config_overrides.remove_scoped(&key, &scope);
        if !changed.contains(&key) {
            changed.push(key);
        }
    }
}

pub(super) fn set_picker_value(
    runtime: &AppRuntime,
    session: &mut AppSession,
    ui: &UiState,
    key: String,
    value: String,
    permanent: bool,
) -> Result<CommandOutput> {
    run_config_set(
        runtime,
        session,
        ui,
        ConfigSetArgs {
            key,
            value: vec![value],
            display_user: None,
            from_file: None,
            scope: ConfigScopeArgs::default(),
            store: ConfigStoreArgs {
                save: permanent,
                ..ConfigStoreArgs::default()
            },
            dry_run: false,
            yes: true,
            explain: false,
        },
    )
}

fn run_config_set(
    runtime: &AppRuntime,
    session: &mut AppSession,
    ui: &UiState,
    args: ConfigSetArgs,
) -> Result<CommandOutput> {
    let requested_key = runtime
        .config
        .resolved()
        .display_key(&args.key.trim().to_ascii_lowercase());
    let key = if requested_key == "user.display_name" {
        let user = args
            .display_user
            .as_deref()
            .or_else(|| runtime.context.authenticated_subject())
            .or_else(|| runtime.config.resolved().get_string("user.name"))
            .filter(|user| !user.trim().is_empty())
            .ok_or_else(|| miette!("select a user with -u or --user"))?;
        if user.contains('.') {
            return Err(miette!("display-name usernames cannot contain a dot"));
        }
        format!("user.display_names.{user}")
    } else {
        if args.display_user.is_some() {
            return Err(miette!("--user selects the account for user.display_name"));
        }
        requested_key
    };
    let schema = ConfigSchema::default().with_product_defaults(runtime.product_defaults());
    schema.validate_writable_key(&key).into_diagnostic()?;
    let declared_secret = is_declared_secret_key(runtime, &key);
    let input = match (args.value.as_slice(), &args.from_file) {
        (values, None) if !values.is_empty() => values.join(" "),
        ([], None) => {
            use std::io::IsTerminal;
            if !declared_secret && !is_sensitive_key(&key) {
                return Err(miette!(
                    "the following required arguments were not provided: <VALUE>"
                ));
            }
            if crate::ui::prompt::input_is_replay() || !std::io::stderr().is_terminal() {
                return Err(miette::Report::new(
                    crate::core::CommandError::new(
                        "the following required arguments were not provided: <VALUE>",
                    )
                    .with_action("Use --from-file - to read the value from stdin."),
                ));
            }
            crate::ui::prompt::read_hidden(&format!("{key}: "))
                .into_diagnostic()
                .wrap_err("failed to read secret config value")?
                .ok_or_else(|| miette!("config set cancelled"))?
        }
        ([], Some(path)) => {
            let content = if path == std::path::Path::new("-") {
                use std::io::Read;
                let mut content = String::new();
                std::io::stdin()
                    .lock()
                    .read_to_string(&mut content)
                    .into_diagnostic()
                    .wrap_err("failed to read config value from stdin")?;
                content
            } else {
                let path = path
                    .to_str()
                    .filter(|path| path.starts_with("~/"))
                    .map(|path| {
                        crate::normalize::expand_home(path)
                            .ok_or_else(|| miette!("unable to locate home directory"))
                    })
                    .transpose()?
                    .unwrap_or_else(|| path.clone());
                std::fs::read_to_string(&path)
                    .into_diagnostic()
                    .wrap_err_with(|| {
                        format!("failed to read config value from {}", path.display())
                    })?
            };
            content.trim_end_matches(['\r', '\n']).to_owned()
        }
        _ => return Err(miette!("provide either a value or --from-file PATH")),
    };
    let value = schema
        .parse_input_value(&key, &input)
        .map_err(|err| {
            if args.from_file.is_some() {
                miette!("imported value is invalid for key `{key}`")
            } else {
                miette::Report::new(err)
            }
        })
        .wrap_err("invalid value for key")?;
    let value = if declared_secret {
        value.into_secret()
    } else {
        value
    };
    validate_bootstrap_value(&key, &value)
        .map_err(|err| {
            if args.from_file.is_some() {
                miette!("imported bootstrap value is invalid for key `{key}`")
            } else {
                miette::Report::new(err)
            }
        })
        .wrap_err("invalid bootstrap value")?;
    let (store, scopes) = resolve_edit_target(runtime, &key, &args.scope, &args.store)?;

    tracing::debug!(
        key = %key,
        store = %config_store_name(store),
        scope_count = scopes.len(),
        dry_run = args.dry_run,
        explain = args.explain,
        "config set"
    );

    let mut messages = MessageBuffer::default();
    if matches!(store, ConfigStore::Config) && is_sensitive_key(&key) {
        messages.warning("writing a sensitive key to config store; prefer --secrets");
    }

    runtime
        .config
        .resolved()
        .validate_candidate(&key, &value)
        .map_err(|err| miette!("invalid proposed value for key `{key}`: {err}"))?;
    let current = if args.dry_run && args.explain {
        let explain = explain_runtime_config(runtime_request(runtime, session), &key)?;
        Some(config_explain_json(
            &explain,
            runtime.config.resolved(),
            false,
        ))
    } else {
        None
    };
    let rows = apply_scoped_edit(
        runtime,
        session,
        store,
        &scopes,
        &key,
        Edit::Set {
            value: &value,
            current: current.as_ref(),
        },
        args.dry_run,
    )?;

    let output = if args.explain && !args.dry_run {
        let request = runtime_request(runtime, session);
        let explain = explain_runtime_config(request.clone(), &key)
            .wrap_err_with(|| format!("failed to explain config for key `{key}` after set"))?;
        let config = resolve_runtime_config(request)
            .wrap_err_with(|| format!("failed to resolve config for key `{key}` after set"))?;
        if matches!(ui.render_settings.requested_format(), OutputFormat::Json) {
            let payload = config_explain_json(&explain, &config, false);
            CommandOutput::json(payload)
        } else {
            CommandOutput::text(render_config_explain_text(&explain, &config, false))
        }
    } else {
        CommandOutput::output(rows_to_output_result(rows), None)
    };

    let persistence = if matches!(store, ConfigStore::Session) {
        "for this session only"
    } else {
        "permanently"
    };
    let mut success = format!(
        "{} value for {key} {persistence} at {} scope",
        if args.dry_run { "would set" } else { "set" },
        scopes
            .first()
            .map(format_scope)
            .unwrap_or_else(|| "global".to_string())
    );
    if matches!(store, ConfigStore::Session)
        && !args.dry_run
        && !declared_secret
        && !is_sensitive_key(&key)
    {
        for scope in &scopes {
            let mut options = " --config".to_string();
            if let Some(profile) = &scope.profile {
                options.push_str(&format!(" --profile {}", escape_for_shell(profile)));
            } else {
                options.push_str(" --global");
            }
            if let Some(terminal) = &scope.terminal {
                options.push_str(&format!(" --terminal {}", escape_for_shell(terminal)));
            }
            let key = escape_for_shell(&key);
            let command = match &args.from_file {
                Some(path) => format!(
                    "config set {key} --from-file {}{options}",
                    escape_for_shell(&path.to_string_lossy())
                ),
                // An option-shaped value must follow `--` to replay literally.
                None if input.starts_with('-') => {
                    format!("config set {key}{options} -- {}", escape_for_shell(&input))
                }
                None => format!("config set {key} {}{options}", escape_for_shell(&input)),
            };
            success.push_str(&format!("\nTo save this setting: {command}"));
        }
    }
    messages.success(success);
    Ok(CommandOutput { messages, ..output })
}

fn run_config_unset(
    runtime: &AppRuntime,
    session: &mut AppSession,
    args: ConfigUnsetArgs,
) -> Result<CommandOutput> {
    let key = runtime
        .config
        .resolved()
        .display_key(&args.key.trim().to_ascii_lowercase());
    ConfigSchema::default()
        .validate_writable_key(&key)
        .into_diagnostic()?;
    let (store, scopes) = resolve_edit_target(runtime, &key, &args.scope, &args.store)?;

    tracing::debug!(
        key = %key,
        store = %config_store_name(store),
        scope_count = scopes.len(),
        dry_run = args.dry_run,
        "config unset"
    );

    let mut messages = MessageBuffer::default();
    let rows = apply_scoped_edit(
        runtime,
        session,
        store,
        &scopes,
        &key,
        Edit::Unset,
        args.dry_run,
    )?;

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

    Ok(CommandOutput {
        messages,
        ..CommandOutput::output(rows_to_output_result(rows), None)
    })
}

#[derive(Clone, Copy)]
enum Edit<'a> {
    Set {
        value: &'a ConfigValue,
        current: Option<&'a serde_json::Value>,
    },
    Unset,
}

impl Edit<'_> {
    fn name(self) -> &'static str {
        match self {
            Self::Set { .. } => "set",
            Self::Unset => "unset",
        }
    }

    fn changed(self, previous: Option<&ConfigValue>) -> bool {
        match self {
            Self::Set { value, .. } => previous != Some(value),
            Self::Unset => previous.is_some(),
        }
    }
}

fn is_declared_secret_key(runtime: &AppRuntime, key: &str) -> bool {
    let key_map = runtime.config.resolved().key_map;
    let canonical = key_map.canonical(key);
    runtime
        .product_defaults()
        .entries()
        .iter()
        .any(|entry| entry.value.is_secret() && key_map.canonical(&entry.key) == canonical)
}

fn resolve_edit_target(
    runtime: &AppRuntime,
    key: &str,
    scope: &ConfigScopeArgs,
    store: &ConfigStoreArgs,
) -> Result<(ConfigStore, Vec<Scope>)> {
    let target = ConfigWriteTarget::from_args(scope, store).with_default(runtime);
    let mut store = resolve_config_store(&target);
    if store != ConfigStore::Session && is_declared_secret_key(runtime, key) {
        store = ConfigStore::Secrets;
    }
    validate_store_key(store, key)?;
    let scopes = resolve_config_scopes(runtime, &target)
        .wrap_err_with(|| format!("failed to resolve config scopes for key `{key}`"))?;
    validate_write_scopes(key, &scopes).into_diagnostic()?;
    Ok((store, scopes))
}

fn apply_scoped_edit(
    runtime: &AppRuntime,
    session: &mut AppSession,
    store: ConfigStore,
    scopes: &[Scope],
    key: &str,
    edit: Edit<'_>,
    dry_run: bool,
) -> Result<Vec<Row>> {
    let key_map = runtime.config.resolved().key_map;
    let canonical = key_map.canonical(key);
    let key = canonical.as_str();
    key_map.normalize_layer(&mut session.config_overrides);
    let paths = config_write_paths(runtime.launch.runtime_load, store)?;
    let mut rows = Vec::new();
    for scope in scopes {
        let mut row = RowBuilder::new();
        row.insert("key", runtime.config.resolved().display_key(key));
        if let Edit::Set { value, .. } = edit {
            let display_value = if matches!(store, ConfigStore::Secrets) {
                value.clone().into_secret()
            } else {
                value.clone()
            };
            row.insert("value", config_value_to_json_for_key(key, &display_value));
        }
        row.insert("scope", format_scope(scope));
        row.insert("store", config_store_name(store));
        row.insert("dry_run", dry_run);

        if let Edit::Set {
            current: Some(current),
            ..
        } = edit
        {
            row.insert("current", current.clone());
        }

        let previous = match store {
            ConfigStore::Session => {
                row.insert("path", serde_json::Value::Null);
                match edit {
                    Edit::Set { value, .. } => {
                        if !dry_run {
                            session.config_overrides.replace_scoped(
                                key,
                                value.clone(),
                                scope.clone(),
                            );
                        }
                        row.insert("changed", true);
                        rows.push(row.build());
                        continue;
                    }
                    Edit::Unset if dry_run => {
                        session_scoped_value(&session.config_overrides, key, scope)
                    }
                    Edit::Unset => session.config_overrides.remove_scoped(key, scope),
                }
            }
            ConfigStore::Config => {
                let target_path = paths
                    .as_ref()
                    .and_then(|paths| paths.config_file.as_deref())
                    .ok_or_else(|| miette!("unable to resolve config path for config"))?;
                tracing::trace!(
                    key = %key, scope = %format_scope(scope), store = "config",
                    path = %target_path.display(), dry_run,
                    "persisting config {}", edit.name()
                );
                let options = store_edit_options(store, dry_run).with_key_map(key_map);
                let result = match edit {
                    Edit::Set { value, .. } => {
                        set_scoped_value_in_toml(target_path, key, value, scope, options)
                    }
                    Edit::Unset => unset_scoped_value_in_toml(target_path, key, scope, options),
                }
                .map_err(|error| key_map.error(error))
                .into_diagnostic()
                .wrap_err_with(|| {
                    format!(
                        "failed to persist config {} for key `{}` at {}",
                        edit.name(),
                        key_map.display(key),
                        target_path.display()
                    )
                })?;
                row.insert("path", target_path.display().to_string());
                result.previous
            }
            ConfigStore::Secrets => {
                let result = edit_secret(runtime, scope, key, edit, dry_run)?;
                row.insert("path", result.location);
                row.insert("backend", result.backend.as_str());
                // Compare unwrapped values before marking previous secret data for redaction.
                row.insert("changed", edit.changed(result.previous.as_ref()));
                row.insert(
                    "previous",
                    result
                        .previous
                        .map(|previous| config_value_to_json_for_key(key, &previous.into_secret()))
                        .unwrap_or(serde_json::Value::Null),
                );
                rows.push(row.build());
                continue;
            }
        };
        row.insert("changed", edit.changed(previous.as_ref()));
        row.insert(
            "previous",
            previous
                .as_ref()
                .map(|previous| config_value_to_json_for_key(key, previous))
                .unwrap_or(serde_json::Value::Null),
        );
        rows.push(row.build());
    }
    // A session override outranks stored values, so leaving it would make the
    // stored edit invisible until restart.
    if !dry_run && store != ConfigStore::Session {
        clear_session_values(runtime, session, &[key.to_owned()], &mut Vec::new());
    }
    Ok(rows)
}

fn session_scoped_value(
    layer: &ConfigLayer,
    key: &str,
    scope: &Scope,
) -> Option<crate::config::ConfigValue> {
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

fn config_write_paths(
    runtime_load: RuntimeLoadOptions,
    store: ConfigStore,
) -> Result<Option<RuntimeConfigPaths>> {
    if matches!(store, ConfigStore::Session) {
        return Ok(None);
    }
    if !runtime_load.include_config_file {
        return Err(miette!("config file writes are disabled for this session"));
    }
    Ok(Some(RuntimeConfigPaths::discover_with(runtime_load)))
}

fn selected_secret_store(
    config: &ResolvedConfig,
    paths: &RuntimeConfigPaths,
) -> Result<RuntimeSecretStore, crate::config::ConfigError> {
    let backend = config
        .get_string("secrets.backend")
        .map(SecretBackendKind::parse)
        .transpose()?
        .unwrap_or(SecretBackendKind::Toml);
    RuntimeSecretStore::from_paths(backend, paths)
}

fn validate_store_key(store: ConfigStore, key: &str) -> Result<()> {
    if matches!(store, ConfigStore::Secrets) && key.eq_ignore_ascii_case("secrets.backend") {
        return Err(miette!(
            "secrets.backend must be written to the regular config store because the secrets store cannot select itself"
        ));
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct ConfigWriteTarget {
    scope: ConfigScopeTarget,
    terminal: Option<String>,
    store: ConfigStoreTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConfigScopeTarget {
    ActiveProfile,
    Global,
    Profile(String),
    AllProfiles,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigStoreTarget {
    Default,
    Session,
    Config,
    Secrets,
}

impl ConfigWriteTarget {
    fn with_default(mut self, runtime: &AppRuntime) -> Self {
        if self.store == ConfigStoreTarget::Session {
            return self;
        }
        if self.scope != ConfigScopeTarget::ActiveProfile {
            if self.store == ConfigStoreTarget::Default {
                self.store = ConfigStoreTarget::Config;
            }
            return self;
        }
        match runtime
            .config
            .resolved()
            .get_string("config.default-target")
            .map(str::trim)
        {
            Some("session") => {
                if self.store == ConfigStoreTarget::Default {
                    self.store = ConfigStoreTarget::Session;
                }
            }
            Some(target) if !target.is_empty() => {
                self.scope = if target == "global" {
                    ConfigScopeTarget::Global
                } else {
                    ConfigScopeTarget::Profile(target.to_string())
                };
                if self.store == ConfigStoreTarget::Default {
                    self.store = ConfigStoreTarget::Config;
                }
            }
            _ => {}
        }
        self
    }

    fn from_args(scope: &ConfigScopeArgs, store: &ConfigStoreArgs) -> Self {
        Self {
            scope: resolve_scope_target(scope.global, scope.profile.clone(), scope.profile_all),
            terminal: scope.terminal.clone(),
            store: resolve_store_target(
                store.session,
                store.config_store,
                store.secrets,
                store.save,
            ),
        }
    }
}

fn resolve_config_store(args: &ConfigWriteTarget) -> ConfigStore {
    match args.store {
        ConfigStoreTarget::Session => ConfigStore::Session,
        ConfigStoreTarget::Config | ConfigStoreTarget::Default => ConfigStore::Config,
        ConfigStoreTarget::Secrets => ConfigStore::Secrets,
    }
}

fn config_store_name(store: ConfigStore) -> &'static str {
    match store {
        ConfigStore::Session => "session",
        ConfigStore::Config => "config",
        ConfigStore::Secrets => "secrets",
    }
}

fn store_edit_options(store: ConfigStore, dry_run: bool) -> TomlStoreEditOptions {
    let options = if dry_run {
        TomlStoreEditOptions::dry_run()
    } else {
        TomlStoreEditOptions::new()
    };
    if matches!(store, ConfigStore::Secrets) {
        options.for_secrets()
    } else {
        options
    }
}

fn resolve_config_scopes(runtime: &AppRuntime, args: &ConfigWriteTarget) -> Result<Vec<Scope>> {
    let terminal = resolve_terminal_selector(
        runtime.context.terminal_kind().as_config_terminal(),
        args.terminal.as_deref(),
    );

    match &args.scope {
        ConfigScopeTarget::AllProfiles => {
            let profiles = if runtime.config.resolved().known_profiles().is_empty() {
                vec![runtime.config.resolved().active_profile().to_string()]
            } else {
                runtime
                    .config
                    .resolved()
                    .known_profiles()
                    .iter()
                    .cloned()
                    .collect::<Vec<String>>()
            };

            Ok(profiles
                .into_iter()
                .map(|profile| {
                    terminal.as_deref().map_or_else(
                        || Scope::profile(&profile),
                        |current| Scope::profile_terminal(&profile, current),
                    )
                })
                .collect())
        }
        ConfigScopeTarget::Global => Ok(vec![
            terminal
                .as_deref()
                .map_or_else(Scope::global, Scope::terminal),
        ]),
        ConfigScopeTarget::Profile(profile) => Ok(vec![terminal.as_deref().map_or_else(
            || Scope::profile(profile),
            |current| Scope::profile_terminal(profile, current),
        )]),
        ConfigScopeTarget::ActiveProfile => {
            let profile = runtime.config.resolved().active_profile();
            Ok(vec![terminal.as_deref().map_or_else(
                || Scope::profile(profile),
                |current| Scope::profile_terminal(profile, current),
            )])
        }
    }
}

fn resolve_scope_target(
    global: bool,
    profile: Option<String>,
    profile_all: bool,
) -> ConfigScopeTarget {
    if profile_all {
        ConfigScopeTarget::AllProfiles
    } else if global {
        ConfigScopeTarget::Global
    } else if let Some(profile) = profile {
        ConfigScopeTarget::Profile(profile)
    } else {
        ConfigScopeTarget::ActiveProfile
    }
}

fn resolve_store_target(
    session: bool,
    config_store: bool,
    secrets: bool,
    save: bool,
) -> ConfigStoreTarget {
    if session {
        ConfigStoreTarget::Session
    } else if config_store {
        ConfigStoreTarget::Config
    } else if secrets {
        ConfigStoreTarget::Secrets
    } else if save {
        ConfigStoreTarget::Config
    } else {
        ConfigStoreTarget::Default
    }
}

fn validate_write_scopes(
    key: &str,
    scopes: &[Scope],
) -> std::result::Result<(), crate::config::ConfigError> {
    for scope in scopes {
        validate_key_scope(key, scope)?;
    }

    Ok(())
}

fn edit_secret(
    runtime: &AppRuntime,
    scope: &Scope,
    key: &str,
    edit: Edit<'_>,
    dry_run: bool,
) -> Result<crate::config::SecretStoreEditResult> {
    let paths = config_write_paths(runtime.launch.runtime_load, ConfigStore::Secrets)?
        .ok_or_else(|| miette!("unable to resolve secrets store paths"))?;
    let secret_store = selected_secret_store(runtime.config.resolved(), &paths)
        .into_diagnostic()
        .wrap_err("failed to select secrets backend")?;
    let key_map = runtime.config.resolved().key_map;
    let canonical = key_map.canonical(key);
    let key = canonical.as_str();
    let options = store_edit_options(ConfigStore::Secrets, dry_run).with_key_map(key_map);
    match edit {
        Edit::Set { value, .. } => secret_store.set_scoped(key, value, scope, options),
        Edit::Unset => secret_store.unset_scoped(key, scope, options),
    }
    .map_err(|error| key_map.error(error))
    .into_diagnostic()
    .wrap_err_with(|| match edit {
        Edit::Set { .. } => format!("failed to persist secret `{}`", key_map.display(key)),
        Edit::Unset => format!("failed to remove secret `{}`", key_map.display(key)),
    })
}

pub(crate) fn store_secret(runtime: &AppRuntime, key: &str, value: &ConfigValue) -> Result<()> {
    validate_store_key(ConfigStore::Secrets, key)?;
    let target = ConfigWriteTarget::from_args(
        &ConfigScopeArgs::default(),
        &ConfigStoreArgs {
            secrets: true,
            ..Default::default()
        },
    );
    let scopes = resolve_config_scopes(runtime, &target)?;
    validate_write_scopes(key, &scopes).into_diagnostic()?;
    for scope in scopes {
        edit_secret(
            runtime,
            &scope,
            key,
            Edit::Set {
                value,
                current: None,
            },
            false,
        )?;
    }
    Ok(())
}

mod picker;

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests;
