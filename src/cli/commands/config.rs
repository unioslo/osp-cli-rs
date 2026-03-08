use crate::app::{
    CURRENT_TERMINAL_SENTINEL, CliCommandResult, ConfigExplainContext, ReplCommandOutput,
    RuntimeConfigRequest, config_explain_json, config_explain_result, config_value_to_json,
    document_from_json, document_from_text, explain_runtime_config, format_scope, is_sensitive_key,
    render_config_explain_text, resolve_runtime_config,
};
use crate::app::{RuntimeContext, TerminalKind, UiState};
use crate::cli::rows::output::rows_to_output_result;
use crate::cli::rows::row::RowBuilder;
use crate::cli::{
    ConfigArgs, ConfigCommands, ConfigGetArgs, ConfigSetArgs, ConfigShowArgs, ConfigUnsetArgs,
};
#[cfg(unix)]
use crate::config::secret_file_mode;
use crate::config::{
    ConfigLayer, ConfigSchema, ResolvedConfig, ResolvedValue, RuntimeConfigPaths,
    RuntimeLoadOptions, Scope, is_bootstrap_only_key, set_scoped_value_in_toml,
    unset_scoped_value_in_toml, validate_bootstrap_value, validate_key_scope,
};
use crate::core::output::OutputFormat;
use crate::core::row::Row;
use crate::ui::messages::MessageBuffer;
use crate::ui::theme_loader::ThemeCatalog;
use miette::{IntoDiagnostic, Result, WrapErr, miette};

pub(crate) struct ConfigCommandContext<'a> {
    pub(crate) context: &'a RuntimeContext,
    pub(crate) config: &'a ResolvedConfig,
    pub(crate) ui: &'a UiState,
    pub(crate) themes: &'a ThemeCatalog,
    pub(crate) config_overrides: &'a mut ConfigLayer,
    pub(crate) runtime_load: RuntimeLoadOptions,
}

#[derive(Clone, Copy)]
pub(crate) struct ConfigReadContext<'a> {
    pub(crate) context: &'a RuntimeContext,
    pub(crate) config: &'a ResolvedConfig,
    pub(crate) ui: &'a UiState,
    pub(crate) themes: &'a ThemeCatalog,
    pub(crate) config_overrides: &'a ConfigLayer,
    pub(crate) runtime_load: RuntimeLoadOptions,
}

impl<'a> ConfigCommandContext<'a> {
    fn read(&self) -> ConfigReadContext<'_> {
        ConfigReadContext {
            context: self.context,
            config: self.config,
            ui: self.ui,
            themes: self.themes,
            config_overrides: &*self.config_overrides,
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
        ConfigCommands::Explain(explain) => config_explain_result(
            &ConfigExplainContext {
                context: read.context,
                config: read.config,
                ui: read.ui,
                session_layer: read.config_overrides,
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
            .with_session_layer(Some(context.config_overrides.clone())),
            &args.key,
        )
        .wrap_err_with(|| format!("failed to resolve bootstrap key `{}`", args.key))?;

        if let Some(entry) = explain.final_entry {
            let row = config_entry_row(&args.key, &entry, args.sources, args.raw);
            return Ok(Some(vec![row]));
        }
    }

    messages.error(format!("config key not found: {}", args.key));
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
                    args.dry_run,
                    matches!(store, ConfigStore::Secrets),
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
        .with_session_layer(Some(context.config_overrides.clone()));
        let explain = explain_runtime_config(request.clone(), &key)
            .wrap_err_with(|| format!("failed to explain config for key `{key}` after set"))?;
        let config = resolve_runtime_config(request)
            .wrap_err_with(|| format!("failed to resolve config for key `{key}` after set"))?;
        if matches!(context.ui.render_settings.format, OutputFormat::Json) {
            let payload = config_explain_json(&explain, &config, false);
            ReplCommandOutput::Document(document_from_json(payload))
        } else {
            ReplCommandOutput::Document(document_from_text(&render_config_explain_text(
                &explain, &config, false,
            )))
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
        stderr_text: None,
        failure_report: None,
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
                    args.dry_run,
                    matches!(store, ConfigStore::Secrets),
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
        output: Some(ReplCommandOutput::Output {
            output: rows_to_output_result(rows),
            format_hint: None,
        }),
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
            scope: resolve_scope_target(args.global, args.profile.clone(), args.profile_all),
            terminal: args.terminal.clone(),
            store: resolve_store_target(args.session, args.config_store, args.secrets, args.save),
        }
    }

    fn from_unset_args(args: &ConfigUnsetArgs) -> Self {
        Self {
            scope: resolve_scope_target(args.global, args.profile.clone(), args.profile_all),
            terminal: args.terminal.clone(),
            store: resolve_store_target(args.session, args.config_store, args.secrets, args.save),
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
mod tests {
    use super::{
        ConfigCommandContext, ConfigReadContext, ConfigScopeTarget, ConfigStore, ConfigStoreTarget,
        ConfigWriteTarget, config_diagnostics_rows, config_get_rows, config_store_name,
        resolve_config_scopes, resolve_config_store, resolve_scope_target, resolve_store_target,
        resolve_terminal_selector, run_config_get, run_config_set, run_config_unset,
        secrets_permissions_diagnostic, session_scoped_value, validate_write_scopes,
    };
    use crate::app::ReplCommandOutput;
    use crate::app::{RuntimeContext, TerminalKind, UiState};
    use crate::cli::{ConfigSetArgs, ConfigUnsetArgs};
    use crate::config::{
        ConfigLayer, ConfigResolver, ResolveOptions, ResolvedConfig, RuntimeLoadOptions, Scope,
    };
    use crate::core::output::OutputFormat;
    use crate::ui::RenderSettings;
    use crate::ui::messages::MessageBuffer;
    use crate::ui::messages::MessageLevel;
    use crate::ui::theme_loader::ThemeCatalog;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};

    fn build_resolved_config(
        defaults: ConfigLayer,
        terminal: TerminalKind,
    ) -> &'static ResolvedConfig {
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        Box::leak(Box::new(
            resolver
                .resolve(ResolveOptions::default().with_terminal(terminal.as_config_terminal()))
                .expect("test config should resolve"),
        ))
    }

    fn read_context(terminal: TerminalKind) -> ConfigReadContext<'static> {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        defaults.set("profile.active", "ops");
        let resolved = build_resolved_config(defaults, terminal);
        let context = Box::leak(Box::new(RuntimeContext::new(None, terminal, None)));
        let ui = Box::leak(Box::new(UiState {
            render_settings: RenderSettings::test_plain(OutputFormat::Table),
            message_verbosity: MessageLevel::Success,
            debug_verbosity: 0,
        }));
        let themes = Box::leak(Box::new(ThemeCatalog::default()));
        let config_overrides = Box::leak(Box::new(ConfigLayer::default()));

        ConfigReadContext {
            context,
            config: resolved,
            ui,
            themes,
            config_overrides,
            runtime_load: RuntimeLoadOptions::default(),
        }
    }

    fn write_target(scope: ConfigScopeTarget) -> ConfigWriteTarget {
        ConfigWriteTarget {
            scope,
            terminal: None,
            store: ConfigStoreTarget::Default,
        }
    }

    fn read_context_with_defaults(
        terminal: TerminalKind,
        defaults: ConfigLayer,
    ) -> ConfigReadContext<'static> {
        let resolved = build_resolved_config(defaults, terminal);
        let context = Box::leak(Box::new(RuntimeContext::new(None, terminal, None)));
        let ui = Box::leak(Box::new(UiState {
            render_settings: RenderSettings::test_plain(OutputFormat::Table),
            message_verbosity: MessageLevel::Success,
            debug_verbosity: 0,
        }));
        let themes = Box::leak(Box::new(ThemeCatalog::default()));
        let config_overrides = Box::leak(Box::new(ConfigLayer::default()));

        ConfigReadContext {
            context,
            config: resolved,
            ui,
            themes,
            config_overrides,
            runtime_load: RuntimeLoadOptions::default(),
        }
    }

    fn command_context(terminal: TerminalKind) -> ConfigCommandContext<'static> {
        command_context_with_format(terminal, OutputFormat::Table)
    }

    fn command_context_with_format(
        terminal: TerminalKind,
        format: OutputFormat,
    ) -> ConfigCommandContext<'static> {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        defaults.set("profile.active", "ops");
        let resolved = build_resolved_config(defaults, terminal);
        let context = Box::leak(Box::new(RuntimeContext::new(None, terminal, None)));
        let ui = Box::leak(Box::new(UiState {
            render_settings: RenderSettings::test_plain(format),
            message_verbosity: MessageLevel::Success,
            debug_verbosity: 0,
        }));
        let themes = Box::leak(Box::new(ThemeCatalog::default()));
        let config_overrides = Box::leak(Box::new(ConfigLayer::default()));

        ConfigCommandContext {
            context,
            config: resolved,
            ui,
            themes,
            config_overrides,
            runtime_load: RuntimeLoadOptions::default(),
        }
    }

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn with_temp_config_paths<T>(callback: impl FnOnce(PathBuf, PathBuf) -> T) -> T {
        let _guard = env_lock().lock().expect("env lock should not be poisoned");
        let root = std::env::temp_dir().join(format!(
            "osp-cli-config-tests-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should be valid")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("temp root should exist");
        let config_path = root.join("config.toml");
        let secrets_path = root.join("secrets.toml");
        let previous_config = std::env::var_os("OSP_CONFIG_FILE");
        let previous_secrets = std::env::var_os("OSP_SECRETS_FILE");
        unsafe {
            std::env::set_var("OSP_CONFIG_FILE", &config_path);
            std::env::set_var("OSP_SECRETS_FILE", &secrets_path);
        }

        let result = callback(config_path.clone(), secrets_path.clone());

        match previous_config {
            Some(value) => unsafe { std::env::set_var("OSP_CONFIG_FILE", value) },
            None => unsafe { std::env::remove_var("OSP_CONFIG_FILE") },
        }
        match previous_secrets {
            Some(value) => unsafe { std::env::set_var("OSP_SECRETS_FILE", value) },
            None => unsafe { std::env::remove_var("OSP_SECRETS_FILE") },
        }
        let _ = std::fs::remove_dir_all(root);
        result
    }

    #[test]
    fn resolve_config_store_defaults_to_session_in_repl_and_config_in_cli() {
        let args = write_target(ConfigScopeTarget::ActiveProfile);

        assert!(matches!(
            resolve_config_store(read_context(TerminalKind::Repl), &args),
            ConfigStore::Session
        ));
        assert!(matches!(
            resolve_config_store(read_context(TerminalKind::Cli), &args),
            ConfigStore::Config
        ));
        assert_eq!(config_store_name(ConfigStore::Secrets), "secrets");
    }

    #[test]
    fn resolve_terminal_selector_handles_current_sentinel_and_blank_values() {
        let repl = read_context(TerminalKind::Repl);
        assert_eq!(
            resolve_terminal_selector(repl, Some(crate::app::CURRENT_TERMINAL_SENTINEL)),
            Some("repl".to_string())
        );
        assert_eq!(resolve_terminal_selector(repl, Some("  ")), None);
        assert_eq!(
            resolve_terminal_selector(repl, Some("CLI")),
            Some("cli".to_string())
        );
    }

    #[test]
    fn resolve_config_scopes_handles_profile_all_global_and_terminal_overrides() {
        let cli = read_context(TerminalKind::Cli);

        let global_scopes = resolve_config_scopes(
            cli,
            &ConfigWriteTarget {
                scope: ConfigScopeTarget::Global,
                terminal: Some("cli".to_string()),
                store: ConfigStoreTarget::Default,
            },
        )
        .expect("global scopes should resolve");
        assert_eq!(global_scopes, vec![Scope::terminal("cli")]);

        let all_profile_scopes = resolve_config_scopes(
            cli,
            &ConfigWriteTarget {
                scope: ConfigScopeTarget::AllProfiles,
                terminal: Some("cli".to_string()),
                store: ConfigStoreTarget::Default,
            },
        )
        .expect("profile-all scopes should resolve");
        assert_eq!(
            all_profile_scopes,
            vec![Scope::profile_terminal("default", "cli")]
        );
    }

    #[test]
    fn resolve_config_store_honors_explicit_targets_over_defaults_unit() {
        let repl = read_context(TerminalKind::Repl);

        assert!(matches!(
            resolve_config_store(
                repl,
                &ConfigWriteTarget {
                    scope: ConfigScopeTarget::ActiveProfile,
                    terminal: None,
                    store: ConfigStoreTarget::Session,
                }
            ),
            ConfigStore::Session
        ));
        assert!(matches!(
            resolve_config_store(
                repl,
                &ConfigWriteTarget {
                    scope: ConfigScopeTarget::ActiveProfile,
                    terminal: None,
                    store: ConfigStoreTarget::Secrets,
                }
            ),
            ConfigStore::Secrets
        ));
        assert!(matches!(
            resolve_config_store(
                repl,
                &ConfigWriteTarget {
                    scope: ConfigScopeTarget::ActiveProfile,
                    terminal: None,
                    store: ConfigStoreTarget::Config,
                }
            ),
            ConfigStore::Config
        ));
        assert_eq!(config_store_name(ConfigStore::Session), "session");
        assert_eq!(config_store_name(ConfigStore::Config), "config");
    }

    #[test]
    fn resolve_config_scopes_uses_explicit_profile_without_terminal_unit() {
        let scopes = resolve_config_scopes(
            read_context(TerminalKind::Cli),
            &ConfigWriteTarget {
                scope: ConfigScopeTarget::Profile("Work".to_string()),
                terminal: None,
                store: ConfigStoreTarget::Default,
            },
        )
        .expect("profile scope should resolve");

        assert_eq!(scopes, vec![Scope::profile("Work")]);
    }

    #[cfg(unix)]
    #[test]
    fn secrets_permissions_diagnostic_warns_for_owner_only_non_600_modes_unit() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!(
            "osp-cli-config-secrets-diagnostic-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should be valid")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir should exist");
        let path = dir.join("secrets.toml");
        std::fs::write(&path, "token = 'secret'\n").expect("fixture should be written");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400))
            .expect("permissions should be set");

        let diagnostic = secrets_permissions_diagnostic(Some(path.clone()));
        assert_eq!(diagnostic.status, "warning");
        assert_eq!(
            diagnostic.mode,
            serde_json::Value::String("400".to_string())
        );
        assert!(diagnostic.message.contains("0600 is recommended"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn config_get_rows_resolves_bootstrap_only_keys_through_runtime_explain_unit() {
        let mut messages = MessageBuffer::default();
        let rows = config_get_rows(
            read_context(TerminalKind::Cli),
            &crate::cli::ConfigGetArgs {
                key: "profile.default".to_string(),
                sources: true,
                raw: false,
            },
            &mut messages,
        )
        .expect("bootstrap-only get should resolve")
        .expect("bootstrap-only key should produce a row");

        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].get("key").and_then(|value| value.as_str()),
            Some("profile.default")
        );
        assert_eq!(
            rows[0].get("source").and_then(|value| value.as_str()),
            Some("defaults")
        );
        assert!(messages.is_empty());
    }

    #[test]
    fn run_config_get_covers_alias_hit_and_missing_key_paths_unit() {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        defaults.set("profile.active", "ops");
        defaults.set("alias.lookup", "ldap user");
        let context = read_context_with_defaults(TerminalKind::Cli, defaults);

        let alias_result = run_config_get(
            context,
            crate::cli::ConfigGetArgs {
                key: "lookup".to_string(),
                sources: false,
                raw: false,
            },
        )
        .expect("alias lookup should succeed");

        assert_eq!(alias_result.exit_code, 0);
        assert!(matches!(
            alias_result.output,
            Some(ReplCommandOutput::Output { .. })
        ));

        let missing_result = run_config_get(
            context,
            crate::cli::ConfigGetArgs {
                key: "missing.key".to_string(),
                sources: false,
                raw: false,
            },
        )
        .expect("missing key should return a structured miss");

        assert_eq!(missing_result.exit_code, 1);
        assert!(missing_result.output.is_none());
        assert!(
            missing_result
                .messages
                .render_grouped(MessageLevel::Error)
                .contains("config key not found: missing.key")
        );
    }

    #[test]
    fn resolve_scope_and_store_targets_cover_precedence_rules_unit() {
        assert!(matches!(
            resolve_scope_target(false, None, false),
            ConfigScopeTarget::ActiveProfile
        ));
        assert!(matches!(
            resolve_scope_target(true, Some("ops".to_string()), false),
            ConfigScopeTarget::Global
        ));
        assert!(matches!(
            resolve_scope_target(false, Some("ops".to_string()), false),
            ConfigScopeTarget::Profile(profile) if profile == "ops"
        ));
        assert!(matches!(
            resolve_scope_target(false, Some("ops".to_string()), true),
            ConfigScopeTarget::AllProfiles
        ));

        assert_eq!(
            resolve_store_target(true, true, true, true),
            ConfigStoreTarget::Session
        );
        assert_eq!(
            resolve_store_target(false, true, true, true),
            ConfigStoreTarget::Config
        );
        assert_eq!(
            resolve_store_target(false, false, true, false),
            ConfigStoreTarget::Secrets
        );
        assert_eq!(
            resolve_store_target(false, false, false, true),
            ConfigStoreTarget::Config
        );
        assert_eq!(
            resolve_store_target(false, false, false, false),
            ConfigStoreTarget::Default
        );
    }

    #[test]
    fn resolve_config_scopes_covers_known_profiles_and_active_profile_terminal_unit() {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "ops");
        defaults.set("profile.active", "ops");
        defaults.insert(
            "ui.format".to_string(),
            crate::config::ConfigValue::from("json"),
            Scope::profile("ops"),
        );
        defaults.insert(
            "ui.format".to_string(),
            crate::config::ConfigValue::from("table"),
            Scope::profile("dev"),
        );
        let context = read_context_with_defaults(TerminalKind::Cli, defaults);

        let all_profiles = resolve_config_scopes(
            context,
            &ConfigWriteTarget {
                scope: ConfigScopeTarget::AllProfiles,
                terminal: None,
                store: ConfigStoreTarget::Default,
            },
        )
        .expect("all known profile scopes should resolve");
        assert_eq!(
            all_profiles,
            vec![Scope::profile("dev"), Scope::profile("ops")]
        );

        let active_profile_terminal = resolve_config_scopes(
            context,
            &ConfigWriteTarget {
                scope: ConfigScopeTarget::ActiveProfile,
                terminal: Some("cli".to_string()),
                store: ConfigStoreTarget::Default,
            },
        )
        .expect("active profile terminal scope should resolve");
        assert_eq!(
            active_profile_terminal,
            vec![Scope::profile_terminal("ops", "cli")]
        );
    }

    #[test]
    fn validate_write_scopes_and_session_lookup_cover_invalid_and_present_paths_unit() {
        let mut layer = ConfigLayer::default();
        layer.insert(
            "ui.format".to_string(),
            crate::config::ConfigValue::from("json"),
            Scope::profile("ops"),
        );
        assert_eq!(
            session_scoped_value(&layer, "ui.format", &Scope::profile("ops")),
            Some(crate::config::ConfigValue::from("json"))
        );
        assert_eq!(
            session_scoped_value(&layer, "ui.format", &Scope::profile("dev")),
            None
        );

        assert!(
            validate_write_scopes("profile.default", &[Scope::profile("ops")]).is_err(),
            "bootstrap-only key should reject profile scope"
        );
    }

    #[test]
    fn run_config_set_and_unset_cover_session_paths_unit() {
        let set_result = run_config_set(
            command_context(TerminalKind::Repl),
            ConfigSetArgs {
                key: "ui.format".to_string(),
                value: "json".to_string(),
                global: false,
                profile: None,
                profile_all: false,
                terminal: None,
                session: false,
                config_store: false,
                secrets: false,
                save: false,
                yes: false,
                explain: false,
                dry_run: false,
            },
        )
        .expect("session config set should succeed");

        assert_eq!(set_result.exit_code, 0);
        assert!(matches!(
            set_result.output,
            Some(ReplCommandOutput::Output { .. })
        ));
        assert!(
            set_result
                .messages
                .render_grouped(MessageLevel::Success)
                .contains("set value for ui.format")
        );

        let unset_context = command_context(TerminalKind::Repl);
        let active_profile = unset_context.config.active_profile().to_string();
        unset_context.config_overrides.insert(
            "ui.format".to_string(),
            crate::config::ConfigValue::from("json"),
            Scope::profile(&active_profile),
        );
        let unset_result = run_config_unset(
            unset_context,
            ConfigUnsetArgs {
                key: "ui.format".to_string(),
                global: false,
                profile: None,
                profile_all: false,
                terminal: None,
                session: false,
                config_store: false,
                secrets: false,
                save: false,
                dry_run: false,
            },
        )
        .expect("session config unset should succeed");

        assert!(matches!(
            unset_result.output,
            Some(ReplCommandOutput::Output { .. })
        ));
        assert!(
            unset_result
                .messages
                .render_grouped(MessageLevel::Success)
                .contains("unset value for ui.format")
        );
    }

    #[test]
    fn run_config_set_covers_session_explain_json_output_unit() {
        let result = run_config_set(
            command_context_with_format(TerminalKind::Repl, OutputFormat::Json),
            ConfigSetArgs {
                key: "ui.format".to_string(),
                value: "json".to_string(),
                global: false,
                profile: None,
                profile_all: false,
                terminal: None,
                session: false,
                config_store: false,
                secrets: false,
                save: false,
                yes: false,
                explain: true,
                dry_run: false,
            },
        )
        .expect("session config set explain should succeed");

        assert!(matches!(
            result.output,
            Some(ReplCommandOutput::Document(_))
        ));
    }

    #[test]
    fn run_config_set_and_unset_cover_persistent_paths_and_warning_unit() {
        with_temp_config_paths(|config_path, secrets_path| {
            let config_set = run_config_set(
                command_context(TerminalKind::Cli),
                ConfigSetArgs {
                    key: "ui.format".to_string(),
                    value: "json".to_string(),
                    global: false,
                    profile: None,
                    profile_all: false,
                    terminal: None,
                    session: false,
                    config_store: true,
                    secrets: false,
                    save: false,
                    yes: false,
                    explain: false,
                    dry_run: false,
                },
            )
            .expect("persistent config set should succeed");
            assert!(config_path.exists());
            assert!(
                std::fs::read_to_string(&config_path)
                    .expect("config file should be readable")
                    .contains("format = \"json\"")
            );
            assert!(
                config_set
                    .messages
                    .render_grouped(MessageLevel::Success)
                    .contains("set value for ui.format")
            );

            let secrets_set = run_config_set(
                command_context(TerminalKind::Cli),
                ConfigSetArgs {
                    key: "ui.format".to_string(),
                    value: "table".to_string(),
                    global: false,
                    profile: None,
                    profile_all: false,
                    terminal: None,
                    session: false,
                    config_store: false,
                    secrets: true,
                    save: false,
                    yes: false,
                    explain: false,
                    dry_run: false,
                },
            )
            .expect("persistent secrets set should succeed");
            assert!(secrets_path.exists());
            assert!(
                std::fs::read_to_string(&secrets_path)
                    .expect("secrets file should be readable")
                    .contains("format = \"table\"")
            );
            assert!(
                secrets_set
                    .messages
                    .render_grouped(MessageLevel::Success)
                    .contains("set value for ui.format")
            );

            let secrets_unset = run_config_unset(
                command_context(TerminalKind::Cli),
                ConfigUnsetArgs {
                    key: "ui.format".to_string(),
                    global: false,
                    profile: None,
                    profile_all: false,
                    terminal: None,
                    session: false,
                    config_store: false,
                    secrets: true,
                    save: false,
                    dry_run: false,
                },
            )
            .expect("persistent secrets unset should succeed");
            assert!(
                secrets_unset
                    .messages
                    .render_grouped(MessageLevel::Success)
                    .contains("unset value for ui.format")
            );

            let missing_unset = run_config_unset(
                command_context(TerminalKind::Cli),
                ConfigUnsetArgs {
                    key: "ui.margin".to_string(),
                    global: false,
                    profile: None,
                    profile_all: false,
                    terminal: None,
                    session: false,
                    config_store: true,
                    secrets: false,
                    save: false,
                    dry_run: false,
                },
            )
            .expect("missing persistent unset should still succeed");
            assert!(
                missing_unset
                    .messages
                    .render_grouped(MessageLevel::Warning)
                    .contains("no matching value for ui.margin")
            );
        });
    }

    #[cfg(unix)]
    #[test]
    fn secrets_permissions_diagnostic_covers_unavailable_missing_ok_and_issue_unit() {
        use std::os::unix::fs::PermissionsExt;

        let missing = secrets_permissions_diagnostic(None);
        assert_eq!(missing.status, "unavailable");

        let dir = std::env::temp_dir().join(format!(
            "osp-cli-config-secrets-diagnostic-extra-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should be valid")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir should exist");

        let absent_path = dir.join("missing.toml");
        let absent = secrets_permissions_diagnostic(Some(absent_path));
        assert_eq!(absent.status, "missing");

        let ok_path = dir.join("ok.toml");
        std::fs::write(&ok_path, "token = 'secret'\n").expect("fixture should be written");
        std::fs::set_permissions(&ok_path, std::fs::Permissions::from_mode(0o600))
            .expect("permissions should be set");
        let ok = secrets_permissions_diagnostic(Some(ok_path.clone()));
        assert_eq!(ok.status, "ok");
        assert_eq!(ok.mode, serde_json::Value::String("600".to_string()));

        let issue_path = dir.join("issue.toml");
        std::fs::write(&issue_path, "token = 'secret'\n").expect("fixture should be written");
        std::fs::set_permissions(&issue_path, std::fs::Permissions::from_mode(0o644))
            .expect("permissions should be set");
        let issue = secrets_permissions_diagnostic(Some(issue_path));
        assert_eq!(issue.status, "issue");
        assert!(
            issue
                .message
                .contains("owner-only permissions are required")
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn config_diagnostics_rows_include_secrets_status_unit() {
        let rows = config_diagnostics_rows(read_context(TerminalKind::Cli));
        assert_eq!(rows.len(), 1);
        assert!(rows[0].contains_key("secrets_permissions_status"));
        assert!(rows[0].contains_key("theme_issue_count"));
    }
}
