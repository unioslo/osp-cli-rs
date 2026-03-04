use clap::Parser;
use miette::{IntoDiagnostic, Result, WrapErr, miette};
use osp_config::{
    ChainedLoader, ConfigExplain, ConfigLayer, ConfigResolver, ConfigValue, EnvSecretsLoader,
    EnvVarLoader, LoaderPipeline, ResolveOptions, ResolvedConfig, ResolvedValue, SecretsTomlLoader,
    StaticLayerLoader, TomlFileLoader,
};
use osp_core::output::OutputFormat;
use osp_core::row::Row;
use osp_dsl::{apply_pipeline, parse_pipeline};
use osp_repl::{ReplPrompt, run_repl};
use osp_ui::messages::{MessageBuffer, render_section_divider};
use osp_ui::render_rows;
use osp_ui::style::{StyleToken, apply_style, apply_style_spec};
use osp_ui::theme::{
    DEFAULT_THEME_NAME, available_theme_names, find_theme, is_known_theme, normalize_theme_name,
};
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::cli::{
    Cli, Commands, ConfigArgs, ConfigCommands, ConfigExplainArgs, ConfigGetArgs, ConfigShowArgs,
    PluginToggleArgs, PluginsArgs, PluginsCommands, ThemeArgs, ThemeCommands, ThemeShowArgs,
    ThemeUseArgs, parse_inline_command_tokens, parse_repl_tokens,
};
use crate::logging::init_developer_logging;
use crate::plugin_manager::{
    CommandCatalogEntry, DoctorReport, PluginDispatchError, PluginManager, PluginSummary,
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

#[derive(Debug, Clone)]
struct ReplCommandSpec {
    name: Cow<'static, str>,
    supports_dsl: bool,
}

enum ReplCommandOutput {
    Rows(Vec<Row>),
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
    init_developer_logging(cli.debug);
    tracing::debug!(debug_count = cli.debug, "developer logging initialized");

    let initial_config = resolve_runtime_config(cli.profile.clone(), Some("cli"))?;
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
    )?;
    let mut render_settings = cli.render_settings();
    cli.seed_render_settings_from_config(&mut render_settings, &config);
    render_settings.theme_name = resolve_theme_name(&cli, &config)?;

    let mut state = AppState::new(
        runtime_context,
        config,
        render_settings,
        cli.message_verbosity(),
        cli.debug,
        PluginManager::new(cli.plugin_dirs.clone()),
    );

    tracing::info!(
        profile = %state.config.resolved().active_profile(),
        terminal = %state.context.terminal_kind().as_config_terminal(),
        "osp session initialized"
    );

    match dispatch.action {
        RunAction::Repl => run_plugin_repl(&mut state),
        RunAction::Plugins(args) => run_plugins_command(&state, args),
        RunAction::Theme(args) => run_theme_command(&mut state, args),
        RunAction::Config(args) => run_config_command(&state, args),
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

fn run_plugin_repl(state: &mut AppState) -> Result<i32> {
    let mut words = state
        .clients
        .plugins
        .completion_words()
        .map_err(|err| miette!("{err:#}"))?;
    words.extend(
        [
            CMD_PLUGINS,
            CMD_CONFIG,
            CMD_THEME,
            CMD_LIST,
            CMD_SHOW,
            CMD_USE,
        ]
        .into_iter()
        .map(str::to_string),
    );
    words.extend(
        available_theme_names()
            .into_iter()
            .map(std::string::ToString::to_string),
    );
    words.sort();
    words.dedup();

    let mut help_text = state
        .clients
        .plugins
        .repl_help_text()
        .map_err(|err| miette!("{err:#}"))?;
    help_text
        .push_str("Backbone theme commands: theme list | theme show [name] | theme use <name>\n");
    if state
        .config
        .resolved()
        .get_bool("repl.intro")
        .unwrap_or(true)
    {
        print!("{}", render_repl_intro(state));
    }
    let prompt = build_repl_prompt(state);
    run_repl(prompt, words, help_text, |line| {
        execute_repl_plugin_line(state, line).map_err(|err| anyhow::anyhow!("{err:#}"))
    })
    .map_err(|err| miette!("{err:#}"))
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
    out.push_str(&render_section_divider(
        "OSP",
        resolved.unicode,
        resolved.width,
        resolved.color,
        theme_name,
        StyleToken::MessageInfo,
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

    match run_repl_command(state, command)? {
        ReplCommandOutput::Rows(mut rows) => {
            rows = apply_pipeline(rows, &parsed.stages).map_err(|err| miette!("{err:#}"))?;
            let rendered = render_rows(&rows, &state.ui.render_settings);
            state.record_repl_rows(line, rows);
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
            let mut commands = plugin_manager
                .command_catalog()
                .map_err(|err| miette!("{err:#}"))?;
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
            print!("{}", render_rows(&rows, &state.ui.render_settings));
            Ok(0)
        }
        ThemeCommands::Show(ThemeShowArgs { name }) => {
            let selected = name.unwrap_or_else(|| state.ui.render_settings.theme_name.clone());
            let rows = theme_show_rows(&selected)?;
            print!("{}", render_rows(&rows, &state.ui.render_settings));
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

fn run_repl_command(state: &mut AppState, command: Commands) -> Result<ReplCommandOutput> {
    match command {
        Commands::Plugins(args) => run_plugins_repl_command(&state.clients.plugins, args),
        Commands::Theme(args) => run_theme_repl_command(state, args),
        Commands::Config(_args) => Err(miette!(
            "config commands are not yet available inside repl in this build"
        )),
        Commands::External(tokens) => run_repl_external_command(state, tokens),
    }
}

fn run_repl_external_command(state: &AppState, tokens: Vec<String>) -> Result<ReplCommandOutput> {
    let (command, args) = tokens
        .split_first()
        .ok_or_else(|| miette!("missing command"))?;
    if is_help_passthrough(args) {
        let raw = state
            .clients
            .plugins
            .dispatch_passthrough(command, args)
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

    let response = state
        .clients
        .plugins
        .dispatch(command, args)
        .map_err(enrich_dispatch_error)?;
    if !response.ok {
        if let Some(error) = response.error {
            return Err(miette!("{}: {}", error.code, error.message));
        }
        return Err(miette!("plugin command failed"));
    }
    Ok(ReplCommandOutput::Rows(response_to_rows(response.data)))
}

fn run_plugins_repl_command(
    plugin_manager: &PluginManager,
    args: PluginsArgs,
) -> Result<ReplCommandOutput> {
    match args.command {
        PluginsCommands::List => {
            let mut plugins = plugin_manager
                .list_plugins()
                .map_err(|err| miette!("{err:#}"))?;
            plugins.sort_by(|a, b| a.plugin_id.cmp(&b.plugin_id));
            Ok(ReplCommandOutput::Rows(plugin_list_rows(&plugins)))
        }
        PluginsCommands::Commands => {
            let mut commands = plugin_manager
                .command_catalog()
                .map_err(|err| miette!("{err:#}"))?;
            commands.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(ReplCommandOutput::Rows(command_catalog_rows(&commands)))
        }
        PluginsCommands::Doctor => {
            let report = plugin_manager.doctor().map_err(|err| miette!("{err:#}"))?;
            Ok(ReplCommandOutput::Rows(doctor_rows(&report)))
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
        ThemeCommands::List => Ok(ReplCommandOutput::Rows(theme_list_rows(
            &state.ui.render_settings.theme_name,
        ))),
        ThemeCommands::Show(ThemeShowArgs { name }) => {
            let selected = name.unwrap_or_else(|| state.ui.render_settings.theme_name.clone());
            Ok(ReplCommandOutput::Rows(theme_show_rows(&selected)?))
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
        Commands::Config(_args) => ReplCommandSpec {
            name: Cow::Borrowed(CMD_CONFIG),
            supports_dsl: false,
        },
    }
}

fn run_config_command(state: &AppState, args: ConfigArgs) -> Result<i32> {
    match args.command {
        ConfigCommands::Show(show) => {
            run_config_show(state, show);
            Ok(0)
        }
        ConfigCommands::Get(get) => run_config_get(state, get),
        ConfigCommands::Explain(explain) => run_config_explain(state, explain),
        ConfigCommands::Set(_set) => Err(miette!(
            "`config set` implementation is in progress; use the existing osprov-cli for writes currently"
        )),
        ConfigCommands::Diagnostics => {
            run_config_diagnostics(state);
            Ok(0)
        }
    }
}

fn run_config_show(state: &AppState, args: ConfigShowArgs) {
    let rows = state
        .config
        .resolved()
        .values()
        .iter()
        .map(|(key, entry)| config_entry_row(key, entry, args.sources, args.raw))
        .collect::<Vec<Row>>();

    print!("{}", render_rows(&rows, &state.ui.render_settings));
}

fn run_config_get(state: &AppState, args: ConfigGetArgs) -> Result<i32> {
    let Some(entry) = state.config.resolved().get_value_entry(&args.key) else {
        let mut messages = MessageBuffer::default();
        messages.error(format!("config key not found: {}", args.key));
        emit_messages(state, &messages);
        return Ok(1);
    };

    let row = config_entry_row(&args.key, entry, args.sources, args.raw);
    print!("{}", render_rows(&vec![row], &state.ui.render_settings));
    Ok(0)
}

fn run_config_explain(state: &AppState, args: ConfigExplainArgs) -> Result<i32> {
    let explain = explain_runtime_config(
        Some(state.config.resolved().active_profile().to_string()),
        state.config.resolved().terminal(),
        &args.key,
    )?;

    if explain.final_entry.is_none() && explain.layers.is_empty() {
        let suggestions = suggest_config_keys(state.config.resolved(), &args.key);
        let mut messages = MessageBuffer::default();
        messages.error(format!("config key not found: {}", args.key));
        if !suggestions.is_empty() {
            messages.info(format!("did you mean: {}", suggestions.join(", ")));
        }
        emit_messages(state, &messages);
        return Ok(1);
    }

    if matches!(state.ui.render_settings.format, OutputFormat::Json) {
        let payload = config_explain_json(&explain, args.show_secrets);
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).into_diagnostic()?
        );
        return Ok(0);
    }

    print!(
        "{}",
        render_config_explain_text(&explain, args.show_secrets)
    );
    Ok(0)
}

fn run_config_diagnostics(state: &AppState) {
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

    print!("{}", render_rows(&vec![row], &state.ui.render_settings));
}

fn explain_runtime_config(
    profile_override: Option<String>,
    terminal: Option<&str>,
    key: &str,
) -> Result<ConfigExplain> {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    defaults.set("theme.name", DEFAULT_THEME_NAME);
    defaults.set("user.name", default_user_name());
    defaults.set("domain", default_domain_name());
    defaults.set("repl.prompt", DEFAULT_REPL_PROMPT);
    defaults.set("repl.simple_prompt", false);
    defaults.set("repl.shell_indicator", "[{shell}]");
    defaults.set("repl.intro", true);
    defaults.set("ui.messages.boxed", true);
    defaults.set("color.prompt.text", "");
    defaults.set("color.prompt.command", "");
    let mut pipeline = LoaderPipeline::new(StaticLayerLoader::new(defaults))
        .with_env(EnvVarLoader::from_process_env());

    if let Some(path) = config_path() {
        pipeline = pipeline.with_file(TomlFileLoader::new(path).optional());
    }
    if let Some(path) = secrets_path() {
        let secret_chain = ChainedLoader::new(SecretsTomlLoader::new(path).optional())
            .with(EnvSecretsLoader::from_process_env());
        pipeline = pipeline.with_secrets(secret_chain);
    } else {
        pipeline = pipeline.with_secrets(ChainedLoader::new(EnvSecretsLoader::from_process_env()));
    }

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

    tracing::debug!(
        command = %command,
        args = ?args,
        "dispatching external command"
    );

    if is_help_passthrough(args) {
        let raw = plugin_manager
            .dispatch_passthrough(command, args)
            .map_err(enrich_dispatch_error)?;
        if !raw.stdout.is_empty() {
            print!("{}", raw.stdout);
        }
        if !raw.stderr.is_empty() {
            eprint!("{}", raw.stderr);
        }
        return Ok(raw.status_code);
    }

    let response = plugin_manager
        .dispatch(command, args)
        .map_err(enrich_dispatch_error)?;

    if !response.ok {
        let mut messages = MessageBuffer::default();
        if let Some(error) = response.error {
            messages.error(format!("{}: {}", error.code, error.message));
        } else {
            messages.error("plugin command failed");
        }
        emit_messages(state, &messages);
        return Ok(1);
    }

    let data = response.data;
    match data {
        serde_json::Value::Array(items)
            if items
                .iter()
                .all(|item| matches!(item, serde_json::Value::Object(_))) =>
        {
            let rows = items
                .into_iter()
                .filter_map(|item| item.as_object().cloned())
                .collect::<Vec<Row>>();
            print!("{}", render_rows(&rows, settings));
        }
        serde_json::Value::Object(map) => {
            let rows = vec![map];
            print!("{}", render_rows(&rows, settings));
        }
        scalar => {
            let rows = vec![{
                let mut row = Row::new();
                row.insert("value".to_string(), scalar);
                row
            }];
            print!("{}", render_rows(&rows, settings));
        }
    }

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

fn emit_messages(state: &AppState, messages: &MessageBuffer) {
    let resolved = state.ui.render_settings.resolve_render_settings();
    let boxed = state
        .config
        .resolved()
        .get_bool("ui.messages.boxed")
        .unwrap_or(true);
    let rendered = messages.render_grouped_styled(
        state.ui.message_verbosity,
        resolved.color,
        resolved.unicode,
        resolved.width,
        &resolved.theme_name,
        boxed,
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
) -> Result<ResolvedConfig> {
    tracing::debug!(
        profile_override = ?profile_override,
        terminal = ?terminal,
        "resolving runtime config"
    );
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    defaults.set("theme.name", DEFAULT_THEME_NAME);
    defaults.set("user.name", default_user_name());
    defaults.set("domain", default_domain_name());
    defaults.set("repl.prompt", DEFAULT_REPL_PROMPT);
    defaults.set("repl.simple_prompt", false);
    defaults.set("repl.shell_indicator", "[{shell}]");
    defaults.set("repl.intro", true);
    defaults.set("ui.messages.boxed", true);
    defaults.set("color.prompt.text", "");
    defaults.set("color.prompt.command", "");
    let mut pipeline = LoaderPipeline::new(StaticLayerLoader::new(defaults))
        .with_env(EnvVarLoader::from_process_env());

    if let Some(path) = config_path() {
        pipeline = pipeline.with_file(TomlFileLoader::new(path).optional());
    }
    if let Some(path) = secrets_path() {
        let secret_chain = ChainedLoader::new(SecretsTomlLoader::new(path).optional())
            .with(EnvSecretsLoader::from_process_env());
        pipeline = pipeline.with_secrets(secret_chain);
    } else {
        pipeline = pipeline.with_secrets(ChainedLoader::new(EnvSecretsLoader::from_process_env()));
    }

    let options = ResolveOptions {
        profile_override,
        terminal: terminal.map(|value| value.to_string()),
    };

    pipeline
        .resolve(options)
        .into_diagnostic()
        .wrap_err("config resolution failed")
}

fn config_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("OSP_CONFIG_FILE") {
        return Some(PathBuf::from(path));
    }

    let home = std::env::var("HOME").ok()?;
    let mut path = PathBuf::from(home);
    path.push(".config");
    path.push("osp");
    path.push("config.toml");
    Some(path)
}

fn secrets_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("OSP_SECRETS_FILE") {
        return Some(PathBuf::from(path));
    }

    let home = std::env::var("HOME").ok()?;
    let mut path = PathBuf::from(home);
    path.push(".config");
    path.push("osp");
    path.push("secrets.toml");
    Some(path)
}

fn default_user_name() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "anonymous".to_string())
}

fn default_domain_name() -> String {
    let host = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "localhost".to_string());
    host.split_once('.')
        .map(|(_, domain)| domain.to_string())
        .filter(|domain| !domain.trim().is_empty())
        .unwrap_or_else(|| "local".to_string())
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

        assert!(super::repl_command_spec(&plugins_list).supports_dsl);
        assert!(!super::repl_command_spec(&plugins_enable).supports_dsl);
        assert!(super::repl_command_spec(&theme_show).supports_dsl);
        assert!(!super::repl_command_spec(&theme_use).supports_dsl);
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
            theme_name: DEFAULT_THEME_NAME.to_string(),
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
