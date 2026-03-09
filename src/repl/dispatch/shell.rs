use crate::repl::ReplLineResult;
use crate::ui::{render_document, render_output};
use miette::{Result, miette};

use crate::app;
use crate::app::{AppClients, AppRuntime, AppSession, CliCommandResult};
use crate::app::{CMD_HELP, ResolvedInvocation};
use crate::cli::invocation::scan_command_tokens;

use super::command::{render_repl_command_output, run_repl_external_command};
use crate::app::sink::UiSink;
use crate::repl::{ReplViewContext, input, presentation, surface};

pub(super) fn maybe_handle_repl_shortcuts(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    parsed: &input::ReplParsedLine,
    base_invocation: &ResolvedInvocation,
    line: &str,
    sink: &mut dyn UiSink,
) -> Result<Option<ReplLineResult>> {
    if let Some(help_invocation) = repl_shortcut_help_invocation(runtime, session, parsed)? {
        return repl_help_result(
            runtime,
            session,
            clients,
            parsed,
            &help_invocation,
            line,
            sink,
        )
        .map(Some);
    }

    if let Some(result) = maybe_handle_single_token_shortcut(
        runtime,
        session,
        clients,
        parsed,
        base_invocation,
        line,
        sink,
    )? {
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
    _runtime: &mut AppRuntime,
    session: &mut AppSession,
    _clients: &AppClients,
    parsed: &input::ReplParsedLine,
    _base_invocation: &ResolvedInvocation,
    _line: &str,
    _sink: &mut dyn UiSink,
) -> Result<Option<ReplLineResult>> {
    if parsed.dispatch_tokens.len() != 1 {
        return Ok(None);
    }

    match parsed.dispatch_tokens[0].as_str() {
        "exit" | "quit" => Ok(handle_repl_exit_request(session)),
        _ => Ok(None),
    }
}

fn repl_shortcut_help_invocation(
    runtime: &AppRuntime,
    session: &AppSession,
    parsed: &input::ReplParsedLine,
) -> Result<Option<ResolvedInvocation>> {
    if parsed.requests_repl_help() {
        return Ok(Some(app::resolve_invocation_ui(
            &runtime.ui,
            &crate::cli::invocation::InvocationOptions::default(),
        )));
    }

    let prefixed_tokens = parsed.prefixed_tokens(&session.scope);
    let command_index = session.scope.commands().len();
    if prefixed_tokens.get(command_index).map(String::as_str) == Some(CMD_HELP) {
        let help_suffix = &prefixed_tokens[command_index + 1..];
        if !help_suffix.is_empty()
            && help_suffix.iter().all(|token| token.starts_with('-'))
            && !help_suffix
                .iter()
                .any(|token| matches!(token.as_str(), "-h" | "--help"))
        {
            let scanned_suffix = scan_command_tokens(help_suffix)?;
            if scanned_suffix.tokens.is_empty() {
                return Ok(Some(app::resolve_invocation_ui(
                    &runtime.ui,
                    &scanned_suffix.invocation,
                )));
            }
        }
    }

    let scanned = scan_command_tokens(&prefixed_tokens)?;
    if scanned.tokens.get(command_index).map(String::as_str) == Some(CMD_HELP)
        && scanned.tokens.len() == command_index + 1
    {
        return Ok(Some(app::resolve_invocation_ui(
            &runtime.ui,
            &scanned.invocation,
        )));
    }

    Ok(None)
}

fn repl_help_result(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    parsed: &input::ReplParsedLine,
    invocation: &ResolvedInvocation,
    line: &str,
    sink: &mut dyn UiSink,
) -> Result<ReplLineResult> {
    if parsed.stages.is_empty() {
        return Ok(ReplLineResult::Continue(repl_help_for_scope(
            runtime, session, clients, invocation,
        )?));
    }

    let result = repl_help_command_result_for_scope(runtime, session, clients, invocation)?;
    let rendered = render_repl_command_output(
        runtime,
        session,
        line,
        &parsed.stages,
        result,
        invocation,
        sink,
    )?;
    Ok(ReplLineResult::Continue(rendered))
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
    scope: &crate::app::ReplScopeStack,
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
    invocation: &ResolvedInvocation,
) -> Result<String> {
    app::ensure_plugin_visible_for(&runtime.auth, command)?;
    let catalog = app::authorized_command_catalog_for(&runtime.auth, clients)?;
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
    invocation: &ResolvedInvocation,
) -> Result<String> {
    match repl_help_command_result_for_scope(runtime, session, clients, invocation)?.output {
        Some(crate::app::ReplCommandOutput::Text(text)) => Ok(text),
        Some(crate::app::ReplCommandOutput::Guide(guide)) => {
            let output = guide.to_output_result();
            Ok(crate::repl::help::render_guide_output(
                &output,
                &invocation.ui.render_settings,
                crate::ui::format::help::GuideRenderOptions {
                    title_prefix: None,
                    layout: crate::ui::presentation::help_layout(runtime.config.resolved()),
                    frame_style: invocation.ui.render_settings.chrome_frame,
                    panel_kind: None,
                },
            ))
        }
        Some(crate::app::ReplCommandOutput::Document(document)) => {
            Ok(render_document(&document, &invocation.ui.render_settings))
        }
        Some(crate::app::ReplCommandOutput::Output {
            output,
            format_hint,
        }) => {
            let render_settings =
                app::resolve_render_settings_with_hint(&invocation.ui.render_settings, format_hint);
            Ok(render_output(&output, &render_settings))
        }
        None => Ok(String::new()),
    }
}

fn repl_help_command_result_for_scope(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    invocation: &ResolvedInvocation,
) -> Result<CliCommandResult> {
    if session.scope.is_root() {
        let catalog = app::authorized_command_catalog_for(&runtime.auth, clients)?;
        let view = ReplViewContext::from_parts(runtime, session);
        let surface = surface::build_repl_surface(view, &catalog);
        return Ok(CliCommandResult::guide(
            presentation::build_repl_command_overview_guide(&surface),
        ));
    }

    let tokens = session.scope.help_tokens();
    run_repl_external_command(runtime, clients, session, tokens, invocation)
}

#[cfg(test)]
mod tests {
    use super::{
        apply_repl_shell_prefix, enter_repl_shell, handle_repl_exit_request,
        maybe_handle_repl_shortcuts, repl_help_for_scope,
    };
    use crate::app::sink::BufferedUiSink;
    use crate::app::{AppState, AppStateInit, LaunchContext, RuntimeContext, TerminalKind};
    use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions};
    use crate::core::output::OutputFormat;
    use crate::core::plugin::{DescribeCommandAuthV1, DescribeVisibilityModeV1};
    use crate::native::{
        NativeCommand, NativeCommandContext, NativeCommandOutcome, NativeCommandRegistry,
    };
    use crate::repl::ReplLineResult;
    use crate::repl::dispatch::base_repl_invocation;
    use crate::repl::input::ReplParsedLine;
    use crate::ui::RenderSettings;
    use crate::ui::messages::MessageLevel;
    use clap::Command;

    struct NativeLdapHelpCommand;

    impl NativeCommand for NativeLdapHelpCommand {
        fn command(&self) -> Command {
            Command::new("ldap")
                .about("Directory lookup")
                .subcommand(Command::new("user").about("Look up a user"))
        }

        fn auth(&self) -> Option<DescribeCommandAuthV1> {
            Some(DescribeCommandAuthV1 {
                visibility: Some(DescribeVisibilityModeV1::Public),
                required_capabilities: Vec::new(),
                feature_flags: Vec::new(),
            })
        }

        fn execute(
            &self,
            args: &[String],
            _context: &NativeCommandContext<'_>,
        ) -> anyhow::Result<NativeCommandOutcome> {
            let text = if args
                .first()
                .is_some_and(|value| value == "help" || value == "--help")
            {
                "LDAP HELP\n".to_string()
            } else {
                format!("ldap ran: {}\n", args.join(" "))
            };
            Ok(NativeCommandOutcome::Help(text))
        }
    }

    fn app_state() -> AppState {
        app_state_with_native(NativeCommandRegistry::default())
    }

    fn app_state_with_native(native_commands: NativeCommandRegistry) -> AppState {
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
            plugins: crate::plugin::PluginManager::new(Vec::new()),
            native_commands,
            themes: crate::ui::theme_loader::ThemeCatalog::default(),
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
        let mut sink = BufferedUiSink::default();

        let help = ReplParsedLine::parse("--help", state.runtime.config.resolved())
            .expect("help should parse");
        assert!(matches!(
            maybe_handle_repl_shortcuts(
                &mut state.runtime,
                &mut state.session,
                &state.clients,
                &help,
                &invocation,
                "--help",
                &mut sink,
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
                "config show",
                &mut sink,
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
            "ldap",
            &mut sink,
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

    #[test]
    fn native_shell_entry_and_scoped_help_render_unit() {
        let native = NativeCommandRegistry::new().with_command(NativeLdapHelpCommand);
        let mut state = app_state_with_native(native);
        let invocation = base_repl_invocation(&state.runtime);

        let entered = enter_repl_shell(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            "ldap",
            &invocation,
        )
        .expect("native ldap shell should enter");
        assert!(entered.contains("Entering ldap shell"));
        assert!(state.session.scope.commands().last().is_some());

        let scoped_help = repl_help_for_scope(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            &invocation,
        )
        .expect("scoped help should render");
        assert!(scoped_help.contains("LDAP HELP"));
    }

    #[test]
    fn root_help_shortcut_supports_dsl_stages_unit() {
        let mut state = app_state();
        let invocation = base_repl_invocation(&state.runtime);
        let mut sink = BufferedUiSink::default();
        let help = ReplParsedLine::parse("help | help", state.runtime.config.resolved())
            .expect("staged help should parse");

        let rendered = maybe_handle_repl_shortcuts(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            &help,
            &invocation,
            "help | help",
            &mut sink,
        )
        .expect("staged help shortcut should succeed");

        assert!(matches!(
            rendered,
            Some(ReplLineResult::Continue(text)) if text.contains("help") || text.contains("Show this command overview")
        ));
    }
}
