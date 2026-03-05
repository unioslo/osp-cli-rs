use crate::app::{
    CURRENT_TERMINAL_SENTINEL, CliCommandResult, ReplCommandOutput, config_explain_json,
    config_explain_output, config_usize, config_value_to_json, emit_messages,
    explain_runtime_config, format_scope, is_sensitive_key, render_config_explain_text,
    resolve_runtime_config,
};
use crate::cli::{ConfigArgs, ConfigCommands, ConfigGetArgs, ConfigSetArgs, ConfigShowArgs};
use crate::rows::RowBuilder;
use crate::rows::output::rows_to_output_result;
use crate::state::{AppState, AuthState, TerminalKind};
use crate::theme_loader;
use miette::{IntoDiagnostic, Result, WrapErr, miette};
use osp_config::{
    ConfigSchema, DEFAULT_SESSION_CACHE_MAX_RESULTS, DEFAULT_UI_WIDTH, ResolvedValue,
    RuntimeConfigPaths, Scope, set_scoped_value_in_toml,
};
use osp_core::output::OutputFormat;
use osp_core::row::Row;
use osp_ui::messages::MessageBuffer;
use osp_ui::theme::DEFAULT_THEME_NAME;

pub(crate) fn run_config_command(
    state: &mut AppState,
    args: ConfigArgs,
) -> Result<CliCommandResult> {
    match args.command {
        ConfigCommands::Show(show) => Ok(CliCommandResult::output(
            rows_to_output_result(config_show_rows(state, show)),
            None,
        )),
        ConfigCommands::Get(get) => match config_get_rows(state, get)? {
            Some(rows) => Ok(CliCommandResult::output(rows_to_output_result(rows), None)),
            None => Ok(CliCommandResult::exit(1)),
        },
        ConfigCommands::Explain(explain) => match config_explain_output(state, explain)? {
            Some(output) => Ok(CliCommandResult::text(output)),
            None => Ok(CliCommandResult::exit(1)),
        },
        ConfigCommands::Set(set) => Ok(CliCommandResult {
            exit_code: 0,
            output: Some(run_config_set(state, set)?),
        }),
        ConfigCommands::Diagnostics => Ok(CliCommandResult::output(
            rows_to_output_result(config_diagnostics_rows(state)),
            None,
        )),
    }
}

fn config_show_rows(state: &AppState, args: ConfigShowArgs) -> Vec<Row> {
    state
        .config
        .resolved()
        .values()
        .iter()
        .map(|(key, entry)| config_entry_row(key, entry, args.sources, args.raw))
        .collect::<Vec<Row>>()
}

fn config_get_rows(state: &AppState, args: ConfigGetArgs) -> Result<Option<Vec<Row>>> {
    let Some(entry) = state.config.resolved().get_value_entry(&args.key) else {
        let mut messages = MessageBuffer::default();
        messages.error(format!("config key not found: {}", args.key));
        emit_messages(state, &messages);
        return Ok(None);
    };

    let row = config_entry_row(&args.key, entry, args.sources, args.raw);
    Ok(Some(vec![row]))
}

fn config_diagnostics_rows(state: &AppState) -> Vec<Row> {
    let known_profiles = serde_json::Value::Array(
        state
            .config
            .resolved()
            .known_profiles()
            .iter()
            .map(|value| value.clone().into())
            .collect(),
    );
    vec![crate::row! {
        "status" => "ok",
        "active_profile" => state.config.resolved().active_profile().to_string(),
        "known_profiles" => known_profiles,
        "resolved_keys" => state.config.resolved().values().len() as i64,
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

fn run_config_set(state: &mut AppState, args: ConfigSetArgs) -> Result<ReplCommandOutput> {
    let key = args.key.trim().to_ascii_lowercase();
    let schema = ConfigSchema::default();
    let value = schema
        .parse_input_value(&key, &args.value)
        .into_diagnostic()
        .wrap_err("invalid value for key")?;
    let store = resolve_config_store(state, &args);
    let scopes = resolve_config_scopes(state, &args)?;

    let mut rows = Vec::new();
    let mut messages = MessageBuffer::default();
    if matches!(store, ConfigStore::Config) && is_sensitive_key(&key) {
        messages.warning("writing a sensitive key to config store; prefer --secrets");
    }

    let paths = RuntimeConfigPaths::discover();
    for scope in &scopes {
        let mut row = RowBuilder::new();
        row.insert("key", key.clone());
        row.insert("value", config_value_to_json(&value));
        row.insert("scope", format_scope(scope));
        row.insert("store", config_store_name(store));
        row.insert("dry_run", args.dry_run);

        match store {
            ConfigStore::Session => {
                if !args.dry_run {
                    state.session.config_overrides.insert(
                        key.clone(),
                        value.clone(),
                        scope.clone(),
                    );
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
                        .map(config_value_to_json)
                        .unwrap_or(serde_json::Value::Null),
                );
            }
        }

        rows.push(row.build());
    }

    if !args.dry_run {
        refresh_runtime_config(state)?;
    }

    let output = if args.explain {
        let explain = explain_runtime_config(
            Some(state.config.resolved().active_profile().to_string()),
            state.config.resolved().terminal(),
            &key,
            Some(state.session.config_overrides.clone()),
        )?;
        if matches!(state.ui.render_settings.format, OutputFormat::Json) {
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
    emit_messages(state, &messages);
    Ok(output)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigStore {
    Session,
    Config,
    Secrets,
}

fn resolve_config_store(state: &AppState, args: &ConfigSetArgs) -> ConfigStore {
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
    if matches!(state.context.terminal_kind(), TerminalKind::Repl) {
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

fn resolve_config_scopes(state: &AppState, args: &ConfigSetArgs) -> Result<Vec<Scope>> {
    let terminal = resolve_terminal_selector(state, args.terminal.as_deref());

    if args.profile_all {
        let profiles = if state.config.resolved().known_profiles().is_empty() {
            vec![state.config.resolved().active_profile().to_string()]
        } else {
            state
                .config
                .resolved()
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
        .unwrap_or_else(|| state.config.resolved().active_profile());
    Ok(vec![terminal.as_deref().map_or_else(
        || Scope::profile(profile),
        |current| Scope::profile_terminal(profile, current),
    )])
}

fn resolve_terminal_selector(state: &AppState, selector: Option<&str>) -> Option<String> {
    let value = selector?;
    if value == CURRENT_TERMINAL_SENTINEL {
        return Some(
            state
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

fn refresh_runtime_config(state: &mut AppState) -> Result<()> {
    let next = resolve_runtime_config(
        state.context.profile_override().map(ToOwned::to_owned),
        Some(state.context.terminal_kind().as_config_terminal()),
        Some(state.session.config_overrides.clone()),
    )?;
    let changed = state.config.replace_resolved(next);
    if changed {
        state.clients.sync_config_revision(state.config.revision());
        state.auth = AuthState::from_resolved(state.config.resolved());
        let theme_load = theme_loader::load_custom_themes(state.config.resolved());
        osp_ui::theme::set_custom_themes(theme_load.themes);
        theme_loader::log_theme_issues(&theme_load.issues);
        state.ui.render_settings.theme_name = state
            .config
            .resolved()
            .get_string("theme.name")
            .unwrap_or(DEFAULT_THEME_NAME)
            .to_string();
        state.ui.render_settings.width = Some(config_usize(
            state.config.resolved(),
            "ui.width",
            DEFAULT_UI_WIDTH as usize,
        ));
        state.session.max_cached_results = config_usize(
            state.config.resolved(),
            "session.cache.max_results",
            DEFAULT_SESSION_CACHE_MAX_RESULTS as usize,
        );
    }
    Ok(())
}

pub(crate) fn run_config_repl_command(
    state: &mut AppState,
    args: ConfigArgs,
) -> Result<ReplCommandOutput> {
    match args.command {
        ConfigCommands::Show(show) => Ok(ReplCommandOutput::Output {
            output: rows_to_output_result(config_show_rows(state, show)),
            format_hint: None,
        }),
        ConfigCommands::Get(get) => match config_get_rows(state, get)? {
            Some(rows) => Ok(ReplCommandOutput::Output {
                output: rows_to_output_result(rows),
                format_hint: None,
            }),
            None => Ok(ReplCommandOutput::Text(String::new())),
        },
        ConfigCommands::Explain(explain) => match config_explain_output(state, explain)? {
            Some(output) => Ok(ReplCommandOutput::Text(output)),
            None => Ok(ReplCommandOutput::Text(String::new())),
        },
        ConfigCommands::Set(set) => run_config_set(state, set),
        ConfigCommands::Diagnostics => Ok(ReplCommandOutput::Output {
            output: rows_to_output_result(config_diagnostics_rows(state)),
            format_hint: None,
        }),
    }
}
