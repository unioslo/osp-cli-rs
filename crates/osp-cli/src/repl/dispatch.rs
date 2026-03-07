use miette::{Result, miette};
use osp_repl::{ReplLineResult, ReplReloadKind, SharedHistory, expand_history};
use osp_ui::{render_document, render_output};
use std::borrow::Cow;
use std::time::Instant;

use crate::app;
use crate::app::{
    BuiltinCommandTransport, CMD_CONFIG, CMD_DOCTOR, CMD_HELP, CMD_HISTORY, CMD_PLUGINS, CMD_THEME,
    EffectiveInvocation, ReplCommandSpec, resolve_effective_invocation,
};
use crate::cli::{
    Commands, ConfigArgs, ConfigCommands, DoctorCommands, HistoryCommands, PluginsCommands,
    ThemeArgs, ThemeCommands, parse_inline_command_tokens,
};
use crate::invocation::{append_invocation_help, scan_command_tokens};
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
    let started = Instant::now();
    match execute_repl_plugin_line_inner(runtime, session, clients, history, line) {
        Ok(result) => Ok(result),
        Err(err) => {
            if runtime.ui.debug_verbosity > 0 {
                session.record_prompt_timing(
                    runtime.ui.debug_verbosity,
                    started.elapsed(),
                    None,
                    None,
                    None,
                );
            }
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
    let started = Instant::now();
    let raw = line.trim();
    if let Some(result) = maybe_execute_repl_builtin(runtime, session, clients, history, raw)? {
        session.record_prompt_timing(
            runtime.ui.debug_verbosity,
            started.elapsed(),
            None,
            None,
            None,
        );
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
        session.record_prompt_timing(
            runtime.ui.debug_verbosity,
            started.elapsed(),
            None,
            None,
            None,
        );
        return Ok(ReplLineResult::Continue(help));
    }

    let base_invocation = base_repl_invocation(runtime);
    if let Some(result) =
        maybe_handle_repl_shortcuts(runtime, session, clients, &parsed, &base_invocation)?
    {
        session.record_prompt_timing(
            runtime.ui.debug_verbosity,
            started.elapsed(),
            None,
            None,
            None,
        );
        return Ok(result);
    }

    let invocation = match parse_repl_invocation(runtime, session, &parsed)? {
        ParsedReplDispatch::Help(rendered) => return Ok(ReplLineResult::Continue(rendered)),
        ParsedReplDispatch::Invocation(invocation) => invocation,
    };
    let parse_finished = Instant::now();
    let output = run_repl_command(
        runtime,
        session,
        clients,
        invocation.command,
        &invocation.effective,
        history,
        invocation.cache_key.as_deref(),
    )?;
    let execute_finished = Instant::now();
    let rendered = render_repl_command_output(
        runtime,
        session,
        line,
        &invocation.stages,
        output,
        &invocation.effective,
    )?;
    let finished = Instant::now();
    session.record_prompt_timing(
        invocation.effective.ui.debug_verbosity,
        finished.saturating_duration_since(started),
        Some(parse_finished.saturating_duration_since(started)),
        Some(execute_finished.saturating_duration_since(parse_finished)),
        Some(finished.saturating_duration_since(execute_finished)),
    );
    Ok(finalize_repl_command(
        session,
        rendered,
        invocation.restart_repl,
        invocation.show_intro_on_reload,
    ))
}

fn base_repl_invocation(runtime: &AppRuntime) -> EffectiveInvocation {
    resolve_effective_invocation(&runtime.ui, &Default::default())
}

fn repl_cache_key_for_command(
    runtime: &AppRuntime,
    command: &Commands,
    invocation: &crate::invocation::InvocationOptions,
) -> Option<String> {
    if !invocation.cache {
        return None;
    }

    let Commands::External(tokens) = command else {
        return None;
    };

    let provider = invocation.plugin_provider.as_deref().unwrap_or_default();
    let encoded_tokens =
        serde_json::to_string(tokens).expect("external command tokens should serialize");

    Some(format!(
        "rev:{}|profile:{}|provider:{}|tokens:{}",
        runtime.config.revision(),
        runtime.config.resolved().active_profile(),
        provider,
        encoded_tokens
    ))
}

fn maybe_handle_repl_shortcuts(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    parsed: &input::ReplParsedLine,
    base_invocation: &EffectiveInvocation,
) -> Result<Option<ReplLineResult>> {
    if parsed.requests_repl_help() {
        return repl_help_result(runtime, session, clients, base_invocation).map(Some);
    }

    if let Some(result) =
        maybe_handle_single_token_shortcut(runtime, session, clients, parsed, base_invocation)?
    {
        return Ok(Some(result));
    }

    if let Some(command) = parsed.shell_entry_command(&session.scope) {
        let entered = enter_repl_shell(runtime, session, clients, command, base_invocation)?;
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
    base_invocation: &EffectiveInvocation,
) -> Result<Option<ReplLineResult>> {
    if parsed.dispatch_tokens.len() != 1 {
        return Ok(None);
    }

    match parsed.dispatch_tokens[0].as_str() {
        CMD_HELP => repl_help_result(runtime, session, clients, base_invocation).map(Some),
        "exit" | "quit" => Ok(handle_repl_exit_request(session)),
        _ => Ok(None),
    }
}

fn repl_help_result(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    invocation: &EffectiveInvocation,
) -> Result<ReplLineResult> {
    Ok(ReplLineResult::Continue(repl_help_for_scope(
        runtime, session, clients, invocation,
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
    effective: EffectiveInvocation,
    stages: Vec<String>,
    cache_key: Option<String>,
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
    let scanned = scan_command_tokens(&prefixed_tokens)?;
    let scoped_tokens =
        input::rewrite_help_alias_tokens_at(&scanned.tokens, session.scope.commands().len())
            .unwrap_or_else(|| scanned.tokens.clone());
    let command = match parse_inline_command_tokens(&scoped_tokens) {
        Ok(Some(command)) => command,
        Ok(None) => return Err(miette!("missing command")),
        Err(err) => {
            if renders_repl_inline_help(err.kind()) {
                let rendered = render_repl_parse_help(
                    ReplViewContext::from_parts(runtime, session),
                    &err.to_string(),
                );
                return Ok(ParsedReplDispatch::Help(rendered));
            }
            return Err(miette!(err.to_string()));
        }
    };
    let spec = repl_command_spec(&command);
    app::ensure_command_supports_dsl(&spec, &parsed.stages)?;
    if !parsed.stages.is_empty() {
        completion::validate_dsl_stages(&parsed.stages)?;
    }

    Ok(ParsedReplDispatch::Invocation(ParsedReplInvocation {
        cache_key: repl_cache_key_for_command(runtime, &command, &scanned.invocation),
        effective: resolve_effective_invocation(&runtime.ui, &scanned.invocation),
        restart_repl: command_restarts_repl(&command),
        show_intro_on_reload: theme_or_palette_change_requires_intro(&command),
        command,
        stages: parsed.stages.clone(),
    }))
}

fn renders_repl_inline_help(kind: clap::error::ErrorKind) -> bool {
    matches!(
        kind,
        clap::error::ErrorKind::DisplayHelp
            | clap::error::ErrorKind::DisplayVersion
            | clap::error::ErrorKind::InvalidSubcommand
            | clap::error::ErrorKind::UnknownArgument
            | clap::error::ErrorKind::MissingRequiredArgument
    )
}

fn render_repl_parse_help(view: ReplViewContext<'_>, error_text: &str) -> String {
    let mut out = String::new();
    if let Some(summary) = summarize_clap_error(error_text) {
        out.push_str(summary);
        out.push_str("\n\n");
    }
    let help_body = strip_clap_error_preamble_and_epilogue(error_text);
    out.push_str(&help::render_repl_help_with_chrome(
        view,
        &append_invocation_help(&help_body),
    ));
    out
}

fn summarize_clap_error(error_text: &str) -> Option<&str> {
    error_text
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("error:").map(str::trim))
}

fn strip_clap_error_preamble_and_epilogue(error_text: &str) -> String {
    error_text
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.starts_with("error:")
                && !trimmed.starts_with("tip:")
                && !trimmed.starts_with("For more information")
        })
        .collect::<Vec<_>>()
        .join("\n")
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
    invocation: &EffectiveInvocation,
) -> Result<String> {
    let crate::app::CliCommandResult {
        exit_code,
        messages,
        output,
        stderr_text,
        failure_report,
        ..
    } = result;

    if exit_code != 0
        && let Some(report) = failure_report
    {
        return Err(miette!("{report}"));
    }

    if !messages.is_empty() {
        app::emit_messages_for_ui(
            runtime.config.resolved(),
            &invocation.ui,
            &messages,
            invocation.ui.message_verbosity,
        );
    }

    let mut rendered = match output {
        Some(crate::app::ReplCommandOutput::Output {
            output,
            format_hint,
        }) => {
            let (output, format_hint) = app::apply_output_stages(output, stages, format_hint)
                .map_err(|err| miette!("{err:#}"))?;

            let render_settings =
                app::resolve_effective_render_settings(&invocation.ui.render_settings, format_hint);
            let rendered = render_output(&output, &render_settings);
            session.record_result(line, output_to_rows(&output));
            app::maybe_copy_output_with_runtime(
                &app::CommandRenderRuntime::new(runtime.config.resolved(), &invocation.ui),
                &output,
            );
            rendered
        }
        Some(crate::app::ReplCommandOutput::Document(document)) => {
            render_document(&document, &invocation.ui.render_settings)
        }
        Some(crate::app::ReplCommandOutput::Text(text)) => text,
        None => String::new(),
    };

    if let Some(stderr_text) = stderr_text
        && !stderr_text.is_empty()
    {
        rendered.push_str(&stderr_text);
    }

    Ok(rendered)
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
            &base_repl_invocation(runtime),
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
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    command: &str,
    invocation: &EffectiveInvocation,
) -> Result<String> {
    app::ensure_plugin_visible_for(&runtime.auth, command)?;
    let catalog = app::authorized_command_catalog_for(&runtime.auth, &clients.plugins)?;
    if !catalog.iter().any(|entry| entry.name == command) {
        return Err(miette!("no plugin provides command: {command}"));
    }

    session.scope.enter(command.to_string());
    let mut out = format!("Entering {command} shell. Type `exit` to leave.\n");
    if let Ok(help) = repl_help_for_scope(runtime, session, clients, invocation) {
        out.push_str(&help);
    }
    Ok(out)
}

fn repl_help_for_scope(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    invocation: &EffectiveInvocation,
) -> Result<String> {
    if session.scope.is_root() {
        let catalog = app::authorized_command_catalog_for(&runtime.auth, &clients.plugins)?;
        let view = ReplViewContext::from_parts(runtime, session);
        let surface = surface::build_repl_surface(view, &catalog);
        return Ok(presentation::render_repl_command_overview(view, &surface));
    }

    let tokens = session.scope.help_tokens();
    match run_repl_external_command(runtime, clients, session, tokens, invocation)?.output {
        Some(crate::app::ReplCommandOutput::Text(text)) => Ok(text),
        Some(crate::app::ReplCommandOutput::Document(document)) => {
            Ok(render_document(&document, &invocation.ui.render_settings))
        }
        Some(crate::app::ReplCommandOutput::Output {
            output,
            format_hint,
        }) => {
            let render_settings =
                app::resolve_effective_render_settings(&invocation.ui.render_settings, format_hint);
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
    invocation: &EffectiveInvocation,
    history: &SharedHistory,
    cache_key: Option<&str>,
) -> Result<crate::app::CliCommandResult> {
    if let Some(cache_key) = cache_key
        && let Some(cached) = session.cached_command(cache_key)
    {
        return Ok(cached);
    }

    let result = match command {
        Commands::External(tokens) => {
            run_repl_external_command(runtime, clients, session, tokens, invocation)
        }
        builtin => app::dispatch_builtin_command_parts(
            runtime,
            session,
            clients,
            BuiltinCommandTransport::Repl { history },
            Some(invocation),
            builtin,
        )
        .and_then(|result| result.ok_or_else(|| miette!("expected builtin command"))),
    }?;

    if let Some(cache_key) = cache_key
        && result.exit_code == 0
        && matches!(
            &result.output,
            Some(crate::app::ReplCommandOutput::Output { .. })
        )
    {
        session.record_cached_command(cache_key, &result);
    }

    Ok(result)
}

fn run_repl_external_command(
    runtime: &mut AppRuntime,
    clients: &AppClients,
    session: &mut AppSession,
    tokens: Vec<String>,
    invocation: &EffectiveInvocation,
) -> Result<crate::app::CliCommandResult> {
    let resolved = invocation.ui.render_settings.resolve_render_settings();
    let layout = crate::ui_presentation::effective_help_layout(runtime.config.resolved());
    app::run_external_command_with_help_renderer(
        runtime,
        session,
        clients,
        &tokens,
        invocation,
        |stdout| help::render_help_with_chrome(stdout, &resolved, layout),
    )
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
