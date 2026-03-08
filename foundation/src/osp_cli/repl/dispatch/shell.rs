use miette::{Result, miette};
use crate::osp_repl::ReplLineResult;
use crate::osp_ui::{render_document, render_output};

use crate::osp_cli::app;
use crate::osp_cli::app::{CMD_HELP, EffectiveInvocation};
use crate::osp_cli::state::{AppClients, AppRuntime, AppSession};

use super::command::run_repl_external_command;
use crate::osp_cli::repl::{ReplViewContext, input, presentation, surface};

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
    scope: &crate::osp_cli::state::ReplScopeStack,
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
        Some(crate::osp_cli::app::ReplCommandOutput::Text(text)) => Ok(text),
        Some(crate::osp_cli::app::ReplCommandOutput::Document(document)) => {
            Ok(render_document(&document, &invocation.ui.render_settings))
        }
        Some(crate::osp_cli::app::ReplCommandOutput::Output {
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

#[cfg(test)]
mod tests {
    use super::{
        apply_repl_shell_prefix, enter_repl_shell, handle_repl_exit_request,
        maybe_handle_repl_shortcuts, repl_help_for_scope,
    };
    use crate::osp_cli::repl::dispatch::base_repl_invocation;
    use crate::osp_cli::repl::input::ReplParsedLine;
    use crate::osp_cli::state::{AppState, AppStateInit, LaunchContext, RuntimeContext, TerminalKind};
    use crate::osp_config::{ConfigLayer, ConfigResolver, ResolveOptions};
    use crate::osp_core::output::OutputFormat;
    use crate::osp_repl::ReplLineResult;
    use crate::osp_ui::RenderSettings;
    use crate::osp_ui::messages::MessageLevel;

    fn app_state() -> AppState {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        let config = resolver
            .resolve(ResolveOptions::default().with_terminal("repl"))
            .expect("test config should resolve");

        AppState::new(AppStateInit {
            context: RuntimeContext::new(None, TerminalKind::Repl, None),
            config,
            render_settings: RenderSettings::test_plain(OutputFormat::Json),
            message_verbosity: MessageLevel::Success,
            debug_verbosity: 0,
            plugins: crate::osp_cli::plugin_manager::PluginManager::new(Vec::new()),
            themes: crate::osp_cli::theme_loader::ThemeCatalog::default(),
            launch: LaunchContext::default(),
        })
    }

    #[test]
    fn shell_helpers_cover_prefix_exit_and_root_help_unit() {
        let mut state = app_state();
        let invocation = base_repl_invocation(&state.runtime);
        let help = repl_help_for_scope(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            &invocation,
        )
        .expect("root help should render");
        assert!(help.contains("help") || help.contains("config"));

        assert_eq!(
            apply_repl_shell_prefix(&state.session.scope, &["user".to_string()]),
            vec!["user".to_string()]
        );
        assert!(matches!(
            handle_repl_exit_request(&mut state.session),
            Some(ReplLineResult::Exit(0))
        ));

        state.session.scope.enter("ldap");
        assert_eq!(
            apply_repl_shell_prefix(&state.session.scope, &["user".to_string()]),
            vec!["ldap".to_string(), "user".to_string()]
        );
        assert!(matches!(
            handle_repl_exit_request(&mut state.session),
            Some(ReplLineResult::Continue(text)) if text.contains("Leaving ldap shell")
        ));
    }

    #[test]
    fn shortcut_handling_covers_help_none_and_shell_entry_error_unit() {
        let mut state = app_state();
        let invocation = base_repl_invocation(&state.runtime);

        let help = ReplParsedLine::parse("--help", state.runtime.config.resolved())
            .expect("help should parse");
        assert!(matches!(
            maybe_handle_repl_shortcuts(
                &mut state.runtime,
                &mut state.session,
                &state.clients,
                &help,
                &invocation,
            )
            .expect("help shortcut should succeed"),
            Some(ReplLineResult::Continue(text)) if text.contains("help") || text.contains("config")
        ));

        let ordinary = ReplParsedLine::parse("config show", state.runtime.config.resolved())
            .expect("ordinary command should parse");
        assert_eq!(
            maybe_handle_repl_shortcuts(
                &mut state.runtime,
                &mut state.session,
                &state.clients,
                &ordinary,
                &invocation,
            )
            .expect("ordinary command should not shortcut"),
            None
        );

        let shell_entry =
            ReplParsedLine::parse("ldap", state.runtime.config.resolved()).expect("ldap parses");
        let err = maybe_handle_repl_shortcuts(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            &shell_entry,
            &invocation,
        )
        .expect_err("missing plugin should reject shell entry");
        assert!(err.to_string().contains("no plugin provides command: ldap"));

        let err = enter_repl_shell(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            "ldap",
            &invocation,
        )
        .expect_err("direct shell entry should also fail");
        assert!(err.to_string().contains("no plugin provides command: ldap"));
    }
}
