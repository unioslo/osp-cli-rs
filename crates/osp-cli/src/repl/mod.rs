pub(crate) mod completion;
pub(crate) mod help;
pub(crate) mod history;

use anyhow::anyhow;
use miette::{Result, miette};
use osp_dsl::apply_pipeline;
use osp_repl::{ReplAppearance, ReplPrompt, SharedHistory, run_repl};
use osp_ui::messages::{adjust_verbosity, render_section_divider_with_overrides};
use osp_ui::render_inline;
use osp_ui::render_output;
use osp_ui::style::{StyleToken, apply_style_spec, apply_style_with_theme};
use std::borrow::Cow;

use crate::cli::commands::{
    config as config_cmd, history as history_cmd, plugins as plugins_cmd, theme as theme_cmd,
};
use crate::cli::{
    Commands, ConfigCommands, HistoryCommands, PluginsCommands, ThemeCommands, parse_repl_tokens,
};
use crate::pipeline::parse_command_text_with_aliases;
use crate::plugin_manager::CommandCatalogEntry;
use crate::rows::output::{output_to_rows, plugin_data_to_output_result, rows_to_output_result};
use crate::state::AppState;

use crate::app;
use crate::app::{
    CMD_CONFIG, CMD_HELP, CMD_HISTORY, CMD_LIST, CMD_PLUGINS, CMD_SHOW, CMD_THEME, CMD_USE,
    DEFAULT_REPL_PROMPT, REPL_SHELLABLE_COMMANDS, ReplCommandOutput, ReplCommandSpec,
    ReplDispatchOverrides,
};

pub(crate) fn run_plugin_repl(state: &mut AppState) -> Result<i32> {
    let catalog = app::authorized_command_catalog(state)?;
    let history_enabled = history::repl_history_enabled(state.config.resolved());
    let mut words = completion::catalog_completion_words(&catalog);
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
    if history_enabled && state.auth.is_builtin_visible(CMD_HISTORY) {
        words.extend([
            CMD_HISTORY.to_string(),
            CMD_LIST.to_string(),
            "prune".to_string(),
            "clear".to_string(),
        ]);
    }
    for (alias_name, _) in completion::collect_alias_entries(state.config.resolved()) {
        words.push(alias_name);
    }
    words.extend(state.themes.ids());
    words.sort();
    words.dedup();

    let help_text = render_repl_command_overview(state, &catalog);
    if state
        .config
        .resolved()
        .get_bool("repl.intro")
        .unwrap_or(true)
    {
        // Match Python REPL startup behavior: clear screen before intro.
        print!("\x1b[2J\x1b[H");
        print!("{}", render_repl_intro(state));
        print!("{}", render_repl_command_overview(state, &catalog));
    }
    let prompt = build_repl_prompt(state);
    let appearance = build_repl_appearance(state);
    let history_config = history::build_history_config(state);
    let completion_tree = completion::build_repl_completion_tree(state, &catalog, &words);
    print!("Preparing prompt...\r");
    run_repl(
        prompt,
        words,
        Some(completion_tree),
        appearance,
        help_text,
        history_config,
        |line, history| {
            execute_repl_plugin_line(state, history, line).map_err(|err| anyhow!("{err:#}"))
        },
    )
    .map_err(|err| miette!("{err:#}"))
}

fn render_repl_intro(state: &AppState) -> String {
    let resolved = state.ui.render_settings.resolve_render_settings();
    let config = state.config.resolved();
    let theme = &resolved.theme;

    let user = config.get_string("user.name").unwrap_or("anonymous");
    let profile = config.active_profile();
    let theme_id = state.ui.render_settings.theme_name.clone();
    let version = env!("CARGO_PKG_VERSION");
    let theme_display = theme_display_name(&theme_id);

    let mut out = String::new();
    out.push_str(&render_section_divider_with_overrides(
        "OSP",
        resolved.unicode,
        resolved.width,
        resolved.color,
        theme,
        StyleToken::PanelBorder,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out.push_str(&render_inline(
        &format!("  Welcome `{user}`!"),
        resolved.color,
        theme,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out.push_str(&render_inline(
        &format!("  Logged in as: `{user}`"),
        resolved.color,
        theme,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out.push_str(&render_inline(
        &format!("  Theme: `{theme_display}`"),
        resolved.color,
        theme,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out.push_str(&render_inline(
        &format!("  Version: `{version}`"),
        resolved.color,
        theme,
        &resolved.style_overrides,
    ));
    out.push_str("\n\n");
    out.push_str(&render_section_divider_with_overrides(
        "Keybindings",
        resolved.unicode,
        resolved.width,
        resolved.color,
        theme,
        StyleToken::PanelBorder,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out.push_str(&render_inline(
        "  `Ctrl-D`    **exit**",
        resolved.color,
        theme,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out.push_str(&render_inline(
        "  `Ctrl-L`    **clear screen**",
        resolved.color,
        theme,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out.push_str(&render_inline(
        "  `Ctrl-R`    **reverse search history**",
        resolved.color,
        theme,
        &resolved.style_overrides,
    ));
    out.push_str("\n\n");
    out.push_str(&render_section_divider_with_overrides(
        "Pipes",
        resolved.unicode,
        resolved.width,
        resolved.color,
        theme,
        StyleToken::PanelBorder,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out.push_str(&render_inline(
        "    `F` key>3 *|* `P` col1 col2 *|* `S` sort_key *|* `G` group_by_k1 k2",
        resolved.color,
        theme,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out.push_str(&render_inline(
        "    *|* `A` metric() *|* `L` limit offset *|* `C` count",
        resolved.color,
        theme,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out.push_str(&render_inline(
        "    `K` key *|* `V` value *|* contains *|* !not *|* ?exist *|* !?not_exist *(= exact, == case-sens.)*",
        resolved.color,
        theme,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out.push_str(&render_inline(
        "    *Help:* `| H` *or* `| H <verb>` *e.g.* `| H F`",
        resolved.color,
        theme,
        &resolved.style_overrides,
    ));
    out.push_str("\n\n");
    out.push_str(&render_inline(
        &format!("Current profile: `{profile}`"),
        resolved.color,
        theme,
        &resolved.style_overrides,
    ));
    out.push_str("\n\n");
    out
}

fn render_repl_command_overview(state: &AppState, catalog: &[CommandCatalogEntry]) -> String {
    let resolved = state.ui.render_settings.resolve_render_settings();
    let theme = &resolved.theme;
    let mut out = String::new();

    out.push_str(&render_section_divider_with_overrides(
        "Usage",
        resolved.unicode,
        resolved.width,
        resolved.color,
        theme,
        StyleToken::PanelBorder,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out.push_str("  [OPTIONS] COMMAND [ARGS]...\n\n");

    out.push_str(&render_section_divider_with_overrides(
        "Commands",
        resolved.unicode,
        resolved.width,
        resolved.color,
        theme,
        StyleToken::PanelBorder,
        &resolved.style_overrides,
    ));
    out.push('\n');

    out.push_str("  exit         Exit application.\n");
    out.push_str("  help         Show this command overview.\n");

    if state.auth.is_builtin_visible(CMD_PLUGINS) {
        out.push_str("  plugins      subcommands: list, commands, enable, disable, doctor\n");
    }
    if state.auth.is_builtin_visible(CMD_CONFIG) {
        out.push_str("  config       subcommands: show, get, explain, set, diagnostics\n");
    }
    if state.auth.is_builtin_visible(CMD_THEME) {
        out.push_str("  theme        subcommands: list, show, use\n");
    }
    if history::repl_history_enabled(state.config.resolved())
        && state.auth.is_builtin_visible(CMD_HISTORY)
    {
        out.push_str("  history      subcommands: list, prune, clear\n");
    }

    for entry in catalog {
        let about = if entry.about.trim().is_empty() {
            "Plugin command".to_string()
        } else {
            entry.about.clone()
        };
        if entry.subcommands.is_empty() {
            out.push_str(&format!("  {:<12} {about}\n", entry.name));
        } else {
            out.push_str(&format!(
                "  {:<12} {} (subcommands: {})\n",
                entry.name,
                about,
                entry.subcommands.join(", ")
            ));
        }
    }

    out.push('\n');
    out.push_str(&render_section_divider_with_overrides(
        "",
        resolved.unicode,
        resolved.width,
        resolved.color,
        theme,
        StyleToken::MessageInfo,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out
}

pub(crate) fn theme_display_name(slug: &str) -> String {
    let normalized = slug
        .split(['-', '_'])
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            let mut chars = segment.chars();
            let Some(first) = chars.next() else {
                return String::new();
            };
            let mut out = first.to_uppercase().to_string();
            out.push_str(&chars.as_str().to_ascii_lowercase());
            out
        })
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.is_empty() {
        slug.to_string()
    } else {
        normalized
    }
}

fn build_repl_appearance(state: &AppState) -> ReplAppearance {
    let resolved = state.ui.render_settings.resolve_render_settings();
    if !resolved.color {
        return ReplAppearance::default();
    }
    let theme = &resolved.theme;
    let config = state.config.resolved();

    let completion_text_style = config
        .get_string("color.prompt.completion.text")
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| theme.repl_completion_text_spec().to_string());
    let completion_background_style = config
        .get_string("color.prompt.completion.background")
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| theme.repl_completion_background_spec().to_string());
    let completion_highlight_style = config
        .get_string("color.prompt.completion.highlight")
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| theme.repl_completion_highlight_spec().to_string());
    let command_highlight_style = config
        .get_string("color.prompt.command")
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| theme.palette.success.to_string());

    ReplAppearance {
        completion_text_style: Some(completion_text_style),
        completion_background_style: Some(completion_background_style),
        completion_highlight_style: Some(completion_highlight_style),
        command_highlight_style: Some(command_highlight_style),
    }
}

fn build_repl_prompt(state: &AppState) -> ReplPrompt {
    let resolved = state.ui.render_settings.resolve_render_settings();
    let config = state.config.resolved();
    let theme = &resolved.theme;
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
        theme,
    );
    let domain_text = style_prompt_fragment(
        config.get_string("color.prompt.text"),
        domain,
        StyleToken::PromptText,
        resolved.color,
        theme,
    );
    let profile_text = style_prompt_fragment(
        config.get_string("color.prompt.command"),
        profile,
        StyleToken::PromptCommand,
        resolved.color,
        theme,
    );
    let indicator_text = style_prompt_fragment(
        config.get_string("color.prompt.text"),
        &indicator,
        StyleToken::PromptText,
        resolved.color,
        theme,
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

pub(crate) fn render_prompt_template(
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
    theme: &osp_ui::theme::ThemeDefinition,
) -> String {
    match config_style.map(str::trim) {
        Some(spec) if !spec.is_empty() => apply_style_spec(value, spec, color),
        _ => apply_style_with_theme(value, fallback, color, theme),
    }
}

pub(crate) fn execute_repl_plugin_line(
    state: &mut AppState,
    history: &SharedHistory,
    line: &str,
) -> Result<String> {
    let parsed = parse_command_text_with_aliases(line, state.config.resolved())?;
    if parsed.tokens.is_empty() {
        return Ok(String::new());
    }
    if let Some(help) = completion::maybe_render_dsl_help(state, &parsed.stages) {
        state.sync_history_shell_context();
        return Ok(help);
    }

    let tokens = parsed.tokens;
    let base_overrides = ReplDispatchOverrides {
        message_verbosity: state.ui.message_verbosity,
        debug_verbosity: state.ui.debug_verbosity,
    };
    if tokens.len() == 1 && (tokens[0] == "--help" || tokens[0] == "-h") {
        return repl_help_for_scope(state, base_overrides);
    }

    let help_rewritten = rewrite_repl_help_tokens(&tokens);
    let tokens_for_parse = help_rewritten.unwrap_or(tokens);

    if tokens_for_parse.len() == 1 {
        match tokens_for_parse[0].as_str() {
            CMD_HELP => return repl_help_for_scope(state, base_overrides),
            "exit" | "quit" => {
                if let Some(message) = leave_repl_shell(state) {
                    state.sync_history_shell_context();
                    return Ok(message);
                }
            }
            _ => {}
        }
    }

    if parsed.stages.is_empty() && should_enter_repl_shell(state, &tokens_for_parse) {
        let entered = enter_repl_shell(state, &tokens_for_parse[0], base_overrides)?;
        state.sync_history_shell_context();
        return Ok(entered);
    }

    let prefixed_tokens = apply_repl_shell_prefix(&state.session.shell_stack, &tokens_for_parse);
    let parsed_command = match parse_repl_tokens(&prefixed_tokens) {
        Ok(parsed) => parsed,
        Err(err) => {
            if err.kind() == clap::error::ErrorKind::DisplayHelp
                || err.kind() == clap::error::ErrorKind::DisplayVersion
            {
                let rendered = help::render_repl_help_with_chrome(state, &err.to_string());
                return Ok(rendered);
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
    if !parsed.stages.is_empty() {
        completion::validate_dsl_stages(&parsed.stages)?;
    }

    let result = match run_repl_command(state, command, overrides, history)? {
        ReplCommandOutput::Output {
            mut output,
            format_hint,
        } => {
            if !parsed.stages.is_empty() {
                let rows = output_to_rows(&output);
                let rows =
                    apply_pipeline(rows, &parsed.stages).map_err(|err| miette!("{err:#}"))?;
                output = rows_to_output_result(rows);
            }

            let render_settings = app::resolve_effective_render_settings(
                &state.ui.render_settings,
                if parsed.stages.is_empty() {
                    format_hint
                } else {
                    None
                },
            );
            let rendered = render_output(&output, &render_settings);
            state.record_repl_rows(line, output_to_rows(&output));
            app::maybe_copy_output(state, &output);
            Ok(rendered)
        }
        ReplCommandOutput::Text(text) => Ok(text),
    };
    state.sync_history_shell_context();
    result
}

pub(crate) fn rewrite_repl_help_tokens(tokens: &[String]) -> Option<Vec<String>> {
    if tokens.first().map(String::as_str) != Some(CMD_HELP) {
        return None;
    }
    if tokens.len() == 1 {
        return None;
    }
    let mut rewritten = tokens[1..].to_vec();
    if !rewritten.iter().any(|arg| arg == "--help" || arg == "-h") {
        rewritten.push("--help".to_string());
    }
    Some(rewritten)
}

pub(crate) fn should_enter_repl_shell(state: &AppState, tokens: &[String]) -> bool {
    if tokens.len() != 1 {
        return false;
    }
    if !is_repl_shellable_command(&tokens[0]) {
        return false;
    }
    !state
        .session
        .shell_stack
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(tokens[0].as_str()))
}

pub(crate) fn is_repl_shellable_command(command: &str) -> bool {
    REPL_SHELLABLE_COMMANDS
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(command.trim()))
}

pub(crate) fn apply_repl_shell_prefix(shell_stack: &[String], tokens: &[String]) -> Vec<String> {
    if shell_stack.is_empty() || tokens.starts_with(shell_stack) {
        return tokens.to_vec();
    }
    let mut full = shell_stack.to_vec();
    full.extend_from_slice(tokens);
    full
}

pub(crate) fn leave_repl_shell(state: &mut AppState) -> Option<String> {
    let command = state.session.shell_stack.pop()?;
    Some(if state.session.shell_stack.is_empty() {
        format!("Leaving {command} shell. Back at root.\n")
    } else {
        format!("Leaving {command} shell.\n")
    })
}

fn enter_repl_shell(
    state: &mut AppState,
    command: &str,
    overrides: ReplDispatchOverrides,
) -> Result<String> {
    app::ensure_plugin_visible(state, command)?;
    let catalog = app::authorized_command_catalog(state)?;
    if !catalog.iter().any(|entry| entry.name == command) {
        return Err(miette!("no plugin provides command: {command}"));
    }

    state.session.shell_stack.push(command.to_string());
    let mut out = format!("Entering {command} shell. Type `exit` to leave.\n");
    if let Ok(help) = repl_help_for_scope(state, overrides) {
        out.push_str(&help);
    }
    Ok(out)
}

fn repl_help_for_scope(state: &AppState, overrides: ReplDispatchOverrides) -> Result<String> {
    if state.session.shell_stack.is_empty() {
        let catalog = app::authorized_command_catalog(state)?;
        return Ok(render_repl_command_overview(state, &catalog));
    }

    let mut tokens = state.session.shell_stack.clone();
    tokens.push("--help".to_string());
    match run_repl_external_command(state, tokens, overrides)? {
        ReplCommandOutput::Text(text) => Ok(text),
        ReplCommandOutput::Output {
            output,
            format_hint,
        } => {
            let render_settings =
                app::resolve_effective_render_settings(&state.ui.render_settings, format_hint);
            Ok(render_output(&output, &render_settings))
        }
    }
}

fn run_repl_command(
    state: &mut AppState,
    command: Commands,
    overrides: ReplDispatchOverrides,
    history: &SharedHistory,
) -> Result<ReplCommandOutput> {
    match command {
        Commands::Plugins(args) => {
            app::ensure_builtin_visible(state, CMD_PLUGINS)?;
            with_repl_verbosity_overrides(state, overrides, |state| {
                plugins_cmd::run_plugins_repl_command(state, args, overrides.message_verbosity)
            })
        }
        Commands::Theme(args) => {
            app::ensure_builtin_visible(state, CMD_THEME)?;
            with_repl_verbosity_overrides(state, overrides, |state| {
                theme_cmd::run_theme_repl_command(state, args)
            })
        }
        Commands::Config(args) => {
            app::ensure_builtin_visible(state, CMD_CONFIG)?;
            with_repl_verbosity_overrides(state, overrides, |state| {
                config_cmd::run_config_repl_command(state, args)
            })
        }
        Commands::History(args) => {
            app::ensure_builtin_visible(state, CMD_HISTORY)?;
            with_repl_verbosity_overrides(state, overrides, |state| {
                history_cmd::run_history_repl_command(state, args, history)
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
    app::ensure_plugin_visible(state, command)?;
    if app::is_help_passthrough(args) {
        let dispatch_context = app::plugin_dispatch_context(state, Some(overrides));
        let raw = state
            .clients
            .plugins
            .dispatch_passthrough(command, args, &dispatch_context)
            .map_err(app::enrich_dispatch_error)?;
        if raw.status_code != 0 {
            return Err(miette!(
                "plugin help command exited with status {}",
                raw.status_code
            ));
        }
        let mut out = String::new();
        if !raw.stdout.is_empty() {
            out.push_str(&help::render_repl_help_with_chrome(state, &raw.stdout));
        }
        if !raw.stderr.is_empty() {
            out.push_str(&raw.stderr);
        }
        return Ok(ReplCommandOutput::Text(out));
    }

    let dispatch_context = app::plugin_dispatch_context(state, Some(overrides));
    let response = state
        .clients
        .plugins
        .dispatch(command, args, &dispatch_context)
        .map_err(app::enrich_dispatch_error)?;
    let mut messages = app::plugin_response_messages(&response);
    if !response.ok {
        let report = if let Some(error) = response.error {
            messages.error(format!("{}: {}", error.code, error.message));
            miette!("{}: {}", error.code, error.message)
        } else {
            messages.error("plugin command failed");
            miette!("plugin command failed")
        };
        app::emit_messages_with_verbosity(state, &messages, overrides.message_verbosity);
        return Err(report);
    }
    if !messages.is_empty() {
        app::emit_messages_with_verbosity(state, &messages, overrides.message_verbosity);
    }
    Ok(ReplCommandOutput::Output {
        output: plugin_data_to_output_result(response.data, Some(&response.meta)),
        format_hint: app::parse_output_format_hint(response.meta.format_hint.as_deref()),
    })
}

pub(crate) fn repl_command_spec(command: &Commands) -> ReplCommandSpec {
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
        Commands::History(args) => ReplCommandSpec {
            name: Cow::Borrowed(CMD_HISTORY),
            supports_dsl: matches!(args.command, HistoryCommands::List),
        },
    }
}
