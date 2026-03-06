use miette::{Result, miette};
use osp_dsl::apply_output_pipeline;
use osp_repl::{ReplLineResult, ReplReloadKind, SharedHistory};
use osp_ui::messages::adjust_verbosity;
use osp_ui::render_output;
use std::borrow::Cow;

use crate::app;
use crate::app::{
    CMD_CONFIG, CMD_DOCTOR, CMD_HELP, CMD_HISTORY, CMD_PLUGINS, CMD_THEME, REPL_SHELLABLE_COMMANDS,
    ReplCommandOutput, ReplCommandSpec, ReplDispatchOverrides,
};
use crate::cli::commands::{
    config as config_cmd, doctor as doctor_cmd, history as history_cmd, plugins as plugins_cmd,
    theme as theme_cmd,
};
use crate::cli::{
    Commands, ConfigArgs, ConfigCommands, DoctorCommands, HistoryCommands, PluginsCommands,
    ThemeArgs, ThemeCommands, parse_repl_tokens,
};
use crate::rows::output::{output_to_rows, plugin_data_to_output_result};
use crate::state::AppState;

use super::{completion, help, presentation, surface};

pub(crate) fn execute_repl_plugin_line(
    state: &mut AppState,
    history: &SharedHistory,
    line: &str,
) -> Result<ReplLineResult> {
    let parsed = crate::pipeline::parse_command_text_with_aliases(line, state.config.resolved())?;
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

    let prefixed_tokens = apply_repl_shell_prefix(&state.session.scope, &tokens_for_parse);
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
    !state.session.scope.contains_command(tokens[0].as_str())
}

pub(crate) fn is_repl_shellable_command(command: &str) -> bool {
    REPL_SHELLABLE_COMMANDS
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(command.trim()))
}

pub(crate) fn apply_repl_shell_prefix(
    scope: &crate::state::ReplScopeStack,
    tokens: &[String],
) -> Vec<String> {
    scope.prefixed_tokens(tokens)
}

pub(crate) fn leave_repl_shell(state: &mut AppState) -> Option<String> {
    let frame = state.session.scope.leave()?;
    Some(if state.session.scope.is_root() {
        format!("Leaving {} shell. Back at root.\n", frame.command())
    } else {
        format!("Leaving {} shell.\n", frame.command())
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

    state.session.scope.enter(command.to_string());
    let mut out = format!("Entering {command} shell. Type `exit` to leave.\n");
    if let Ok(help) = repl_help_for_scope(state, overrides) {
        out.push_str(&help);
    }
    Ok(out)
}

fn repl_help_for_scope(state: &AppState, overrides: ReplDispatchOverrides) -> Result<String> {
    if state.session.scope.is_root() {
        let catalog = app::authorized_command_catalog(state)?;
        let surface = surface::build_repl_surface(state, &catalog);
        return Ok(presentation::render_repl_command_overview(state, &surface));
    }

    let tokens = state.session.scope.help_tokens();
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
