use miette::{Result, miette};
use osp_repl::{ReplLineResult, ReplReloadKind, SharedHistory, expand_history};
use osp_ui::{render_document, render_output};
use std::borrow::Cow;
use std::time::Instant;

use crate::app;
use crate::app::{
    CMD_CONFIG, CMD_DOCTOR, CMD_HELP, CMD_HISTORY, CMD_PLUGINS, CMD_THEME, EffectiveInvocation,
    ReplCommandSpec, resolve_effective_invocation,
};
use crate::cli::{
    Commands, ConfigArgs, ConfigCommands, DoctorCommands, HistoryCommands, PluginsCommands,
    ThemeArgs, ThemeCommands, parse_inline_command_tokens,
};
use crate::invocation::{append_invocation_help_if_verbose, scan_command_tokens};
use crate::rows::output::output_to_rows;
use crate::state::{AppClients, AppRuntime, AppSession};
use crate::ui_sink::{StdIoUiSink, UiSink};

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
    let mut sink = StdIoUiSink;
    match execute_repl_plugin_line_inner(runtime, session, clients, history, line, &mut sink) {
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
    sink: &mut dyn UiSink,
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
        ParsedReplDispatch::Help {
            rendered,
            effective,
        } => {
            let finished = Instant::now();
            session.record_prompt_timing(
                effective.ui.debug_verbosity,
                finished.saturating_duration_since(started),
                Some(finished.saturating_duration_since(started)),
                None,
                None,
            );
            return Ok(ReplLineResult::Continue(rendered));
        }
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
        sink,
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
        invocation.side_effects.restart_repl,
        invocation.side_effects.show_intro_on_reload,
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
    side_effects: CommandSideEffects,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct CommandSideEffects {
    restart_repl: bool,
    show_intro_on_reload: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ParsedClapHelp<'a> {
    summary: Option<&'a str>,
    body: &'a str,
}

enum ParsedReplDispatch {
    Help {
        rendered: String,
        effective: Box<EffectiveInvocation>,
    },
    Invocation(Box<ParsedReplInvocation>),
}

fn parse_repl_invocation(
    runtime: &AppRuntime,
    session: &AppSession,
    parsed: &input::ReplParsedLine,
) -> Result<ParsedReplDispatch> {
    let prefixed_tokens = parsed.prefixed_tokens(&session.scope);
    let scanned = scan_command_tokens(&prefixed_tokens)?;
    let command_index = session.scope.commands().len();
    if scanned.tokens.get(command_index).map(String::as_str) == Some(CMD_HELP)
        && !input::has_valid_help_alias_target(&scanned.tokens, command_index)
    {
        return Ok(ParsedReplDispatch::Help {
            rendered: render_invalid_help_alias(
                ReplViewContext::from_parts(runtime, session),
                input::help_alias_target_at(&scanned.tokens, command_index),
            ),
            effective: Box::new(resolve_effective_invocation(
                &runtime.ui,
                &scanned.invocation,
            )),
        });
    }
    let scoped_tokens = input::rewrite_help_alias_tokens_at(&scanned.tokens, command_index)
        .unwrap_or_else(|| scanned.tokens.clone());
    let command = match parse_inline_command_tokens(&scoped_tokens) {
        Ok(Some(command)) => command,
        Ok(None) => return Err(miette!("missing command")),
        Err(err) => {
            if renders_repl_inline_help(err.kind()) {
                let rendered = render_repl_parse_help(
                    ReplViewContext::from_parts(runtime, session),
                    &scanned.invocation,
                    &err.to_string(),
                );
                return Ok(ParsedReplDispatch::Help {
                    rendered,
                    effective: Box::new(resolve_effective_invocation(
                        &runtime.ui,
                        &scanned.invocation,
                    )),
                });
            }
            return Err(miette!(err.to_string()));
        }
    };
    let spec = repl_command_spec(&command);
    app::ensure_command_supports_dsl(&spec, &parsed.stages)?;
    if !parsed.stages.is_empty() {
        completion::validate_dsl_stages(&parsed.stages)?;
    }

    Ok(ParsedReplDispatch::Invocation(Box::new(
        ParsedReplInvocation {
            cache_key: repl_cache_key_for_command(runtime, &command, &scanned.invocation),
            effective: resolve_effective_invocation(&runtime.ui, &scanned.invocation),
            side_effects: command_side_effects(&command),
            command,
            stages: parsed.stages.clone(),
        },
    )))
}

fn render_invalid_help_alias(view: ReplViewContext<'_>, target: Option<&str>) -> String {
    let mut out = String::new();
    let detail = match target {
        Some(target) => format!("invalid help target: `{target}`"),
        None => "help expects a command target".to_string(),
    };
    out.push_str(&detail);
    out.push_str("\n\n");
    out.push_str(&help::render_repl_help_with_chrome(
        view,
        "Usage: help <command>\n\nNotes:\n  Use bare `help` for the REPL overview.\n  `help help` and `help --help` are not valid.\n  Add -v to include common invocation options.\n",
    ));
    out
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

fn render_repl_parse_help(
    view: ReplViewContext<'_>,
    invocation: &crate::invocation::InvocationOptions,
    error_text: &str,
) -> String {
    let parsed = parse_clap_help(error_text);
    let mut out = String::new();
    if let Some(summary) = parsed.summary {
        out.push_str(summary);
        out.push_str("\n\n");
    }
    out.push_str(&help::render_repl_help_with_chrome(
        view,
        &append_invocation_help_if_verbose(parsed.body, invocation),
    ));
    out
}

fn parse_clap_help(error_text: &str) -> ParsedClapHelp<'_> {
    let lines = error_text.lines().collect::<Vec<_>>();
    let summary = lines
        .iter()
        .map(|line| line.trim())
        .find_map(|line| line.strip_prefix("error:").map(str::trim));

    let body_start = lines
        .iter()
        .position(|line| line.trim_start().starts_with("Usage:"))
        .unwrap_or(0);
    let mut body_end = lines.len();
    while body_end > body_start {
        let trimmed = lines[body_end - 1].trim();
        if trimmed.is_empty() {
            body_end -= 1;
            continue;
        }
        if trimmed.starts_with("tip:") || trimmed.starts_with("For more information") {
            body_end -= 1;
            continue;
        }
        break;
    }

    let body = if body_start < body_end {
        &error_text[line_start_offset(&lines, body_start)..line_end_offset(&lines, body_end)]
    } else {
        ""
    };

    ParsedClapHelp { summary, body }
}

fn line_start_offset(lines: &[&str], line_index: usize) -> usize {
    lines
        .iter()
        .take(line_index)
        .map(|line| line.len() + 1)
        .sum()
}

fn line_end_offset(lines: &[&str], line_count: usize) -> usize {
    let mut offset = line_start_offset(lines, line_count);
    if line_count > 0 {
        offset = offset.saturating_sub(1);
    }
    offset
}

fn command_side_effects(command: &Commands) -> CommandSideEffects {
    match command {
        Commands::Theme(ThemeArgs {
            command: ThemeCommands::Use(_),
        }) => CommandSideEffects {
            restart_repl: true,
            show_intro_on_reload: true,
        },
        Commands::Config(ConfigArgs {
            command: ConfigCommands::Set(set),
        }) if !set.dry_run => CommandSideEffects {
            restart_repl: true,
            show_intro_on_reload: config_key_change_requires_intro(&set.key),
        },
        Commands::Config(ConfigArgs {
            command: ConfigCommands::Unset(unset),
        }) if !unset.dry_run => CommandSideEffects {
            restart_repl: true,
            show_intro_on_reload: config_key_change_requires_intro(&unset.key),
        },
        _ => CommandSideEffects::default(),
    }
}

fn render_repl_command_output(
    runtime: &AppRuntime,
    session: &mut AppSession,
    line: &str,
    stages: &[String],
    result: crate::app::CliCommandResult,
    invocation: &EffectiveInvocation,
    sink: &mut dyn UiSink,
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
            sink,
        );
    }

    let rendered = match output {
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
                sink,
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
        sink.write_stderr(&stderr_text);
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
            Some(history),
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

#[cfg(test)]
mod tests {
    use clap::error::ErrorKind;
    use osp_core::output::OutputFormat;
    use osp_repl::{HistoryConfig, ReplLineResult, ReplReloadKind, SharedHistory};
    use osp_ui::RenderSettings;
    use osp_ui::messages::MessageLevel;

    use super::{
        BangCommand, command_side_effects, config_key_change_requires_intro, current_history_scope,
        enter_repl_shell, execute_bang_command, finalize_repl_command, handle_repl_exit_request,
        is_repl_bang_request, leave_repl_shell, parse_bang_command, parse_clap_help,
        parse_repl_builtin, render_repl_command_output, renders_repl_inline_help,
        repl_command_spec, repl_help_for_scope, run_repl_command, strip_history_scope,
    };
    use crate::app::{CliCommandResult, ReplCommandOutput};
    use crate::cli::{
        Commands, ConfigArgs, ConfigCommands, ConfigSetArgs, ConfigUnsetArgs, DebugCompleteArgs,
        HistoryArgs, HistoryCommands, PluginsArgs, PluginsCommands, ReplArgs, ReplCommands,
        ThemeArgs, ThemeCommands, ThemeUseArgs,
    };
    use crate::state::{
        AppSession, AppState, AppStateInit, LaunchContext, RuntimeContext, TerminalKind,
    };
    use osp_config::{ConfigLayer, ConfigResolver, ResolveOptions};

    #[test]
    fn clap_error_helpers_extract_summary_and_body_unit() {
        let error = "\
error: unknown argument '--wat'\n\
\n\
Usage: osp config show [OPTIONS]\n\
\n\
tip: try --help\n\
For more information, try '--help'.\n";

        let parsed = parse_clap_help(error);
        assert_eq!(parsed.summary, Some("unknown argument '--wat'"));
        assert_eq!(parsed.body, "Usage: osp config show [OPTIONS]");
    }

    #[test]
    fn repl_exit_behaves_differently_for_root_and_nested_shells_unit() {
        let mut root = AppSession::with_cache_limit(4);
        assert!(matches!(
            handle_repl_exit_request(&mut root),
            Some(ReplLineResult::Exit(0))
        ));

        let mut nested = AppSession::with_cache_limit(4);
        nested.scope.enter("ldap");
        assert!(matches!(
            handle_repl_exit_request(&mut nested),
            Some(ReplLineResult::Continue(message))
                if message == "Leaving ldap shell. Back at root.\n"
        ));
        assert!(nested.scope.is_root());

        let mut deep = AppSession::with_cache_limit(4);
        deep.scope.enter("ldap");
        deep.scope.enter("user");
        let message = leave_repl_shell(&mut deep).expect("nested shell should leave");
        assert_eq!(message, "Leaving user shell.\n");
        assert_eq!(deep.scope.commands(), vec!["ldap".to_string()]);
    }

    #[test]
    fn repl_restart_detection_covers_mutating_commands_unit() {
        let theme = Commands::Theme(ThemeArgs {
            command: ThemeCommands::Use(ThemeUseArgs {
                name: "dracula".to_string(),
            }),
        });
        let theme_effects = command_side_effects(&theme);
        assert!(theme_effects.restart_repl);
        assert!(theme_effects.show_intro_on_reload);

        let config_set = Commands::Config(ConfigArgs {
            command: ConfigCommands::Set(ConfigSetArgs {
                key: "ui.format".to_string(),
                value: "json".to_string(),
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
        let config_set_effects = command_side_effects(&config_set);
        assert!(config_set_effects.restart_repl);
        assert!(!config_set_effects.show_intro_on_reload);

        let config_unset_dry_run = Commands::Config(ConfigArgs {
            command: ConfigCommands::Unset(ConfigUnsetArgs {
                key: "ui.format".to_string(),
                global: false,
                profile: None,
                profile_all: false,
                terminal: None,
                session: false,
                config_store: false,
                secrets: false,
                save: false,
                dry_run: true,
            }),
        });
        assert_eq!(
            command_side_effects(&config_unset_dry_run),
            Default::default()
        );
    }

    #[test]
    fn repl_inline_help_kinds_match_supported_clap_errors_unit() {
        assert!(renders_repl_inline_help(ErrorKind::DisplayHelp));
        assert!(renders_repl_inline_help(ErrorKind::UnknownArgument));
        assert!(renders_repl_inline_help(ErrorKind::InvalidSubcommand));
        assert!(!renders_repl_inline_help(ErrorKind::ValueValidation));
    }

    #[test]
    fn leave_repl_shell_returns_none_at_root_unit() {
        let mut session = AppSession::with_cache_limit(4);
        assert!(leave_repl_shell(&mut session).is_none());
        assert!(matches!(
            finalize_repl_command(&session, String::new(), true, false),
            ReplLineResult::Restart {
                output,
                reload: ReplReloadKind::Default
            } if output.is_empty()
        ));
    }

    #[test]
    fn finalize_repl_command_uses_intro_reload_when_requested_unit() {
        let session = AppSession::with_cache_limit(4);
        assert!(matches!(
            finalize_repl_command(&session, "saved\n".to_string(), true, true),
            ReplLineResult::Restart {
                output,
                reload: ReplReloadKind::WithIntro
            } if output == "saved\n"
        ));
    }

    #[test]
    fn clap_error_helpers_handle_missing_summary_gracefully_unit() {
        let error = "\nUsage: osp ldap user\nFor more information, try '--help'.\n";
        let parsed = parse_clap_help(error);
        assert_eq!(parsed.summary, None);
        assert_eq!(parsed.body, "Usage: osp ldap user");
    }

    #[test]
    fn repl_builtin_and_bang_parsers_cover_shortcuts_unit() {
        assert!(matches!(
            parse_repl_builtin("--help").expect("help parses"),
            Some(super::ReplBuiltin::Help)
        ));
        assert!(matches!(
            parse_repl_builtin("quit").expect("exit parses"),
            Some(super::ReplBuiltin::Exit)
        ));
        assert!(matches!(
            parse_repl_builtin("!?ops").expect("contains parses"),
            Some(super::ReplBuiltin::Bang(BangCommand::Contains(term))) if term == "ops"
        ));
        assert!(matches!(
            parse_bang_command("!!").expect("last parses"),
            Some(BangCommand::Last)
        ));
        assert!(matches!(
            parse_bang_command("!-2").expect("relative parses"),
            Some(BangCommand::Relative(2))
        ));
        assert!(matches!(
            parse_bang_command("!7").expect("absolute parses"),
            Some(BangCommand::Absolute(7))
        ));
        assert!(matches!(
            parse_bang_command("!pref").expect("prefix parses"),
            Some(BangCommand::Prefix(prefix)) if prefix == "pref"
        ));
        assert!(
            parse_bang_command("!?   ")
                .expect_err("contains search requires text")
                .to_string()
                .contains("expects search text")
        );
        assert!(
            parse_bang_command("!-0")
                .expect_err("relative bang ids must be positive")
                .to_string()
                .contains("N >= 1")
        );
        assert!(
            parse_bang_command("!0")
                .expect_err("absolute bang ids must be positive")
                .to_string()
                .contains("N >= 1")
        );
    }

    #[test]
    fn bang_execution_and_scope_helpers_cover_help_matches_and_replace_unit() {
        let history = SharedHistory::new(HistoryConfig {
            path: None,
            max_entries: 20,
            enabled: true,
            dedupe: true,
            profile_scoped: false,
            exclude_patterns: Vec::new(),
            profile: None,
            terminal: None,
            shell_context: Default::default(),
        })
        .expect("history should initialize");
        history
            .save_command_line("ldap user alice")
            .expect("first command saves");
        history
            .save_command_line("ldap netgroup ops")
            .expect("second command saves");
        history
            .save_command_line("config show")
            .expect("third command saves");

        let mut session = AppSession::with_cache_limit(4);
        session.scope.enter("ldap");
        assert_eq!(current_history_scope(&session).as_deref(), Some("ldap "));
        assert_eq!(
            strip_history_scope("ldap user alice", Some("ldap")),
            "user alice".to_string()
        );
        assert_eq!(
            strip_history_scope("config show", Some("ldap")),
            "config show".to_string()
        );

        assert!(matches!(
            execute_bang_command(&mut session, &history, "!", BangCommand::Prefix(String::new()))
                .expect("empty prefix renders help"),
            ReplLineResult::Continue(help) if help.contains("Bang history shortcuts")
        ));
        assert!(matches!(
            execute_bang_command(
                &mut session,
                &history,
                "!?ops",
                BangCommand::Contains("ops".to_string())
            )
            .expect("contains search should expand"),
            ReplLineResult::ReplaceInput(value) if value == "netgroup ops"
        ));
        assert!(matches!(
            execute_bang_command(
                &mut session,
                &history,
                "!user",
                BangCommand::Prefix("user".to_string())
            )
            .expect("prefix search should expand"),
            ReplLineResult::ReplaceInput(value) if value == "user alice"
        ));
        assert!(matches!(
            execute_bang_command(
                &mut session,
                &history,
                "!missing",
                BangCommand::Prefix("missing".to_string())
            )
            .expect("missing bang match should still succeed"),
            ReplLineResult::Continue(value) if value.contains("No history match")
        ));
        assert!(is_repl_bang_request(" !prefix"));
        assert!(!is_repl_bang_request("help"));
    }

    #[test]
    fn intro_reload_keys_cover_theme_color_and_palette_mutations_unit() {
        assert!(config_key_change_requires_intro("theme.name"));
        assert!(config_key_change_requires_intro(" color.message.info "));
        assert!(config_key_change_requires_intro("palette.custom"));
        assert!(!config_key_change_requires_intro("ui.format"));

        let config_unset = Commands::Config(ConfigArgs {
            command: ConfigCommands::Unset(ConfigUnsetArgs {
                key: "color.message.info".to_string(),
                global: false,
                profile: None,
                profile_all: false,
                terminal: None,
                session: false,
                config_store: false,
                secrets: false,
                save: false,
                dry_run: false,
            }),
        });
        let side_effects = command_side_effects(&config_unset);
        assert!(side_effects.restart_repl);
        assert!(side_effects.show_intro_on_reload);
    }

    fn make_state_with_plugins(plugins: crate::plugin_manager::PluginManager) -> AppState {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        let config = resolver
            .resolve(ResolveOptions::default().with_terminal("repl"))
            .expect("test config should resolve");

        let settings = RenderSettings::test_plain(OutputFormat::Json);
        AppState::new(AppStateInit {
            context: RuntimeContext::new(None, TerminalKind::Repl, None),
            config,
            render_settings: settings,
            message_verbosity: MessageLevel::Success,
            debug_verbosity: 0,
            plugins,
            themes: crate::theme_loader::ThemeCatalog::default(),
            launch: LaunchContext::default(),
        })
    }

    #[test]
    fn root_help_rendering_and_shell_prefix_helpers_cover_root_paths_unit() {
        let mut state =
            make_state_with_plugins(crate::plugin_manager::PluginManager::new(Vec::new()));
        let invocation = super::base_repl_invocation(&state.runtime);
        let help = repl_help_for_scope(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            &invocation,
        )
        .expect("root help should render");
        assert!(help.contains("help"));
        assert_eq!(
            super::apply_repl_shell_prefix(&state.session.scope, &["config".to_string()]),
            vec!["config".to_string()]
        );

        state.session.scope.enter("ldap");
        assert_eq!(
            super::apply_repl_shell_prefix(&state.session.scope, &["user".to_string()]),
            vec!["ldap".to_string(), "user".to_string()]
        );
    }

    #[test]
    fn repl_command_spec_covers_repl_variant_and_builtin_dsl_matrix_unit() {
        let repl = repl_command_spec(&Commands::Repl(ReplArgs {
            command: ReplCommands::DebugComplete(DebugCompleteArgs {
                line: String::new(),
                cursor: None,
                width: 80,
                height: 24,
                steps: Vec::new(),
                menu_ansi: false,
                menu_unicode: false,
            }),
        }));
        assert_eq!(repl.name.as_ref(), "repl");
        assert!(!repl.supports_dsl);

        let plugins = repl_command_spec(&Commands::Plugins(PluginsArgs {
            command: PluginsCommands::Doctor,
        }));
        assert!(plugins.supports_dsl);

        let history = repl_command_spec(&Commands::History(HistoryArgs {
            command: HistoryCommands::Clear,
        }));
        assert!(!history.supports_dsl);
    }

    #[test]
    fn render_repl_command_output_handles_text_none_and_stderr_unit() {
        use crate::ui_sink::BufferedUiSink;

        let mut state =
            make_state_with_plugins(crate::plugin_manager::PluginManager::new(Vec::new()));
        let invocation = super::base_repl_invocation(&state.runtime);
        let mut sink = BufferedUiSink::default();

        let rendered = render_repl_command_output(
            &state.runtime,
            &mut state.session,
            "doctor last",
            &[],
            CliCommandResult {
                exit_code: 0,
                messages: Default::default(),
                output: Some(ReplCommandOutput::Text("hello".to_string())),
                stderr_text: Some("\nwarn\n".to_string()),
                failure_report: None,
            },
            &invocation,
            &mut sink,
        )
        .expect("text output should render");
        assert_eq!(rendered, "hello");
        assert_eq!(sink.stderr, "\nwarn\n");

        let empty = render_repl_command_output(
            &state.runtime,
            &mut state.session,
            "doctor last",
            &[],
            CliCommandResult::exit(0),
            &invocation,
            &mut sink,
        )
        .expect("empty result should render");
        assert!(empty.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn shell_entry_help_and_repl_command_cache_paths_cover_external_flow_unit() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!(
            "osp-cli-repl-dispatch-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time should be valid")
                .as_nanos()
        ));
        let plugins_dir = root.join("plugins");
        std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");
        let plugin_path = plugins_dir.join("osp-cache");
        std::fs::write(
            &plugin_path,
            r#"#!/usr/bin/env bash
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{"protocol_version":1,"plugin_id":"cache","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{"name":"cache","about":"cache plugin","args":[],"flags":{},"subcommands":[]}]}
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
            .expect("plugin metadata should be readable")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&plugin_path, perms).expect("plugin should be executable");

        let mut state = make_state_with_plugins(crate::plugin_manager::PluginManager::new(vec![
            plugins_dir.clone(),
        ]));
        let invocation = super::base_repl_invocation(&state.runtime);

        let entered = enter_repl_shell(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            "cache",
            &invocation,
        )
        .expect("shell entry should succeed");
        assert!(entered.contains("Entering cache shell"));
        assert!(!state.session.scope.is_root());

        let nested_help = repl_help_for_scope(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            &invocation,
        )
        .expect("nested help should render");
        assert!(!nested_help.is_empty());

        let first = run_repl_command(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            Commands::External(vec!["cache".to_string()]),
            &invocation,
            &SharedHistory::new(HistoryConfig {
                path: None,
                max_entries: 8,
                enabled: true,
                dedupe: true,
                profile_scoped: false,
                exclude_patterns: Vec::new(),
                profile: None,
                terminal: None,
                shell_context: Default::default(),
            })
            .expect("history should initialize"),
            Some("cache-key"),
        )
        .expect("first external run should succeed");
        assert_eq!(first.exit_code, 0);

        let cached = run_repl_command(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            Commands::External(vec!["cache".to_string()]),
            &invocation,
            &SharedHistory::new(HistoryConfig {
                path: None,
                max_entries: 8,
                enabled: true,
                dedupe: true,
                profile_scoped: false,
                exclude_patterns: Vec::new(),
                profile: None,
                terminal: None,
                shell_context: Default::default(),
            })
            .expect("history should initialize"),
            Some("cache-key"),
        )
        .expect("cached external run should succeed");
        assert_eq!(cached.exit_code, 0);

        let _ = std::fs::remove_dir_all(&root);
    }
}
