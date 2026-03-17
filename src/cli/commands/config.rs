use crate::app::{AppRuntime, AppSession, RuntimeContext, TerminalKind, UiState};
use crate::app::{
    CURRENT_TERMINAL_SENTINEL, CliCommandResult, ConfigExplainContext, ReplCommandOutput,
    RuntimeConfigRequest, config_explain_json, config_explain_result, config_value_to_json,
    explain_runtime_config, format_scope, is_sensitive_key, push_missing_config_key_messages,
    render_config_explain_text, resolve_runtime_config,
};
use crate::cli::rows::output::rows_to_output_result;
use crate::cli::rows::row::RowBuilder;
use crate::cli::{
    ConfigArgs, ConfigCommands, ConfigGetArgs, ConfigSetArgs, ConfigShowArgs, ConfigUnsetArgs,
};
#[cfg(unix)]
use crate::config::secret_file_mode;
use crate::config::{
    ConfigLayer, ConfigSchema, ResolvedConfig, ResolvedValue, RuntimeConfigPaths,
    RuntimeLoadOptions, Scope, TomlStoreEditOptions, is_bootstrap_only_key,
    set_scoped_value_in_toml, unset_scoped_value_in_toml, validate_bootstrap_value,
    validate_key_scope,
};
use crate::core::output::OutputFormat;
use crate::core::row::Row;
use crate::ui::messages::MessageBuffer;
use crate::ui::theme_catalog::ThemeCatalog;
use miette::{IntoDiagnostic, Result, WrapErr, miette};

pub(crate) struct ConfigCommandContext<'a> {
    pub(crate) context: &'a RuntimeContext,
    pub(crate) config: &'a ResolvedConfig,
    pub(crate) ui: &'a UiState,
    pub(crate) themes: &'a ThemeCatalog,
    pub(crate) config_overrides: &'a mut ConfigLayer,
    pub(crate) product_defaults: &'a ConfigLayer,
    pub(crate) runtime_load: RuntimeLoadOptions,
}

#[derive(Clone, Copy)]
pub(crate) struct ConfigReadContext<'a> {
    pub(crate) context: &'a RuntimeContext,
    pub(crate) config: &'a ResolvedConfig,
    pub(crate) ui: &'a UiState,
    pub(crate) themes: &'a ThemeCatalog,
    pub(crate) config_overrides: &'a ConfigLayer,
    pub(crate) product_defaults: &'a ConfigLayer,
    pub(crate) runtime_load: RuntimeLoadOptions,
}

impl<'a> ConfigCommandContext<'a> {
    pub(crate) fn from_parts(
        runtime: &'a AppRuntime,
        session: &'a mut AppSession,
        ui: &'a UiState,
    ) -> Self {
        Self {
            context: &runtime.context,
            config: runtime.config.resolved(),
            ui,
            themes: &runtime.themes,
            config_overrides: &mut session.config_overrides,
            product_defaults: runtime.product_defaults(),
            runtime_load: runtime.launch.runtime_load,
        }
    }

    fn read(&self) -> ConfigReadContext<'_> {
        ConfigReadContext {
            context: self.context,
            config: self.config,
            ui: self.ui,
            themes: self.themes,
            config_overrides: &*self.config_overrides,
            product_defaults: self.product_defaults,
            runtime_load: self.runtime_load,
        }
    }
}

impl<'a> ConfigReadContext<'a> {
    pub(crate) fn from_parts(
        runtime: &'a AppRuntime,
        session: &'a AppSession,
        ui: &'a UiState,
    ) -> Self {
        Self {
            context: &runtime.context,
            config: runtime.config.resolved(),
            ui,
            themes: &runtime.themes,
            config_overrides: &session.config_overrides,
            product_defaults: runtime.product_defaults(),
            runtime_load: runtime.launch.runtime_load,
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
        ConfigCommands::Explain(explain) => config_explain_result(
            &ConfigExplainContext {
                context: read.context,
                config: read.config,
                ui: read.ui,
                session_layer: read.config_overrides,
                product_defaults: read.product_defaults,
                runtime_load: read.runtime_load,
            },
            explain,
        ),
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
        .map(|(key, entry)| config_entry_row(key, entry, args.output.sources, args.output.raw))
        .collect()
}

fn run_config_get(context: ConfigReadContext<'_>, args: ConfigGetArgs) -> Result<CliCommandResult> {
    let mut messages = MessageBuffer::default();
    let rows = config_get_rows(context, &args, &mut messages)?;
    match rows {
        Some(rows) => Ok(CliCommandResult {
            exit_code: 0,
            messages,
            output: Some(ReplCommandOutput::Output(
                crate::app::StructuredCommandOutput {
                    source_guide: None,
                    output: rows_to_output_result(rows),
                    format_hint: None,
                },
            )),
            stderr_text: None,
            failure_report: None,
        }),
        None => Ok(CliCommandResult {
            exit_code: 1,
            messages,
            output: None,
            stderr_text: None,
            failure_report: None,
        }),
    }
}

fn config_get_rows(
    context: ConfigReadContext<'_>,
    args: &ConfigGetArgs,
    messages: &mut MessageBuffer,
) -> Result<Option<Vec<Row>>> {
    if let Some(entry) = context.config.get_value_entry(&args.key) {
        let row = config_entry_row(&args.key, entry, args.output.sources, args.output.raw);
        return Ok(Some(vec![row]));
    }

    if let Some(entry) = context.config.get_alias_entry(&args.key) {
        let key = if args.key.starts_with("alias.") {
            args.key.clone()
        } else {
            format!("alias.{}", args.key.trim().to_ascii_lowercase())
        };
        let row = config_entry_row(&key, entry, args.output.sources, args.output.raw);
        return Ok(Some(vec![row]));
    }

    if is_bootstrap_only_key(&args.key) {
        let explain = explain_runtime_config(
            RuntimeConfigRequest::new(
                context.context.profile_override().map(str::to_owned),
                Some(context.context.terminal_kind().as_config_terminal()),
            )
            .with_runtime_load(context.runtime_load)
            .with_session_layer(Some(context.config_overrides.clone())),
            &args.key,
        )
        .wrap_err_with(|| format!("failed to resolve bootstrap key `{}`", args.key))?;

        if let Some(entry) = explain.final_entry {
            let row = config_entry_row(&args.key, &entry, args.output.sources, args.output.raw);
            return Ok(Some(vec![row]));
        }
    }

    push_missing_config_key_messages(messages, context.config, &args.key);
    Ok(None)
}

pub(crate) fn config_diagnostics_rows(context: ConfigReadContext<'_>) -> Vec<Row> {
    let secrets = secrets_permissions_diagnostic(RuntimeConfigPaths::discover().secrets_file);
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
        "secrets_file" => secrets.path,
        "secrets_permissions_status" => secrets.status,
        "secrets_permissions_mode" => secrets.mode,
        "secrets_permissions_message" => secrets.message,
    }]
}

struct SecretsPermissionsDiagnostic {
    path: serde_json::Value,
    status: &'static str,
    mode: serde_json::Value,
    message: String,
}

fn secrets_permissions_diagnostic(
    path: Option<std::path::PathBuf>,
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
                        "secrets file mode is {:o}; 0600 is recommended so `config set --secrets` stays predictable",
                        mode
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
    schema.validate_writable_key(&key).into_diagnostic()?;
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
    let scopes = resolve_config_scopes(read, &target)
        .wrap_err_with(|| format!("failed to resolve config scopes for key `{key}`"))?;
    validate_write_scopes(&key, &scopes).into_diagnostic()?;

    tracing::debug!(
        key = %key,
        store = %config_store_name(store),
        scope_count = scopes.len(),
        dry_run = args.dry_run,
        explain = args.explain,
        "config set"
    );

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
                        .config_overrides
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
                tracing::trace!(
                    key = %key,
                    scope = %format_scope(scope),
                    store = %config_store_name(store),
                    path = %target_path.display(),
                    dry_run = args.dry_run,
                    "persisting config set"
                );

                let set_result = set_scoped_value_in_toml(
                    target_path,
                    &key,
                    &value,
                    scope,
                    store_edit_options(store, args.dry_run),
                )
                .into_diagnostic()
                .wrap_err_with(|| {
                    format!(
                        "failed to persist config set for key `{key}` at {}",
                        target_path.display()
                    )
                })?;

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
        let request = RuntimeConfigRequest::new(
            context.context.profile_override().map(str::to_owned),
            Some(context.context.terminal_kind().as_config_terminal()),
        )
        .with_runtime_load(context.runtime_load)
        .with_product_defaults(context.product_defaults.clone())
        .with_session_layer(Some(context.config_overrides.clone()));
        let explain = explain_runtime_config(request.clone(), &key)
            .wrap_err_with(|| format!("failed to explain config for key `{key}` after set"))?;
        let config = resolve_runtime_config(request)
            .wrap_err_with(|| format!("failed to resolve config for key `{key}` after set"))?;
        if matches!(context.ui.render_settings.format, OutputFormat::Json) {
            let payload = config_explain_json(&explain, &config, false);
            ReplCommandOutput::Json(payload)
        } else {
            ReplCommandOutput::Text(render_config_explain_text(&explain, &config, false))
        }
    } else {
        ReplCommandOutput::Output(crate::app::StructuredCommandOutput {
            source_guide: None,
            output: rows_to_output_result(rows),
            format_hint: None,
        })
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
        stderr_text: None,
        failure_report: None,
    })
}

fn run_config_unset(
    context: ConfigCommandContext<'_>,
    args: ConfigUnsetArgs,
) -> Result<CliCommandResult> {
    let key = args.key.trim().to_ascii_lowercase();
    ConfigSchema::default()
        .validate_writable_key(&key)
        .into_diagnostic()?;
    let target = ConfigWriteTarget::from_unset_args(&args);
    let read = context.read();
    let store = resolve_config_store(read, &target);
    let scopes = resolve_config_scopes(read, &target)
        .wrap_err_with(|| format!("failed to resolve config scopes for key `{key}`"))?;
    validate_write_scopes(&key, &scopes).into_diagnostic()?;

    tracing::debug!(
        key = %key,
        store = %config_store_name(store),
        scope_count = scopes.len(),
        dry_run = args.dry_run,
        "config unset"
    );

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
                    session_scoped_value(context.config_overrides, &key, scope)
                } else {
                    context.config_overrides.remove_scoped(&key, scope)
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
                tracing::trace!(
                    key = %key,
                    scope = %format_scope(scope),
                    store = %config_store_name(store),
                    path = %target_path.display(),
                    dry_run = args.dry_run,
                    "persisting config unset"
                );

                let edit_result = unset_scoped_value_in_toml(
                    target_path,
                    &key,
                    scope,
                    store_edit_options(store, args.dry_run),
                )
                .into_diagnostic()
                .wrap_err_with(|| {
                    format!(
                        "failed to persist config unset for key `{key}` at {}",
                        target_path.display()
                    )
                })?;

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
        output: Some(ReplCommandOutput::Output(
            crate::app::StructuredCommandOutput {
                source_guide: None,
                output: rows_to_output_result(rows),
                format_hint: None,
            },
        )),
        stderr_text: None,
        failure_report: None,
    })
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
    fn from_set_args(args: &ConfigSetArgs) -> Self {
        Self {
            scope: resolve_scope_target(
                args.scope.global,
                args.scope.profile.clone(),
                args.scope.profile_all,
            ),
            terminal: args.scope.terminal.clone(),
            store: resolve_store_target(
                args.store.session,
                args.store.config_store,
                args.store.secrets,
                args.store.save,
            ),
        }
    }

    fn from_unset_args(args: &ConfigUnsetArgs) -> Self {
        Self {
            scope: resolve_scope_target(
                args.scope.global,
                args.scope.profile.clone(),
                args.scope.profile_all,
            ),
            terminal: args.scope.terminal.clone(),
            store: resolve_store_target(
                args.store.session,
                args.store.config_store,
                args.store.secrets,
                args.store.save,
            ),
        }
    }
}

fn resolve_config_store(context: ConfigReadContext<'_>, args: &ConfigWriteTarget) -> ConfigStore {
    match args.store {
        ConfigStoreTarget::Session => ConfigStore::Session,
        ConfigStoreTarget::Config => ConfigStore::Config,
        ConfigStoreTarget::Secrets => ConfigStore::Secrets,
        ConfigStoreTarget::Default => {
            if matches!(context.context.terminal_kind(), TerminalKind::Repl) {
                ConfigStore::Session
            } else {
                ConfigStore::Config
            }
        }
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

fn resolve_config_scopes(
    context: ConfigReadContext<'_>,
    args: &ConfigWriteTarget,
) -> Result<Vec<Scope>> {
    let terminal = resolve_terminal_selector(context, args.terminal.as_deref());

    match &args.scope {
        ConfigScopeTarget::AllProfiles => {
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
            let profile = context.config.active_profile();
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
) -> std::result::Result<(), crate::config::ConfigError> {
    for scope in scopes {
        validate_key_scope(key, scope)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests;
