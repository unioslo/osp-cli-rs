use clap::Parser;
use miette::{Result, miette};
use osp_config::{ConfigValue, DEFAULT_UI_WIDTH, ResolvedConfig};
use osp_core::output::OutputFormat;
use osp_core::runtime::{RuntimeHints, RuntimeTerminalKind, UiVerbosity};

use osp_ui::messages::MessageLevel;
use osp_ui::theme::{DEFAULT_THEME_NAME, normalize_theme_name};
use osp_ui::{RenderRuntime, RenderSettings, render_output};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::io::IsTerminal;
use terminal_size::{Width, terminal_size};

use crate::cli::commands::{
    config as config_cmd, doctor as doctor_cmd, history as history_cmd, plugins as plugins_cmd,
    theme as theme_cmd,
};
use crate::cli::{Cli, Commands};
use crate::logging::init_developer_logging;
use crate::plugin_manager::{
    CommandCatalogEntry, PluginDispatchContext, PluginDispatchError, PluginManager,
};
use crate::state::{AppClients, AppRuntime, AppSession, AuthState, LaunchContext, TerminalKind};

use crate::repl;

mod bootstrap;
mod command_output;
mod config_explain;
mod dispatch;
mod external;
mod help;
mod repl_lifecycle;

use crate::repl::help as repl_help;
use crate::theme_loader;
pub(crate) use bootstrap::{
    RuntimeConfigRequest, build_app_state, build_cli_session_layer, build_logging_config,
    build_runtime_context, effective_debug_verbosity, effective_message_verbosity,
    resolve_runtime_config,
};
pub(crate) use command_output::{
    CliCommandResult, CommandRenderRuntime, PreparedPluginResponse, ReplCommandOutput,
    apply_output_stages, emit_messages_for_ui, emit_messages_with_runtime,
    maybe_copy_output_with_runtime, prepare_plugin_response, run_cli_command,
};
pub(crate) use config_explain::{
    ConfigExplainContext, config_explain_json, config_explain_output, config_value_to_json,
    explain_runtime_config, format_scope, is_sensitive_key, render_config_explain_text,
};
pub(crate) use dispatch::{
    RunAction, build_dispatch_plan, ensure_builtin_visible_for, ensure_dispatch_visibility,
    ensure_plugin_visible_for, normalize_cli_profile, normalize_profile_override,
};
pub(crate) use external::is_help_passthrough;
use external::{ExternalCommandRuntime, run_external_command};
pub(crate) use repl_lifecycle::rebuild_repl_parts;
#[cfg(test)]
pub(crate) use repl_lifecycle::rebuild_repl_state;

pub(crate) const CMD_PLUGINS: &str = "plugins";
pub(crate) const CMD_DOCTOR: &str = "doctor";
pub(crate) const CMD_CONFIG: &str = "config";
pub(crate) const CMD_THEME: &str = "theme";
pub(crate) const CMD_HISTORY: &str = "history";
pub(crate) const CMD_HELP: &str = "help";
pub(crate) const CMD_LIST: &str = "list";
pub(crate) const CMD_SHOW: &str = "show";
pub(crate) const CMD_USE: &str = "use";
pub(crate) const DEFAULT_REPL_PROMPT: &str = "╭─{user}@{domain} {indicator}\n╰─{profile}> ";
pub(crate) const CURRENT_TERMINAL_SENTINEL: &str = "__current__";
pub(crate) const REPL_SHELLABLE_COMMANDS: [&str; 5] = ["nh", "mreg", "ldap", "vm", "orch"];
const SHARED_PLUGIN_ENV_PREFIX: &str = "extensions.plugins.env.";
const PLUGIN_ENV_ROOT_PREFIX: &str = "extensions.plugins.";
const PLUGIN_ENV_SEPARATOR: &str = ".env.";
const PLUGIN_CONFIG_ENV_PREFIX: &str = "OSP_PLUGIN_CFG_";

#[derive(Debug, Clone)]
pub(crate) struct ReplCommandSpec {
    pub(crate) name: Cow<'static, str>,
    pub(crate) supports_dsl: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ReplDispatchOverrides {
    pub(crate) message_verbosity: MessageLevel,
    pub(crate) debug_verbosity: u8,
}

#[derive(Debug, Clone, Default)]
struct PluginConfigEnv {
    shared: Vec<PluginConfigEntry>,
    by_plugin_id: HashMap<String, Vec<PluginConfigEntry>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PluginConfigScope {
    Shared,
    Plugin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PluginConfigEntry {
    pub(crate) env_key: String,
    pub(crate) value: String,
    pub(crate) config_key: String,
    pub(crate) scope: PluginConfigScope,
}

pub fn run_from<I, T>(args: I) -> Result<i32>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let argv = args.into_iter().map(Into::into).collect::<Vec<OsString>>();
    match Cli::try_parse_from(argv.iter().cloned()) {
        Ok(cli) => run(cli),
        Err(err) => handle_clap_parse_error(&argv, err),
    }
}

fn handle_clap_parse_error(args: &[OsString], err: clap::Error) -> Result<i32> {
    match err.kind() {
        clap::error::ErrorKind::DisplayHelp => {
            let settings = help::render_settings_for_help(args);
            let rendered = repl_help::render_help_with_chrome(
                &err.to_string(),
                &settings.resolve_render_settings(),
            );
            print!("{rendered}");
            Ok(0)
        }
        clap::error::ErrorKind::DisplayVersion => {
            print!("{err}");
            Ok(0)
        }
        _ => Err(miette!(err.to_string())),
    }
}

// Keep the top-level CLI entrypoint readable as a table of contents:
// normalize input -> bootstrap runtime state -> hand off to the selected mode.
fn run(mut cli: Cli) -> Result<i32> {
    let normalized_profile = normalize_cli_profile(&mut cli);
    let runtime_load = cli.runtime_load_options();
    // Startup resolves config in three phases:
    // 1. bootstrap once to discover known profiles
    // 2. build the session layer, including derived overrides
    // 3. resolve again with the full session layer applied
    let initial_config = resolve_runtime_config(
        RuntimeConfigRequest::new(normalized_profile.clone(), Some("cli"))
            .with_runtime_load(runtime_load),
    )?;
    let known_profiles = initial_config.known_profiles().clone();
    let dispatch = build_dispatch_plan(&mut cli, &known_profiles)?;

    let terminal_kind = dispatch.action.terminal_kind();
    let runtime_context = build_runtime_context(dispatch.profile_override.clone(), terminal_kind);
    let session_layer = build_cli_session_layer(
        &cli,
        runtime_context.profile_override().map(ToOwned::to_owned),
        runtime_context.terminal_kind(),
        runtime_load,
    )?;
    let launch_context = LaunchContext {
        plugin_dirs: cli.plugin_dirs.clone(),
        config_root: None,
        cache_root: None,
        runtime_load,
    };

    let config = resolve_runtime_config(
        RuntimeConfigRequest::new(
            runtime_context.profile_override().map(ToOwned::to_owned),
            Some(runtime_context.terminal_kind().as_config_terminal()),
        )
        .with_runtime_load(launch_context.runtime_load)
        .with_session_layer(session_layer.clone()),
    )?;
    let theme_catalog = theme_loader::load_theme_catalog(&config);
    let mut render_settings = cli.render_settings();
    render_settings.runtime = build_render_runtime(runtime_context.terminal_env());
    crate::cli::apply_render_settings_from_config(&mut render_settings, &config);
    render_settings.width = Some(resolve_default_render_width(&config));
    render_settings.theme_name = resolve_theme_name(&cli, &config, &theme_catalog)?;
    render_settings.theme = theme_catalog
        .resolve(&render_settings.theme_name)
        .map(|entry| entry.theme.clone());
    let message_verbosity = effective_message_verbosity(&config);
    let debug_verbosity = effective_debug_verbosity(&config);
    init_developer_logging(build_logging_config(&config, debug_verbosity));
    theme_loader::log_theme_issues(&theme_catalog.issues);
    tracing::debug!(
        debug_count = debug_verbosity,
        "developer logging initialized"
    );

    let mut state = build_app_state(
        runtime_context,
        config,
        render_settings,
        message_verbosity,
        debug_verbosity,
        PluginManager::new(cli.plugin_dirs.clone()),
        theme_catalog.clone(),
        launch_context,
    );
    if let Some(layer) = session_layer {
        state.session.config_overrides = layer;
    }
    ensure_dispatch_visibility(&state.runtime.auth, &dispatch.action)?;

    tracing::info!(
        profile = %state.runtime.config.resolved().active_profile(),
        terminal = %state.runtime.context.terminal_kind().as_config_terminal(),
        "osp session initialized"
    );

    match dispatch.action {
        RunAction::Repl => repl::run_plugin_repl(&mut state),
        RunAction::ReplCommand(args) => run_builtin_cli_command_parts(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            Commands::Repl(args),
        ),
        RunAction::Plugins(args) => run_builtin_cli_command_parts(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            Commands::Plugins(args),
        ),
        RunAction::Doctor(args) => run_builtin_cli_command_parts(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            Commands::Doctor(args),
        ),
        RunAction::Theme(args) => run_builtin_cli_command_parts(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            Commands::Theme(args),
        ),
        RunAction::Config(args) => run_builtin_cli_command_parts(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            Commands::Config(args),
        ),
        RunAction::History(args) => run_builtin_cli_command_parts(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            Commands::History(args),
        ),
        RunAction::External(tokens) => run_external_command(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            &tokens,
        ),
    }
}

pub(crate) fn authorized_command_catalog_for(
    auth: &AuthState,
    plugins: &PluginManager,
) -> Result<Vec<CommandCatalogEntry>> {
    let all = plugins
        .command_catalog()
        .map_err(|err| miette!("{err:#}"))?;
    Ok(all
        .into_iter()
        .filter(|entry| auth.is_plugin_command_visible(&entry.name))
        .collect())
}

fn run_builtin_cli_command_parts(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    command: Commands,
) -> Result<i32> {
    let result = dispatch_builtin_command_parts(runtime, session, clients, command)?
        .ok_or_else(|| miette!("expected builtin command"))?;
    run_cli_command(
        &CommandRenderRuntime::new(runtime.config.resolved(), &runtime.ui),
        result,
    )
}

fn run_inline_builtin_command(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    command: Commands,
    stages: &[String],
) -> Result<Option<CliCommandResult>> {
    if matches!(command, Commands::External(_)) {
        return Ok(None);
    }

    let spec = repl::repl_command_spec(&command);
    ensure_command_supports_dsl(&spec, stages)?;
    dispatch_builtin_command_parts(runtime, session, clients, command)
}

fn dispatch_builtin_command_parts(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    command: Commands,
) -> Result<Option<CliCommandResult>> {
    match command {
        Commands::Plugins(args) => {
            ensure_builtin_visible_for(&runtime.auth, CMD_PLUGINS)?;
            plugins_cmd::run_plugins_command(
                plugins_cmd::PluginsCommandContext {
                    config: runtime.config.resolved(),
                    ui: &runtime.ui,
                    auth: &runtime.auth,
                    plugin_manager: &clients.plugins,
                },
                args,
            )
            .map(Some)
        }
        Commands::Doctor(args) => {
            ensure_builtin_visible_for(&runtime.auth, CMD_DOCTOR)?;
            doctor_cmd::run_doctor_command(
                doctor_cmd::DoctorCommandContext {
                    config: config_cmd::ConfigReadContext {
                        context: &runtime.context,
                        config: runtime.config.resolved(),
                        ui: &runtime.ui,
                        themes: &runtime.themes,
                        session_layer: &session.config_overrides,
                        runtime_load: runtime.launch.runtime_load,
                    },
                    plugins: plugins_cmd::PluginsCommandContext {
                        config: runtime.config.resolved(),
                        ui: &runtime.ui,
                        auth: &runtime.auth,
                        plugin_manager: &clients.plugins,
                    },
                    ui: &runtime.ui,
                    auth: &runtime.auth,
                    themes: &runtime.themes,
                    last_failure: session.last_failure.as_ref(),
                },
                args,
            )
            .map(Some)
        }
        Commands::Theme(args) => {
            ensure_builtin_visible_for(&runtime.auth, CMD_THEME)?;
            let config = runtime.config.resolved();
            let ui = &runtime.ui;
            let themes = &runtime.themes;
            theme_cmd::run_theme_command(
                &mut session.config_overrides,
                theme_cmd::ThemeCommandContext { config, ui, themes },
                args,
            )
            .map(Some)
        }
        Commands::Config(args) => {
            ensure_builtin_visible_for(&runtime.auth, CMD_CONFIG)?;
            let context = &runtime.context;
            let config = runtime.config.resolved();
            let ui = &runtime.ui;
            let themes = &runtime.themes;
            let runtime_load = runtime.launch.runtime_load;
            config_cmd::run_config_command(
                config_cmd::ConfigCommandContext {
                    context,
                    config,
                    ui,
                    themes,
                    session_overrides: &mut session.config_overrides,
                    runtime_load,
                },
                args,
            )
            .map(Some)
        }
        Commands::History(args) => {
            ensure_builtin_visible_for(&runtime.auth, CMD_HISTORY)?;
            history_cmd::run_history_command(args).map(Some)
        }
        Commands::Repl(args) => {
            repl::run_repl_debug_command_for(runtime, session, clients, args).map(Some)
        }
        Commands::External(_) => Ok(None),
    }
}

pub(crate) fn ensure_command_supports_dsl(spec: &ReplCommandSpec, stages: &[String]) -> Result<()> {
    if stages.is_empty() || spec.supports_dsl {
        return Ok(());
    }

    Err(miette!(
        "`{}` does not support DSL pipeline stages",
        spec.name
    ))
}

fn resolve_theme_name(
    cli: &Cli,
    config: &ResolvedConfig,
    catalog: &theme_loader::ThemeCatalog,
) -> Result<String> {
    let selected = cli.selected_theme_name(config);
    resolve_known_theme_name(&selected, catalog)
}

pub(crate) fn resolve_known_theme_name(
    value: &str,
    catalog: &theme_loader::ThemeCatalog,
) -> Result<String> {
    let normalized = normalize_theme_name(value);
    if catalog.resolve(&normalized).is_some() {
        return Ok(normalized);
    }

    let known = catalog.ids().join(", ");
    Err(miette!("unknown theme: {value}. available themes: {known}"))
}

pub(crate) fn enrich_dispatch_error(err: PluginDispatchError) -> miette::Report {
    match err {
        not_found @ PluginDispatchError::CommandNotFound { .. } => miette!(
            "{not_found}\nHint: run `osp plugins list` and set --plugin-dir or OSP_PLUGIN_PATH"
        ),
        other => miette!("{other}"),
    }
}

pub(crate) fn config_usize(config: &ResolvedConfig, key: &str, fallback: usize) -> usize {
    match config.get(key).map(ConfigValue::reveal) {
        Some(ConfigValue::Integer(value)) if *value > 0 => *value as usize,
        Some(ConfigValue::String(raw)) => raw
            .trim()
            .parse::<usize>()
            .ok()
            .filter(|value| *value > 0)
            .unwrap_or(fallback),
        _ => fallback,
    }
}

fn resolve_default_render_width(config: &ResolvedConfig) -> usize {
    let configured = config_usize(config, "ui.width", DEFAULT_UI_WIDTH as usize);
    if configured != DEFAULT_UI_WIDTH as usize {
        return configured;
    }

    detect_terminal_width()
        .or_else(|| {
            std::env::var("COLUMNS")
                .ok()
                .and_then(|value| value.trim().parse::<usize>().ok())
                .filter(|value| *value > 0)
        })
        .unwrap_or(configured)
}

fn detect_terminal_width() -> Option<usize> {
    if !std::io::stdout().is_terminal() {
        return None;
    }
    terminal_size()
        .map(|(Width(columns), _)| columns as usize)
        .filter(|value| *value > 0)
}

fn detect_columns_env() -> Option<usize> {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
}

fn locale_utf8_hint_from_env() -> Option<bool> {
    for key in ["LC_ALL", "LC_CTYPE", "LANG"] {
        if let Ok(value) = std::env::var(key) {
            let lower = value.to_ascii_lowercase();
            if lower.contains("utf-8") || lower.contains("utf8") {
                return Some(true);
            }
            return Some(false);
        }
    }
    None
}

pub(crate) fn build_render_runtime(terminal_env: Option<&str>) -> RenderRuntime {
    RenderRuntime {
        stdout_is_tty: std::io::stdout().is_terminal(),
        terminal: terminal_env.map(ToOwned::to_owned),
        no_color: std::env::var("NO_COLOR").is_ok(),
        width: detect_terminal_width().or_else(detect_columns_env),
        locale_utf8: locale_utf8_hint_from_env(),
    }
}

fn to_ui_verbosity(level: MessageLevel) -> UiVerbosity {
    match level {
        MessageLevel::Error => UiVerbosity::Error,
        MessageLevel::Warning => UiVerbosity::Warning,
        MessageLevel::Success => UiVerbosity::Success,
        MessageLevel::Info => UiVerbosity::Info,
        MessageLevel::Trace => UiVerbosity::Trace,
    }
}

pub(crate) fn plugin_dispatch_context_for_runtime(
    runtime: &crate::state::AppRuntime,
    overrides: Option<ReplDispatchOverrides>,
) -> PluginDispatchContext {
    build_plugin_dispatch_context(
        &runtime.context,
        runtime.config.resolved(),
        &runtime.ui,
        overrides,
    )
}

fn plugin_dispatch_context_for(
    runtime: &ExternalCommandRuntime<'_>,
    overrides: Option<ReplDispatchOverrides>,
) -> PluginDispatchContext {
    build_plugin_dispatch_context(runtime.context, runtime.config, runtime.ui, overrides)
}

fn build_plugin_dispatch_context(
    context: &crate::state::RuntimeContext,
    config: &ResolvedConfig,
    ui: &crate::state::UiState,
    overrides: Option<ReplDispatchOverrides>,
) -> PluginDispatchContext {
    let config_env = collect_plugin_config_env(config);
    let ui_verbosity = overrides
        .map(|value| value.message_verbosity)
        .unwrap_or(ui.message_verbosity);
    let debug_verbosity = overrides
        .map(|value| value.debug_verbosity)
        .unwrap_or(ui.debug_verbosity);
    let terminal_kind = match context.terminal_kind() {
        TerminalKind::Cli => RuntimeTerminalKind::Cli,
        TerminalKind::Repl => RuntimeTerminalKind::Repl,
    };
    PluginDispatchContext {
        runtime_hints: RuntimeHints {
            ui_verbosity: to_ui_verbosity(ui_verbosity),
            debug_level: debug_verbosity.min(3),
            format: ui.render_settings.format,
            color: ui.render_settings.color,
            unicode: ui.render_settings.unicode,
            profile: Some(config.active_profile().to_string()),
            terminal: context.terminal_env().map(ToOwned::to_owned),
            terminal_kind,
        },
        shared_env: config_env
            .shared
            .iter()
            .map(|entry| (entry.env_key.clone(), entry.value.clone()))
            .collect(),
        plugin_env: config_env
            .by_plugin_id
            .into_iter()
            .map(|(plugin_id, entries)| {
                (
                    plugin_id,
                    entries
                        .into_iter()
                        .map(|entry| (entry.env_key, entry.value))
                        .collect(),
                )
            })
            .collect(),
    }
}

fn collect_plugin_config_env(config: &ResolvedConfig) -> PluginConfigEnv {
    let mut shared: BTreeMap<String, PluginConfigEntry> = BTreeMap::new();
    let mut by_plugin_id: HashMap<String, BTreeMap<String, PluginConfigEntry>> = HashMap::new();

    for (key, entry) in config.values() {
        if let Some(name) = key.strip_prefix(SHARED_PLUGIN_ENV_PREFIX) {
            if let Some(env_entry) =
                plugin_env_mapping(key, name, &entry.value, PluginConfigScope::Shared)
            {
                shared.insert(env_entry.env_key.clone(), env_entry);
            }
            continue;
        }

        let Some(plugin_key) = key.strip_prefix(PLUGIN_ENV_ROOT_PREFIX) else {
            continue;
        };
        let Some((plugin_id, name)) = plugin_key.split_once(PLUGIN_ENV_SEPARATOR) else {
            continue;
        };
        if plugin_id.is_empty() {
            continue;
        }
        if let Some(env_entry) =
            plugin_env_mapping(key, name, &entry.value, PluginConfigScope::Plugin)
        {
            by_plugin_id
                .entry(plugin_id.to_string())
                .or_default()
                .insert(env_entry.env_key.clone(), env_entry);
        }
    }

    PluginConfigEnv {
        shared: shared.into_values().collect(),
        by_plugin_id: by_plugin_id
            .into_iter()
            .map(|(plugin_id, env)| (plugin_id, env.into_values().collect()))
            .collect(),
    }
}

pub(crate) fn effective_plugin_config_entries(
    config: &ResolvedConfig,
    plugin_id: &str,
) -> Vec<PluginConfigEntry> {
    let config_env = collect_plugin_config_env(config);
    let mut effective = BTreeMap::new();
    for entry in config_env.shared {
        effective.insert(entry.env_key.clone(), entry);
    }
    if let Some(entries) = config_env.by_plugin_id.get(plugin_id) {
        for entry in entries {
            effective.insert(entry.env_key.clone(), entry.clone());
        }
    }
    effective.into_values().collect()
}

fn plugin_env_mapping(
    config_key: &str,
    name: &str,
    value: &ConfigValue,
    scope: PluginConfigScope,
) -> Option<PluginConfigEntry> {
    Some(PluginConfigEntry {
        env_key: plugin_config_env_name(name)?,
        value: config_value_to_plugin_env(value),
        config_key: config_key.to_string(),
        scope,
    })
}

pub(crate) fn plugin_config_env_name(name: &str) -> Option<String> {
    let mut normalized = String::new();
    let mut last_was_separator = true;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch.to_ascii_uppercase());
            last_was_separator = false;
        } else if !last_was_separator {
            normalized.push('_');
            last_was_separator = true;
        }
    }
    while normalized.ends_with('_') {
        normalized.pop();
    }
    if normalized.is_empty() {
        return None;
    }
    Some(format!("{PLUGIN_CONFIG_ENV_PREFIX}{normalized}"))
}

pub(crate) fn config_value_to_plugin_env(value: &ConfigValue) -> String {
    match value {
        ConfigValue::Secret(secret) => config_value_to_plugin_env(secret.expose()),
        ConfigValue::String(value) => value.clone(),
        ConfigValue::Bool(value) => value.to_string(),
        ConfigValue::Integer(value) => value.to_string(),
        ConfigValue::Float(value) => value.to_string(),
        // Lists are encoded as JSON so plugins can round-trip structured values.
        ConfigValue::List(values) => serde_json::Value::Array(
            values
                .iter()
                .map(config_value_to_plugin_env_json)
                .collect::<Vec<_>>(),
        )
        .to_string(),
    }
}

fn config_value_to_plugin_env_json(value: &ConfigValue) -> serde_json::Value {
    match value {
        ConfigValue::Secret(secret) => config_value_to_plugin_env_json(secret.expose()),
        ConfigValue::String(value) => value.clone().into(),
        ConfigValue::Bool(value) => (*value).into(),
        ConfigValue::Integer(value) => (*value).into(),
        ConfigValue::Float(value) => (*value).into(),
        ConfigValue::List(values) => serde_json::Value::Array(
            values
                .iter()
                .map(config_value_to_plugin_env_json)
                .collect::<Vec<_>>(),
        ),
    }
}

pub(crate) fn resolve_effective_render_settings(
    settings: &RenderSettings,
    format_hint: Option<OutputFormat>,
) -> RenderSettings {
    if matches!(settings.format, OutputFormat::Auto)
        && let Some(format) = format_hint
    {
        let mut effective = settings.clone();
        effective.format = format;
        return effective;
    }
    settings.clone()
}

#[cfg(test)]
mod tests;
