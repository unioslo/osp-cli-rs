use clap::Parser;
use miette::{Result, miette};
use osp_config::{ConfigValue, DEFAULT_UI_WIDTH, ResolvedConfig};
use osp_core::output::OutputFormat;
use osp_core::plugin::{ResponseMessageLevelV1, ResponseV1};
use osp_core::runtime::{RuntimeHints, RuntimeTerminalKind, UiVerbosity};
use osp_dsl::apply_output_pipeline;

use osp_ui::messages::{MessageBuffer, MessageLevel};
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
use crate::state::{AppState, LaunchContext, TerminalKind};

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
    CliCommandResult, CommandRenderRuntime, ReplCommandOutput, emit_messages, emit_messages_for_ui,
    emit_messages_with_runtime, emit_messages_with_verbosity, maybe_copy_output,
    maybe_copy_output_with_runtime, run_cli_command,
};
pub(crate) use config_explain::{
    config_explain_json, config_explain_output, config_value_to_json, explain_runtime_config,
    format_scope, is_sensitive_key, render_config_explain_text,
};
pub(crate) use dispatch::{
    RunAction, build_dispatch_plan, ensure_builtin_visible, ensure_dispatch_visibility,
    ensure_plugin_visible, ensure_plugin_visible_for, normalize_cli_profile,
    normalize_profile_override,
};
pub(crate) use external::is_help_passthrough;
use external::{ExternalCommandRuntime, run_external_command};
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
    ensure_dispatch_visibility(&state, &dispatch.action)?;

    tracing::info!(
        profile = %state.config.resolved().active_profile(),
        terminal = %state.context.terminal_kind().as_config_terminal(),
        "osp session initialized"
    );

    match dispatch.action {
        RunAction::Repl => repl::run_plugin_repl(&mut state),
        RunAction::ReplCommand(args) => run_builtin_cli_command(&mut state, Commands::Repl(args)),
        RunAction::Plugins(args) => run_builtin_cli_command(&mut state, Commands::Plugins(args)),
        RunAction::Doctor(args) => run_builtin_cli_command(&mut state, Commands::Doctor(args)),
        RunAction::Theme(args) => run_builtin_cli_command(&mut state, Commands::Theme(args)),
        RunAction::Config(args) => run_builtin_cli_command(&mut state, Commands::Config(args)),
        RunAction::History(args) => run_builtin_cli_command(&mut state, Commands::History(args)),
        RunAction::External(tokens) => run_external_command(&mut state, &tokens),
    }
}

pub(crate) fn authorized_command_catalog(state: &AppState) -> Result<Vec<CommandCatalogEntry>> {
    let all = state
        .clients
        .plugins
        .command_catalog()
        .map_err(|err| miette!("{err:#}"))?;
    Ok(all
        .into_iter()
        .filter(|entry| state.auth.is_plugin_command_visible(&entry.name))
        .collect())
}

fn run_builtin_cli_command(state: &mut AppState, command: Commands) -> Result<i32> {
    let result = dispatch_builtin_command(state, command)?
        .ok_or_else(|| miette!("expected builtin command"))?;
    run_cli_command(&CommandRenderRuntime::from_state(state), result)
}

fn run_inline_builtin_command(
    state: &mut AppState,
    command: Commands,
    stages: &[String],
) -> Result<Option<CliCommandResult>> {
    if matches!(command, Commands::External(_)) {
        return Ok(None);
    }

    let spec = repl::repl_command_spec(&command);
    ensure_command_supports_dsl(&spec, stages)?;
    dispatch_builtin_command(state, command)
}

fn dispatch_builtin_command(
    state: &mut AppState,
    command: Commands,
) -> Result<Option<CliCommandResult>> {
    match command {
        Commands::Plugins(args) => {
            ensure_builtin_visible(state, CMD_PLUGINS)?;
            plugins_cmd::run_plugins_command(state, args).map(Some)
        }
        Commands::Doctor(args) => {
            ensure_builtin_visible(state, CMD_DOCTOR)?;
            doctor_cmd::run_doctor_command(state, args).map(Some)
        }
        Commands::Theme(args) => {
            ensure_builtin_visible(state, CMD_THEME)?;
            theme_cmd::run_theme_command(
                &mut state.session.config_overrides,
                theme_cmd::ThemeCommandContext {
                    config: state.config.resolved(),
                    ui: &state.ui,
                    themes: &state.themes,
                },
                args,
            )
            .map(Some)
        }
        Commands::Config(args) => {
            ensure_builtin_visible(state, CMD_CONFIG)?;
            config_cmd::run_config_command(state, args).map(Some)
        }
        Commands::History(args) => {
            ensure_builtin_visible(state, CMD_HISTORY)?;
            history_cmd::run_history_command(state, args).map(Some)
        }
        Commands::Repl(args) => repl::run_repl_debug_command(state, args).map(Some),
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

pub(crate) fn plugin_dispatch_context(
    state: &AppState,
    overrides: Option<ReplDispatchOverrides>,
) -> PluginDispatchContext {
    plugin_dispatch_context_for(&ExternalCommandRuntime::from_state(state), overrides)
}

fn plugin_dispatch_context_for(
    runtime: &ExternalCommandRuntime<'_>,
    overrides: Option<ReplDispatchOverrides>,
) -> PluginDispatchContext {
    let config_env = collect_plugin_config_env(runtime.config);
    let ui_verbosity = overrides
        .map(|value| value.message_verbosity)
        .unwrap_or(runtime.ui.message_verbosity);
    let debug_verbosity = overrides
        .map(|value| value.debug_verbosity)
        .unwrap_or(runtime.ui.debug_verbosity);
    let terminal_kind = match runtime.context.terminal_kind() {
        TerminalKind::Cli => RuntimeTerminalKind::Cli,
        TerminalKind::Repl => RuntimeTerminalKind::Repl,
    };
    PluginDispatchContext {
        runtime_hints: RuntimeHints {
            ui_verbosity: to_ui_verbosity(ui_verbosity),
            debug_level: debug_verbosity.min(3),
            format: runtime.ui.render_settings.format,
            color: runtime.ui.render_settings.color,
            unicode: runtime.ui.render_settings.unicode,
            profile: Some(runtime.config.active_profile().to_string()),
            terminal: runtime.context.terminal_env().map(ToOwned::to_owned),
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

pub(crate) fn plugin_response_messages(response: &ResponseV1) -> MessageBuffer {
    let mut out = MessageBuffer::default();
    for message in &response.messages {
        let level = match message.level {
            ResponseMessageLevelV1::Error => MessageLevel::Error,
            ResponseMessageLevelV1::Warning => MessageLevel::Warning,
            ResponseMessageLevelV1::Success => MessageLevel::Success,
            ResponseMessageLevelV1::Info => MessageLevel::Info,
            ResponseMessageLevelV1::Trace => MessageLevel::Trace,
        };
        out.push(level, message.text.clone());
    }
    out
}

pub(crate) fn parse_output_format_hint(value: Option<&str>) -> Option<OutputFormat> {
    let normalized = value?.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "auto" => Some(OutputFormat::Auto),
        "json" => Some(OutputFormat::Json),
        "table" => Some(OutputFormat::Table),
        "md" | "markdown" => Some(OutputFormat::Markdown),
        "mreg" => Some(OutputFormat::Mreg),
        "value" => Some(OutputFormat::Value),
        _ => None,
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
mod tests {
    use super::help::parse_help_render_overrides;
    use super::{
        PluginConfigEntry, PluginConfigScope, ReplCommandOutput, RunAction,
        build_cli_session_layer, build_dispatch_plan, collect_plugin_config_env,
        config_value_to_plugin_env, doctor_cmd, is_sensitive_key, parse_output_format_hint,
        plugin_config_env_name, resolve_effective_render_settings, run_inline_builtin_command,
    };
    use crate::cli::{Cli, Commands, ConfigCommands, PluginsCommands, ThemeCommands};
    use crate::plugin_manager::{CommandCatalogEntry, PluginManager, PluginSource};
    use crate::repl;
    use crate::repl::{completion, help as repl_help, surface};
    use crate::state::{AppState, AppStateInit, LaunchContext, RuntimeContext, TerminalKind};
    use clap::Parser;
    use osp_config::{
        ConfigLayer, ConfigResolver, ConfigValue, ResolveOptions, RuntimeLoadOptions,
    };
    use osp_core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
    use osp_repl::{HistoryConfig, HistoryShellContext, SharedHistory};
    use osp_ui::messages::MessageLevel;
    use osp_ui::theme::DEFAULT_THEME_NAME;
    use osp_ui::{RenderRuntime, RenderSettings};
    use std::collections::BTreeSet;
    use std::ffi::OsString;

    fn profiles(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    fn make_completion_state(auth_visible_builtins: Option<&str>) -> AppState {
        make_completion_state_with_entries(auth_visible_builtins, &[])
    }

    fn make_completion_state_with_entries(
        auth_visible_builtins: Option<&str>,
        entries: &[(&str, &str)],
    ) -> AppState {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        if let Some(allowlist) = auth_visible_builtins {
            defaults.set("auth.visible.builtins", allowlist);
        }
        for (key, value) in entries {
            defaults.set(*key, *value);
        }
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        let config = resolver
            .resolve(ResolveOptions::default().with_terminal("repl"))
            .expect("test config should resolve");

        let settings = RenderSettings {
            format: OutputFormat::Json,
            mode: RenderMode::Plain,
            color: ColorMode::Never,
            unicode: UnicodeMode::Never,
            width: None,
            margin: 0,
            indent_size: 2,
            short_list_max: 1,
            medium_list_max: 5,
            grid_padding: 4,
            grid_columns: None,
            column_weight: 3,
            table_overflow: osp_ui::TableOverflow::Clip,
            mreg_stack_min_col_width: 10,
            mreg_stack_overflow_ratio: 200,
            theme_name: DEFAULT_THEME_NAME.to_string(),
            theme: None,
            style_overrides: osp_ui::StyleOverrides::default(),
            runtime: RenderRuntime::default(),
        };

        AppState::new(AppStateInit {
            context: RuntimeContext::new(None, TerminalKind::Repl, None),
            config,
            render_settings: settings,
            message_verbosity: MessageLevel::Success,
            debug_verbosity: 0,
            plugins: PluginManager::new(Vec::new()),
            themes: crate::theme_loader::ThemeCatalog::default(),
            launch: LaunchContext::default(),
        })
    }

    fn sample_catalog() -> Vec<CommandCatalogEntry> {
        vec![CommandCatalogEntry {
            name: "orch".to_string(),
            about: "Provision orchestrator resources".to_string(),
            subcommands: vec!["provision".to_string(), "status".to_string()],
            completion: osp_completion::CommandSpec {
                name: "orch".to_string(),
                tooltip: Some("Provision orchestrator resources".to_string()),
                subcommands: vec![
                    osp_completion::CommandSpec::new("provision"),
                    osp_completion::CommandSpec::new("status"),
                ],
                ..osp_completion::CommandSpec::default()
            },
            provider: "mock-provider".to_string(),
            providers: vec!["mock-provider (explicit)".to_string()],
            conflicted: false,
            source: PluginSource::Explicit,
        }]
    }

    #[test]
    fn theme_slug_is_rendered_as_title_case_display_name_unit() {
        assert_eq!(repl::theme_display_name("rose-pine-moon"), "Rose Pine Moon");
        assert_eq!(repl::theme_display_name("dracula"), "Dracula");
    }

    #[test]
    fn plugin_format_hint_parser_supports_known_values_unit() {
        assert_eq!(
            parse_output_format_hint(Some("table")),
            Some(OutputFormat::Table)
        );
        assert_eq!(
            parse_output_format_hint(Some("mreg")),
            Some(OutputFormat::Mreg)
        );
        assert_eq!(
            parse_output_format_hint(Some("markdown")),
            Some(OutputFormat::Markdown)
        );
        assert_eq!(parse_output_format_hint(Some("unknown")), None);
    }

    #[test]
    fn plugin_config_env_name_normalizes_extension_keys_unit() {
        assert_eq!(
            plugin_config_env_name("api.token"),
            Some("OSP_PLUGIN_CFG_API_TOKEN".to_string())
        );
        assert_eq!(
            plugin_config_env_name("nested-value/path"),
            Some("OSP_PLUGIN_CFG_NESTED_VALUE_PATH".to_string())
        );
        assert_eq!(plugin_config_env_name("..."), None);
    }

    #[test]
    fn plugin_config_env_serializes_lists_and_secrets_unit() {
        assert_eq!(
            config_value_to_plugin_env(&ConfigValue::List(vec![
                ConfigValue::String("alpha".to_string()),
                ConfigValue::Integer(2),
                ConfigValue::Bool(true),
            ])),
            r#"["alpha",2,true]"#
        );
        assert_eq!(
            config_value_to_plugin_env(&ConfigValue::String("sekrit".to_string()).into_secret()),
            "sekrit"
        );
    }

    #[test]
    fn plugin_config_env_collects_shared_and_plugin_specific_entries_unit() {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        defaults.set(
            "extensions.plugins.env.shared.url",
            "https://common.example",
        );
        defaults.set("extensions.plugins.env.endpoint", "shared");
        defaults.set("extensions.plugins.cfg.env.endpoint", "plugin");
        defaults.set("extensions.plugins.cfg.env.api.token", "token-123");
        defaults.set("extensions.plugins.other.env.endpoint", "other");
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        let config = resolver
            .resolve(ResolveOptions::default())
            .expect("test config should resolve");

        let env = collect_plugin_config_env(&config);

        assert_eq!(
            env.shared,
            vec![
                PluginConfigEntry {
                    env_key: "OSP_PLUGIN_CFG_ENDPOINT".to_string(),
                    value: "shared".to_string(),
                    config_key: "extensions.plugins.env.endpoint".to_string(),
                    scope: PluginConfigScope::Shared,
                },
                PluginConfigEntry {
                    env_key: "OSP_PLUGIN_CFG_SHARED_URL".to_string(),
                    value: "https://common.example".to_string(),
                    config_key: "extensions.plugins.env.shared.url".to_string(),
                    scope: PluginConfigScope::Shared,
                },
            ]
        );
        assert_eq!(
            env.by_plugin_id.get("cfg"),
            Some(&vec![
                PluginConfigEntry {
                    env_key: "OSP_PLUGIN_CFG_API_TOKEN".to_string(),
                    value: "token-123".to_string(),
                    config_key: "extensions.plugins.cfg.env.api.token".to_string(),
                    scope: PluginConfigScope::Plugin,
                },
                PluginConfigEntry {
                    env_key: "OSP_PLUGIN_CFG_ENDPOINT".to_string(),
                    value: "plugin".to_string(),
                    config_key: "extensions.plugins.cfg.env.endpoint".to_string(),
                    scope: PluginConfigScope::Plugin,
                },
            ])
        );
        assert_eq!(
            env.by_plugin_id.get("other"),
            Some(&vec![PluginConfigEntry {
                env_key: "OSP_PLUGIN_CFG_ENDPOINT".to_string(),
                value: "other".to_string(),
                config_key: "extensions.plugins.other.env.endpoint".to_string(),
                scope: PluginConfigScope::Plugin,
            }])
        );
    }

    fn layer_value<'a>(layer: &'a ConfigLayer, key: &str) -> Option<&'a ConfigValue> {
        layer
            .entries()
            .iter()
            .find(|entry| entry.key == key)
            .map(|entry| &entry.value)
    }

    #[test]
    fn cli_launch_render_flags_seed_session_layer_unit() {
        let cli = Cli::parse_from([
            "osp", "--json", "--mode", "plain", "--color", "never", "--ascii",
        ]);

        let layer = build_cli_session_layer(
            &cli,
            None,
            TerminalKind::Repl,
            RuntimeLoadOptions::default(),
        )
        .expect("session layer should build")
        .expect("launch flags should create session overrides");

        assert_eq!(
            layer_value(&layer, "ui.format"),
            Some(&ConfigValue::from("json"))
        );
        assert_eq!(
            layer_value(&layer, "ui.mode"),
            Some(&ConfigValue::from("plain"))
        );
        assert_eq!(
            layer_value(&layer, "ui.color.mode"),
            Some(&ConfigValue::from("never"))
        );
        assert_eq!(
            layer_value(&layer, "ui.unicode.mode"),
            Some(&ConfigValue::from("never"))
        );
    }

    #[test]
    fn cli_launch_quiet_adjusts_session_base_verbosity_unit() {
        let cli = Cli::parse_from(["osp", "-q"]);

        let layer = build_cli_session_layer(
            &cli,
            None,
            TerminalKind::Repl,
            RuntimeLoadOptions::default(),
        )
        .expect("session layer should build")
        .expect("quiet flag should create session overrides");

        assert_eq!(
            layer_value(&layer, "ui.verbosity.level"),
            Some(&ConfigValue::from("warning"))
        );
    }

    #[test]
    fn cli_runtime_load_options_follow_disable_flags_unit() {
        let cli = Cli::parse_from(["osp", "--no-env", "--no-config"]);
        let options = cli.runtime_load_options();
        assert!(!options.include_env);
        assert!(!options.include_config_file);
    }

    #[test]
    fn effective_settings_use_plugin_hint_only_when_auto_unit() {
        let base = RenderSettings {
            format: OutputFormat::Auto,
            mode: RenderMode::Plain,
            color: ColorMode::Never,
            unicode: UnicodeMode::Never,
            width: None,
            margin: 0,
            indent_size: 2,
            short_list_max: 1,
            medium_list_max: 5,
            grid_padding: 4,
            grid_columns: None,
            column_weight: 3,
            table_overflow: osp_ui::TableOverflow::Clip,
            mreg_stack_min_col_width: 10,
            mreg_stack_overflow_ratio: 200,
            theme_name: DEFAULT_THEME_NAME.to_string(),
            theme: None,
            style_overrides: osp_ui::StyleOverrides::default(),
            runtime: RenderRuntime::default(),
        };
        let hinted = resolve_effective_render_settings(&base, Some(OutputFormat::Table));
        assert_eq!(hinted.format, OutputFormat::Table);

        let pinned = resolve_effective_render_settings(
            &RenderSettings {
                format: OutputFormat::Json,
                ..base
            },
            Some(OutputFormat::Table),
        );
        assert_eq!(pinned.format, OutputFormat::Json);
    }

    #[test]
    fn positional_profile_only_routes_to_repl_unit() {
        let mut cli = Cli::parse_from(["osp", "tsd"]);
        let plan = build_dispatch_plan(&mut cli, &profiles(&["uio", "tsd"]))
            .expect("dispatch plan should parse");

        assert_eq!(plan.profile_override.as_deref(), Some("tsd"));
        assert!(matches!(plan.action, RunAction::Repl));
    }

    #[test]
    fn positional_profile_with_command_routes_external_unit() {
        let mut cli = Cli::parse_from(["osp", "tsd", "ldap", "user", "oistes"]);
        let plan = build_dispatch_plan(&mut cli, &profiles(&["uio", "tsd"]))
            .expect("dispatch plan should parse");

        assert_eq!(plan.profile_override.as_deref(), Some("tsd"));
        match plan.action {
            RunAction::External(tokens) => {
                assert_eq!(
                    tokens,
                    vec!["ldap".to_string(), "user".to_string(), "oistes".to_string()]
                );
            }
            _ => panic!("expected external action"),
        }
    }

    #[test]
    fn positional_profile_with_plugins_routes_builtin_unit() {
        let mut cli = Cli::parse_from(["osp", "tsd", "plugins", "list"]);
        let plan = build_dispatch_plan(&mut cli, &profiles(&["uio", "tsd"]))
            .expect("dispatch plan should parse");

        assert_eq!(plan.profile_override.as_deref(), Some("tsd"));
        match plan.action {
            RunAction::Plugins(args) => {
                assert!(matches!(args.command, PluginsCommands::List));
            }
            _ => panic!("expected plugins action"),
        }
    }

    #[test]
    fn unknown_first_token_is_command_unit() {
        let mut cli = Cli::parse_from(["osp", "prod", "ldap", "user", "oistes"]);
        let plan = build_dispatch_plan(&mut cli, &profiles(&["uio", "tsd"]))
            .expect("dispatch plan should parse");

        assert_eq!(plan.profile_override, None);
        match plan.action {
            RunAction::External(tokens) => {
                assert_eq!(
                    tokens,
                    vec![
                        "prod".to_string(),
                        "ldap".to_string(),
                        "user".to_string(),
                        "oistes".to_string()
                    ]
                );
            }
            _ => panic!("expected external action"),
        }
    }

    #[test]
    fn explicit_profile_overrides_positional_unit() {
        let mut cli = Cli::parse_from(["osp", "--profile", "uio", "tsd", "plugins", "list"]);
        let plan = build_dispatch_plan(&mut cli, &profiles(&["uio", "tsd"]))
            .expect("dispatch plan should parse");

        assert_eq!(plan.profile_override.as_deref(), Some("uio"));
        match plan.action {
            RunAction::External(tokens) => {
                assert_eq!(
                    tokens,
                    vec!["tsd".to_string(), "plugins".to_string(), "list".to_string()]
                );
            }
            _ => panic!("expected external action"),
        }
    }

    #[test]
    fn explicit_profile_is_normalized_unit() {
        let mut cli = Cli::parse_from(["osp", "--profile", "TSD"]);
        let plan =
            build_dispatch_plan(&mut cli, &profiles(&["tsd"])).expect("dispatch plan should parse");
        assert_eq!(plan.profile_override.as_deref(), Some("tsd"));
    }

    #[test]
    fn direct_plugins_command_keeps_clap_action_unit() {
        let mut cli = Cli::parse_from(["osp", "plugins", "doctor"]);
        let plan = build_dispatch_plan(&mut cli, &profiles(&["uio", "tsd"]))
            .expect("dispatch plan should parse");

        assert_eq!(plan.profile_override, None);
        assert!(matches!(
            plan.action,
            RunAction::Plugins(crate::cli::PluginsArgs {
                command: PluginsCommands::Doctor
            })
        ));
        assert!(matches!(cli.command, None | Some(Commands::Plugins(_))));
    }

    #[test]
    fn positional_profile_with_config_uses_clap_parser_unit() {
        let mut cli = Cli::parse_from(["osp", "tsd", "config", "show", "--sources"]);
        let plan = build_dispatch_plan(&mut cli, &profiles(&["uio", "tsd"]))
            .expect("dispatch plan should parse");

        assert_eq!(plan.profile_override.as_deref(), Some("tsd"));
        match plan.action {
            RunAction::Config(args) => {
                assert!(matches!(
                    args.command,
                    ConfigCommands::Show(crate::cli::ConfigShowArgs {
                        sources: true,
                        raw: false,
                    })
                ));
            }
            _ => panic!("expected config action"),
        }
    }

    #[test]
    fn repl_dsl_capability_is_declared_per_command_unit() {
        let plugins_list = Commands::Plugins(crate::cli::PluginsArgs {
            command: PluginsCommands::List,
        });
        let plugins_enable = Commands::Plugins(crate::cli::PluginsArgs {
            command: PluginsCommands::Enable(crate::cli::PluginToggleArgs {
                plugin_id: "uio-ldap".to_string(),
            }),
        });
        let theme_show = Commands::Theme(crate::cli::ThemeArgs {
            command: ThemeCommands::Show(crate::cli::ThemeShowArgs { name: None }),
        });
        let theme_use = Commands::Theme(crate::cli::ThemeArgs {
            command: ThemeCommands::Use(crate::cli::ThemeUseArgs {
                name: "nord".to_string(),
            }),
        });
        let config_show = Commands::Config(crate::cli::ConfigArgs {
            command: ConfigCommands::Show(crate::cli::ConfigShowArgs {
                sources: false,
                raw: false,
            }),
        });
        let config_set = Commands::Config(crate::cli::ConfigArgs {
            command: ConfigCommands::Set(crate::cli::ConfigSetArgs {
                key: "ui.mode".to_string(),
                value: "plain".to_string(),
                global: false,
                profile: None,
                profile_all: false,
                terminal: None,
                session: false,
                config_store: false,
                secrets: false,
                save: false,
                dry_run: false,
                yes: false,
                explain: false,
            }),
        });
        let history_list = Commands::History(crate::cli::HistoryArgs {
            command: crate::cli::HistoryCommands::List,
        });
        let history_prune = Commands::History(crate::cli::HistoryArgs {
            command: crate::cli::HistoryCommands::Prune(crate::cli::HistoryPruneArgs { keep: 5 }),
        });

        assert!(repl::repl_command_spec(&plugins_list).supports_dsl);
        assert!(!repl::repl_command_spec(&plugins_enable).supports_dsl);
        assert!(repl::repl_command_spec(&theme_show).supports_dsl);
        assert!(!repl::repl_command_spec(&theme_use).supports_dsl);
        assert!(repl::repl_command_spec(&config_show).supports_dsl);
        assert!(!repl::repl_command_spec(&config_set).supports_dsl);
        assert!(repl::repl_command_spec(&history_list).supports_dsl);
        assert!(!repl::repl_command_spec(&history_prune).supports_dsl);
    }

    #[test]
    fn external_inline_builtin_reuses_repl_dsl_policy_unit() {
        let mut state = make_completion_state(None);
        let command = Commands::Config(crate::cli::ConfigArgs {
            command: ConfigCommands::Set(crate::cli::ConfigSetArgs {
                key: "ui.mode".to_string(),
                value: "plain".to_string(),
                global: false,
                profile: None,
                profile_all: false,
                terminal: None,
                session: false,
                config_store: false,
                secrets: false,
                save: false,
                dry_run: false,
                yes: false,
                explain: false,
            }),
        });

        let err = match run_inline_builtin_command(&mut state, command, &["uid".to_string()]) {
            Ok(_) => panic!("expected DSL rejection"),
            Err(err) => err,
        };
        assert_eq!(
            err.to_string(),
            "`config` does not support DSL pipeline stages"
        );
    }

    #[test]
    fn repl_prompt_template_substitutes_profile_and_indicator_unit() {
        let rendered = repl::render_prompt_template(
            "╭─{user}@{domain} {indicator}\n╰─{profile}> ",
            "oistes",
            "uio.no",
            "uio",
            "[orch]",
        );
        assert!(rendered.contains("oistes@uio.no [orch]"));
        assert!(rendered.contains("╰─uio> "));
    }

    #[test]
    fn repl_prompt_template_appends_indicator_when_missing_placeholder_unit() {
        let rendered =
            repl::render_prompt_template("{profile}>", "oistes", "uio.no", "tsd", "[shell]");
        assert_eq!(rendered, "tsd> [shell]");
    }

    #[test]
    fn repl_help_alias_rewrites_to_command_help_unit() {
        let state = make_completion_state(None);
        let rewritten = repl::ReplParsedLine::parse("help ldap user", state.config.resolved())
            .expect("help alias should parse");
        assert_eq!(
            rewritten.dispatch_tokens,
            vec!["ldap".to_string(), "user".to_string(), "--help".to_string()]
        );
    }

    #[test]
    fn repl_help_alias_preserves_existing_help_flag_unit() {
        let state = make_completion_state(None);
        let rewritten = repl::ReplParsedLine::parse("help ldap --help", state.config.resolved())
            .expect("help alias should parse");
        assert_eq!(
            rewritten.dispatch_tokens,
            vec!["ldap".to_string(), "--help".to_string()]
        );
    }

    #[test]
    fn repl_help_alias_skips_bare_help_unit() {
        let state = make_completion_state(None);
        let parsed = repl::ReplParsedLine::parse("help", state.config.resolved())
            .expect("bare help should parse");
        assert_eq!(parsed.command_tokens, vec!["help".to_string()]);
        assert_eq!(parsed.dispatch_tokens, vec!["help".to_string()]);
    }

    #[test]
    fn repl_shellable_commands_include_ldap_unit() {
        assert!(repl::is_repl_shellable_command("ldap"));
        assert!(repl::is_repl_shellable_command("LDAP"));
        assert!(!repl::is_repl_shellable_command("theme"));
    }

    #[test]
    fn repl_shell_prefix_applies_once_unit() {
        let mut stack = crate::state::ReplScopeStack::default();
        stack.enter("ldap");
        let bare =
            repl::apply_repl_shell_prefix(&stack, &["user".to_string(), "oistes".to_string()]);
        assert_eq!(
            bare,
            vec!["ldap".to_string(), "user".to_string(), "oistes".to_string()]
        );

        let already_prefixed = repl::apply_repl_shell_prefix(
            &stack,
            &["ldap".to_string(), "user".to_string(), "oistes".to_string()],
        );
        assert_eq!(
            already_prefixed,
            vec!["ldap".to_string(), "user".to_string(), "oistes".to_string()]
        );
    }

    #[test]
    fn repl_shell_leave_message_unit() {
        let mut state = make_completion_state(None);
        state.session.scope.enter("ldap");
        let message = repl::leave_repl_shell(&mut state).expect("shell should leave");
        assert_eq!(message, "Leaving ldap shell. Back at root.\n");
        assert!(state.session.scope.is_root());
    }

    #[test]
    fn repl_shell_enter_only_from_root_unit() {
        let mut state = make_completion_state(None);
        let ldap = repl::ReplParsedLine::parse("ldap", state.config.resolved())
            .expect("ldap should parse");
        assert_eq!(ldap.shell_entry_command(&state.session.scope), Some("ldap"));
        state.session.scope.enter("ldap");
        let mreg = repl::ReplParsedLine::parse("mreg", state.config.resolved())
            .expect("mreg should parse");
        assert_eq!(mreg.shell_entry_command(&state.session.scope), Some("mreg"));
        assert_eq!(ldap.shell_entry_command(&state.session.scope), None);
    }

    #[test]
    fn repl_partial_root_completion_does_not_enter_shell_unit() {
        let state = make_completion_state(None);
        let catalog = sample_catalog();
        let surface = surface::build_repl_surface(&state, &catalog);
        let tree = completion::build_repl_completion_tree(&state, &surface);
        let engine = osp_completion::CompletionEngine::new(tree);

        let (_, suggestions) = engine.complete("or", 2);
        assert!(suggestions.into_iter().any(|entry| matches!(
            entry,
            osp_completion::SuggestionOutput::Item(item) if item.text == "orch"
        )));

        let parsed = repl::ReplParsedLine::parse("or", state.config.resolved())
            .expect("partial command should parse");
        assert_eq!(parsed.shell_entry_command(&state.session.scope), None);
    }

    #[test]
    fn repl_shell_scoped_completion_and_dispatch_prefix_align_unit() {
        let mut state = make_completion_state(None);
        state.session.scope.enter("orch");
        let catalog = sample_catalog();
        let surface = surface::build_repl_surface(&state, &catalog);
        let tree = completion::build_repl_completion_tree(&state, &surface);
        let engine = osp_completion::CompletionEngine::new(tree);

        let (_, suggestions) = engine.complete("prov", 4);
        assert!(suggestions.into_iter().any(|entry| matches!(
            entry,
            osp_completion::SuggestionOutput::Item(item) if item.text == "provision"
        )));

        let parsed = repl::ReplParsedLine::parse("provision --os alma", state.config.resolved())
            .expect("scoped command should parse");
        assert_eq!(
            parsed.prefixed_tokens(&state.session.scope),
            vec![
                "orch".to_string(),
                "provision".to_string(),
                "--os".to_string(),
                "alma".to_string()
            ]
        );
    }

    #[test]
    fn repl_alias_partial_completion_does_not_trigger_shell_entry_unit() {
        let state = make_completion_state_with_entries(
            None,
            &[("alias.ops", "orch provision --provider vmware")],
        );
        let catalog = sample_catalog();
        let surface = surface::build_repl_surface(&state, &catalog);
        let tree = completion::build_repl_completion_tree(&state, &surface);
        let engine = osp_completion::CompletionEngine::new(tree);

        let (_, suggestions) = engine.complete("op", 2);
        assert!(suggestions.into_iter().any(|entry| matches!(
            entry,
            osp_completion::SuggestionOutput::Item(item) if item.text == "ops"
        )));

        let parsed = repl::ReplParsedLine::parse("op", state.config.resolved())
            .expect("partial alias should parse");
        assert_eq!(parsed.shell_entry_command(&state.session.scope), None);
    }

    #[test]
    fn repl_help_chrome_replaces_clap_headings_unit() {
        let state = make_completion_state(None);
        let raw =
            "Usage: config <COMMAND>\n\nCommands:\n  show\n\nOptions:\n  -h, --help  Print help\n";
        let rendered = repl_help::render_repl_help_with_chrome(&state, raw);
        assert!(rendered.contains("  Usage: config <COMMAND>"));
        assert!(rendered.contains("Commands:"));
        assert!(rendered.contains("Options:"));
    }

    #[test]
    fn repl_help_chrome_passthrough_without_known_sections_unit() {
        let state = make_completion_state(None);
        let raw = "custom help text";
        assert_eq!(repl_help::render_repl_help_with_chrome(&state, raw), raw);
    }

    #[test]
    fn help_render_overrides_parse_long_flags_unit() {
        let args = vec![
            OsString::from("osp"),
            OsString::from("--profile"),
            OsString::from("tsd"),
            OsString::from("--theme=dracula"),
            OsString::from("--mode"),
            OsString::from("plain"),
            OsString::from("--color=always"),
            OsString::from("--unicode"),
            OsString::from("never"),
            OsString::from("--no-env"),
            OsString::from("--no-config-file"),
            OsString::from("--ascii"),
        ];

        let parsed = parse_help_render_overrides(&args);
        assert_eq!(parsed.profile.as_deref(), Some("tsd"));
        assert_eq!(parsed.theme.as_deref(), Some("dracula"));
        assert_eq!(parsed.mode, Some(osp_core::output::RenderMode::Plain));
        assert_eq!(parsed.color, Some(osp_core::output::ColorMode::Always));
        assert_eq!(parsed.unicode, Some(osp_core::output::UnicodeMode::Never));
        assert!(parsed.no_env);
        assert!(parsed.no_config_file);
        assert!(parsed.ascii_legacy);
    }

    #[test]
    fn help_render_overrides_skips_next_flag_value_unit() {
        let args = vec![
            OsString::from("osp"),
            OsString::from("--mode"),
            OsString::from("--profile"),
            OsString::from("tsd"),
        ];
        let parsed = parse_help_render_overrides(&args);
        assert_eq!(parsed.mode, None);
        assert_eq!(parsed.profile.as_deref(), Some("tsd"));
    }

    #[test]
    fn help_chrome_uses_unicode_dividers_when_enabled_unit() {
        let state = make_completion_state(None);
        let mut resolved = state.ui.render_settings.resolve_render_settings();
        resolved.unicode = true;
        let rendered = repl_help::render_help_with_chrome(
            "Usage: osp [OPTIONS]\n\nCommands:\n  help\n\nOptions:\n  -h, --help\n",
            &resolved,
        );
        assert!(rendered.contains("Usage: osp [OPTIONS]"));
        assert!(rendered.contains("Commands:"));
        assert!(rendered.contains("Options:"));
    }

    #[test]
    fn sensitive_key_detection_handles_common_variants_unit() {
        assert!(is_sensitive_key("auth.api_key"));
        assert!(is_sensitive_key("ssh.private_key"));
        assert!(is_sensitive_key("oauth.access_token"));
        assert!(is_sensitive_key("client_secret"));
        assert!(is_sensitive_key("bearer_token"));
        assert!(!is_sensitive_key("ui.keybinding"));
        assert!(!is_sensitive_key("monkey.business"));
    }

    #[test]
    fn repl_completion_tree_contains_builtin_and_plugin_commands_unit() {
        let state = make_completion_state(None);
        let catalog = sample_catalog();
        let surface = surface::build_repl_surface(&state, &catalog);

        let tree = completion::build_repl_completion_tree(&state, &surface);
        assert!(tree.root.children.contains_key("help"));
        assert!(tree.root.children.contains_key("exit"));
        assert!(tree.root.children.contains_key("quit"));
        assert!(tree.root.children.contains_key("plugins"));
        assert!(tree.root.children.contains_key("theme"));
        assert!(tree.root.children.contains_key("config"));
        assert!(tree.root.children.contains_key("history"));
        assert!(tree.root.children.contains_key("orch"));
        assert!(
            tree.root.children["orch"]
                .children
                .contains_key("provision")
        );
        assert_eq!(
            tree.root.children["orch"].tooltip.as_deref(),
            Some("Provision orchestrator resources")
        );
        assert!(tree.pipe_verbs.contains_key("F"));
    }

    #[test]
    fn repl_completion_tree_injects_config_set_schema_keys_unit() {
        let state = make_completion_state(None);
        let catalog = sample_catalog();
        let surface = surface::build_repl_surface(&state, &catalog);

        let tree = completion::build_repl_completion_tree(&state, &surface);
        let set_node = &tree.root.children["config"].children["set"];
        let ui_mode = &set_node.children["ui.mode"];
        assert!(ui_mode.value_key);
        assert!(ui_mode.children.contains_key("auto"));
        assert!(ui_mode.children.contains_key("plain"));
        assert!(ui_mode.children.contains_key("rich"));

        let repl_intro = &set_node.children["repl.intro"];
        assert!(repl_intro.children.contains_key("true"));
        assert!(repl_intro.children.contains_key("false"));
    }

    #[test]
    fn repl_completion_tree_respects_builtin_visibility_unit() {
        let state = make_completion_state(Some("theme"));
        let catalog = sample_catalog();
        let surface = surface::build_repl_surface(&state, &catalog);

        let tree = completion::build_repl_completion_tree(&state, &surface);
        assert!(tree.root.children.contains_key("theme"));
        assert!(!tree.root.children.contains_key("config"));
        assert!(!tree.root.children.contains_key("plugins"));
        assert!(!tree.root.children.contains_key("history"));
    }

    #[test]
    fn repl_completion_tree_roots_to_active_shell_scope_unit() {
        let mut state = make_completion_state(None);
        state.session.scope.enter("orch");
        let catalog = sample_catalog();
        let surface = surface::build_repl_surface(&state, &catalog);

        let tree = completion::build_repl_completion_tree(&state, &surface);
        assert!(!tree.root.children.contains_key("orch"));
        assert!(tree.root.children.contains_key("provision"));
        assert!(tree.root.children.contains_key("help"));
        assert!(tree.root.children.contains_key("exit"));
        assert!(tree.root.children.contains_key("quit"));
    }

    #[test]
    fn repl_surface_drives_overview_and_completion_visibility_unit() {
        let state = make_completion_state(Some("theme config"));
        let catalog = sample_catalog();
        let surface = surface::build_repl_surface(&state, &catalog);

        let names = surface
            .overview_entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names[..2], ["exit", "help"]);
        assert!(names.contains(&"theme"));
        assert!(names.contains(&"config"));
        assert!(names.contains(&"orch"));
        assert!(!names.contains(&"plugins"));
        assert!(!names.contains(&"history"));
        assert!(surface.root_words.contains(&"theme".to_string()));
        assert!(surface.root_words.contains(&"config".to_string()));
        assert!(surface.root_words.contains(&"orch".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn repl_plugin_error_payload_is_handled_as_error_unit() {
        use std::os::unix::fs::PermissionsExt;

        let dir = make_temp_dir("osp-cli-repl-error-plugin");
        let plugin_path = dir.join("osp-fail");
        std::fs::write(
            &plugin_path,
            r#"#!/usr/bin/env bash
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{"protocol_version":1,"plugin_id":"fail","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{"name":"fail","about":"fail","subcommands":[],"args":[],"flags":{}}]}
JSON
  exit 0
fi
cat <<'JSON'
{"protocol_version":1,"ok":false,"data":{},"error":{"code":"MOCK_ERR","message":"mock failure","details":{}},"meta":{}}
JSON
"#,
        )
        .expect("plugin script should be written");
        let mut perms = std::fs::metadata(&plugin_path)
            .expect("metadata should be readable")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&plugin_path, perms).expect("script should be executable");

        let mut state = make_test_state(vec![dir.clone()]);

        let history = make_test_history(&mut state);
        let err = repl::execute_repl_plugin_line(&mut state, &history, "fail")
            .expect_err("response ok=false should become repl error");
        assert!(err.to_string().contains("MOCK_ERR: mock failure"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn repl_records_last_rows_and_bounded_cache_unit() {
        use std::os::unix::fs::PermissionsExt;

        let dir = make_temp_dir("osp-cli-repl-session-plugin");
        let plugin_path = dir.join("osp-cache");
        std::fs::write(
            &plugin_path,
            r#"#!/usr/bin/env bash
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{"protocol_version":1,"plugin_id":"cache","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{"name":"cache","about":"cache plugin","subcommands":[],"args":[],"flags":{}}]}
JSON
  exit 0
fi
cat <<'JSON'
{"protocol_version":1,"ok":true,"data":{"message":"ok"},"error":null,"meta":{"format_hint":"table","columns":["message"]}}
JSON
"#,
        )
        .expect("plugin script should be written");
        let mut perms = std::fs::metadata(&plugin_path)
            .expect("metadata should be readable")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&plugin_path, perms).expect("script should be executable");

        let mut state = make_test_state(vec![dir.clone()]);
        state.session.max_cached_results = 1;

        let history = make_test_history(&mut state);
        let first = repl::execute_repl_plugin_line(&mut state, &history, "cache first")
            .expect("first command should succeed");
        match first {
            osp_repl::ReplLineResult::Continue(text) => assert!(text.contains("ok")),
            other => panic!("unexpected repl result: {other:?}"),
        }

        let second = repl::execute_repl_plugin_line(&mut state, &history, "cache second")
            .expect("second command should succeed");
        match second {
            osp_repl::ReplLineResult::Continue(text) => assert!(text.contains("ok")),
            other => panic!("unexpected repl result: {other:?}"),
        }

        assert_eq!(state.repl_cache_size(), 1);
        assert!(state.cached_repl_rows("cache first").is_none());
        assert!(state.cached_repl_rows("cache second").is_some());
        assert!(!state.last_repl_rows().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn rebuild_repl_state_preserves_session_defaults_and_shell_context_unit() {
        let mut state = make_test_state(Vec::new());
        state
            .session
            .config_overrides
            .set("user.name", "launch-user");
        state
            .session
            .config_overrides
            .set("ui.verbosity.level", "trace");
        state.session.config_overrides.set("debug.level", 2i64);
        state.session.config_overrides.set("ui.format", "json");
        state.session.config_overrides.set("theme.name", "dracula");
        state.session.scope.enter("orch");

        state.repl.history_shell = HistoryShellContext::default();
        state.sync_history_shell_context();

        let next = super::rebuild_repl_state(&state).expect("rebuild should succeed");

        assert_eq!(
            next.config.resolved().get_string("user.name"),
            Some("launch-user")
        );
        assert_eq!(next.ui.message_verbosity, MessageLevel::Trace);
        assert_eq!(next.ui.debug_verbosity, 2);
        assert_eq!(next.ui.render_settings.format, OutputFormat::Json);
        assert_eq!(next.ui.render_settings.theme_name, "dracula");
        assert_eq!(next.session.scope.commands(), vec!["orch".to_string()]);
        assert_eq!(next.repl.history_shell.prefix(), Some("orch ".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn rebuild_repl_state_preserves_session_render_defaults_unit() {
        let mut state = make_test_state(Vec::new());
        state.session.config_overrides.set("ui.format", "table");

        let next = super::rebuild_repl_state(&state).expect("rebuild should succeed");

        assert_eq!(next.ui.render_settings.format, OutputFormat::Table);
    }

    #[cfg(unix)]
    #[test]
    fn repl_reload_intent_matches_command_scope_unit() {
        let mut state = make_test_state(Vec::new());
        state.themes = crate::theme_loader::load_theme_catalog(state.config.resolved());
        let history = make_test_history(&mut state);

        let theme_result =
            repl::execute_repl_plugin_line(&mut state, &history, "theme use dracula")
                .expect("theme use should succeed");
        assert!(matches!(
            theme_result,
            osp_repl::ReplLineResult::Restart {
                reload: osp_repl::ReplReloadKind::WithIntro,
                ..
            }
        ));

        let format_result =
            repl::execute_repl_plugin_line(&mut state, &history, "config set ui.format json")
                .expect("config set should succeed");
        assert!(matches!(
            format_result,
            osp_repl::ReplLineResult::Restart {
                reload: osp_repl::ReplReloadKind::Default,
                ..
            }
        ));

        let color_result = repl::execute_repl_plugin_line(
            &mut state,
            &history,
            "config set color.prompt.text '#ffffff'",
        )
        .expect("color config set should succeed");
        assert!(matches!(
            color_result,
            osp_repl::ReplLineResult::Restart {
                reload: osp_repl::ReplReloadKind::WithIntro,
                ..
            }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn repl_exit_is_host_owned_at_root_but_leaves_shell_in_scope_unit() {
        let mut state = make_test_state(Vec::new());
        let history = make_test_history(&mut state);

        let root_exit = repl::execute_repl_plugin_line(&mut state, &history, "exit")
            .expect("root exit should be handled by host dispatch");
        assert_eq!(root_exit, osp_repl::ReplLineResult::Exit(0));

        state.session.scope.enter("orch");
        let shell_exit = repl::execute_repl_plugin_line(&mut state, &history, "exit")
            .expect("shell exit should leave the current shell");
        match shell_exit {
            osp_repl::ReplLineResult::Continue(text) => {
                assert!(text.contains("Leaving orch shell"));
            }
            other => panic!("unexpected repl result: {other:?}"),
        }
        assert!(state.session.scope.is_root());
    }

    #[cfg(unix)]
    #[test]
    fn repl_failure_is_cached_for_doctor_last_unit() {
        let mut state = make_test_state(Vec::new());
        let history = make_test_history(&mut state);

        let err = repl::execute_repl_plugin_line(&mut state, &history, "missing")
            .expect_err("unknown command should fail");
        assert!(
            err.to_string()
                .contains("no plugin provides command: missing")
        );

        let last = state
            .last_repl_failure()
            .expect("last failure should be recorded");
        assert_eq!(last.command_line, "missing");
        assert!(last.summary.contains("no plugin provides command: missing"));

        let rendered = doctor_cmd::run_doctor_repl_command(
            &mut state,
            crate::cli::DoctorArgs {
                command: Some(crate::cli::DoctorCommands::Last),
            },
            MessageLevel::Success,
        )
        .expect("doctor last should render");
        match rendered {
            ReplCommandOutput::Text(text) => {
                assert!(text.contains("\"status\": \"error\""));
                assert!(text.contains("\"command\": \"missing\""));
            }
            ReplCommandOutput::Output { .. } => panic!("unexpected doctor output variant"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn rebuild_repl_state_preserves_last_failure_unit() {
        let mut state = make_test_state(Vec::new());
        state.record_repl_failure("ldap user nope", "boom", "boom detail");

        let next = super::rebuild_repl_state(&state).expect("rebuild should succeed");
        let last = next
            .last_repl_failure()
            .expect("last failure should survive rebuild");

        assert_eq!(last.command_line, "ldap user nope");
        assert_eq!(last.summary, "boom");
        assert_eq!(last.detail, "boom detail");
    }

    #[cfg(unix)]
    #[test]
    fn repl_bang_expands_last_visible_command_unit() {
        use std::os::unix::fs::PermissionsExt;

        let dir = make_temp_dir("osp-cli-repl-bang-plugin");
        let plugin_path = dir.join("osp-cache");
        std::fs::write(
            &plugin_path,
            r#"#!/usr/bin/env bash
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{"protocol_version":1,"plugin_id":"cache","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{"name":"cache","about":"cache plugin","subcommands":[],"args":[],"flags":{}}]}
JSON
  exit 0
fi
printf '{"protocol_version":1,"ok":true,"data":{"message":"ok","arg":"%s"},"error":null,"meta":{"format_hint":"table","columns":["message","arg"]}}\n' "$2"
"#,
        )
        .expect("plugin script should be written");
        let mut perms = std::fs::metadata(&plugin_path)
            .expect("metadata should be readable")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&plugin_path, perms).expect("script should be executable");

        let mut state = make_test_state(vec![dir.clone()]);
        let history = make_test_history(&mut state);

        repl::execute_repl_plugin_line(&mut state, &history, "cache first")
            .expect("seed command should succeed");
        history
            .save_command_line("cache first")
            .expect("history seed should save");
        let cache_size_before = state.repl_cache_size();
        let expanded = repl::execute_repl_plugin_line(&mut state, &history, "!!")
            .expect("bang expansion should succeed");
        match expanded {
            osp_repl::ReplLineResult::ReplaceInput(text) => {
                assert_eq!(text, "cache first");
            }
            other => panic!("unexpected repl result: {other:?}"),
        }
        assert_eq!(state.repl_cache_size(), cache_size_before);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn repl_bang_contains_search_expands_matching_command_unit() {
        use std::os::unix::fs::PermissionsExt;

        let dir = make_temp_dir("osp-cli-repl-bang-contains-plugin");
        let plugin_path = dir.join("osp-cache");
        std::fs::write(
            &plugin_path,
            r#"#!/usr/bin/env bash
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{"protocol_version":1,"plugin_id":"cache","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{"name":"cache","about":"cache plugin","subcommands":[],"args":[],"flags":{}}]}
JSON
  exit 0
fi
printf '{"protocol_version":1,"ok":true,"data":{"message":"ok","arg":"%s"},"error":null,"meta":{"format_hint":"table","columns":["message","arg"]}}\n' "$2"
"#,
        )
        .expect("plugin script should be written");
        let mut perms = std::fs::metadata(&plugin_path)
            .expect("metadata should be readable")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&plugin_path, perms).expect("script should be executable");

        let mut state = make_test_state(vec![dir.clone()]);
        let history = make_test_history(&mut state);

        repl::execute_repl_plugin_line(&mut state, &history, "cache alpha")
            .expect("first seed command should succeed");
        history
            .save_command_line("cache alpha")
            .expect("history seed should save");
        repl::execute_repl_plugin_line(&mut state, &history, "cache beta")
            .expect("second seed command should succeed");
        history
            .save_command_line("cache beta")
            .expect("history seed should save");
        let cache_size_before = state.repl_cache_size();
        let expanded = repl::execute_repl_plugin_line(&mut state, &history, "!?alpha")
            .expect("contains bang expansion should succeed");
        match expanded {
            osp_repl::ReplLineResult::ReplaceInput(text) => {
                assert_eq!(text, "cache alpha");
            }
            other => panic!("unexpected repl result: {other:?}"),
        }
        assert_eq!(state.repl_cache_size(), cache_size_before);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    fn make_test_history(state: &mut AppState) -> SharedHistory {
        let history_dir = make_temp_dir("osp-cli-test-history");
        let history_path = history_dir.join("history.jsonl");
        let history_shell = state.repl.history_shell.clone();
        state.sync_history_shell_context();

        let history_config = HistoryConfig {
            path: Some(history_path),
            max_entries: 128,
            enabled: true,
            dedupe: true,
            profile_scoped: true,
            exclude_patterns: Vec::new(),
            profile: Some(state.config.resolved().active_profile().to_string()),
            terminal: Some(
                state
                    .context
                    .terminal_kind()
                    .as_config_terminal()
                    .to_string(),
            ),
            shell_context: history_shell,
        }
        .normalized();

        SharedHistory::new(history_config).expect("history should init")
    }

    #[cfg(unix)]
    fn make_test_state(plugin_dirs: Vec<std::path::PathBuf>) -> AppState {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        let config = resolver
            .resolve(ResolveOptions::default().with_terminal("repl"))
            .expect("test config should resolve");

        let settings = RenderSettings {
            format: OutputFormat::Json,
            mode: RenderMode::Plain,
            color: ColorMode::Never,
            unicode: UnicodeMode::Never,
            width: None,
            margin: 0,
            indent_size: 2,
            short_list_max: 1,
            medium_list_max: 5,
            grid_padding: 4,
            grid_columns: None,
            column_weight: 3,
            table_overflow: osp_ui::TableOverflow::Clip,
            mreg_stack_min_col_width: 10,
            mreg_stack_overflow_ratio: 200,
            theme_name: DEFAULT_THEME_NAME.to_string(),
            theme: None,
            style_overrides: osp_ui::StyleOverrides::default(),
            runtime: RenderRuntime::default(),
        };

        let config_root = make_temp_dir("osp-cli-test-config");
        let cache_root = make_temp_dir("osp-cli-test-cache");
        let launch = LaunchContext {
            plugin_dirs: plugin_dirs.clone(),
            config_root: Some(config_root.clone()),
            cache_root: Some(cache_root.clone()),
            runtime_load: RuntimeLoadOptions::default(),
        };

        AppState::new(AppStateInit {
            context: RuntimeContext::new(None, TerminalKind::Repl, None),
            config,
            render_settings: settings,
            message_verbosity: MessageLevel::Success,
            debug_verbosity: 0,
            plugins: PluginManager::new(plugin_dirs)
                .with_roots(Some(config_root), Some(cache_root)),
            themes: crate::theme_loader::ThemeCatalog::default(),
            launch,
        })
    }

    #[cfg(unix)]
    fn make_temp_dir(prefix: &str) -> std::path::PathBuf {
        let mut dir = std::env::temp_dir();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be valid")
            .as_nanos();
        dir.push(format!("{prefix}-{nonce}"));
        std::fs::create_dir_all(&dir).expect("temp dir should be created");
        dir
    }
}
