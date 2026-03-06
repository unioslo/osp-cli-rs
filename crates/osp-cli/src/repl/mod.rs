pub(crate) mod completion;
pub(crate) mod help;
pub(crate) mod history;

use anyhow::anyhow;
use miette::{Result, miette};
use osp_dsl::apply_output_pipeline;
use osp_repl::{
    DebugStep, ReplAppearance, ReplLineResult, ReplPrompt, ReplReloadKind, ReplRunResult,
    SharedHistory, run_repl,
};
use osp_ui::messages::{adjust_verbosity, render_section_divider_with_overrides};
use osp_ui::render_inline;
use osp_ui::render_output;
use osp_ui::style::{
    StyleToken, apply_style_spec, apply_style_with_theme, apply_style_with_theme_overrides,
};
use std::borrow::Cow;

use crate::cli::commands::{
    config as config_cmd, doctor as doctor_cmd, history as history_cmd, plugins as plugins_cmd,
    theme as theme_cmd,
};
use crate::cli::{
    Commands, ConfigArgs, ConfigCommands, DebugCompleteArgs, DoctorCommands, HistoryCommands,
    PluginsCommands, ReplArgs, ReplCommands, ThemeArgs, ThemeCommands, parse_repl_tokens,
};
use crate::pipeline::parse_command_text_with_aliases;
use crate::plugin_manager::CommandCatalogEntry;
use crate::rows::output::{output_to_rows, plugin_data_to_output_result};
use crate::state::AppState;

use crate::app;
use crate::app::{
    CMD_CONFIG, CMD_DOCTOR, CMD_HELP, CMD_HISTORY, CMD_LIST, CMD_PLUGINS, CMD_SHOW, CMD_THEME,
    CMD_USE, CliCommandResult, DEFAULT_REPL_PROMPT, REPL_SHELLABLE_COMMANDS, ReplCommandOutput,
    ReplCommandSpec, ReplDispatchOverrides,
};
use osp_completion::CompletionTree;

pub(crate) fn run_plugin_repl(state: &mut AppState) -> Result<i32> {
    let mut force_intro = state
        .config
        .resolved()
        .get_bool("repl.intro")
        .unwrap_or(true);
    let mut pending_reload = false;
    let mut pending_output = String::new();

    loop {
        if std::mem::take(&mut pending_reload) {
            let next = app::rebuild_repl_state(state)?;
            *state = next;
        }
        let catalog = app::authorized_command_catalog(state)?;
        let (words, completion_tree) = build_repl_completion_inputs(state, &catalog);
        let help_text = render_repl_command_overview(state, &catalog);

        if force_intro {
            print!("\x1b[2J\x1b[H");
            print!("{}", render_repl_intro(state));
            print!("{help_text}");
        }
        if !pending_output.is_empty() {
            print!("{pending_output}");
            pending_output.clear();
        }

        let prompt = build_repl_prompt(state);
        let appearance = build_repl_appearance(state);
        let history_config = history::build_history_config(state);
        print!("Preparing prompt...\r");

        match run_repl(
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
        .map_err(|err| miette!("{err:#}"))?
        {
            ReplRunResult::Exit(code) => return Ok(code),
            ReplRunResult::Restart { output, reload } => {
                pending_reload = true;
                force_intro = matches!(reload, ReplReloadKind::WithIntro);
                if force_intro {
                    pending_output = output;
                } else if !output.is_empty() {
                    print!("{output}");
                }
            }
        }
    }
}

pub(crate) fn run_repl_debug_command(state: &AppState, args: ReplArgs) -> Result<CliCommandResult> {
    match args.command {
        ReplCommands::DebugComplete(args) => run_repl_debug_complete(state, args),
    }
}

fn run_repl_debug_complete(state: &AppState, args: DebugCompleteArgs) -> Result<CliCommandResult> {
    let catalog = app::authorized_command_catalog(state)?;
    let (_words, completion_tree) = build_repl_completion_inputs(state, &catalog);
    let appearance = build_repl_appearance(state);
    let cursor = args.cursor.unwrap_or(args.line.len());

    let steps = args
        .steps
        .iter()
        .map(|raw| DebugStep::parse(raw).ok_or_else(|| miette!("Unknown debug step '{raw}'")))
        .collect::<Result<Vec<_>>>()?;

    let payload = if steps.is_empty() {
        let debug = osp_repl::debug_completion(
            &completion_tree,
            &args.line,
            cursor,
            args.width,
            args.height,
            args.menu_ansi,
            args.menu_unicode,
            Some(&appearance),
        );
        serde_json::to_string_pretty(&debug).map_err(|err| miette!("{err:#}"))?
    } else {
        let frames = osp_repl::debug_completion_steps(
            &completion_tree,
            &args.line,
            cursor,
            args.width,
            args.height,
            args.menu_ansi,
            args.menu_unicode,
            Some(&appearance),
            &steps,
        );
        serde_json::to_string_pretty(&frames).map_err(|err| miette!("{err:#}"))?
    };
    Ok(CliCommandResult::text(format!("{payload}\n")))
}

fn build_repl_completion_inputs(
    state: &AppState,
    catalog: &[CommandCatalogEntry],
) -> (Vec<String>, CompletionTree) {
    let history_enabled = history::repl_history_enabled(state.config.resolved());
    let mut words = completion::catalog_completion_words(catalog);
    if state.auth.is_builtin_visible(CMD_PLUGINS) {
        words.extend([CMD_PLUGINS.to_string(), CMD_LIST.to_string()]);
    }
    if state.auth.is_builtin_visible(CMD_DOCTOR) {
        words.push(CMD_DOCTOR.to_string());
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
            "doctor".to_string(),
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

    let tree = completion::build_repl_completion_tree(state, catalog, &words);
    (words, tree)
}

fn render_repl_intro(state: &AppState) -> String {
    let resolved = state.ui.render_settings.resolve_render_settings();
    let config = state.config.resolved();
    let theme = &resolved.theme;

    let user = config.get_string("user.name").unwrap_or("anonymous");
    let display_name = config
        .get_string("user.display_name")
        .or_else(|| config.get_string("user.full_name"))
        .unwrap_or(user);
    let theme_id = state.ui.render_settings.theme_name.clone();
    let version = env!("CARGO_PKG_VERSION");
    let theme_display = theme_display_name(&theme_id);

    let mut out = String::new();
    out.push('\n');
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
        &format!("  Welcome `{display_name}`!"),
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
        "  `F` key>3 *|* `P` col1 col2 *|* `S` sort_key *|* `G` group_by_k1 k2",
        resolved.color,
        theme,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out.push_str(&render_inline(
        "  *|* `A` metric() *|* `L` limit offset *|* `C` count",
        resolved.color,
        theme,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out.push_str(&render_inline(
        "  `K` key *|* `V` value *|* contains *|* !not *|* ?exist *|* !?not_exist *(= exact, == case-sens.)*",
        resolved.color,
        theme,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out.push_str(&render_inline(
        "  *Help:* `| H` *or* `| H <verb>` *e.g.* `| H F`",
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

    let usage_label = if resolved.color {
        apply_style_with_theme_overrides(
            "Usage:",
            StyleToken::MessageInfo,
            true,
            theme,
            &resolved.style_overrides,
        )
    } else {
        "Usage:".to_string()
    };
    out.push(' ');
    out.push_str(&usage_label);
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

    let exit_name = if resolved.color {
        apply_style_with_theme_overrides(
            "exit         ",
            StyleToken::Key,
            true,
            theme,
            &resolved.style_overrides,
        )
    } else {
        "exit         ".to_string()
    };
    out.push_str("  ");
    out.push_str(&exit_name);
    out.push_str("Exit application.\n");

    let help_name = if resolved.color {
        apply_style_with_theme_overrides(
            "help         ",
            StyleToken::Key,
            true,
            theme,
            &resolved.style_overrides,
        )
    } else {
        "help         ".to_string()
    };
    out.push_str("  ");
    out.push_str(&help_name);
    out.push_str("Show this command overview.\n");

    if state.auth.is_builtin_visible(CMD_PLUGINS) {
        out.push_str("  ");
        out.push_str(&style_command_name(&resolved, theme, "plugins      "));
        out.push_str("subcommands: list, commands, enable, disable, doctor\n");
    }
    if state.auth.is_builtin_visible(CMD_DOCTOR) {
        out.push_str("  ");
        out.push_str(&style_command_name(&resolved, theme, "doctor       "));
        out.push_str("subcommands: all, config, plugins, theme\n");
    }
    if state.auth.is_builtin_visible(CMD_CONFIG) {
        out.push_str("  ");
        out.push_str(&style_command_name(&resolved, theme, "config       "));
        out.push_str("subcommands: show, get, explain, set, doctor\n");
    }
    if state.auth.is_builtin_visible(CMD_THEME) {
        out.push_str("  ");
        out.push_str(&style_command_name(&resolved, theme, "theme        "));
        out.push_str("subcommands: list, show, use\n");
    }
    if history::repl_history_enabled(state.config.resolved())
        && state.auth.is_builtin_visible(CMD_HISTORY)
    {
        out.push_str("  ");
        out.push_str(&style_command_name(&resolved, theme, "history      "));
        out.push_str("subcommands: list, prune, clear\n");
    }

    for entry in catalog {
        let about = if entry.about.trim().is_empty() {
            "Plugin command".to_string()
        } else {
            entry.about.clone()
        };
        if entry.subcommands.is_empty() {
            let name = format!("{:<12}", entry.name);
            out.push_str("  ");
            out.push_str(&style_command_name(&resolved, theme, &name));
            out.push_str(&format!("{about}\n"));
        } else {
            let name = format!("{:<12}", entry.name);
            out.push_str("  ");
            out.push_str(&style_command_name(&resolved, theme, &name));
            out.push_str(&format!(
                "{} (subcommands: {})\n",
                about,
                entry.subcommands.join(", ")
            ));
        }
    }

    out.push_str(&render_section_divider_with_overrides(
        "",
        resolved.unicode,
        resolved.width,
        resolved.color,
        theme,
        StyleToken::PanelBorder,
        &resolved.style_overrides,
    ));
    out.push('\n');
    out
}

fn style_command_name(
    resolved: &osp_ui::ResolvedRenderSettings,
    theme: &osp_ui::theme::ThemeDefinition,
    name: &str,
) -> String {
    if resolved.color {
        apply_style_with_theme_overrides(
            name,
            StyleToken::Key,
            true,
            theme,
            &resolved.style_overrides,
        )
    } else {
        name.to_string()
    }
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

    let config_style = |key: &str| {
        config
            .get_string(key)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    };

    let completion_text_style = config_style("color.prompt.completion.text")
        .unwrap_or_else(|| theme.repl_completion_text_spec().to_string());
    let completion_background_style = config_style("color.prompt.completion.background")
        .unwrap_or_else(|| theme.repl_completion_background_spec().to_string());
    let completion_highlight_style = config_style("color.prompt.completion.highlight")
        .unwrap_or_else(|| theme.repl_completion_highlight_spec().to_string());
    let command_highlight_style =
        config_style("color.prompt.command").unwrap_or_else(|| theme.palette.success.to_string());

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
    let prompt_style = config.get_string("color.prompt.text");

    let user_text = style_prompt_fragment(
        prompt_style,
        user,
        StyleToken::PromptText,
        resolved.color,
        theme,
    );
    let domain_text = style_prompt_fragment(
        prompt_style,
        domain,
        StyleToken::PromptText,
        resolved.color,
        theme,
    );
    let profile_text = style_prompt_fragment(
        prompt_style,
        profile,
        StyleToken::PromptText,
        resolved.color,
        theme,
    );
    let indicator_text = style_prompt_fragment(
        prompt_style,
        &indicator,
        StyleToken::PromptText,
        resolved.color,
        theme,
    );

    let prompt = if simple {
        let suffix = style_prompt_fragment(
            prompt_style,
            "> ",
            StyleToken::PromptText,
            resolved.color,
            theme,
        );
        format!("{profile_text}{suffix}")
    } else {
        let template = config
            .get_string("repl.prompt")
            .unwrap_or(DEFAULT_REPL_PROMPT);
        render_prompt_template_styled(
            template,
            &user_text,
            &domain_text,
            &profile_text,
            &indicator_text,
            prompt_style,
            resolved.color,
            theme,
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

#[cfg(test)]
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

fn render_prompt_template_styled(
    template: &str,
    user: &str,
    domain: &str,
    profile: &str,
    indicator: &str,
    literal_style: Option<&str>,
    color: bool,
    theme: &osp_ui::theme::ThemeDefinition,
) -> String {
    let mut out = String::new();
    let mut cursor = 0;

    let style_literal = |text: &str| {
        style_prompt_fragment(literal_style, text, StyleToken::PromptText, color, theme)
    };

    while cursor < template.len() {
        let remainder = &template[cursor..];
        let Some(open) = remainder.find('{') else {
            out.push_str(&style_literal(remainder));
            break;
        };
        let open = cursor + open;
        if open > cursor {
            out.push_str(&style_literal(&template[cursor..open]));
        }
        let tail = &template[open..];
        if let Some((replacement, consumed)) =
            prompt_placeholder_replacement(tail, user, domain, profile, indicator)
        {
            out.push_str(replacement);
            cursor = open + consumed;
            continue;
        }
        out.push_str(&style_literal("{"));
        cursor = open + 1;
    }

    if !template.contains("{indicator}") && !indicator.trim().is_empty() {
        if !out.ends_with(' ') {
            out.push_str(&style_literal(" "));
        }
        out.push_str(indicator);
    }

    out
}

fn prompt_placeholder_replacement<'a>(
    tail: &'a str,
    user: &'a str,
    domain: &'a str,
    profile: &'a str,
    indicator: &'a str,
) -> Option<(&'a str, usize)> {
    if tail.starts_with("{user}") {
        return Some((user, "{user}".len()));
    }
    if tail.starts_with("{domain}") {
        return Some((domain, "{domain}".len()));
    }
    if tail.starts_with("{profile}") {
        return Some((profile, "{profile}".len()));
    }
    if tail.starts_with("{context}") {
        return Some((profile, "{context}".len()));
    }
    if tail.starts_with("{indicator}") {
        return Some((indicator, "{indicator}".len()));
    }
    None
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
) -> Result<ReplLineResult> {
    let parsed = parse_command_text_with_aliases(line, state.config.resolved())?;
    if parsed.tokens.is_empty() {
        return Ok(ReplLineResult::Continue(String::new()));
    }
    if let Some(help) = completion::maybe_render_dsl_help(state, &parsed.stages) {
        state.sync_history_shell_context();
        return Ok(ReplLineResult::Continue(help));
    }

    let tokens = parsed.tokens;
    let base_overrides = ReplDispatchOverrides {
        message_verbosity: state.ui.message_verbosity,
        debug_verbosity: state.ui.debug_verbosity,
    };
    if tokens.len() == 1 && (tokens[0] == "--help" || tokens[0] == "-h") {
        return Ok(ReplLineResult::Continue(repl_help_for_scope(
            state,
            base_overrides,
        )?));
    }

    let help_rewritten = rewrite_repl_help_tokens(&tokens);
    let tokens_for_parse = help_rewritten.unwrap_or(tokens);

    if tokens_for_parse.len() == 1 {
        match tokens_for_parse[0].as_str() {
            CMD_HELP => {
                return Ok(ReplLineResult::Continue(repl_help_for_scope(
                    state,
                    base_overrides,
                )?));
            }
            "exit" | "quit" => {
                if let Some(message) = leave_repl_shell(state) {
                    state.sync_history_shell_context();
                    return Ok(ReplLineResult::Continue(message));
                }
            }
            _ => {}
        }
    }

    if parsed.stages.is_empty() && should_enter_repl_shell(state, &tokens_for_parse) {
        let entered = enter_repl_shell(state, &tokens_for_parse[0], base_overrides)?;
        state.sync_history_shell_context();
        return Ok(ReplLineResult::Continue(entered));
    }

    let prefixed_tokens = apply_repl_shell_prefix(&state.session.shell_stack, &tokens_for_parse);
    let parsed_command = match parse_repl_tokens(&prefixed_tokens) {
        Ok(parsed) => parsed,
        Err(err) => {
            if err.kind() == clap::error::ErrorKind::DisplayHelp
                || err.kind() == clap::error::ErrorKind::DisplayVersion
            {
                let rendered = help::render_repl_help_with_chrome(state, &err.to_string());
                return Ok(ReplLineResult::Continue(rendered));
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
    let restart_repl = matches!(
        &command,
        Commands::Theme(ThemeArgs {
            command: ThemeCommands::Use(_)
        }) | Commands::Config(ConfigArgs {
            command: ConfigCommands::Set(_)
        })
    );
    let spec = repl_command_spec(&command);
    let show_intro_on_reload = theme_or_palette_change_requires_intro(&command);
    if !spec.supports_dsl && !parsed.stages.is_empty() {
        return Err(miette!(
            "`{}` does not support DSL pipeline stages",
            spec.name
        ));
    }
    if !parsed.stages.is_empty() {
        completion::validate_dsl_stages(&parsed.stages)?;
    }

    let rendered = match run_repl_command(state, command, overrides, history)? {
        ReplCommandOutput::Output {
            mut output,
            format_hint,
        } => {
            if !parsed.stages.is_empty() {
                output = apply_output_pipeline(output, &parsed.stages)
                    .map_err(|err| miette!("{err:#}"))?;
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
            rendered
        }
        ReplCommandOutput::Text(text) => text,
    };
    state.sync_history_shell_context();
    if restart_repl {
        Ok(ReplLineResult::Restart {
            output: rendered,
            reload: if show_intro_on_reload {
                ReplReloadKind::WithIntro
            } else {
                ReplReloadKind::Default
            },
        })
    } else {
        Ok(ReplLineResult::Continue(rendered))
    }
}

fn theme_or_palette_change_requires_intro(command: &Commands) -> bool {
    match command {
        Commands::Theme(ThemeArgs {
            command: ThemeCommands::Use(_),
        }) => true,
        Commands::Config(args) => match &args.command {
            ConfigCommands::Set(set) => {
                let key = set.key.trim().to_ascii_lowercase();
                key == "theme.name"
                    || key.starts_with("theme.")
                    || key.starts_with("color.")
                    || key.starts_with("palette.")
            }
            _ => false,
        },
        _ => false,
    }
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
        Commands::Doctor(args) => {
            app::ensure_builtin_visible(state, CMD_DOCTOR)?;
            with_repl_verbosity_overrides(state, overrides, |state| {
                doctor_cmd::run_doctor_repl_command(state, args, overrides.message_verbosity)
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
        Commands::Repl(_) => Err(miette!("`repl` debug commands are not available in REPL")),
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
                ConfigCommands::Show(_) | ConfigCommands::Get(_) | ConfigCommands::Doctor
            ),
        },
        Commands::Doctor(args) => ReplCommandSpec {
            name: Cow::Borrowed(CMD_DOCTOR),
            supports_dsl: matches!(
                args.command,
                Some(DoctorCommands::Config)
                    | Some(DoctorCommands::Plugins)
                    | Some(DoctorCommands::Theme)
            ),
        },
        Commands::History(args) => ReplCommandSpec {
            name: Cow::Borrowed(CMD_HISTORY),
            supports_dsl: matches!(args.command, HistoryCommands::List),
        },
        Commands::Repl(_) => ReplCommandSpec {
            name: Cow::Borrowed("repl"),
            supports_dsl: false,
        },
    }
}
