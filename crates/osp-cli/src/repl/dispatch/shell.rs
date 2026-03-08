use miette::{Result, miette};
use osp_repl::ReplLineResult;
use osp_ui::{render_document, render_output};

use crate::app;
use crate::app::{CMD_HELP, EffectiveInvocation};
use crate::state::{AppClients, AppRuntime, AppSession};

use super::command::run_repl_external_command;
use crate::repl::{ReplViewContext, input, presentation, surface};

pub(super) fn maybe_handle_repl_shortcuts(
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

pub(super) fn handle_repl_exit_request(session: &mut AppSession) -> Option<ReplLineResult> {
    if session.scope.is_root() {
        session.sync_history_shell_context();
        return Some(ReplLineResult::Exit(0));
    }

    let message = leave_repl_shell(session)?;
    session.sync_history_shell_context();
    Some(ReplLineResult::Continue(message))
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

pub(super) fn enter_repl_shell(
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

pub(super) fn repl_help_for_scope(
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
