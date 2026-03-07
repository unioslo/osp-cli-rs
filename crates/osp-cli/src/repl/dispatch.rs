use miette::{Result, miette};
use osp_repl::{ReplLineResult, ReplReloadKind, SharedHistory, expand_history};
use osp_ui::messages::adjust_verbosity;
use osp_ui::render_output;
use std::borrow::Cow;

use crate::app;
use crate::app::{
    BuiltinCommandTransport, CMD_CONFIG, CMD_DOCTOR, CMD_HELP, CMD_HISTORY, CMD_PLUGINS, CMD_THEME,
    ReplCommandSpec, ReplDispatchOverrides,
};
use crate::cli::{
    Commands, ConfigArgs, ConfigCommands, DoctorCommands, HistoryCommands, PluginsCommands,
    ThemeArgs, ThemeCommands, parse_repl_tokens,
};
use crate::rows::output::output_to_rows;
use crate::state::{AppClients, AppRuntime, AppSession};

use super::{ReplViewContext, completion, help, input, presentation, surface};

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReplBuiltin {
    Help,
    Exit,
    Bang(BangCommand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BangCommand {
    Last,
    Relative(usize),
    Absolute(usize),
    Prefix(String),
    Contains(String),
}

pub(crate) fn execute_repl_plugin_line(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    history: &SharedHistory,
    line: &str,
) -> Result<ReplLineResult> {
    match execute_repl_plugin_line_inner(runtime, session, clients, history, line) {
        Ok(result) => Ok(result),
        Err(err) => {
            if !is_repl_bang_request(line) {
                let summary = err.to_string();
                let detail = format!("{err:#}");
                session.record_failure(line, summary, detail);
            }
            Err(err)
        }
    }
}

fn execute_repl_plugin_line_inner(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    history: &SharedHistory,
    line: &str,
) -> Result<ReplLineResult> {
    let raw = line.trim();
    if let Some(result) = maybe_execute_repl_builtin(runtime, session, clients, history, raw)? {
        return Ok(result);
    }

    let parsed = input::ReplParsedLine::parse(line, runtime.config.resolved())?;
    if parsed.is_empty() {
        return Ok(ReplLineResult::Continue(String::new()));
    }
    if let Some(help) = completion::maybe_render_dsl_help(
        ReplViewContext::from_parts(runtime, session),
        &parsed.stages,
    ) {
        session.sync_history_shell_context();
        return Ok(ReplLineResult::Continue(help));
    }

    let base_overrides = base_repl_overrides(runtime);
    if let Some(result) =
        maybe_handle_repl_shortcuts(runtime, session, clients, &parsed, base_overrides)?
    {
        return Ok(result);
    }

    let invocation = match parse_repl_invocation(runtime, session, &parsed)? {
        ParsedReplDispatch::Help(rendered) => return Ok(ReplLineResult::Continue(rendered)),
        ParsedReplDispatch::Invocation(invocation) => invocation,
    };
    let output = run_repl_command(
        runtime,
        session,
        clients,
        invocation.command,
        invocation.overrides,
        history,
    )?;
    let rendered = render_repl_command_output(
        runtime,
        session,
        line,
        &invocation.stages,
        output,
        invocation.overrides.message_verbosity,
    )?;
    Ok(finalize_repl_command(
        session,
        rendered,
        invocation.restart_repl,
        invocation.show_intro_on_reload,
    ))
}

fn base_repl_overrides(runtime: &AppRuntime) -> ReplDispatchOverrides {
    ReplDispatchOverrides {
        message_verbosity: runtime.ui.message_verbosity,
        debug_verbosity: runtime.ui.debug_verbosity,
    }
}

fn maybe_handle_repl_shortcuts(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    parsed: &input::ReplParsedLine,
    base_overrides: ReplDispatchOverrides,
) -> Result<Option<ReplLineResult>> {
    if parsed.requests_repl_help() {
        return repl_help_result(runtime, session, clients, base_overrides).map(Some);
    }

    if let Some(result) =
        maybe_handle_single_token_shortcut(runtime, session, clients, parsed, base_overrides)?
    {
        return Ok(Some(result));
    }

    if let Some(command) = parsed.shell_entry_command(&session.scope) {
        let entered = enter_repl_shell(runtime, session, clients, command, base_overrides)?;
        session.sync_history_shell_context();
        return Ok(Some(ReplLineResult::Continue(entered)));
    }

    Ok(None)
}

fn maybe_handle_single_token_shortcut(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    parsed: &input::ReplParsedLine,
    base_overrides: ReplDispatchOverrides,
) -> Result<Option<ReplLineResult>> {
    if parsed.dispatch_tokens.len() != 1 {
        return Ok(None);
    }

    match parsed.dispatch_tokens[0].as_str() {
        CMD_HELP => repl_help_result(runtime, session, clients, base_overrides).map(Some),
        "exit" | "quit" => Ok(handle_repl_exit_request(session)),
        _ => Ok(None),
    }
}

fn repl_help_result(
    runtime: &AppRuntime,
    session: &AppSession,
    clients: &AppClients,
    overrides: ReplDispatchOverrides,
) -> Result<ReplLineResult> {
    Ok(ReplLineResult::Continue(repl_help_for_scope(
        runtime, session, clients, overrides,
    )?))
}

fn handle_repl_exit_request(session: &mut AppSession) -> Option<ReplLineResult> {
    if session.scope.is_root() {
        session.sync_history_shell_context();
        return Some(ReplLineResult::Exit(0));
    }

    let message = leave_repl_shell(session)?;
    session.sync_history_shell_context();
    Some(ReplLineResult::Continue(message))
}

struct ParsedReplInvocation {
    command: Commands,
    overrides: ReplDispatchOverrides,
    stages: Vec<String>,
    restart_repl: bool,
    show_intro_on_reload: bool,
}

enum ParsedReplDispatch {
    Help(String),
    Invocation(ParsedReplInvocation),
}

fn parse_repl_invocation(
    runtime: &AppRuntime,
    session: &AppSession,
    parsed: &input::ReplParsedLine,
) -> Result<ParsedReplDispatch> {
    let prefixed_tokens = parsed.prefixed_tokens(&session.scope);
    let parsed_command = match parse_repl_tokens(&prefixed_tokens) {
        Ok(parsed) => parsed,
        Err(err) => {
            if err.kind() == clap::error::ErrorKind::DisplayHelp
                || err.kind() == clap::error::ErrorKind::DisplayVersion
            {
                let rendered = help::render_repl_help_with_chrome(
                    ReplViewContext::from_parts(runtime, session),
                    &err.to_string(),
                );
                return Ok(ParsedReplDispatch::Help(rendered));
            }
            return Err(miette!(err.to_string()));
        }
    };
    let command = parsed_command
        .command
        .ok_or_else(|| miette!("missing command"))?;
    let spec = repl_command_spec(&command);
    app::ensure_command_supports_dsl(&spec, &parsed.stages)?;
    if !parsed.stages.is_empty() {
        completion::validate_dsl_stages(&parsed.stages)?;
    }

    Ok(ParsedReplDispatch::Invocation(ParsedReplInvocation {
        overrides: ReplDispatchOverrides {
            message_verbosity: adjust_verbosity(
                runtime.ui.message_verbosity,
                parsed_command.verbose,
                parsed_command.quiet,
            ),
            debug_verbosity: if parsed_command.debug > 0 {
                parsed_command.debug.min(3)
            } else {
                runtime.ui.debug_verbosity
            },
        },
        restart_repl: command_restarts_repl(&command),
        show_intro_on_reload: theme_or_palette_change_requires_intro(&command),
        command,
        stages: parsed.stages.clone(),
    }))
}

fn command_restarts_repl(command: &Commands) -> bool {
    matches!(
        command,
        Commands::Theme(ThemeArgs {
            command: ThemeCommands::Use(_)
        })
    ) || matches!(
        command,
        Commands::Config(ConfigArgs {
            command: ConfigCommands::Set(set),
        }) if !set.dry_run
    ) || matches!(
        command,
        Commands::Config(ConfigArgs {
            command: ConfigCommands::Unset(unset),
        }) if !unset.dry_run
    )
}

fn render_repl_command_output(
    runtime: &AppRuntime,
    session: &mut AppSession,
    line: &str,
    stages: &[String],
    result: crate::app::CliCommandResult,
    verbosity: osp_ui::messages::MessageLevel,
) -> Result<String> {
    if !result.messages.is_empty() {
        app::emit_messages_for_ui(
            runtime.config.resolved(),
            &runtime.ui,
            &result.messages,
            verbosity,
        );
    }

    match result.output {
        Some(crate::app::ReplCommandOutput::Output {
            output,
            format_hint,
        }) => {
            let (output, format_hint) = app::apply_output_stages(output, stages, format_hint)
                .map_err(|err| miette!("{err:#}"))?;

            let render_settings =
                app::resolve_effective_render_settings(&runtime.ui.render_settings, format_hint);
            let rendered = render_output(&output, &render_settings);
            session.record_result(line, output_to_rows(&output));
            app::maybe_copy_output_with_runtime(
                &app::CommandRenderRuntime::new(runtime.config.resolved(), &runtime.ui),
                &output,
            );
            Ok(rendered)
        }
        Some(crate::app::ReplCommandOutput::Text(text)) => Ok(text),
        None => Ok(String::new()),
    }
}

fn finalize_repl_command(
    session: &AppSession,
    rendered: String,
    restart_repl: bool,
    show_intro_on_reload: bool,
) -> ReplLineResult {
    session.sync_history_shell_context();
    if restart_repl {
        ReplLineResult::Restart {
            output: rendered,
            reload: if show_intro_on_reload {
                ReplReloadKind::WithIntro
            } else {
                ReplReloadKind::Default
            },
        }
    } else {
        ReplLineResult::Continue(rendered)
    }
}

fn maybe_execute_repl_builtin(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    history: &SharedHistory,
    raw: &str,
) -> Result<Option<ReplLineResult>> {
    let Some(builtin) = parse_repl_builtin(raw)? else {
        return Ok(None);
    };

    match builtin {
        ReplBuiltin::Help => Ok(Some(ReplLineResult::Continue(repl_help_for_scope(
            runtime,
            session,
            clients,
            ReplDispatchOverrides {
                message_verbosity: runtime.ui.message_verbosity,
                debug_verbosity: runtime.ui.debug_verbosity,
            },
        )?))),
        ReplBuiltin::Exit => {
            if session.scope.is_root() {
                session.sync_history_shell_context();
                Ok(Some(ReplLineResult::Exit(0)))
            } else if let Some(message) = leave_repl_shell(session) {
                session.sync_history_shell_context();
                Ok(Some(ReplLineResult::Continue(message)))
            } else {
                Ok(Some(ReplLineResult::Exit(0)))
            }
        }
        ReplBuiltin::Bang(command) => {
            execute_bang_command(session, history, raw, command).map(Some)
        }
    }
}

fn parse_repl_builtin(raw: &str) -> Result<Option<ReplBuiltin>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    if raw == CMD_HELP || raw == "--help" || raw == "-h" {
        return Ok(Some(ReplBuiltin::Help));
    }
    if raw == "exit" || raw == "quit" {
        return Ok(Some(ReplBuiltin::Exit));
    }
    if let Some(command) = parse_bang_command(raw)? {
        return Ok(Some(ReplBuiltin::Bang(command)));
    }
    Ok(None)
}

fn parse_bang_command(raw: &str) -> Result<Option<BangCommand>> {
    let raw = raw.trim();
    if !raw.starts_with('!') {
        return Ok(None);
    }
    if raw == "!" {
        return Ok(Some(BangCommand::Prefix(String::new())));
    }
    if raw == "!!" {
        return Ok(Some(BangCommand::Last));
    }
    if let Some(rest) = raw.strip_prefix("!?") {
        let term = rest.trim();
        if term.is_empty() {
            return Err(miette!("`!?` expects search text"));
        }
        return Ok(Some(BangCommand::Contains(term.to_string())));
    }
    if let Some(rest) = raw.strip_prefix("!-") {
        let offset = rest
            .trim()
            .parse::<usize>()
            .map_err(|_| miette!("`!-N` expects a positive integer"))?;
        if offset == 0 {
            return Err(miette!("`!-N` expects N >= 1"));
        }
        return Ok(Some(BangCommand::Relative(offset)));
    }
    let rest = raw.trim_start_matches('!').trim();
    if rest.is_empty() {
        return Ok(Some(BangCommand::Prefix(String::new())));
    }
    if rest.chars().all(|ch| ch.is_ascii_digit()) {
        let id = rest
            .parse::<usize>()
            .map_err(|_| miette!("`!N` expects a positive integer"))?;
        if id == 0 {
            return Err(miette!("`!N` expects N >= 1"));
        }
        return Ok(Some(BangCommand::Absolute(id)));
    }
    Ok(Some(BangCommand::Prefix(rest.to_string())))
}

fn execute_bang_command(
    session: &mut AppSession,
    history: &SharedHistory,
    raw: &str,
    command: BangCommand,
) -> Result<ReplLineResult> {
    let scope = current_history_scope(session);
    let recent = history.recent_commands_for(scope.as_deref());

    let expanded = match command {
        BangCommand::Last => expand_history("!!", &recent, scope.as_deref(), true),
        BangCommand::Relative(offset) => {
            expand_history(&format!("!-{offset}"), &recent, scope.as_deref(), true)
        }
        BangCommand::Absolute(id) => {
            expand_history(&format!("!{id}"), &recent, scope.as_deref(), true)
        }
        BangCommand::Prefix(prefix) => {
            if prefix.is_empty() {
                return Ok(ReplLineResult::Continue(render_bang_help()));
            }
            expand_history(&format!("!{prefix}"), &recent, scope.as_deref(), true)
        }
        BangCommand::Contains(term) => {
            let mut found = None;
            for full in recent.iter().rev() {
                let visible = strip_history_scope(full, scope.as_deref());
                if visible.contains(&term) {
                    found = Some(visible);
                    break;
                }
            }
            found
        }
    };

    let Some(expanded) = expanded else {
        return Ok(ReplLineResult::Continue(format!(
            "No history match for: {raw}\n"
        )));
    };

    Ok(ReplLineResult::ReplaceInput(expanded))
}

fn current_history_scope(session: &AppSession) -> Option<String> {
    let prefix = session.scope.history_prefix();
    if prefix.is_empty() {
        None
    } else {
        Some(prefix)
    }
}

fn strip_history_scope(command: &str, scope: Option<&str>) -> String {
    let trimmed = command.trim();
    match scope {
        Some(prefix) => trimmed
            .strip_prefix(prefix)
            .map(|rest| rest.trim_start().to_string())
            .unwrap_or_else(|| trimmed.to_string()),
        None => trimmed.to_string(),
    }
}

fn render_bang_help() -> String {
    let mut out = String::new();
    out.push_str("Bang history shortcuts:\n");
    out.push_str("  !!       last visible command\n");
    out.push_str("  !-N      Nth previous visible command\n");
    out.push_str("  !N       visible history entry by id\n");
    out.push_str("  !prefix  latest visible command starting with prefix\n");
    out.push_str("  !?text   latest visible command containing text\n");
    out
}

fn is_repl_bang_request(raw: &str) -> bool {
    raw.trim_start().starts_with('!')
}

fn theme_or_palette_change_requires_intro(command: &Commands) -> bool {
    match command {
        Commands::Theme(ThemeArgs {
            command: ThemeCommands::Use(_),
        }) => true,
        Commands::Config(args) => match &args.command {
            ConfigCommands::Set(set) => !set.dry_run && config_key_change_requires_intro(&set.key),
            ConfigCommands::Unset(unset) => {
                !unset.dry_run && config_key_change_requires_intro(&unset.key)
            }
            _ => false,
        },
        _ => false,
    }
}

fn config_key_change_requires_intro(key: &str) -> bool {
    let key = key.trim().to_ascii_lowercase();
    key == "theme.name"
        || key.starts_with("theme.")
        || key.starts_with("color.")
        || key.starts_with("palette.")
}

#[cfg(test)]
pub(crate) fn apply_repl_shell_prefix(
    scope: &crate::state::ReplScopeStack,
    tokens: &[String],
) -> Vec<String> {
    scope.prefixed_tokens(tokens)
}

pub(crate) fn leave_repl_shell(session: &mut AppSession) -> Option<String> {
    let frame = session.scope.leave()?;
    Some(if session.scope.is_root() {
        format!("Leaving {} shell. Back at root.\n", frame.command())
    } else {
        format!("Leaving {} shell.\n", frame.command())
    })
}

fn enter_repl_shell(
    runtime: &AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    command: &str,
    overrides: ReplDispatchOverrides,
) -> Result<String> {
    app::ensure_plugin_visible_for(&runtime.auth, command)?;
    let catalog = app::authorized_command_catalog_for(&runtime.auth, &clients.plugins)?;
    if !catalog.iter().any(|entry| entry.name == command) {
        return Err(miette!("no plugin provides command: {command}"));
    }

    session.scope.enter(command.to_string());
    let mut out = format!("Entering {command} shell. Type `exit` to leave.\n");
    if let Ok(help) = repl_help_for_scope(runtime, session, clients, overrides) {
        out.push_str(&help);
    }
    Ok(out)
}

fn repl_help_for_scope(
    runtime: &AppRuntime,
    session: &AppSession,
    clients: &AppClients,
    overrides: ReplDispatchOverrides,
) -> Result<String> {
    if session.scope.is_root() {
        let catalog = app::authorized_command_catalog_for(&runtime.auth, &clients.plugins)?;
        let view = ReplViewContext::from_parts(runtime, session);
        let surface = surface::build_repl_surface(view, &catalog);
        return Ok(presentation::render_repl_command_overview(view, &surface));
    }

    let tokens = session.scope.help_tokens();
    match run_repl_external_command(runtime, clients, session, tokens, overrides)?.output {
        Some(crate::app::ReplCommandOutput::Text(text)) => Ok(text),
        Some(crate::app::ReplCommandOutput::Output {
            output,
            format_hint,
        }) => {
            let render_settings =
                app::resolve_effective_render_settings(&runtime.ui.render_settings, format_hint);
            Ok(render_output(&output, &render_settings))
        }
        None => Ok(String::new()),
    }
}

fn run_repl_command(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    command: Commands,
    overrides: ReplDispatchOverrides,
    history: &SharedHistory,
) -> Result<crate::app::CliCommandResult> {
    match command {
        Commands::External(tokens) => {
            run_repl_external_command(runtime, clients, session, tokens, overrides)
        }
        builtin => app::dispatch_builtin_command_parts(
            runtime,
            session,
            clients,
            BuiltinCommandTransport::Repl { history },
            Some(overrides),
            builtin,
        )
        .and_then(|result| result.ok_or_else(|| miette!("expected builtin command"))),
    }
}

fn run_repl_external_command(
    runtime: &AppRuntime,
    clients: &AppClients,
    session: &AppSession,
    tokens: Vec<String>,
    overrides: ReplDispatchOverrides,
) -> Result<crate::app::CliCommandResult> {
    let (command, args) = tokens
        .split_first()
        .ok_or_else(|| miette!("missing command"))?;
    app::ensure_plugin_visible_for(&runtime.auth, command)?;
    emit_repl_command_conflict_warning(runtime, clients, command, overrides);
    if app::is_help_passthrough(args) {
        let dispatch_context =
            app::plugin_dispatch_context_for_runtime(runtime, clients, Some(overrides));
        let raw = clients
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
            out.push_str(&help::render_repl_help_with_chrome(
                ReplViewContext::from_parts(runtime, session),
                &raw.stdout,
            ));
        }
        if !raw.stderr.is_empty() {
            out.push_str(&raw.stderr);
        }
        return Ok(crate::app::CliCommandResult::text(out));
    }

    let dispatch_context =
        app::plugin_dispatch_context_for_runtime(runtime, clients, Some(overrides));
    let response = clients
        .plugins
        .dispatch(command, args, &dispatch_context)
        .map_err(app::enrich_dispatch_error)?;
    match app::prepare_plugin_response(response, &[]) {
        Ok(app::PreparedPluginResponse::Failure(failure)) => Err(miette!(failure.report)),
        Ok(app::PreparedPluginResponse::Output(prepared)) => Ok(crate::app::CliCommandResult {
            exit_code: 0,
            messages: prepared.messages,
            output: Some(crate::app::ReplCommandOutput::Output {
                output: prepared.output,
                format_hint: prepared.format_hint,
            }),
        }),
        Err(err) => Err(miette!("{err:#}")),
    }
}

fn emit_repl_command_conflict_warning(
    runtime: &AppRuntime,
    clients: &AppClients,
    command: &str,
    overrides: ReplDispatchOverrides,
) {
    let Some(message) = clients.plugins.conflict_warning(command) else {
        return;
    };
    let mut messages = osp_ui::messages::MessageBuffer::default();
    messages.warning(message);
    app::emit_messages_for_ui(
        runtime.config.resolved(),
        &runtime.ui,
        &messages,
        overrides.message_verbosity,
    );
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
                PluginsCommands::List
                    | PluginsCommands::Commands
                    | PluginsCommands::Config(_)
                    | PluginsCommands::Doctor
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
