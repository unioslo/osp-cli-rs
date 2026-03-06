use clap::Parser;
use miette::{IntoDiagnostic, Result, WrapErr, miette};
use osp_config::{
    ConfigLayer, ConfigValue, DEFAULT_UI_WIDTH, ResolveOptions, ResolvedConfig, RuntimeConfigPaths,
    RuntimeDefaults, build_runtime_pipeline,
};
use osp_core::output::OutputFormat;
use osp_core::output_model::OutputResult;
use osp_core::plugin::{ResponseMessageLevelV1, ResponseV1};
use osp_core::runtime::{RuntimeHints, RuntimeTerminalKind, UiVerbosity};
use osp_dsl::apply_output_pipeline;

use osp_ui::clipboard::ClipboardService;
use osp_ui::messages::{MessageBuffer, MessageLevel, MessageRenderFormat};
use osp_ui::theme::{DEFAULT_THEME_NAME, normalize_theme_name};
use osp_ui::{RenderRuntime, RenderSettings, copy_output_to_clipboard, render_output};
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::IsTerminal;
use std::path::PathBuf;
use terminal_size::{Width, terminal_size};

use crate::cli::commands::{
    config as config_cmd, doctor as doctor_cmd, history as history_cmd, plugins as plugins_cmd,
    theme as theme_cmd,
};
use crate::cli::{
    Cli, Commands, ConfigArgs, DoctorArgs, HistoryArgs, PluginsArgs, ReplArgs, ThemeArgs,
    parse_inline_command_tokens,
};
use crate::logging::{
    DeveloperLoggingConfig, FileLoggingConfig, init_developer_logging, parse_level_filter,
};
use crate::pipeline::parse_command_tokens_with_aliases;
use crate::plugin_manager::{
    CommandCatalogEntry, PluginDispatchContext, PluginDispatchError, PluginManager,
};
use crate::rows::output::plugin_data_to_output_result;
use crate::state::{AppState, LaunchContext, RuntimeContext, TerminalKind};

use crate::repl;

mod config_explain;
mod help;

use crate::repl::completion;
use crate::repl::help as repl_help;
use crate::theme_loader;
pub(crate) use config_explain::{
    config_explain_json, config_explain_output, config_value_to_json, explain_runtime_config,
    format_scope, is_sensitive_key, render_config_explain_text,
};

enum RunAction {
    Repl,
    ReplCommand(ReplArgs),
    Plugins(PluginsArgs),
    Doctor(DoctorArgs),
    Theme(ThemeArgs),
    Config(ConfigArgs),
    History(HistoryArgs),
    External(Vec<String>),
}

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

pub(crate) enum ReplCommandOutput {
    Output {
        output: OutputResult,
        format_hint: Option<OutputFormat>,
    },
    Text(String),
}

pub(crate) struct CliCommandResult {
    pub(crate) exit_code: i32,
    pub(crate) output: Option<ReplCommandOutput>,
}

impl CliCommandResult {
    pub(crate) fn exit(exit_code: i32) -> Self {
        Self {
            exit_code,
            output: None,
        }
    }

    pub(crate) fn output(output: OutputResult, format_hint: Option<OutputFormat>) -> Self {
        Self {
            exit_code: 0,
            output: Some(ReplCommandOutput::Output {
                output,
                format_hint,
            }),
        }
    }

    pub(crate) fn text(text: impl Into<String>) -> Self {
        Self {
            exit_code: 0,
            output: Some(ReplCommandOutput::Text(text.into())),
        }
    }
}

struct DispatchPlan {
    action: RunAction,
    profile_override: Option<String>,
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

fn run(mut cli: Cli) -> Result<i32> {
    let normalized_profile = normalize_profile_override(cli.profile.clone());
    cli.profile = normalized_profile.clone();
    let initial_config = resolve_runtime_config(normalized_profile.clone(), Some("cli"), None)?;
    let known_profiles = initial_config.known_profiles().clone();
    let dispatch = build_dispatch_plan(&mut cli, &known_profiles)?;

    let terminal_kind = match dispatch.action {
        RunAction::Repl | RunAction::ReplCommand(_) => TerminalKind::Repl,
        RunAction::Plugins(_)
        | RunAction::Doctor(_)
        | RunAction::Theme(_)
        | RunAction::Config(_)
        | RunAction::History(_)
        | RunAction::External(_) => TerminalKind::Cli,
    };
    let runtime_context = RuntimeContext::new(
        dispatch.profile_override.clone(),
        terminal_kind,
        std::env::var("TERM").ok(),
    );
    let session_layer = build_cli_session_layer(
        &cli,
        runtime_context.profile_override().map(ToOwned::to_owned),
        runtime_context.terminal_kind(),
    )?;
    let launch_context = LaunchContext {
        plugin_dirs: cli.plugin_dirs.clone(),
        config_root: None,
        cache_root: None,
    };

    let config = resolve_runtime_config(
        runtime_context.profile_override().map(ToOwned::to_owned),
        Some(runtime_context.terminal_kind().as_config_terminal()),
        session_layer.clone(),
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

    let mut state = AppState::new(
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
        RunAction::ReplCommand(args) => {
            let result = repl::run_repl_debug_command(&state, args)?;
            run_cli_command(&state, result)
        }
        RunAction::Plugins(args) => {
            let result = plugins_cmd::run_plugins_command(&state, args)?;
            run_cli_command(&state, result)
        }
        RunAction::Doctor(args) => {
            let result = doctor_cmd::run_doctor_command(&mut state, args)?;
            run_cli_command(&state, result)
        }
        RunAction::Theme(args) => {
            let result = theme_cmd::run_theme_command(&mut state, args)?;
            run_cli_command(&state, result)
        }
        RunAction::Config(args) => {
            let result = config_cmd::run_config_command(&mut state, args)?;
            run_cli_command(&state, result)
        }
        RunAction::History(args) => {
            let result = history_cmd::run_history_command(&mut state, args)?;
            run_cli_command(&state, result)
        }
        RunAction::External(tokens) => run_external_command(&mut state, &tokens),
    }
}

fn build_cli_session_layer(
    cli: &Cli,
    profile_override: Option<String>,
    terminal_kind: TerminalKind,
) -> Result<Option<ConfigLayer>> {
    let mut layer = ConfigLayer::default();
    cli.append_static_session_overrides(&mut layer);
    let bootstrap_layer = if layer.entries().is_empty() {
        None
    } else {
        Some(layer.clone())
    };
    let config = resolve_runtime_config(
        profile_override,
        Some(terminal_kind.as_config_terminal()),
        bootstrap_layer,
    )?;
    cli.append_derived_session_overrides(&mut layer, &config);
    Ok(if layer.entries().is_empty() {
        None
    } else {
        Some(layer)
    })
}

pub(crate) fn rebuild_repl_state(current: &AppState) -> Result<AppState> {
    let context = current.context.clone();
    let shell_stack = current.session.shell_stack.clone();
    let history_shell = current.repl.history_shell.clone();
    let result_cache = current.session.result_cache.clone();
    let cache_order = current.session.cache_order.clone();
    let last_rows = current.session.last_rows.clone();
    let session_overrides = current.session.config_overrides.clone();
    let launch = current.launch.clone();

    let config = resolve_runtime_config(
        context.profile_override().map(ToOwned::to_owned),
        Some(context.terminal_kind().as_config_terminal()),
        if session_overrides.entries().is_empty() {
            None
        } else {
            Some(session_overrides.clone())
        },
    )?;
    let theme_catalog = theme_loader::load_theme_catalog(&config);
    let mut render_settings = crate::cli::default_render_settings();
    crate::cli::apply_render_settings_from_config(&mut render_settings, &config);
    render_settings.width = Some(resolve_default_render_width(&config));
    render_settings.theme_name = resolve_known_theme_name(
        config
            .get_string("theme.name")
            .unwrap_or(DEFAULT_THEME_NAME),
        &theme_catalog,
    )?;
    render_settings.theme = theme_catalog
        .resolve(&render_settings.theme_name)
        .map(|entry| entry.theme.clone());

    let message_verbosity = config
        .get_string("ui.verbosity.level")
        .and_then(parse_message_level)
        .unwrap_or(MessageLevel::Success);
    let debug_verbosity = match config.get("debug.level").map(ConfigValue::reveal) {
        Some(ConfigValue::Integer(value)) => (*value).clamp(0, 3) as u8,
        Some(ConfigValue::String(raw)) => raw.trim().parse::<u8>().map_or(0, |level| level.min(3)),
        _ => 0,
    };

    init_developer_logging(build_logging_config(&config, debug_verbosity));
    theme_loader::log_theme_issues(&theme_catalog.issues);

    let plugin_manager = PluginManager::new(launch.plugin_dirs.clone())
        .with_roots(launch.config_root.clone(), launch.cache_root.clone());
    let mut next = AppState::new(
        context,
        config,
        render_settings,
        message_verbosity,
        debug_verbosity,
        plugin_manager,
        theme_catalog,
        launch,
    );
    next.session.config_overrides = session_overrides;
    next.session.shell_stack = shell_stack;
    next.session.last_rows = last_rows;
    next.session.result_cache = result_cache;
    next.session.cache_order = cache_order;
    next.repl.history_shell = history_shell;
    next.sync_history_shell_context();
    Ok(next)
}

fn build_dispatch_plan(cli: &mut Cli, known_profiles: &BTreeSet<String>) -> Result<DispatchPlan> {
    let explicit_profile = normalize_profile_override(cli.profile.clone());
    cli.profile = explicit_profile.clone();
    let command = cli.command.take();
    let normalized_profiles = known_profiles
        .iter()
        .map(|profile| normalize_identifier(profile))
        .collect::<BTreeSet<_>>();

    match command {
        None => Ok(DispatchPlan {
            action: RunAction::Repl,
            profile_override: explicit_profile,
        }),
        Some(Commands::Plugins(args)) => Ok(DispatchPlan {
            action: RunAction::Plugins(args),
            profile_override: explicit_profile,
        }),
        Some(Commands::Doctor(args)) => Ok(DispatchPlan {
            action: RunAction::Doctor(args),
            profile_override: explicit_profile,
        }),
        Some(Commands::Theme(args)) => Ok(DispatchPlan {
            action: RunAction::Theme(args),
            profile_override: explicit_profile,
        }),
        Some(Commands::Config(args)) => Ok(DispatchPlan {
            action: RunAction::Config(args),
            profile_override: explicit_profile,
        }),
        Some(Commands::History(args)) => Ok(DispatchPlan {
            action: RunAction::History(args),
            profile_override: explicit_profile,
        }),
        Some(Commands::Repl(args)) => Ok(DispatchPlan {
            action: RunAction::ReplCommand(args),
            profile_override: explicit_profile,
        }),
        Some(Commands::External(tokens)) => {
            let Some(first) = tokens.first() else {
                return Ok(DispatchPlan {
                    action: RunAction::Repl,
                    profile_override: explicit_profile,
                });
            };

            if explicit_profile.is_none() {
                let normalized = normalize_identifier(first);
                if normalized_profiles.contains(&normalized) {
                    let remaining = tokens[1..].to_vec();
                    if remaining.is_empty() {
                        return Ok(DispatchPlan {
                            action: RunAction::Repl,
                            profile_override: Some(normalized),
                        });
                    }
                    let parsed = parse_inline_command_tokens(&remaining)
                        .map_err(|err| miette!(err.to_string()))?;
                    let action = match parsed {
                        Some(Commands::Plugins(args)) => RunAction::Plugins(args),
                        Some(Commands::Doctor(args)) => RunAction::Doctor(args),
                        Some(Commands::Theme(args)) => RunAction::Theme(args),
                        Some(Commands::Config(args)) => RunAction::Config(args),
                        Some(Commands::History(args)) => RunAction::History(args),
                        Some(Commands::Repl(args)) => RunAction::ReplCommand(args),
                        Some(Commands::External(external)) => RunAction::External(external),
                        None => RunAction::Repl,
                    };

                    return Ok(DispatchPlan {
                        action,
                        profile_override: Some(normalized),
                    });
                }
            }

            Ok(DispatchPlan {
                action: RunAction::External(tokens),
                profile_override: explicit_profile,
            })
        }
    }
}

fn ensure_dispatch_visibility(state: &AppState, action: &RunAction) -> Result<()> {
    match action {
        RunAction::Plugins(_) => ensure_builtin_visible(state, CMD_PLUGINS),
        RunAction::Doctor(_) => ensure_builtin_visible(state, CMD_DOCTOR),
        RunAction::Theme(_) => ensure_builtin_visible(state, CMD_THEME),
        RunAction::Config(_) => ensure_builtin_visible(state, CMD_CONFIG),
        RunAction::History(_) => ensure_builtin_visible(state, CMD_HISTORY),
        RunAction::ReplCommand(_) => Ok(()),
        RunAction::External(tokens) => {
            if let Some(command) = tokens.first() {
                ensure_plugin_visible(state, command)?;
            }
            Ok(())
        }
        RunAction::Repl => Ok(()),
    }
}

pub(crate) fn ensure_builtin_visible(state: &AppState, command: &str) -> Result<()> {
    if state.auth.is_builtin_visible(command) {
        Ok(())
    } else {
        Err(miette!(
            "command `{command}` is hidden by current auth policy"
        ))
    }
}

pub(crate) fn ensure_plugin_visible(state: &AppState, command: &str) -> Result<()> {
    if state.auth.is_plugin_command_visible(command) {
        Ok(())
    } else {
        Err(miette!(
            "plugin command `{command}` is hidden by current auth policy"
        ))
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

fn run_external_command(state: &mut AppState, tokens: &[String]) -> Result<i32> {
    let parsed = parse_command_tokens_with_aliases(tokens, state.config.resolved())?;
    if parsed.tokens.is_empty() {
        return Err(miette!("missing external command"));
    }
    if let Some(help) = completion::maybe_render_dsl_help(state, &parsed.stages) {
        print!("{help}");
        return Ok(0);
    }
    let stages = parsed.stages;
    let tokens = parsed.tokens;

    let parsed_inline = match parse_inline_command_tokens(&tokens) {
        Ok(command) => command,
        Err(err) => {
            if err.kind() == clap::error::ErrorKind::DisplayHelp
                || err.kind() == clap::error::ErrorKind::DisplayVersion
            {
                let resolved = state.ui.render_settings.resolve_render_settings();
                print!(
                    "{}",
                    repl_help::render_help_with_chrome(&err.to_string(), &resolved)
                );
                return Ok(0);
            }
            return Err(miette!(err.to_string()));
        }
    };

    if let Some(command) = parsed_inline {
        match command {
            Commands::Plugins(args) => {
                if !stages.is_empty() {
                    return Err(miette!(
                        "`{}` does not support DSL pipeline stages",
                        CMD_PLUGINS
                    ));
                }
                ensure_builtin_visible(state, CMD_PLUGINS)?;
                let result = plugins_cmd::run_plugins_command(state, args)?;
                return run_cli_command(state, result);
            }
            Commands::Doctor(args) => {
                if !stages.is_empty() {
                    return Err(miette!(
                        "`{}` does not support DSL pipeline stages",
                        CMD_DOCTOR
                    ));
                }
                ensure_builtin_visible(state, CMD_DOCTOR)?;
                let result = doctor_cmd::run_doctor_command(state, args)?;
                return run_cli_command(state, result);
            }
            Commands::Theme(args) => {
                if !stages.is_empty() {
                    return Err(miette!(
                        "`{}` does not support DSL pipeline stages",
                        CMD_THEME
                    ));
                }
                ensure_builtin_visible(state, CMD_THEME)?;
                let result = theme_cmd::run_theme_command(state, args)?;
                return run_cli_command(state, result);
            }
            Commands::Config(args) => {
                if !stages.is_empty() {
                    return Err(miette!(
                        "`{}` does not support DSL pipeline stages",
                        CMD_CONFIG
                    ));
                }
                ensure_builtin_visible(state, CMD_CONFIG)?;
                let result = config_cmd::run_config_command(state, args)?;
                return run_cli_command(state, result);
            }
            Commands::History(args) => {
                if !stages.is_empty() {
                    return Err(miette!(
                        "`{}` does not support DSL pipeline stages",
                        CMD_HISTORY
                    ));
                }
                ensure_builtin_visible(state, CMD_HISTORY)?;
                let result = history_cmd::run_history_command(state, args)?;
                return run_cli_command(state, result);
            }
            Commands::Repl(args) => {
                if !stages.is_empty() {
                    return Err(miette!("`repl` does not support DSL pipeline stages"));
                }
                let result = repl::run_repl_debug_command(state, args)?;
                return run_cli_command(state, result);
            }
            Commands::External(_) => {}
        }
    }
    if !stages.is_empty() {
        completion::validate_dsl_stages(&stages)?;
    }

    let plugin_manager = &state.clients.plugins;
    let settings = &state.ui.render_settings;
    let (command, args) = tokens
        .split_first()
        .ok_or_else(|| miette!("missing external command"))?;
    ensure_plugin_visible(state, command)?;
    emit_command_conflict_warning(state, command, plugin_manager);

    tracing::debug!(
        command = %command,
        args = ?args,
        "dispatching external command"
    );

    if is_help_passthrough(args) {
        let dispatch_context = plugin_dispatch_context(state, None);
        let raw = plugin_manager
            .dispatch_passthrough(command, args, &dispatch_context)
            .map_err(enrich_dispatch_error)?;
        if !raw.stdout.is_empty() {
            let resolved = settings.resolve_render_settings();
            print!(
                "{}",
                repl_help::render_help_with_chrome(&raw.stdout, &resolved)
            );
        }
        if !raw.stderr.is_empty() {
            eprint!("{}", raw.stderr);
        }
        return Ok(raw.status_code);
    }

    let dispatch_context = plugin_dispatch_context(state, None);
    let response = plugin_manager
        .dispatch(command, args, &dispatch_context)
        .map_err(enrich_dispatch_error)?;

    let mut messages = plugin_response_messages(&response);
    if !response.ok {
        if let Some(error) = response.error {
            messages.error(format!("{}: {}", error.code, error.message));
        } else {
            messages.error("plugin command failed");
        }
        emit_messages(state, &messages);
        return Ok(1);
    }
    if !messages.is_empty() {
        emit_messages(state, &messages);
    }

    let mut output = plugin_data_to_output_result(response.data, Some(&response.meta));
    if !stages.is_empty() {
        output = apply_output_pipeline(output, &stages).map_err(|err| miette!("{err:#}"))?;
    }
    let effective = resolve_effective_render_settings(
        settings,
        parse_output_format_hint(response.meta.format_hint.as_deref()),
    );
    print!("{}", render_output(&output, &effective));
    maybe_copy_output(state, &output);

    Ok(0)
}

fn emit_command_conflict_warning(state: &AppState, command: &str, plugin_manager: &PluginManager) {
    let providers = plugin_manager.command_providers(command);
    if providers.len() <= 1 {
        return;
    }
    let selected = plugin_manager
        .selected_provider_label(command)
        .unwrap_or_else(|| {
            providers
                .first()
                .cloned()
                .unwrap_or_else(|| "unknown".to_string())
        });
    let mut messages = MessageBuffer::default();
    messages.warning(format!(
        "command `{command}` is provided by multiple plugins: {}. Using {selected}.",
        providers.join(", ")
    ));
    emit_messages_with_verbosity(state, &messages, state.ui.message_verbosity);
}

fn resolve_theme_name(
    cli: &Cli,
    config: &ResolvedConfig,
    catalog: &theme_loader::ThemeCatalog,
) -> Result<String> {
    let selected = cli.selected_theme_name(config);
    resolve_known_theme_name(&selected, catalog)
}

fn run_cli_command(state: &AppState, result: CliCommandResult) -> Result<i32> {
    if let Some(output) = result.output {
        render_cli_output(state, output);
    }
    Ok(result.exit_code)
}

fn render_cli_output(state: &AppState, output: ReplCommandOutput) {
    match output {
        ReplCommandOutput::Output {
            output,
            format_hint,
        } => {
            let effective =
                resolve_effective_render_settings(&state.ui.render_settings, format_hint);
            print!("{}", render_output(&output, &effective));
            maybe_copy_output(state, &output);
        }
        ReplCommandOutput::Text(text) => {
            print!("{text}");
        }
    }
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

pub(crate) fn is_help_passthrough(args: &[String]) -> bool {
    if args.is_empty() {
        return false;
    }

    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        return true;
    }

    matches!(args.first(), Some(first) if first == CMD_HELP)
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

fn build_logging_config(config: &ResolvedConfig, debug_verbosity: u8) -> DeveloperLoggingConfig {
    let file = if config.get_bool("log.file.enabled").unwrap_or(false) {
        let level = config
            .get_string("log.file.level")
            .and_then(parse_level_filter)
            .or_else(|| parse_level_filter("warn"))
            .unwrap_or(tracing_subscriber::filter::LevelFilter::WARN);
        let path = config
            .get_string("log.file.path")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        path.map(|path| FileLoggingConfig { path, level })
    } else {
        None
    };

    DeveloperLoggingConfig {
        debug_count: debug_verbosity,
        file,
    }
}

fn effective_message_verbosity(config: &ResolvedConfig) -> MessageLevel {
    config
        .get_string("ui.verbosity.level")
        .and_then(parse_message_level)
        .unwrap_or(MessageLevel::Success)
}

fn effective_debug_verbosity(config: &ResolvedConfig) -> u8 {
    match config.get("debug.level").map(ConfigValue::reveal) {
        Some(ConfigValue::Integer(level)) => (*level).clamp(0, 3) as u8,
        Some(ConfigValue::String(raw)) => raw.trim().parse::<u8>().map_or(0, |level| level.min(3)),
        _ => 0,
    }
}

fn parse_message_level(value: &str) -> Option<MessageLevel> {
    match value.trim().to_ascii_lowercase().as_str() {
        "error" => Some(MessageLevel::Error),
        "warning" | "warn" => Some(MessageLevel::Warning),
        "success" => Some(MessageLevel::Success),
        "info" => Some(MessageLevel::Info),
        "trace" => Some(MessageLevel::Trace),
        _ => None,
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
    let ui_verbosity = overrides
        .map(|value| value.message_verbosity)
        .unwrap_or(state.ui.message_verbosity);
    let debug_verbosity = overrides
        .map(|value| value.debug_verbosity)
        .unwrap_or(state.ui.debug_verbosity);
    let terminal_kind = match state.context.terminal_kind() {
        TerminalKind::Cli => RuntimeTerminalKind::Cli,
        TerminalKind::Repl => RuntimeTerminalKind::Repl,
    };
    PluginDispatchContext {
        runtime_hints: RuntimeHints {
            ui_verbosity: to_ui_verbosity(ui_verbosity),
            debug_level: debug_verbosity.min(3),
            format: state.ui.render_settings.format,
            color: state.ui.render_settings.color,
            unicode: state.ui.render_settings.unicode,
            profile: Some(state.config.resolved().active_profile().to_string()),
            terminal: state.context.terminal_env().map(ToOwned::to_owned),
            terminal_kind,
        },
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

pub(crate) fn emit_messages(state: &AppState, messages: &MessageBuffer) {
    emit_messages_with_verbosity(state, messages, state.ui.message_verbosity);
}

pub(crate) fn emit_messages_with_verbosity(
    state: &AppState,
    messages: &MessageBuffer,
    verbosity: MessageLevel,
) {
    let resolved = state.ui.render_settings.resolve_render_settings();
    let message_format = state
        .config
        .resolved()
        .get_string("ui.messages.format")
        .and_then(MessageRenderFormat::parse)
        .unwrap_or(MessageRenderFormat::Rules);
    let rendered = messages.render_grouped_styled_with_overrides(
        verbosity,
        resolved.color,
        resolved.unicode,
        resolved.width,
        &resolved.theme,
        message_format,
        &resolved.style_overrides,
    );
    if !rendered.is_empty() {
        eprint!("{rendered}");
    }
}

pub(crate) fn maybe_copy_output(state: &AppState, output: &OutputResult) {
    if !output.meta.wants_copy {
        return;
    }
    let clipboard = ClipboardService::new();
    if let Err(err) = copy_output_to_clipboard(output, &state.ui.render_settings, &clipboard) {
        let mut messages = MessageBuffer::default();
        messages.warning(format!("clipboard copy failed: {err}"));
        emit_messages(state, &messages);
    }
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

pub(crate) fn resolve_runtime_config(
    profile_override: Option<String>,
    terminal: Option<&str>,
    session_layer: Option<ConfigLayer>,
) -> Result<ResolvedConfig> {
    tracing::debug!(
        profile_override = ?profile_override,
        terminal = ?terminal,
        "resolving runtime config"
    );
    let defaults = RuntimeDefaults::from_process_env(DEFAULT_THEME_NAME, DEFAULT_REPL_PROMPT);
    let paths = RuntimeConfigPaths::discover();
    let pipeline = build_runtime_pipeline(defaults.to_layer(), &paths, None, session_layer);

    let options = ResolveOptions {
        profile_override,
        terminal: terminal.map(|value| value.to_string()),
    };

    pipeline
        .resolve(options)
        .into_diagnostic()
        .wrap_err("config resolution failed")
}

fn normalize_identifier(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn normalize_profile_override(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let normalized = normalize_identifier(&value);
        if normalized.is_empty() {
            None
        } else {
            Some(normalized)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::help::parse_help_render_overrides;
    use super::{
        RunAction, build_cli_session_layer, build_dispatch_plan, is_sensitive_key,
        parse_output_format_hint, resolve_effective_render_settings,
    };
    use crate::cli::{Cli, Commands, ConfigCommands, PluginsCommands, ThemeCommands};
    use crate::plugin_manager::{CommandCatalogEntry, PluginManager, PluginSource};
    use crate::repl;
    use crate::repl::{completion, help as repl_help};
    use crate::state::{AppState, LaunchContext, RuntimeContext, TerminalKind};
    use clap::Parser;
    use osp_config::{ConfigLayer, ConfigResolver, ConfigValue, ResolveOptions};
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
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        if let Some(allowlist) = auth_visible_builtins {
            defaults.set("auth.visible.builtins", allowlist);
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

        AppState::new(
            RuntimeContext::new(None, TerminalKind::Repl, None),
            config,
            settings,
            MessageLevel::Success,
            0,
            PluginManager::new(Vec::new()),
            crate::theme_loader::ThemeCatalog::default(),
            LaunchContext::default(),
        )
    }

    fn sample_catalog() -> Vec<CommandCatalogEntry> {
        vec![CommandCatalogEntry {
            name: "orch".to_string(),
            about: "Provision orchestrator resources".to_string(),
            subcommands: vec!["provision".to_string(), "status".to_string()],
            provider: "mock-provider".to_string(),
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

        let layer = build_cli_session_layer(&cli, None, TerminalKind::Repl)
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

        let layer = build_cli_session_layer(&cli, None, TerminalKind::Repl)
            .expect("session layer should build")
            .expect("quiet flag should create session overrides");

        assert_eq!(
            layer_value(&layer, "ui.verbosity.level"),
            Some(&ConfigValue::from("warning"))
        );
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
        let rewritten = repl::rewrite_repl_help_tokens(&[
            "help".to_string(),
            "ldap".to_string(),
            "user".to_string(),
        ])
        .expect("help alias should rewrite");
        assert_eq!(
            rewritten,
            vec!["ldap".to_string(), "user".to_string(), "--help".to_string()]
        );
    }

    #[test]
    fn repl_help_alias_preserves_existing_help_flag_unit() {
        let rewritten = repl::rewrite_repl_help_tokens(&[
            "help".to_string(),
            "ldap".to_string(),
            "--help".to_string(),
        ])
        .expect("help alias should rewrite");
        assert_eq!(rewritten, vec!["ldap".to_string(), "--help".to_string()]);
    }

    #[test]
    fn repl_help_alias_skips_bare_help_unit() {
        assert!(repl::rewrite_repl_help_tokens(&["help".to_string()]).is_none());
    }

    #[test]
    fn repl_shellable_commands_include_ldap_unit() {
        assert!(repl::is_repl_shellable_command("ldap"));
        assert!(repl::is_repl_shellable_command("LDAP"));
        assert!(!repl::is_repl_shellable_command("theme"));
    }

    #[test]
    fn repl_shell_prefix_applies_once_unit() {
        let stack = vec!["ldap".to_string()];
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
        state.session.shell_stack.push("ldap".to_string());
        let message = repl::leave_repl_shell(&mut state).expect("shell should leave");
        assert_eq!(message, "Leaving ldap shell. Back at root.\n");
        assert!(state.session.shell_stack.is_empty());
    }

    #[test]
    fn repl_shell_enter_only_from_root_unit() {
        let mut state = make_completion_state(None);
        assert!(repl::should_enter_repl_shell(&state, &["ldap".to_string()]));
        state.session.shell_stack.push("ldap".to_string());
        assert!(repl::should_enter_repl_shell(&state, &["mreg".to_string()]));
        assert!(!repl::should_enter_repl_shell(
            &state,
            &["ldap".to_string()]
        ));
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
            OsString::from("--ascii"),
        ];

        let parsed = parse_help_render_overrides(&args);
        assert_eq!(parsed.profile.as_deref(), Some("tsd"));
        assert_eq!(parsed.theme.as_deref(), Some("dracula"));
        assert_eq!(parsed.mode, Some(osp_core::output::RenderMode::Plain));
        assert_eq!(parsed.color, Some(osp_core::output::ColorMode::Always));
        assert_eq!(parsed.unicode, Some(osp_core::output::UnicodeMode::Never));
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
        let words = completion::catalog_completion_words(&catalog);

        let tree = completion::build_repl_completion_tree(&state, &catalog, &words);
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
        let words = completion::catalog_completion_words(&catalog);

        let tree = completion::build_repl_completion_tree(&state, &catalog, &words);
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
        let words = completion::catalog_completion_words(&catalog);

        let tree = completion::build_repl_completion_tree(&state, &catalog, &words);
        assert!(tree.root.children.contains_key("theme"));
        assert!(!tree.root.children.contains_key("config"));
        assert!(!tree.root.children.contains_key("plugins"));
        assert!(!tree.root.children.contains_key("history"));
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
{"protocol_version":1,"plugin_id":"fail","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{"name":"fail","about":"fail","subcommands":[]}]}
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
{"protocol_version":1,"plugin_id":"cache","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{"name":"cache","about":"cache plugin","subcommands":[]}]}
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
        state.session.shell_stack = vec!["orch".to_string()];

        let history_shell = HistoryShellContext::new(String::new());
        state.repl.history_shell = Some(history_shell);
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
        assert_eq!(next.session.shell_stack, vec!["orch".to_string()]);
        assert!(next.repl.history_shell.is_some());
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
    fn make_test_history(state: &mut AppState) -> SharedHistory {
        let history_dir = make_temp_dir("osp-cli-test-history");
        let history_path = history_dir.join("history.jsonl");
        let history_shell = state
            .repl
            .history_shell
            .clone()
            .unwrap_or_else(|| HistoryShellContext::new(String::new()));
        state.repl.history_shell = Some(history_shell.clone());
        state.sync_history_shell_context();

        let history_config = HistoryConfig::new(
            Some(history_path),
            128,
            true,
            true,
            true,
            Vec::new(),
            Some(state.config.resolved().active_profile().to_string()),
            Some(
                state
                    .context
                    .terminal_kind()
                    .as_config_terminal()
                    .to_string(),
            ),
            Some(history_shell),
        );

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
        };

        AppState::new(
            RuntimeContext::new(None, TerminalKind::Repl, None),
            config,
            settings,
            MessageLevel::Success,
            0,
            PluginManager::new(plugin_dirs).with_roots(Some(config_root), Some(cache_root)),
            crate::theme_loader::ThemeCatalog::default(),
            launch,
        )
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
