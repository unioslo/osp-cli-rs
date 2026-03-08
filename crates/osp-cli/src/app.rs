use clap::Parser;
use miette::{IntoDiagnostic, Result, miette};
use osp_config::{ConfigValue, DEFAULT_UI_WIDTH, ResolvedConfig};
use osp_core::output::OutputFormat;
use osp_core::runtime::{RuntimeHints, RuntimeTerminalKind, UiVerbosity};
use osp_repl::SharedHistory;

use osp_ui::messages::MessageLevel;
use osp_ui::theme::{DEFAULT_THEME_NAME, normalize_theme_name};
use osp_ui::{RenderRuntime, RenderSettings};
use std::borrow::Cow;
use std::ffi::OsString;
use std::io::IsTerminal;
use std::time::Instant;
use terminal_size::{Width, terminal_size};

use crate::cli::commands::{
    config as config_cmd, doctor as doctor_cmd, history as history_cmd, plugins as plugins_cmd,
    theme as theme_cmd,
};
use crate::cli::{Cli, Commands};
use crate::invocation::{InvocationOptions, append_invocation_help_if_verbose, scan_cli_argv};
use crate::logging::{bootstrap_logging_config, init_developer_logging};
use crate::plugin_manager::{
    CommandCatalogEntry, DEFAULT_PLUGIN_PROCESS_TIMEOUT_MS, PluginDispatchContext,
    PluginDispatchError, PluginManager,
};
use crate::state::{
    AppClients, AppRuntime, AppSession, AuthState, LaunchContext, TerminalKind, UiState,
};
use crate::ui_sink::{StdIoUiSink, UiSink};

use crate::repl;

mod bootstrap;
mod command_output;
mod config_explain;
mod dispatch;
mod external;
mod help;
mod repl_lifecycle;
mod timing;

pub(crate) use crate::plugin_config::{
    PluginConfigEntry, PluginConfigScope, effective_plugin_config_entries,
};
#[cfg(test)]
pub(crate) use crate::plugin_config::{
    collect_plugin_config_env, config_value_to_plugin_env, plugin_config_env_name,
};
use crate::repl::help as repl_help;
use crate::theme_loader;
pub(crate) use bootstrap::{
    RuntimeConfigRequest, build_app_state, build_cli_session_layer, build_logging_config,
    build_runtime_context, effective_debug_verbosity, effective_message_verbosity,
    resolve_runtime_config,
};
pub(crate) use command_output::{
    CliCommandResult, CommandRenderRuntime, PreparedPluginResponse, ReplCommandOutput,
    apply_output_stages, document_from_json, document_from_text, emit_messages_for_ui,
    maybe_copy_output_with_runtime, prepare_plugin_response, run_cli_command,
};
pub(crate) use config_explain::{
    ConfigExplainContext, config_explain_json, config_explain_result, config_value_to_json,
    explain_runtime_config, format_scope, is_sensitive_key, render_config_explain_text,
};
pub(crate) use dispatch::{
    RunAction, build_dispatch_plan, ensure_builtin_visible_for, ensure_dispatch_visibility,
    ensure_plugin_visible_for, normalize_cli_profile, normalize_profile_override,
};
pub(crate) use external::run_external_command_with_help_renderer;
use external::{ExternalCommandRuntime, run_external_command};
pub(crate) use repl_lifecycle::rebuild_repl_parts;
#[cfg(test)]
pub(crate) use repl_lifecycle::rebuild_repl_state;
pub(crate) use timing::{TimingSummary, format_timing_badge, right_align_timing_line};

pub(crate) const CMD_PLUGINS: &str = "plugins";
pub(crate) const CMD_DOCTOR: &str = "doctor";
pub(crate) const CMD_CONFIG: &str = "config";
pub(crate) const CMD_THEME: &str = "theme";
pub(crate) const CMD_HISTORY: &str = "history";
pub(crate) const CMD_HELP: &str = "help";
pub(crate) const CMD_LIST: &str = "list";
pub(crate) const CMD_SHOW: &str = "show";
pub(crate) const CMD_USE: &str = "use";
pub const EXIT_CODE_ERROR: i32 = 1;
pub const EXIT_CODE_USAGE: i32 = 2;
pub const EXIT_CODE_CONFIG: i32 = 3;
pub const EXIT_CODE_PLUGIN: i32 = 4;
pub(crate) const DEFAULT_REPL_PROMPT: &str = "╭─{user}@{domain} {indicator}\n╰─{profile}> ";
pub(crate) const CURRENT_TERMINAL_SENTINEL: &str = "__current__";
pub(crate) const REPL_SHELLABLE_COMMANDS: [&str; 5] = ["nh", "mreg", "ldap", "vm", "orch"];

#[derive(Debug, Clone)]
pub(crate) struct ReplCommandSpec {
    pub(crate) name: Cow<'static, str>,
    pub(crate) supports_dsl: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct EffectiveInvocation {
    pub(crate) ui: UiState,
    pub(crate) plugin_provider: Option<String>,
    pub(crate) show_invocation_help: bool,
}

#[derive(Debug)]
struct ContextError<E> {
    context: &'static str,
    source: E,
}

#[derive(Clone, Copy)]
struct KnownErrorChain<'a> {
    clap: Option<&'a clap::Error>,
    config: Option<&'a osp_config::ConfigError>,
    plugin: Option<&'a PluginDispatchError>,
}

impl<'a> KnownErrorChain<'a> {
    fn inspect(err: &'a miette::Report) -> Self {
        Self {
            clap: find_error_in_chain::<clap::Error>(err),
            config: find_error_in_chain::<osp_config::ConfigError>(err),
            plugin: find_error_in_chain::<PluginDispatchError>(err),
        }
    }
}

impl<E> std::fmt::Display for ContextError<E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.context)
    }
}

impl<E> std::error::Error for ContextError<E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

pub fn run_from<I, T>(args: I) -> Result<i32>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let mut sink = StdIoUiSink;
    run_from_with_sink(args, &mut sink)
}

pub(crate) fn run_from_with_sink<I, T>(args: I, sink: &mut dyn UiSink) -> Result<i32>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let argv = args.into_iter().map(Into::into).collect::<Vec<OsString>>();
    init_developer_logging(bootstrap_logging_config(&argv));
    let scanned = scan_cli_argv(&argv)?;
    match Cli::try_parse_from(scanned.argv.iter().cloned()) {
        Ok(cli) => run(cli, scanned.invocation, sink),
        Err(err) => handle_clap_parse_error(&argv, &scanned.invocation, err, sink),
    }
}

fn handle_clap_parse_error(
    args: &[OsString],
    invocation: &InvocationOptions,
    err: clap::Error,
    sink: &mut dyn UiSink,
) -> Result<i32> {
    match err.kind() {
        clap::error::ErrorKind::DisplayHelp => {
            let help_context = help::render_settings_for_help(args);
            let rendered = repl_help::render_help_with_chrome(
                &append_invocation_help_if_verbose(&err.to_string(), invocation),
                &help_context.settings.resolve_render_settings(),
                help_context.layout,
            );
            sink.write_stdout(&rendered);
            Ok(0)
        }
        clap::error::ErrorKind::DisplayVersion => {
            sink.write_stdout(&err.to_string());
            Ok(0)
        }
        _ => Err(report_std_error_with_context(
            err,
            "failed to parse CLI arguments",
        )),
    }
}

// Keep the top-level CLI entrypoint readable as a table of contents:
// normalize input -> bootstrap runtime state -> hand off to the selected mode.
fn run(mut cli: Cli, invocation: InvocationOptions, sink: &mut dyn UiSink) -> Result<i32> {
    let run_started = Instant::now();
    if invocation.cache {
        return Err(miette!(
            "`--cache` is only available inside the interactive REPL"
        ));
    }

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
    tracing::debug!(
        action = ?dispatch.action,
        profile_override = ?dispatch.profile_override,
        known_profiles = known_profiles.len(),
        "built dispatch plan"
    );

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

    let plugin_manager = PluginManager::new(cli.plugin_dirs.clone())
        .with_process_timeout(plugin_process_timeout(&config))
        .with_path_discovery(plugin_path_discovery_enabled(&config));

    let mut state = build_app_state(crate::state::AppStateInit {
        context: runtime_context,
        config,
        render_settings,
        message_verbosity,
        debug_verbosity,
        plugins: plugin_manager,
        themes: theme_catalog.clone(),
        launch: launch_context,
    });
    if let Some(layer) = session_layer {
        state.session.config_overrides = layer;
    }
    ensure_dispatch_visibility(&state.runtime.auth, &dispatch.action)?;
    let effective_invocation = resolve_effective_invocation(&state.runtime.ui, &invocation);
    init_developer_logging(build_logging_config(
        state.runtime.config.resolved(),
        effective_invocation.ui.debug_verbosity,
    ));
    theme_loader::log_theme_issues(&theme_catalog.issues);
    tracing::debug!(
        debug_count = effective_invocation.ui.debug_verbosity,
        "developer logging initialized"
    );

    tracing::info!(
        profile = %state.runtime.config.resolved().active_profile(),
        terminal = %state.runtime.context.terminal_kind().as_config_terminal(),
        action = ?dispatch.action,
        plugin_timeout_ms = plugin_process_timeout(state.runtime.config.resolved()).as_millis(),
        "osp session initialized"
    );

    let action_started = Instant::now();
    let is_repl = matches!(dispatch.action, RunAction::Repl);
    let action = dispatch.action;
    let result = match action {
        RunAction::Repl => {
            state.runtime.ui = effective_invocation.ui.clone();
            repl::run_plugin_repl(&mut state)
        }
        RunAction::ReplCommand(args) => run_builtin_cli_command_parts(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            &effective_invocation,
            Commands::Repl(args),
            sink,
        ),
        RunAction::Plugins(args) => run_builtin_cli_command_parts(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            &effective_invocation,
            Commands::Plugins(args),
            sink,
        ),
        RunAction::Doctor(args) => run_builtin_cli_command_parts(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            &effective_invocation,
            Commands::Doctor(args),
            sink,
        ),
        RunAction::Theme(args) => run_builtin_cli_command_parts(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            &effective_invocation,
            Commands::Theme(args),
            sink,
        ),
        RunAction::Config(args) => run_builtin_cli_command_parts(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            &effective_invocation,
            Commands::Config(args),
            sink,
        ),
        RunAction::History(args) => run_builtin_cli_command_parts(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            &effective_invocation,
            Commands::History(args),
            sink,
        ),
        RunAction::External(ref tokens) => run_external_command(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            tokens,
            &effective_invocation,
        )
        .and_then(|result| {
            run_cli_command(
                &CommandRenderRuntime::new(
                    state.runtime.config.resolved(),
                    &effective_invocation.ui,
                ),
                result,
                sink,
            )
        }),
    };

    if !is_repl && effective_invocation.ui.debug_verbosity > 0 {
        let total = run_started.elapsed();
        let startup = action_started.saturating_duration_since(run_started);
        let command = total.saturating_sub(startup);
        let footer = right_align_timing_line(
            TimingSummary {
                total,
                parse: if effective_invocation.ui.debug_verbosity >= 3 {
                    Some(startup)
                } else {
                    None
                },
                execute: if effective_invocation.ui.debug_verbosity >= 3 {
                    Some(command)
                } else {
                    None
                },
                render: None,
            },
            effective_invocation.ui.debug_verbosity,
            &effective_invocation
                .ui
                .render_settings
                .resolve_render_settings(),
        );
        if !footer.is_empty() {
            sink.write_stderr(&footer);
        }
    }

    result
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
    invocation: &EffectiveInvocation,
    command: Commands,
    sink: &mut dyn UiSink,
) -> Result<i32> {
    let result =
        dispatch_builtin_command_parts(runtime, session, clients, None, Some(invocation), command)?
            .ok_or_else(|| miette!("expected builtin command"))?;
    run_cli_command(
        &CommandRenderRuntime::new(runtime.config.resolved(), &invocation.ui),
        result,
        sink,
    )
}

fn run_inline_builtin_command(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    invocation: Option<&EffectiveInvocation>,
    command: Commands,
    stages: &[String],
) -> Result<Option<CliCommandResult>> {
    if matches!(command, Commands::External(_)) {
        return Ok(None);
    }

    let spec = repl::repl_command_spec(&command);
    ensure_command_supports_dsl(&spec, stages)?;
    dispatch_builtin_command_parts(runtime, session, clients, None, invocation, command)
}

pub(crate) fn dispatch_builtin_command_parts(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    repl_history: Option<&SharedHistory>,
    invocation: Option<&EffectiveInvocation>,
    command: Commands,
) -> Result<Option<CliCommandResult>> {
    let effective_ui = effective_ui_state(&runtime.ui, invocation);
    match command {
        Commands::Plugins(args) => {
            ensure_builtin_visible_for(&runtime.auth, CMD_PLUGINS)?;
            plugins_cmd::run_plugins_command(plugins_command_context(runtime, clients), args)
                .map(Some)
        }
        Commands::Doctor(args) => {
            ensure_builtin_visible_for(&runtime.auth, CMD_DOCTOR)?;
            doctor_cmd::run_doctor_command(
                doctor_cmd::DoctorCommandContext {
                    config: config_read_context(runtime, session, &effective_ui),
                    plugins: plugins_command_context(runtime, clients),
                    ui: &effective_ui,
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
            let ui = &effective_ui;
            let themes = &runtime.themes;
            theme_cmd::run_theme_command(
                &mut session.config_overrides,
                theme_cmd::ThemeCommandContext { ui, themes },
                args,
            )
            .map(Some)
        }
        Commands::Config(args) => {
            ensure_builtin_visible_for(&runtime.auth, CMD_CONFIG)?;
            config_cmd::run_config_command(
                config_command_context(runtime, session, &effective_ui),
                args,
            )
            .map(Some)
        }
        Commands::History(args) => {
            ensure_builtin_visible_for(&runtime.auth, CMD_HISTORY)?;
            match repl_history {
                Some(history) => {
                    history_cmd::run_history_repl_command(session, args, history).map(Some)
                }
                None => history_cmd::run_history_command(args).map(Some),
            }
        }
        Commands::Repl(args) => {
            if repl_history.is_some() {
                Err(miette!("`repl` debug commands are not available in REPL"))
            } else {
                repl::run_repl_debug_command_for(runtime, session, clients, args).map(Some)
            }
        }
        Commands::External(_) => Ok(None),
    }
}

fn plugins_command_context<'a>(
    runtime: &'a AppRuntime,
    clients: &'a AppClients,
) -> plugins_cmd::PluginsCommandContext<'a> {
    plugins_cmd::PluginsCommandContext {
        config: runtime.config.resolved(),
        config_state: Some(&runtime.config),
        auth: &runtime.auth,
        clients: Some(clients),
        plugin_manager: &clients.plugins,
    }
}

fn config_read_context<'a>(
    runtime: &'a AppRuntime,
    session: &'a AppSession,
    ui: &'a UiState,
) -> config_cmd::ConfigReadContext<'a> {
    config_cmd::ConfigReadContext {
        context: &runtime.context,
        config: runtime.config.resolved(),
        ui,
        themes: &runtime.themes,
        config_overrides: &session.config_overrides,
        runtime_load: runtime.launch.runtime_load,
    }
}

fn config_command_context<'a>(
    runtime: &'a AppRuntime,
    session: &'a mut AppSession,
    ui: &'a UiState,
) -> config_cmd::ConfigCommandContext<'a> {
    config_cmd::ConfigCommandContext {
        context: &runtime.context,
        config: runtime.config.resolved(),
        ui,
        themes: &runtime.themes,
        config_overrides: &mut session.config_overrides,
        runtime_load: runtime.launch.runtime_load,
    }
}

fn effective_ui_state(ui: &UiState, invocation: Option<&EffectiveInvocation>) -> UiState {
    let Some(invocation) = invocation else {
        return UiState {
            render_settings: ui.render_settings.clone(),
            message_verbosity: ui.message_verbosity,
            debug_verbosity: ui.debug_verbosity,
        };
    };
    invocation.ui.clone()
}

pub(crate) fn resolve_effective_invocation(
    ui: &UiState,
    invocation: &InvocationOptions,
) -> EffectiveInvocation {
    let mut render_settings = ui.render_settings.clone();
    if let Some(format) = invocation.format {
        render_settings.format = format;
    }
    if let Some(mode) = invocation.mode {
        render_settings.mode = mode;
    }
    if let Some(color) = invocation.color {
        render_settings.color = color;
    }
    if let Some(unicode) = invocation.unicode {
        render_settings.unicode = unicode;
    }

    EffectiveInvocation {
        ui: UiState {
            render_settings,
            message_verbosity: osp_ui::messages::adjust_verbosity(
                ui.message_verbosity,
                invocation.verbose,
                invocation.quiet,
            ),
            debug_verbosity: if invocation.debug > 0 {
                invocation.debug.min(3)
            } else {
                ui.debug_verbosity
            },
        },
        plugin_provider: invocation.plugin_provider.clone(),
        show_invocation_help: invocation.verbose > 0,
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
    report_std_error_with_context(err, "plugin command failed")
}

pub fn classify_exit_code(err: &miette::Report) -> i32 {
    let known = KnownErrorChain::inspect(err);
    if known.clap.is_some() {
        EXIT_CODE_USAGE
    } else if known.config.is_some() {
        EXIT_CODE_CONFIG
    } else if known.plugin.is_some() {
        EXIT_CODE_PLUGIN
    } else {
        EXIT_CODE_ERROR
    }
}

pub fn render_report_message(err: &miette::Report, verbosity: MessageLevel) -> String {
    if verbosity >= MessageLevel::Trace {
        return format!("{err:?}");
    }

    let known = KnownErrorChain::inspect(err);
    let mut message = base_error_message(err, &known);

    if verbosity >= MessageLevel::Info {
        let mut next: Option<&(dyn std::error::Error + 'static)> = Some(err.as_ref());
        while let Some(source) = next {
            let source_text = source.to_string();
            if !source_text.is_empty() && !message.contains(&source_text) {
                message.push_str(": ");
                message.push_str(&source_text);
            }
            next = source.source();
        }
    }

    if verbosity >= MessageLevel::Success
        && let Some(hint) = known_error_hint(&known)
        && !message.contains(hint)
    {
        message.push_str("\nHint: ");
        message.push_str(hint);
    }

    message
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

pub(crate) fn plugin_process_timeout(config: &ResolvedConfig) -> std::time::Duration {
    std::time::Duration::from_millis(config_usize(
        config,
        "extensions.plugins.timeout_ms",
        DEFAULT_PLUGIN_PROCESS_TIMEOUT_MS,
    ) as u64)
}

pub(crate) fn plugin_path_discovery_enabled(config: &ResolvedConfig) -> bool {
    config
        .get_bool("extensions.plugins.discovery.path")
        .unwrap_or(false)
}

fn known_error_hint(known: &KnownErrorChain<'_>) -> Option<&'static str> {
    if let Some(plugin_err) = known.plugin {
        return Some(match plugin_err {
            PluginDispatchError::CommandNotFound { .. } => {
                "run `osp plugins list` and set --plugin-dir or OSP_PLUGIN_PATH"
            }
            PluginDispatchError::CommandAmbiguous { .. } => {
                "rerun with --plugin-provider <plugin-id> or persist a default with `osp plugins select-provider <command> <plugin-id>`"
            }
            PluginDispatchError::ProviderNotFound { .. } => {
                "pick one of the available providers from `osp plugins commands` or `osp plugins doctor`"
            }
            PluginDispatchError::ExecuteFailed { .. } => {
                "verify the plugin executable exists and is executable"
            }
            PluginDispatchError::TimedOut { .. } => {
                "increase extensions.plugins.timeout_ms or inspect the plugin executable"
            }
            PluginDispatchError::NonZeroExit { .. } => {
                "inspect the plugin stderr output or rerun with -v/-vv for more context"
            }
            PluginDispatchError::InvalidJsonResponse { .. }
            | PluginDispatchError::InvalidResponsePayload { .. } => {
                "inspect the plugin response contract and stderr output"
            }
        });
    }

    if let Some(config_err) = known.config {
        return Some(match config_err {
            osp_config::ConfigError::UnknownProfile { .. } => {
                "run `osp config explain profile.default` or choose a known profile"
            }
            osp_config::ConfigError::InsecureSecretsPermissions { .. } => {
                "restrict the secrets file permissions to 0600"
            }
            _ => "run `osp config explain <key>` to inspect config provenance",
        });
    }

    if known.clap.is_some() {
        return Some("use --help to inspect accepted flags and subcommands");
    }

    None
}

fn base_error_message(err: &miette::Report, known: &KnownErrorChain<'_>) -> String {
    if let Some(plugin_err) = known.plugin {
        return plugin_err.to_string();
    }

    if let Some(config_err) = known.config {
        return config_err.to_string();
    }

    if let Some(clap_err) = known.clap {
        return clap_err.to_string();
    }

    err.to_string()
}

fn report_std_error_with_context<E>(err: E, context: &'static str) -> miette::Report
where
    E: std::error::Error + Send + Sync + 'static,
{
    Err::<(), ContextError<E>>(ContextError {
        context,
        source: err,
    })
    .into_diagnostic()
    .unwrap_err()
}

fn find_error_in_chain<E>(err: &miette::Report) -> Option<&E>
where
    E: std::error::Error + 'static,
{
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err.as_ref());
    while let Some(source) = current {
        if let Some(found) = source.downcast_ref::<E>() {
            return Some(found);
        }
        current = source.source();
    }
    None
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

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn plugin_dispatch_context_for_runtime(
    runtime: &crate::state::AppRuntime,
    clients: &AppClients,
    invocation: Option<&EffectiveInvocation>,
) -> PluginDispatchContext {
    build_plugin_dispatch_context(
        &runtime.context,
        &runtime.config,
        clients,
        invocation.map(|value| &value.ui).unwrap_or(&runtime.ui),
    )
}

fn plugin_dispatch_context_for(
    runtime: &ExternalCommandRuntime<'_>,
    invocation: Option<&EffectiveInvocation>,
) -> PluginDispatchContext {
    build_plugin_dispatch_context(
        runtime.context,
        runtime.config_state,
        runtime.clients,
        invocation.map(|value| &value.ui).unwrap_or(runtime.ui),
    )
}

fn build_plugin_dispatch_context(
    context: &crate::state::RuntimeContext,
    config: &crate::state::ConfigState,
    clients: &AppClients,
    ui: &crate::state::UiState,
) -> PluginDispatchContext {
    let config_env = clients.plugin_config_env(config);
    let terminal_kind = match context.terminal_kind() {
        TerminalKind::Cli => RuntimeTerminalKind::Cli,
        TerminalKind::Repl => RuntimeTerminalKind::Repl,
    };
    PluginDispatchContext {
        runtime_hints: RuntimeHints {
            ui_verbosity: to_ui_verbosity(ui.message_verbosity),
            debug_level: ui.debug_verbosity.min(3),
            format: ui.render_settings.format,
            color: ui.render_settings.color,
            unicode: ui.render_settings.unicode,
            profile: Some(config.resolved().active_profile().to_string()),
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
        provider_override: None,
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
