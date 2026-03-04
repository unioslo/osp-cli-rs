use clap::Parser;
use miette::{IntoDiagnostic, Result, WrapErr, miette};
use osp_config::{
    ConfigExplain, ConfigLayer, ConfigResolver, ConfigSchema, ConfigValue,
    DEFAULT_REPL_HISTORY_MAX_ENTRIES, DEFAULT_SESSION_CACHE_MAX_RESULTS, DEFAULT_UI_WIDTH,
    ResolveOptions, ResolvedConfig, ResolvedValue, RuntimeConfigPaths, RuntimeDefaults, Scope,
    build_runtime_pipeline, set_scoped_value_in_toml,
};
use osp_core::output::OutputFormat;
use osp_core::output_model::{OutputItems, OutputMeta, OutputResult};
use osp_core::plugin::{ResponseMessageLevelV1, ResponseMetaV1, ResponseV1};
use osp_core::row::Row;
use osp_core::runtime::{RuntimeHints, RuntimeTerminalKind, UiVerbosity};
use osp_dsl::{apply_pipeline, parse_pipeline};
use osp_repl::{ReplPrompt, run_repl};
use osp_ui::messages::{
    MessageBuffer, MessageLevel, MessageRenderFormat, adjust_verbosity,
    render_section_divider_with_overrides,
};
use osp_ui::render_output;
use osp_ui::style::{StyleToken, apply_style, apply_style_spec};
use osp_ui::theme::{
    DEFAULT_THEME_NAME, available_theme_names, find_theme, is_known_theme, normalize_theme_name,
};
use std::borrow::Cow;
use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;

use crate::cli::{
    Cli, Commands, ConfigArgs, ConfigCommands, ConfigExplainArgs, ConfigGetArgs, ConfigSetArgs,
    ConfigShowArgs, PluginToggleArgs, PluginsArgs, PluginsCommands, ThemeArgs, ThemeCommands,
    ThemeShowArgs, ThemeUseArgs, parse_inline_command_tokens, parse_repl_tokens,
};
use crate::logging::{
    DeveloperLoggingConfig, FileLoggingConfig, init_developer_logging, parse_level_filter,
};
use crate::plugin_manager::{
    CommandCatalogEntry, DoctorReport, PluginDispatchContext, PluginDispatchError, PluginManager,
    PluginSummary,
};
use crate::state::{AppState, RuntimeContext, TerminalKind};

enum RunAction {
    Repl,
    Plugins(PluginsArgs),
    Theme(ThemeArgs),
    Config(ConfigArgs),
    External(Vec<String>),
}

const CMD_PLUGINS: &str = "plugins";
const CMD_CONFIG: &str = "config";
const CMD_THEME: &str = "theme";
const CMD_HELP: &str = "help";
const CMD_LIST: &str = "list";
const CMD_SHOW: &str = "show";
const CMD_USE: &str = "use";
const DEFAULT_REPL_PROMPT: &str = "╭─{user}@{domain} {indicator}\n╰─{profile}> ";
const CURRENT_TERMINAL_SENTINEL: &str = "__current__";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigStore {
    Session,
    Config,
    Secrets,
}

#[derive(Debug, Clone)]
struct ReplCommandSpec {
    name: Cow<'static, str>,
    supports_dsl: bool,
}

#[derive(Debug, Clone, Copy)]
struct ReplDispatchOverrides {
    message_verbosity: MessageLevel,
    debug_verbosity: u8,
}

enum ReplCommandOutput {
    Output(OutputResult),
    Text(String),
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
    let cli = Cli::parse_from(args);
    run(cli)
}

fn run(mut cli: Cli) -> Result<i32> {
    let initial_config = resolve_runtime_config(cli.profile.clone(), Some("cli"), None)?;
    let known_profiles = initial_config.known_profiles().clone();
    let dispatch = build_dispatch_plan(&mut cli, &known_profiles)?;

    let terminal_kind = match dispatch.action {
        RunAction::Repl => TerminalKind::Repl,
        RunAction::Plugins(_)
        | RunAction::Theme(_)
        | RunAction::Config(_)
        | RunAction::External(_) => TerminalKind::Cli,
    };
    let runtime_context = RuntimeContext::new(
        dispatch.profile_override.clone(),
        terminal_kind,
        std::env::var("TERM").ok(),
    );

    let config = resolve_runtime_config(
        runtime_context.profile_override().map(ToOwned::to_owned),
        Some(runtime_context.terminal_kind().as_config_terminal()),
        None,
    )?;
    let mut render_settings = cli.render_settings();
    cli.seed_render_settings_from_config(&mut render_settings, &config);
    if render_settings.width.is_none() {
        render_settings.width = Some(config_usize(&config, "ui.width", DEFAULT_UI_WIDTH as usize));
    }
    render_settings.theme_name = resolve_theme_name(&cli, &config)?;
    let message_verbosity = effective_message_verbosity(&cli, &config);
    let debug_verbosity = effective_debug_verbosity(&cli, &config);
    init_developer_logging(build_logging_config(&config, debug_verbosity));
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
    );
    ensure_dispatch_visibility(&state, &dispatch.action)?;

    tracing::info!(
        profile = %state.config.resolved().active_profile(),
        terminal = %state.context.terminal_kind().as_config_terminal(),
        "osp session initialized"
    );

    match dispatch.action {
        RunAction::Repl => run_plugin_repl(&mut state),
        RunAction::Plugins(args) => run_plugins_command(&state, args),
        RunAction::Theme(args) => run_theme_command(&mut state, args),
        RunAction::Config(args) => run_config_command(&mut state, args),
        RunAction::External(tokens) => run_external_command(&state, &tokens),
    }
}

fn build_dispatch_plan(cli: &mut Cli, known_profiles: &BTreeSet<String>) -> Result<DispatchPlan> {
    let explicit_profile = cli.profile.clone();
    let command = cli.command.take();

    match command {
        None => Ok(DispatchPlan {
            action: RunAction::Repl,
            profile_override: explicit_profile,
        }),
        Some(Commands::Plugins(args)) => Ok(DispatchPlan {
            action: RunAction::Plugins(args),
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
        Some(Commands::External(tokens)) => {
            let Some(first) = tokens.first() else {
                return Ok(DispatchPlan {
                    action: RunAction::Repl,
                    profile_override: explicit_profile,
                });
            };

            if explicit_profile.is_none() {
                let normalized = normalize_identifier(first);
                if known_profiles.contains(&normalized) {
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
                        Some(Commands::Theme(args)) => RunAction::Theme(args),
                        Some(Commands::Config(args)) => RunAction::Config(args),
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
        RunAction::Theme(_) => ensure_builtin_visible(state, CMD_THEME),
        RunAction::Config(_) => ensure_builtin_visible(state, CMD_CONFIG),
        RunAction::External(tokens) => {
            if let Some(command) = tokens.first() {
                ensure_plugin_visible(state, command)?;
            }
            Ok(())
        }
        RunAction::Repl => Ok(()),
    }
}

fn ensure_builtin_visible(state: &AppState, command: &str) -> Result<()> {
    if state.auth.is_builtin_visible(command) {
        Ok(())
    } else {
        Err(miette!(
            "command `{command}` is hidden by current auth policy"
        ))
    }
}

fn ensure_plugin_visible(state: &AppState, command: &str) -> Result<()> {
    if state.auth.is_plugin_command_visible(command) {
        Ok(())
    } else {
        Err(miette!(
            "plugin command `{command}` is hidden by current auth policy"
        ))
    }
}

fn run_plugin_repl(state: &mut AppState) -> Result<i32> {
    let catalog = authorized_command_catalog(state)?;
    let mut words = catalog_completion_words(&catalog);
    if state.auth.is_builtin_visible(CMD_PLUGINS) {
        words.extend([CMD_PLUGINS.to_string(), CMD_LIST.to_string()]);
    }
    if state.auth.is_builtin_visible(CMD_THEME) {
        words.extend([
            CMD_THEME.to_string(),
            CMD_LIST.to_string(),
            CMD_SHOW.to_string(),
            CMD_USE.to_string(),
        ]);
    }
    if state.auth.is_builtin_visible(CMD_CONFIG) {
        words.extend([
            CMD_CONFIG.to_string(),
            "get".to_string(),
            "show".to_string(),
            "explain".to_string(),
            "set".to_string(),
            "diagnostics".to_string(),
        ]);
    }
    words.extend(
        available_theme_names()
            .into_iter()
            .map(std::string::ToString::to_string),
    );
    words.sort();
    words.dedup();

    let mut help_text = catalog_help_text(&catalog);
    if state.auth.is_builtin_visible(CMD_THEME) {
        help_text.push_str(
            "Backbone theme commands: theme list | theme show [name] | theme use <name>\n",
        );
    }
    if state.auth.is_builtin_visible(CMD_CONFIG) {
        help_text.push_str("Backbone config commands: config show|get|explain|set|diagnostics\n");
    }
    if state
        .config
        .resolved()
        .get_bool("repl.intro")
        .unwrap_or(true)
    {
        print!("{}", render_repl_intro(state));
    }
    let prompt = build_repl_prompt(state);
    let history_path = state
        .config
        .resolved()
        .get_string("repl.history.path")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let defaults =
                RuntimeDefaults::from_process_env(DEFAULT_THEME_NAME, DEFAULT_REPL_PROMPT);
            PathBuf::from(defaults.repl_history_path)
        });
    let history_max_entries = config_usize(
        state.config.resolved(),
        "repl.history.max_entries",
        DEFAULT_REPL_HISTORY_MAX_ENTRIES as usize,
    );
    run_repl(
        prompt,
        words,
        help_text,
        history_path,
        history_max_entries,
        |line| execute_repl_plugin_line(state, line).map_err(|err| anyhow::anyhow!("{err:#}")),
    )
    .map_err(|err| miette!("{err:#}"))
}

fn authorized_command_catalog(state: &AppState) -> Result<Vec<CommandCatalogEntry>> {
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

fn catalog_completion_words(catalog: &[CommandCatalogEntry]) -> Vec<String> {
    let mut words = vec![
        "help".to_string(),
        "exit".to_string(),
        "quit".to_string(),
        "P".to_string(),
        "F".to_string(),
        "V".to_string(),
        "|".to_string(),
    ];
    for entry in catalog {
        words.push(entry.name.clone());
        words.extend(entry.subcommands.clone());
    }
    words.sort();
    words.dedup();
    words
}

fn catalog_help_text(catalog: &[CommandCatalogEntry]) -> String {
    let mut out = String::new();
    out.push_str("Backbone commands: help, exit, quit\n");
    if catalog.is_empty() {
        out.push_str("No plugin commands available.\n");
        return out;
    }

    out.push_str("Plugin commands:\n");
    for command in catalog {
        let subs = if command.subcommands.is_empty() {
            "".to_string()
        } else {
            format!(" [{}]", command.subcommands.join(", "))
        };
        let about = if command.about.trim().is_empty() {
            "-".to_string()
        } else {
            command.about.clone()
        };
        out.push_str(&format!(
            "  {name}{subs} - {about} ({provider}/{source})\n",
            name = command.name,
            provider = command.provider,
            source = command.source,
        ));
    }
    out
}

fn render_repl_intro(state: &AppState) -> String {
    let resolved = state.ui.render_settings.resolve_render_settings();
    let config = state.config.resolved();
    let theme_name = resolved.theme_name.as_str();

    let user = config.get_string("user.name").unwrap_or("anonymous");
    let profile = config.active_profile();
    let theme = state.ui.render_settings.theme_name.clone();

    let user_text = style_prompt_fragment(
        config.get_string("color.prompt.text"),
        user,
        StyleToken::PromptText,
        resolved.color,
        theme_name,
    );
    let profile_text = style_prompt_fragment(
        config.get_string("color.prompt.command"),
        profile,
        StyleToken::PromptCommand,
        resolved.color,
        theme_name,
    );
    let theme_text = style_prompt_fragment(
        config.get_string("color.prompt.command"),
        &theme,
        StyleToken::PromptCommand,
        resolved.color,
        theme_name,
    );

    let mut out = String::new();
    out.push_str(&render_section_divider_with_overrides(
        "OSP",
        resolved.unicode,
        resolved.width,
        resolved.color,
        theme_name,
        StyleToken::MessageInfo,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out.push_str(&format!("Welcome {user_text}!\n"));
    out.push_str(&format!("Profile: {profile_text}\n"));
    out.push_str(&format!("Theme: {theme_text}\n\n"));
    out
}

fn build_repl_prompt(state: &AppState) -> ReplPrompt {
    let resolved = state.ui.render_settings.resolve_render_settings();
    let config = state.config.resolved();
    let theme_name = resolved.theme_name.as_str();
    let simple = config.get_bool("repl.simple_prompt").unwrap_or(false);
    let profile = config.active_profile();
    let user = config.get_string("user.name").unwrap_or("anonymous");
    let domain = config.get_string("domain").unwrap_or("local");
    let indicator = build_shell_indicator(state);

    let user_text = style_prompt_fragment(
        config.get_string("color.prompt.text"),
        user,
        StyleToken::PromptText,
        resolved.color,
        theme_name,
    );
    let domain_text = style_prompt_fragment(
        config.get_string("color.prompt.text"),
        domain,
        StyleToken::PromptText,
        resolved.color,
        theme_name,
    );
    let profile_text = style_prompt_fragment(
        config.get_string("color.prompt.command"),
        profile,
        StyleToken::PromptCommand,
        resolved.color,
        theme_name,
    );
    let indicator_text = style_prompt_fragment(
        config.get_string("color.prompt.text"),
        &indicator,
        StyleToken::PromptText,
        resolved.color,
        theme_name,
    );

    let prompt = if simple {
        format!("{profile_text}> ")
    } else {
        let template = config
            .get_string("repl.prompt")
            .unwrap_or(DEFAULT_REPL_PROMPT);
        render_prompt_template(
            template,
            &user_text,
            &domain_text,
            &profile_text,
            &indicator_text,
        )
    };

    ReplPrompt::simple(prompt)
}

fn build_shell_indicator(state: &AppState) -> String {
    if state.session.shell_stack.is_empty() {
        return String::new();
    }

    let stack = state.session.shell_stack.join(" / ");
    let template = state
        .config
        .resolved()
        .get_string("repl.shell_indicator")
        .unwrap_or("[{shell}]");
    if template.contains("{shell}") {
        template.replace("{shell}", &stack)
    } else {
        template.to_string()
    }
}

fn render_prompt_template(
    template: &str,
    user: &str,
    domain: &str,
    profile: &str,
    indicator: &str,
) -> String {
    let mut out = template
        .replace("{user}", user)
        .replace("{domain}", domain)
        .replace("{profile}", profile)
        .replace("{context}", profile);

    if out.contains("{indicator}") {
        out = out.replace("{indicator}", indicator);
    } else if !indicator.trim().is_empty() {
        if !out.ends_with(' ') {
            out.push(' ');
        }
        out.push_str(indicator);
    }

    out
}

fn style_prompt_fragment(
    config_style: Option<&str>,
    value: &str,
    fallback: StyleToken,
    color: bool,
    theme_name: &str,
) -> String {
    match config_style.map(str::trim) {
        Some(spec) if !spec.is_empty() => apply_style_spec(value, spec, color),
        _ => apply_style(value, fallback, color, theme_name),
    }
}

fn execute_repl_plugin_line(state: &mut AppState, line: &str) -> Result<String> {
    let parsed = parse_pipeline(line);
    if parsed.command.is_empty() {
        return Ok(String::new());
    }

    let tokens = shell_words::split(&parsed.command)
        .into_diagnostic()
        .wrap_err("failed to parse command")?;
    let parsed_command = match parse_repl_tokens(&tokens) {
        Ok(parsed) => parsed,
        Err(err) => {
            if err.kind() == clap::error::ErrorKind::DisplayHelp
                || err.kind() == clap::error::ErrorKind::DisplayVersion
            {
                return Ok(err.to_string());
            }
            return Err(miette!(err.to_string()));
        }
    };
    let overrides = ReplDispatchOverrides {
        message_verbosity: adjust_verbosity(
            state.ui.message_verbosity,
            parsed_command.verbose,
            parsed_command.quiet,
        ),
        debug_verbosity: if parsed_command.debug > 0 {
            parsed_command.debug.min(3)
        } else {
            state.ui.debug_verbosity
        },
    };
    let command = parsed_command
        .command
        .ok_or_else(|| miette!("missing command"))?;
    let spec = repl_command_spec(&command);
    if !spec.supports_dsl && !parsed.stages.is_empty() {
        return Err(miette!(
            "`{}` does not support DSL pipeline stages",
            spec.name
        ));
    }

    match run_repl_command(state, command, overrides)? {
        ReplCommandOutput::Output(mut output) => {
            if !parsed.stages.is_empty() {
                let rows = output_to_rows(&output);
                let rows =
                    apply_pipeline(rows, &parsed.stages).map_err(|err| miette!("{err:#}"))?;
                output = rows_to_output_result(rows);
            }

            let rendered = render_output(&output, &state.ui.render_settings);
            state.record_repl_rows(line, output_to_rows(&output));
            Ok(rendered)
        }
        ReplCommandOutput::Text(text) => Ok(text),
    }
}

fn run_plugins_command(state: &AppState, args: PluginsArgs) -> Result<i32> {
    let plugin_manager = &state.clients.plugins;
    match args.command {
        PluginsCommands::List => {
            let mut plugins = plugin_manager
                .list_plugins()
                .map_err(|err| miette!("{err:#}"))?;
            plugins.sort_by(|a, b| a.plugin_id.cmp(&b.plugin_id));
            print!("{}", format_plugin_list(&plugins));
            Ok(0)
        }
        PluginsCommands::Commands => {
            let mut commands = authorized_command_catalog(state)?;
            commands.sort_by(|a, b| a.name.cmp(&b.name));
            print!("{}", format_command_catalog(&commands));
            Ok(0)
        }
        PluginsCommands::Enable(PluginToggleArgs { plugin_id }) => {
            plugin_manager
                .set_enabled(&plugin_id, true)
                .map_err(|err| miette!("{err:#}"))?;
            let mut messages = MessageBuffer::default();
            messages.success(format!("enabled plugin: {plugin_id}"));
            emit_messages(state, &messages);
            Ok(0)
        }
        PluginsCommands::Disable(PluginToggleArgs { plugin_id }) => {
            plugin_manager
                .set_enabled(&plugin_id, false)
                .map_err(|err| miette!("{err:#}"))?;
            let mut messages = MessageBuffer::default();
            messages.success(format!("disabled plugin: {plugin_id}"));
            emit_messages(state, &messages);
            Ok(0)
        }
        PluginsCommands::Doctor => {
            let report = plugin_manager.doctor().map_err(|err| miette!("{err:#}"))?;
            print!("{}", format_doctor_report(&report));
            Ok(0)
        }
    }
}

fn run_theme_command(state: &mut AppState, args: ThemeArgs) -> Result<i32> {
    match args.command {
        ThemeCommands::List => {
            let rows = theme_list_rows(&state.ui.render_settings.theme_name);
            print!(
                "{}",
                render_output(&rows_to_output_result(rows), &state.ui.render_settings)
            );
            Ok(0)
        }
        ThemeCommands::Show(ThemeShowArgs { name }) => {
            let selected = name.unwrap_or_else(|| state.ui.render_settings.theme_name.clone());
            let rows = theme_show_rows(&selected)?;
            print!(
                "{}",
                render_output(&rows_to_output_result(rows), &state.ui.render_settings)
            );
            Ok(0)
        }
        ThemeCommands::Use(ThemeUseArgs { name }) => {
            let selected = resolve_known_theme_name(&name)?;
            state.ui.render_settings.theme_name = selected.clone();

            let mut messages = MessageBuffer::default();
            messages.success(format!("active theme set to: {selected}"));
            messages.info(
                "theme change is for the current process; persistent writes land with `config set`",
            );
            emit_messages(state, &messages);
            Ok(0)
        }
    }
}

fn run_repl_command(
    state: &mut AppState,
    command: Commands,
    overrides: ReplDispatchOverrides,
) -> Result<ReplCommandOutput> {
    match command {
        Commands::Plugins(args) => {
            ensure_builtin_visible(state, CMD_PLUGINS)?;
            with_repl_verbosity_overrides(state, overrides, |state| {
                run_plugins_repl_command(state, args)
            })
        }
        Commands::Theme(args) => {
            ensure_builtin_visible(state, CMD_THEME)?;
            with_repl_verbosity_overrides(state, overrides, |state| {
                run_theme_repl_command(state, args)
            })
        }
        Commands::Config(args) => {
            ensure_builtin_visible(state, CMD_CONFIG)?;
            with_repl_verbosity_overrides(state, overrides, |state| {
                run_config_repl_command(state, args)
            })
        }
        Commands::External(tokens) => run_repl_external_command(state, tokens, overrides),
    }
}

fn with_repl_verbosity_overrides<T, F>(
    state: &mut AppState,
    overrides: ReplDispatchOverrides,
    run: F,
) -> Result<T>
where
    F: FnOnce(&mut AppState) -> Result<T>,
{
    let previous_message = state.ui.message_verbosity;
    let previous_debug = state.ui.debug_verbosity;
    state.ui.message_verbosity = overrides.message_verbosity;
    state.ui.debug_verbosity = overrides.debug_verbosity;
    let result = run(state);
    state.ui.message_verbosity = previous_message;
    state.ui.debug_verbosity = previous_debug;
    result
}

fn run_repl_external_command(
    state: &AppState,
    tokens: Vec<String>,
    overrides: ReplDispatchOverrides,
) -> Result<ReplCommandOutput> {
    let (command, args) = tokens
        .split_first()
        .ok_or_else(|| miette!("missing command"))?;
    ensure_plugin_visible(state, command)?;
    if is_help_passthrough(args) {
        let dispatch_context = plugin_dispatch_context(state, Some(overrides));
        let raw = state
            .clients
            .plugins
            .dispatch_passthrough(command, args, &dispatch_context)
            .map_err(enrich_dispatch_error)?;
        if raw.status_code != 0 {
            return Err(miette!(
                "plugin help command exited with status {}",
                raw.status_code
            ));
        }
        let mut out = String::new();
        out.push_str(&raw.stdout);
        if !raw.stderr.is_empty() {
            out.push_str(&raw.stderr);
        }
        return Ok(ReplCommandOutput::Text(out));
    }

    let dispatch_context = plugin_dispatch_context(state, Some(overrides));
    let response = state
        .clients
        .plugins
        .dispatch(command, args, &dispatch_context)
        .map_err(enrich_dispatch_error)?;
    let mut messages = plugin_response_messages(&response);
    if !response.ok {
        let report = if let Some(error) = response.error {
            messages.error(format!("{}: {}", error.code, error.message));
            miette!("{}: {}", error.code, error.message)
        } else {
            messages.error("plugin command failed");
            miette!("plugin command failed")
        };
        emit_messages_with_verbosity(state, &messages, overrides.message_verbosity);
        return Err(report);
    }
    if !messages.is_empty() {
        emit_messages_with_verbosity(state, &messages, overrides.message_verbosity);
    }
    Ok(ReplCommandOutput::Output(plugin_data_to_output_result(
        response.data,
        Some(&response.meta),
    )))
}

fn run_plugins_repl_command(state: &AppState, args: PluginsArgs) -> Result<ReplCommandOutput> {
    let plugin_manager = &state.clients.plugins;
    match args.command {
        PluginsCommands::List => {
            let mut plugins = plugin_manager
                .list_plugins()
                .map_err(|err| miette!("{err:#}"))?;
            plugins.sort_by(|a, b| a.plugin_id.cmp(&b.plugin_id));
            Ok(ReplCommandOutput::Output(rows_to_output_result(
                plugin_list_rows(&plugins),
            )))
        }
        PluginsCommands::Commands => {
            let mut commands = authorized_command_catalog(state)?;
            commands.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(ReplCommandOutput::Output(rows_to_output_result(
                command_catalog_rows(&commands),
            )))
        }
        PluginsCommands::Doctor => {
            let report = plugin_manager.doctor().map_err(|err| miette!("{err:#}"))?;
            Ok(ReplCommandOutput::Output(rows_to_output_result(
                doctor_rows(&report),
            )))
        }
        PluginsCommands::Enable(PluginToggleArgs { plugin_id }) => {
            plugin_manager
                .set_enabled(&plugin_id, true)
                .map_err(|err| miette!("{err:#}"))?;
            Ok(ReplCommandOutput::Text(format!(
                "enabled plugin: {plugin_id}\n"
            )))
        }
        PluginsCommands::Disable(PluginToggleArgs { plugin_id }) => {
            plugin_manager
                .set_enabled(&plugin_id, false)
                .map_err(|err| miette!("{err:#}"))?;
            Ok(ReplCommandOutput::Text(format!(
                "disabled plugin: {plugin_id}\n"
            )))
        }
    }
}

fn run_theme_repl_command(state: &mut AppState, args: ThemeArgs) -> Result<ReplCommandOutput> {
    match args.command {
        ThemeCommands::List => Ok(ReplCommandOutput::Output(rows_to_output_result(
            theme_list_rows(&state.ui.render_settings.theme_name),
        ))),
        ThemeCommands::Show(ThemeShowArgs { name }) => {
            let selected = name.unwrap_or_else(|| state.ui.render_settings.theme_name.clone());
            Ok(ReplCommandOutput::Output(rows_to_output_result(
                theme_show_rows(&selected)?,
            )))
        }
        ThemeCommands::Use(ThemeUseArgs { name }) => {
            let selected = resolve_known_theme_name(&name)?;
            state.ui.render_settings.theme_name = selected.clone();
            Ok(ReplCommandOutput::Text(format!(
                "active theme set to: {selected}\n"
            )))
        }
    }
}

fn repl_command_spec(command: &Commands) -> ReplCommandSpec {
    match command {
        Commands::External(tokens) => ReplCommandSpec {
            name: Cow::Owned(
                tokens
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "external".to_string()),
            ),
            supports_dsl: true,
        },
        Commands::Plugins(args) => ReplCommandSpec {
            name: Cow::Borrowed(CMD_PLUGINS),
            supports_dsl: matches!(
                args.command,
                PluginsCommands::List | PluginsCommands::Commands | PluginsCommands::Doctor
            ),
        },
        Commands::Theme(args) => ReplCommandSpec {
            name: Cow::Borrowed(CMD_THEME),
            supports_dsl: matches!(args.command, ThemeCommands::List | ThemeCommands::Show(_)),
        },
        Commands::Config(args) => ReplCommandSpec {
            name: Cow::Borrowed(CMD_CONFIG),
            supports_dsl: matches!(
                args.command,
                ConfigCommands::Show(_) | ConfigCommands::Get(_) | ConfigCommands::Diagnostics
            ),
        },
    }
}

fn run_config_command(state: &mut AppState, args: ConfigArgs) -> Result<i32> {
    match args.command {
        ConfigCommands::Show(show) => {
            let rows = config_show_rows(state, show);
            print!(
                "{}",
                render_output(&rows_to_output_result(rows), &state.ui.render_settings)
            );
            Ok(0)
        }
        ConfigCommands::Get(get) => match config_get_rows(state, get)? {
            Some(rows) => {
                print!(
                    "{}",
                    render_output(&rows_to_output_result(rows), &state.ui.render_settings)
                );
                Ok(0)
            }
            None => Ok(1),
        },
        ConfigCommands::Explain(explain) => match config_explain_output(state, explain)? {
            Some(output) => {
                print!("{output}");
                Ok(0)
            }
            None => Ok(1),
        },
        ConfigCommands::Set(set) => run_config_set(state, set),
        ConfigCommands::Diagnostics => {
            let rows = config_diagnostics_rows(state);
            print!(
                "{}",
                render_output(&rows_to_output_result(rows), &state.ui.render_settings)
            );
            Ok(0)
        }
    }
}

fn run_config_repl_command(state: &mut AppState, args: ConfigArgs) -> Result<ReplCommandOutput> {
    match args.command {
        ConfigCommands::Show(show) => Ok(ReplCommandOutput::Output(rows_to_output_result(
            config_show_rows(state, show),
        ))),
        ConfigCommands::Get(get) => match config_get_rows(state, get)? {
            Some(rows) => Ok(ReplCommandOutput::Output(rows_to_output_result(rows))),
            None => Ok(ReplCommandOutput::Text(String::new())),
        },
        ConfigCommands::Explain(explain) => match config_explain_output(state, explain)? {
            Some(output) => Ok(ReplCommandOutput::Text(output)),
            None => Ok(ReplCommandOutput::Text(String::new())),
        },
        ConfigCommands::Set(set) => {
            run_config_set(state, set)?;
            Ok(ReplCommandOutput::Text(String::new()))
        }
        ConfigCommands::Diagnostics => Ok(ReplCommandOutput::Output(rows_to_output_result(
            config_diagnostics_rows(state),
        ))),
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

fn config_explain_output(state: &AppState, args: ConfigExplainArgs) -> Result<Option<String>> {
    let explain = explain_runtime_config(
        Some(state.config.resolved().active_profile().to_string()),
        state.config.resolved().terminal(),
        &args.key,
        Some(state.session.config_overrides.clone()),
    )?;

    if explain.final_entry.is_none() && explain.layers.is_empty() {
        let suggestions = suggest_config_keys(state.config.resolved(), &args.key);
        let mut messages = MessageBuffer::default();
        messages.error(format!("config key not found: {}", args.key));
        if !suggestions.is_empty() {
            messages.info(format!("did you mean: {}", suggestions.join(", ")));
        }
        emit_messages(state, &messages);
        return Ok(None);
    }

    if matches!(state.ui.render_settings.format, OutputFormat::Json) {
        let payload = config_explain_json(&explain, args.show_secrets);
        return Ok(Some(format!(
            "{}\n",
            serde_json::to_string_pretty(&payload).into_diagnostic()?
        )));
    }

    Ok(Some(render_config_explain_text(
        &explain,
        args.show_secrets,
    )))
}

fn config_diagnostics_rows(state: &AppState) -> Vec<Row> {
    let mut row = Row::new();
    row.insert("status".to_string(), "ok".into());
    row.insert(
        "active_profile".to_string(),
        state.config.resolved().active_profile().to_string().into(),
    );
    row.insert(
        "known_profiles".to_string(),
        serde_json::Value::Array(
            state
                .config
                .resolved()
                .known_profiles()
                .iter()
                .map(|value| value.clone().into())
                .collect(),
        ),
    );
    row.insert(
        "resolved_keys".to_string(),
        (state.config.resolved().values().len() as i64).into(),
    );
    vec![row]
}

fn run_config_set(state: &mut AppState, args: ConfigSetArgs) -> Result<i32> {
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
        let mut row = Row::new();
        row.insert("key".to_string(), key.clone().into());
        row.insert("value".to_string(), config_value_to_json(&value));
        row.insert("scope".to_string(), format_scope(scope).into());
        row.insert("store".to_string(), config_store_name(store).into());
        row.insert("dry_run".to_string(), serde_json::Value::Bool(args.dry_run));

        match store {
            ConfigStore::Session => {
                if !args.dry_run {
                    state.session.config_overrides.insert(
                        key.clone(),
                        value.clone(),
                        scope.clone(),
                    );
                }
                row.insert("path".to_string(), serde_json::Value::Null);
                row.insert("changed".to_string(), serde_json::Value::Bool(true));
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

                row.insert("path".to_string(), target_path.display().to_string().into());
                row.insert(
                    "changed".to_string(),
                    serde_json::Value::Bool(set_result.previous.as_ref() != Some(&value)),
                );
                row.insert(
                    "previous".to_string(),
                    set_result
                        .previous
                        .as_ref()
                        .map(config_value_to_json)
                        .unwrap_or(serde_json::Value::Null),
                );
            }
        }

        rows.push(row);
    }

    if !args.dry_run {
        refresh_runtime_config(state)?;
    }

    if args.explain {
        let explain = explain_runtime_config(
            Some(state.config.resolved().active_profile().to_string()),
            state.config.resolved().terminal(),
            &key,
            Some(state.session.config_overrides.clone()),
        )?;
        if matches!(state.ui.render_settings.format, OutputFormat::Json) {
            println!(
                "{}",
                serde_json::to_string_pretty(&config_explain_json(&explain, false))
                    .into_diagnostic()?
            );
        } else {
            print!("{}", render_config_explain_text(&explain, false));
        }
    } else {
        print!(
            "{}",
            render_output(&rows_to_output_result(rows), &state.ui.render_settings)
        );
    }

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
    Ok(0)
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
        state.auth = crate::state::AuthState::from_resolved(state.config.resolved());
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

fn explain_runtime_config(
    profile_override: Option<String>,
    terminal: Option<&str>,
    key: &str,
    session_layer: Option<ConfigLayer>,
) -> Result<ConfigExplain> {
    let defaults = RuntimeDefaults::from_process_env(DEFAULT_THEME_NAME, DEFAULT_REPL_PROMPT);
    let paths = RuntimeConfigPaths::discover();
    let pipeline = build_runtime_pipeline(defaults.to_layer(), &paths, None, session_layer);

    let layers = pipeline
        .load_layers()
        .into_diagnostic()
        .wrap_err("config layer loading failed")?;
    let resolver = ConfigResolver::from_loaded_layers(layers);
    resolver
        .explain_key(
            key,
            ResolveOptions {
                profile_override,
                terminal: terminal.map(|value| value.to_string()),
            },
        )
        .into_diagnostic()
        .wrap_err("config explain failed")
}

fn render_config_explain_text(explain: &ConfigExplain, show_secrets: bool) -> String {
    let mut out = String::new();
    out.push_str(&format!("key: {}\n", explain.key));

    if let Some(final_entry) = &explain.final_entry {
        let value_display = display_value(&explain.key, &final_entry.value, show_secrets);
        out.push_str(&format!(
            "value: {} ({})\n\n",
            value_display,
            config_value_type(&final_entry.value)
        ));
        out.push_str("winner:\n");
        out.push_str(&format!("  source: {}\n", final_entry.source));
        out.push_str(&format!("  scope: {}\n", format_scope(&final_entry.scope)));
        out.push_str(&format!(
            "  origin: {}\n\n",
            final_entry.origin.as_deref().unwrap_or("-")
        ));
    } else {
        out.push_str("value: not set\n\n");
    }

    out.push_str("context:\n");
    out.push_str(&format!("  active_profile: {}\n", explain.active_profile));
    out.push_str(&format!(
        "  terminal: {}\n\n",
        explain.terminal.as_deref().unwrap_or("none")
    ));

    let precedence = effective_precedence_chain(explain);
    if !precedence.is_empty() {
        out.push_str("candidates (in priority order):\n");
        for (is_winner, source, scope, origin, value) in precedence {
            let marker = if is_winner { "  ✅" } else { "   " };
            out.push_str(&format!(
                "{marker} {source} ({scope}) = {}",
                display_value(&explain.key, &value, show_secrets),
            ));
            if let Some(origin_hint) = origin {
                out.push_str(&format!(" [{origin_hint}]"));
            }
            out.push('\n');
        }
        out.push('\n');
    }

    if let Some(interpolation) = &explain.interpolation {
        out.push_str("interpolation:\n");
        out.push_str(&format!(
            "  template: {}\n",
            display_value(
                &explain.key,
                &ConfigValue::String(interpolation.template.clone()),
                show_secrets
            )
        ));
        for step in &interpolation.steps {
            out.push_str(&format!(
                "  ${{{}}} -> {} (from {}, {})\n",
                step.placeholder,
                display_value(&step.placeholder, &step.value, show_secrets),
                step.source,
                format_scope(&step.scope),
            ));
        }
        if !show_secrets && contains_sensitive_values(explain) {
            out.push_str("  note: some values are redacted; pass --show-secrets to display them\n");
        }
    }

    out
}

fn config_explain_json(explain: &ConfigExplain, show_secrets: bool) -> serde_json::Value {
    let mut root = serde_json::Map::new();
    root.insert("key".to_string(), explain.key.clone().into());
    root.insert(
        "active_profile".to_string(),
        explain.active_profile.clone().into(),
    );
    root.insert(
        "terminal".to_string(),
        explain
            .terminal
            .clone()
            .map_or(serde_json::Value::Null, Into::into),
    );

    if let Some(final_entry) = &explain.final_entry {
        root.insert(
            "value".to_string(),
            redact_value_json(&explain.key, &final_entry.value, show_secrets),
        );
        root.insert(
            "value_type".to_string(),
            config_value_type(&final_entry.value).to_string().into(),
        );
        root.insert("source".to_string(), final_entry.source.to_string().into());
        root.insert("scope".to_string(), format_scope(&final_entry.scope).into());
        root.insert(
            "origin".to_string(),
            final_entry
                .origin
                .clone()
                .map_or(serde_json::Value::Null, Into::into),
        );
    } else {
        root.insert("value".to_string(), serde_json::Value::Null);
        root.insert("value_type".to_string(), "none".into());
        root.insert("source".to_string(), serde_json::Value::Null);
        root.insert("scope".to_string(), serde_json::Value::Null);
        root.insert("origin".to_string(), serde_json::Value::Null);
    }

    let mut candidates = Vec::new();
    for (is_winner, source, scope, origin, value) in effective_precedence_chain(explain) {
        let mut row = serde_json::Map::new();
        row.insert("winner".to_string(), is_winner.into());
        row.insert("source".to_string(), source.to_string().into());
        row.insert("scope".to_string(), scope.into());
        row.insert(
            "origin".to_string(),
            origin.map_or(serde_json::Value::Null, Into::into),
        );
        row.insert(
            "value".to_string(),
            redact_value_json(&explain.key, &value, show_secrets),
        );
        candidates.push(serde_json::Value::Object(row));
    }
    root.insert(
        "candidates".to_string(),
        serde_json::Value::Array(candidates),
    );

    if let Some(interpolation) = &explain.interpolation {
        let mut section = serde_json::Map::new();
        section.insert(
            "template".to_string(),
            redact_value_json(
                &explain.key,
                &ConfigValue::String(interpolation.template.clone()),
                show_secrets,
            ),
        );
        let mut steps = Vec::new();
        for step in &interpolation.steps {
            let mut item = serde_json::Map::new();
            item.insert("placeholder".to_string(), step.placeholder.clone().into());
            item.insert(
                "value".to_string(),
                redact_value_json(&step.placeholder, &step.value, show_secrets),
            );
            item.insert("source".to_string(), step.source.to_string().into());
            item.insert("scope".to_string(), format_scope(&step.scope).into());
            item.insert(
                "origin".to_string(),
                step.origin
                    .clone()
                    .map_or(serde_json::Value::Null, Into::into),
            );
            steps.push(serde_json::Value::Object(item));
        }
        section.insert("steps".to_string(), serde_json::Value::Array(steps));
        root.insert(
            "interpolation".to_string(),
            serde_json::Value::Object(section),
        );
    }

    serde_json::Value::Object(root)
}

fn effective_precedence_chain(
    explain: &ConfigExplain,
) -> Vec<(bool, String, String, Option<String>, ConfigValue)> {
    let winner_source = explain.final_entry.as_ref().map(|entry| entry.source);
    let mut chain = Vec::new();

    for layer in &explain.layers {
        let Some(candidate) = layer
            .candidates
            .iter()
            .find(|candidate| candidate.selected_in_layer && candidate.rank.is_some())
        else {
            continue;
        };

        chain.push((
            winner_source == Some(layer.source),
            layer.source.to_string(),
            format_scope(&candidate.scope),
            candidate.origin.clone(),
            candidate.value.clone(),
        ));
    }

    chain
}

fn config_value_type(value: &ConfigValue) -> &'static str {
    match value {
        ConfigValue::String(_) => "string",
        ConfigValue::Bool(_) => "bool",
        ConfigValue::Integer(_) => "integer",
        ConfigValue::Float(_) => "float",
        ConfigValue::List(_) => "list",
    }
}

fn redact_value_json(key: &str, value: &ConfigValue, show_secrets: bool) -> serde_json::Value {
    if show_secrets || !is_sensitive_key(key) {
        return config_value_to_json(value);
    }

    "[REDACTED]".into()
}

fn display_value(key: &str, value: &ConfigValue, show_secrets: bool) -> String {
    if show_secrets || !is_sensitive_key(key) {
        return match value {
            ConfigValue::String(v) => v.clone(),
            _ => config_value_to_json(value).to_string(),
        };
    }

    "[REDACTED]".to_string()
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase();
    normalized.contains("password")
        || normalized.contains("token")
        || normalized.contains("secret")
        || normalized.ends_with(".key")
}

fn format_scope(scope: &osp_config::Scope) -> String {
    match (scope.profile.as_deref(), scope.terminal.as_deref()) {
        (Some(profile), Some(terminal)) => format!("profile:{profile} terminal:{terminal}"),
        (Some(profile), None) => format!("profile:{profile}"),
        (None, Some(terminal)) => format!("terminal:{terminal}"),
        (None, None) => "global".to_string(),
    }
}

fn contains_sensitive_values(explain: &ConfigExplain) -> bool {
    if is_sensitive_key(&explain.key) {
        return true;
    }

    explain.interpolation.as_ref().is_some_and(|trace| {
        trace
            .steps
            .iter()
            .any(|step| is_sensitive_key(&step.placeholder))
    })
}

fn suggest_config_keys(config: &ResolvedConfig, key: &str) -> Vec<String> {
    let key_lc = key.to_ascii_lowercase();
    let mut prefix_matches = config
        .values()
        .keys()
        .filter(|candidate| candidate.starts_with(&key_lc) || candidate.contains(&key_lc))
        .take(5)
        .cloned()
        .collect::<Vec<String>>();

    if prefix_matches.is_empty() {
        prefix_matches = config
            .values()
            .keys()
            .filter(|candidate| {
                let left = candidate.split('.').next().unwrap_or_default();
                let right = key_lc.split('.').next().unwrap_or_default();
                left == right
            })
            .take(5)
            .cloned()
            .collect();
    }

    prefix_matches
}

fn run_external_command(state: &AppState, tokens: &[String]) -> Result<i32> {
    let plugin_manager = &state.clients.plugins;
    let settings = &state.ui.render_settings;
    let (command, args) = tokens
        .split_first()
        .ok_or_else(|| miette!("missing external command"))?;
    ensure_plugin_visible(state, command)?;

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
            print!("{}", raw.stdout);
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

    let output = plugin_data_to_output_result(response.data, Some(&response.meta));
    print!("{}", render_output(&output, settings));

    Ok(0)
}

fn resolve_theme_name(cli: &Cli, config: &ResolvedConfig) -> Result<String> {
    let selected = cli.selected_theme_name(config);
    resolve_known_theme_name(&selected)
}

fn resolve_known_theme_name(value: &str) -> Result<String> {
    let normalized = normalize_theme_name(value);
    if is_known_theme(&normalized) {
        return Ok(normalized);
    }

    let known = available_theme_names().join(", ");
    Err(miette!("unknown theme: {value}. available themes: {known}"))
}

fn theme_list_rows(active_theme: &str) -> Vec<Row> {
    let active = normalize_theme_name(active_theme);
    available_theme_names()
        .into_iter()
        .map(|name| {
            let mut row = Row::new();
            row.insert("name".to_string(), name.to_string().into());
            row.insert(
                "active".to_string(),
                serde_json::Value::Bool(name == active.as_str()),
            );
            row.insert(
                "default".to_string(),
                serde_json::Value::Bool(name == DEFAULT_THEME_NAME),
            );
            row
        })
        .collect()
}

fn theme_show_rows(name: &str) -> Result<Vec<Row>> {
    let selected = resolve_known_theme_name(name)?;
    let theme = find_theme(&selected).ok_or_else(|| miette!("theme missing: {selected}"))?;
    let palette = theme.palette;

    let mut row = Row::new();
    row.insert("name".to_string(), theme.name.to_string().into());
    row.insert("text".to_string(), palette.text.to_string().into());
    row.insert("muted".to_string(), palette.muted.to_string().into());
    row.insert("accent".to_string(), palette.accent.to_string().into());
    row.insert("info".to_string(), palette.info.to_string().into());
    row.insert("warning".to_string(), palette.warning.to_string().into());
    row.insert("success".to_string(), palette.success.to_string().into());
    row.insert("error".to_string(), palette.error.to_string().into());
    row.insert("border".to_string(), palette.border.to_string().into());
    row.insert("title".to_string(), palette.title.to_string().into());
    Ok(vec![row])
}

fn is_help_passthrough(args: &[String]) -> bool {
    if args.is_empty() {
        return false;
    }

    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        return true;
    }

    matches!(args.first(), Some(first) if first == CMD_HELP)
}

fn enrich_dispatch_error(err: PluginDispatchError) -> miette::Report {
    match err {
        not_found @ PluginDispatchError::CommandNotFound { .. } => miette!(
            "{not_found}\nHint: run `osp plugins list` and set --plugin-dir or OSP_PLUGIN_PATH"
        ),
        other => miette!("{other}"),
    }
}

fn config_usize(config: &ResolvedConfig, key: &str, fallback: usize) -> usize {
    match config.get(key) {
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

fn effective_message_verbosity(cli: &Cli, config: &ResolvedConfig) -> MessageLevel {
    let base = config
        .get_string("ui.verbosity.level")
        .and_then(parse_message_level)
        .unwrap_or(MessageLevel::Success);
    adjust_verbosity(base, cli.verbose, cli.quiet)
}

fn effective_debug_verbosity(cli: &Cli, config: &ResolvedConfig) -> u8 {
    if cli.debug > 0 {
        return cli.debug.min(3);
    }

    match config.get("debug.level") {
        Some(ConfigValue::Integer(level)) => (*level).clamp(0, 3) as u8,
        Some(ConfigValue::String(raw)) => raw.trim().parse::<u8>().unwrap_or(0).min(3),
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

fn plugin_dispatch_context(
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

fn plugin_response_messages(response: &ResponseV1) -> MessageBuffer {
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

fn emit_messages(state: &AppState, messages: &MessageBuffer) {
    emit_messages_with_verbosity(state, messages, state.ui.message_verbosity);
}

fn emit_messages_with_verbosity(
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
        &resolved.theme_name,
        message_format,
        &resolved.style_overrides,
    );
    if !rendered.is_empty() {
        eprint!("{rendered}");
    }
}

fn response_to_rows(data: serde_json::Value) -> Vec<Row> {
    match data {
        serde_json::Value::Array(items)
            if items
                .iter()
                .all(|item| matches!(item, serde_json::Value::Object(_))) =>
        {
            items
                .into_iter()
                .filter_map(|item| item.as_object().cloned())
                .collect::<Vec<Row>>()
        }
        serde_json::Value::Object(map) => vec![map],
        scalar => {
            let mut row = Row::new();
            row.insert("value".to_string(), scalar);
            vec![row]
        }
    }
}

fn rows_to_output_result(rows: Vec<Row>) -> OutputResult {
    OutputResult {
        meta: OutputMeta {
            key_index: compute_key_index(&rows),
            wants_copy: false,
            grouped: false,
        },
        items: OutputItems::Rows(rows),
    }
}

fn output_to_rows(output: &OutputResult) -> Vec<Row> {
    match &output.items {
        OutputItems::Rows(rows) => rows.clone(),
        OutputItems::Groups(groups) => {
            let mut out = Vec::new();
            for group in groups {
                if group.rows.is_empty() {
                    out.push(merge_group_header_row(group));
                    continue;
                }
                for row in &group.rows {
                    out.push(merge_group_row(group, row));
                }
            }
            out
        }
    }
}

fn compute_key_index(rows: &[Row]) -> Vec<String> {
    let mut keys = Vec::new();
    let mut seen = HashSet::new();
    for row in rows {
        for key in row.keys() {
            if seen.insert(key.clone()) {
                keys.push(key.clone());
            }
        }
    }
    keys
}

fn merge_group_header_row(group: &osp_core::output_model::Group) -> Row {
    let mut row = group.groups.clone();
    for (key, value) in &group.aggregates {
        row.insert(key.clone(), value.clone());
    }
    row
}

fn merge_group_row(group: &osp_core::output_model::Group, row: &Row) -> Row {
    let mut merged = group.groups.clone();
    for (key, value) in &group.aggregates {
        merged.insert(key.clone(), value.clone());
    }
    for (key, value) in row {
        merged.insert(key.clone(), value.clone());
    }
    merged
}

fn plugin_data_to_output_result(
    data: serde_json::Value,
    meta: Option<&ResponseMetaV1>,
) -> OutputResult {
    let rows = response_to_rows(data);
    let key_index = meta
        .and_then(|value| value.columns.clone())
        .filter(|columns| !columns.is_empty())
        .unwrap_or_else(|| compute_key_index(&rows));
    OutputResult {
        items: OutputItems::Rows(rows),
        meta: OutputMeta {
            key_index,
            wants_copy: false,
            grouped: false,
        },
    }
}

fn plugin_list_rows(plugins: &[PluginSummary]) -> Vec<Row> {
    if plugins.is_empty() {
        let mut row = Row::new();
        row.insert("status".to_string(), "empty".into());
        row.insert("message".to_string(), "No plugins discovered.".into());
        return vec![row];
    }

    plugins
        .iter()
        .map(|plugin| {
            let mut row = Row::new();
            row.insert("plugin_id".to_string(), plugin.plugin_id.clone().into());
            row.insert(
                "enabled".to_string(),
                serde_json::Value::Bool(plugin.enabled),
            );
            row.insert(
                "healthy".to_string(),
                serde_json::Value::Bool(plugin.healthy),
            );
            row.insert("source".to_string(), plugin.source.to_string().into());
            row.insert(
                "plugin_version".to_string(),
                plugin
                    .plugin_version
                    .clone()
                    .map_or(serde_json::Value::Null, Into::into),
            );
            row.insert(
                "path".to_string(),
                plugin.executable.display().to_string().into(),
            );
            row.insert(
                "commands".to_string(),
                serde_json::Value::Array(
                    plugin
                        .commands
                        .iter()
                        .map(|command| command.clone().into())
                        .collect(),
                ),
            );
            row.insert(
                "issue".to_string(),
                plugin
                    .issue
                    .clone()
                    .map_or(serde_json::Value::Null, Into::into),
            );
            row
        })
        .collect()
}

fn command_catalog_rows(commands: &[CommandCatalogEntry]) -> Vec<Row> {
    if commands.is_empty() {
        let mut row = Row::new();
        row.insert("status".to_string(), "empty".into());
        row.insert(
            "message".to_string(),
            "No plugin commands discovered.".into(),
        );
        return vec![row];
    }

    commands
        .iter()
        .map(|command| {
            let mut row = Row::new();
            row.insert("name".to_string(), command.name.clone().into());
            row.insert("about".to_string(), command.about.clone().into());
            row.insert("provider".to_string(), command.provider.clone().into());
            row.insert("source".to_string(), command.source.to_string().into());
            row.insert(
                "subcommands".to_string(),
                serde_json::Value::Array(
                    command
                        .subcommands
                        .iter()
                        .map(|value| value.clone().into())
                        .collect(),
                ),
            );
            row
        })
        .collect()
}

fn doctor_rows(report: &DoctorReport) -> Vec<Row> {
    let mut rows = Vec::new();

    let mut summary = Row::new();
    summary.insert("kind".to_string(), "summary".into());
    summary.insert(
        "plugins".to_string(),
        serde_json::Value::from(report.plugins.len() as i64),
    );
    summary.insert(
        "broken_enabled".to_string(),
        serde_json::Value::from(
            report
                .plugins
                .iter()
                .filter(|plugin| plugin.enabled && !plugin.healthy)
                .count() as i64,
        ),
    );
    summary.insert(
        "conflicts".to_string(),
        serde_json::Value::from(report.conflicts.len() as i64),
    );
    rows.push(summary);

    for conflict in &report.conflicts {
        let mut row = Row::new();
        row.insert("kind".to_string(), "conflict".into());
        row.insert("command".to_string(), conflict.command.clone().into());
        row.insert(
            "providers".to_string(),
            serde_json::Value::Array(
                conflict
                    .providers
                    .iter()
                    .map(|provider| provider.clone().into())
                    .collect(),
            ),
        );
        rows.push(row);
    }

    rows
}

fn format_plugin_list(plugins: &[PluginSummary]) -> String {
    if plugins.is_empty() {
        return "No plugins discovered.\n".to_string();
    }

    let mut out = String::new();
    for plugin in plugins {
        let commands = if plugin.commands.is_empty() {
            "-".to_string()
        } else {
            plugin.commands.join(",")
        };
        let status = if !plugin.enabled {
            "disabled"
        } else if plugin.healthy {
            "ok"
        } else {
            "error"
        };
        let version = plugin.plugin_version.as_deref().unwrap_or("-");

        out.push_str(&format!(
            "{id}\t{status}\t{source}\t{version}\t{commands}\t{path}\n",
            id = plugin.plugin_id,
            source = plugin.source,
            path = plugin.executable.display(),
        ));

        if let Some(issue) = &plugin.issue {
            out.push_str(&format!("  issue: {issue}\n"));
        }
    }

    out
}

fn format_command_catalog(commands: &[CommandCatalogEntry]) -> String {
    if commands.is_empty() {
        return "No plugin commands discovered.\n".to_string();
    }

    let mut out = String::new();
    for command in commands {
        let subs = if command.subcommands.is_empty() {
            "-".to_string()
        } else {
            command.subcommands.join(",")
        };
        let about = if command.about.trim().is_empty() {
            "-".to_string()
        } else {
            command.about.clone()
        };
        out.push_str(&format!(
            "{name}\t{subs}\t{about}\t{provider}\t{source}\n",
            name = command.name,
            about = about,
            provider = command.provider,
            source = command.source,
        ));
    }
    out
}

fn format_doctor_report(report: &DoctorReport) -> String {
    let mut out = String::new();
    out.push_str(&format!("Plugins: {}\n", report.plugins.len()));
    let broken = report
        .plugins
        .iter()
        .filter(|plugin| plugin.enabled && !plugin.healthy)
        .count();
    out.push_str(&format!("Broken enabled plugins: {broken}\n"));

    if report.conflicts.is_empty() {
        out.push_str("Command conflicts: none\n");
    } else {
        out.push_str("Command conflicts:\n");
        for conflict in &report.conflicts {
            out.push_str(&format!(
                "  {} -> {}\n",
                conflict.command,
                conflict.providers.join(" | ")
            ));
        }
    }

    out
}

fn config_entry_row(
    key: &str,
    entry: &ResolvedValue,
    include_sources: bool,
    show_raw: bool,
) -> Row {
    let mut row = Row::new();
    row.insert("key".to_string(), key.to_string().into());
    row.insert(
        "value".to_string(),
        config_value_to_json(if show_raw {
            &entry.raw_value
        } else {
            &entry.value
        }),
    );

    if include_sources {
        row.insert("source".to_string(), entry.source.to_string().into());
        row.insert(
            "origin".to_string(),
            entry
                .origin
                .clone()
                .map_or(serde_json::Value::Null, Into::into),
        );
        row.insert(
            "scope_profile".to_string(),
            entry
                .scope
                .profile
                .clone()
                .map_or(serde_json::Value::Null, |v| v.into()),
        );
        row.insert(
            "scope_terminal".to_string(),
            entry
                .scope
                .terminal
                .clone()
                .map_or(serde_json::Value::Null, |v| v.into()),
        );
    }

    row
}

fn config_value_to_json(value: &ConfigValue) -> serde_json::Value {
    match value {
        ConfigValue::String(v) => v.clone().into(),
        ConfigValue::Bool(v) => (*v).into(),
        ConfigValue::Integer(v) => (*v).into(),
        ConfigValue::Float(v) => (*v).into(),
        ConfigValue::List(values) => {
            serde_json::Value::Array(values.iter().map(config_value_to_json).collect())
        }
    }
}

fn resolve_runtime_config(
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

#[cfg(test)]
mod tests {
    use super::{RunAction, build_dispatch_plan};
    use crate::cli::{Cli, Commands, ConfigCommands, PluginsCommands, ThemeCommands};
    use crate::plugin_manager::PluginManager;
    use crate::state::{AppState, RuntimeContext, TerminalKind};
    use clap::Parser;
    use osp_config::{ConfigLayer, ConfigResolver, ResolveOptions};
    use osp_core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
    use osp_ui::RenderSettings;
    use osp_ui::messages::MessageLevel;
    use osp_ui::theme::DEFAULT_THEME_NAME;
    use std::collections::BTreeSet;

    fn profiles(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| name.to_string()).collect()
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

        assert!(super::repl_command_spec(&plugins_list).supports_dsl);
        assert!(!super::repl_command_spec(&plugins_enable).supports_dsl);
        assert!(super::repl_command_spec(&theme_show).supports_dsl);
        assert!(!super::repl_command_spec(&theme_use).supports_dsl);
        assert!(super::repl_command_spec(&config_show).supports_dsl);
        assert!(!super::repl_command_spec(&config_set).supports_dsl);
    }

    #[test]
    fn repl_prompt_template_substitutes_profile_and_indicator_unit() {
        let rendered = super::render_prompt_template(
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
            super::render_prompt_template("{profile}>", "oistes", "uio.no", "tsd", "[shell]");
        assert_eq!(rendered, "tsd> [shell]");
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

        let err = super::execute_repl_plugin_line(&mut state, "fail")
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

        let first = super::execute_repl_plugin_line(&mut state, "cache first")
            .expect("first command should succeed");
        assert!(first.contains("ok"));

        let second = super::execute_repl_plugin_line(&mut state, "cache second")
            .expect("second command should succeed");
        assert!(second.contains("ok"));

        assert_eq!(state.repl_cache_size(), 1);
        assert!(state.cached_repl_rows("cache first").is_none());
        assert!(state.cached_repl_rows("cache second").is_some());
        assert!(!state.last_repl_rows().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
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
            theme_name: DEFAULT_THEME_NAME.to_string(),
            style_overrides: osp_ui::StyleOverrides::default(),
        };

        AppState::new(
            RuntimeContext::new(None, TerminalKind::Repl, None),
            config,
            settings,
            MessageLevel::Success,
            0,
            PluginManager::new(plugin_dirs),
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
