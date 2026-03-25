use crate::repl::ReplLineResult;
use miette::{Result, miette};

use super::command::{
    ParsedReplDispatch, execute_repl_command_dispatch, run_repl_external_command,
};
use crate::app;
use crate::app::dispatch::resolve_external_command_source;
use crate::app::sink::UiSink;
use crate::app::{AppClients, AppRuntime, AppSession, CliCommandResult};
use crate::app::{CMD_HELP, ResolvedInvocation};
use crate::cli::invocation::scan_command_tokens;
use crate::repl::{input, lifecycle, presentation};

#[derive(Debug)]
pub(super) enum ReplShortcutPlan {
    Help {
        invocation: ResolvedInvocation,
    },
    CurrentShellHelp {
        command: String,
        invocation: ResolvedInvocation,
        warn_on_repeat: bool,
    },
    ShellEntry {
        command: String,
        invocation: ResolvedInvocation,
    },
}

pub(super) fn classify_repl_shortcut(
    runtime: &AppRuntime,
    session: &AppSession,
    parsed: &input::ReplParsedLine,
    base_invocation: &ResolvedInvocation,
) -> Result<Option<ReplShortcutPlan>> {
    // This ordering is deliberate. In an active shell, repeating the current
    // root name is a help affordance and must win before bare-root shell entry
    // is considered, otherwise `ldap` inside `(ldap)` would fall back toward
    // nested shell entry instead of answering "what does this subcommand do?".
    if let Some(current_help) = parsed.current_shell_help_command(&session.scope) {
        return Ok(Some(ReplShortcutPlan::CurrentShellHelp {
            command: current_help.command.to_string(),
            invocation: base_invocation.clone(),
            warn_on_repeat: current_help.warn_on_repeat,
        }));
    }

    if let Some(help_invocation) = repl_shortcut_help_invocation(runtime, session, parsed)? {
        return Ok(Some(ReplShortcutPlan::Help {
            invocation: help_invocation,
        }));
    }

    if let Some(command) = parsed.shell_entry_command(&session.scope) {
        return Ok(Some(ReplShortcutPlan::ShellEntry {
            command: command.to_string(),
            invocation: base_invocation.clone(),
        }));
    }

    Ok(None)
}

pub(super) fn execute_repl_shortcut(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    parsed: &input::ReplParsedLine,
    shortcut: ReplShortcutPlan,
    line: &str,
    sink: &mut dyn UiSink,
) -> Result<ReplLineResult> {
    match shortcut {
        ReplShortcutPlan::Help { invocation } => {
            repl_help_result(runtime, session, clients, parsed, &invocation, line, sink)
        }
        ReplShortcutPlan::CurrentShellHelp {
            command,
            invocation,
            warn_on_repeat,
        } => {
            let mut rendered = render_repl_help_for_scope(
                runtime,
                session,
                clients,
                &invocation,
                line,
                &parsed.stages,
                sink,
            )?;
            if warn_on_repeat {
                rendered = format!("{}{}", repeated_shell_root_help_warning(&command), rendered);
            }
            Ok(ReplLineResult::Continue(rendered))
        }
        ReplShortcutPlan::ShellEntry {
            command,
            invocation,
        } => {
            let entered = enter_repl_shell(runtime, session, clients, &command, &invocation, sink)?;
            Ok(ReplLineResult::Continue(entered))
        }
    }
}

fn repl_shortcut_help_invocation(
    runtime: &AppRuntime,
    session: &AppSession,
    parsed: &input::ReplParsedLine,
) -> Result<Option<ResolvedInvocation>> {
    if parsed.requests_repl_help() {
        return Ok(Some(app::resolve_invocation_ui(
            runtime.config.resolved(),
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
                    runtime.config.resolved(),
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
            runtime.config.resolved(),
            &runtime.ui,
            &scanned.invocation,
        )));
    }

    Ok(None)
}

fn repeated_shell_root_help_warning(command: &str) -> String {
    format!(
        "`{command}` inside the `{command}` shell was interpreted as `{command} --help`.\n\
This shell treats repeating the current root as a help shortcut.\n\
If you meant a same-named nested shell, use hidden `cd {command}`.\n\
If you meant help explicitly, use `help {command}` or `{command} --help`.\n\n"
    )
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
    Ok(ReplLineResult::Continue(render_repl_help_for_scope(
        runtime,
        session,
        clients,
        invocation,
        line,
        &parsed.stages,
        sink,
    )?))
}

pub(super) fn handle_repl_exit_request(session: &mut AppSession) -> ReplLineResult {
    match session.request_repl_exit() {
        crate::app::session::ReplExitTransition::ExitRoot => ReplLineResult::Exit(0),
        crate::app::session::ReplExitTransition::LeftShell { frame, now_root } => {
            ReplLineResult::Continue(render_repl_shell_leave_message(&frame, now_root))
        }
    }
}

#[cfg(test)]
pub(crate) fn apply_repl_shell_prefix(
    scope: &crate::app::ReplScopeStack,
    tokens: &[String],
) -> Vec<String> {
    scope.prefixed_tokens(tokens)
}

#[cfg(test)]
pub(crate) fn leave_repl_shell(session: &mut AppSession) -> Option<String> {
    match session.request_repl_exit() {
        crate::app::session::ReplExitTransition::ExitRoot => None,
        crate::app::session::ReplExitTransition::LeftShell { frame, now_root } => {
            Some(render_repl_shell_leave_message(&frame, now_root))
        }
    }
}

fn render_repl_shell_leave_message(frame: &crate::app::ReplScopeFrame, now_root: bool) -> String {
    if now_root {
        format!("Leaving {} shell. Back at root.\n", frame.command())
    } else {
        format!("Leaving {} shell.\n", frame.command())
    }
}

pub(super) fn enter_repl_shell(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    command: &str,
    invocation: &ResolvedInvocation,
    sink: &mut dyn UiSink,
) -> Result<String> {
    app::ensure_plugin_visible_for(&runtime.auth, command)?;
    let catalog = app::authorized_command_catalog_for(&runtime.auth, clients)?;
    resolve_external_command_source(
        &runtime.auth,
        clients,
        command,
        invocation.plugin_provider.as_deref(),
    )?;
    ensure_shell_entry_dispatchable(&catalog, command)?;

    session.enter_repl_scope(command.to_string());
    let mut out = format!("Entering {command} shell. Type `exit` to leave.\n");
    if let Ok(help) =
        render_repl_help_for_scope(runtime, session, clients, invocation, "", &[], sink)
    {
        out.push_str(&help);
    }
    Ok(out)
}

fn ensure_shell_entry_dispatchable(
    catalog: &[crate::plugin::CommandCatalogEntry],
    command: &str,
) -> Result<()> {
    let matching = catalog
        .iter()
        .filter(|entry| entry.name == command)
        .collect::<Vec<_>>();
    let Some(entry) = matching.first().copied() else {
        return Err(miette!("no command provides `{command}`"));
    };

    if entry.requires_selection {
        return Err(miette!(
            "command `{command}` requires provider selection; available: {}; use --plugin-provider <plugin-id> or `plugins select-provider {command} <plugin-id>`",
            entry.providers.join(", ")
        ));
    }

    Ok(())
}

pub(super) fn render_repl_help_for_scope(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    invocation: &ResolvedInvocation,
    line: &str,
    stages: &[String],
    sink: &mut dyn UiSink,
) -> Result<String> {
    let result = repl_help_command_result_for_scope(runtime, session, clients, invocation)?;
    match execute_repl_command_dispatch(
        runtime,
        session,
        clients,
        None,
        line,
        ParsedReplDispatch::Help {
            result: Box::new(result),
            effective: Box::new(invocation.clone()),
            stages: stages.to_vec(),
        },
        sink,
    )?
    .result
    {
        ReplLineResult::Continue(rendered) => Ok(rendered),
        other => Err(miette!("unexpected REPL help result: {other:?}")),
    }
}

#[cfg(test)]
pub(super) fn repl_help_for_scope(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    invocation: &ResolvedInvocation,
) -> Result<String> {
    let mut sink = crate::app::sink::BufferedUiSink::default();
    render_repl_help_for_scope(runtime, session, clients, invocation, "", &[], &mut sink)
}

fn repl_help_command_result_for_scope(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    invocation: &ResolvedInvocation,
) -> Result<CliCommandResult> {
    if session.scope.is_root() {
        let prepared = lifecycle::prepare_repl_surface_state(runtime, session, clients)?;
        return Ok(CliCommandResult::guide(
            presentation::build_repl_command_overview_view(&prepared.surface)
                .filtered_for_help_level(invocation.help_level),
        ));
    }

    let tokens = session.scope.help_tokens();
    run_repl_external_command(runtime, clients, session, tokens, invocation)
}

#[cfg(test)]
mod tests {
    use super::{
        ReplShortcutPlan, apply_repl_shell_prefix, classify_repl_shortcut, enter_repl_shell,
        execute_repl_shortcut, handle_repl_exit_request, repl_help_for_scope,
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
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(unix)]
    use std::path::Path;
    #[cfg(unix)]
    use std::time::{SystemTime, UNIX_EPOCH};

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
        app_state_with_parts(
            crate::plugin::PluginManager::new(Vec::new()),
            NativeCommandRegistry::default(),
        )
    }

    fn app_state_with_native(native_commands: NativeCommandRegistry) -> AppState {
        app_state_with_parts(
            crate::plugin::PluginManager::new(Vec::new()),
            native_commands,
        )
    }

    fn app_state_with_parts(
        plugins: crate::plugin::PluginManager,
        native_commands: NativeCommandRegistry,
    ) -> AppState {
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
            error_detail: crate::app::ErrorDetail::Terse,
            debug_verbosity: 0,
            plugins,
            native_commands,
            themes: crate::ui::theme_catalog::ThemeCatalog::default(),
            launch: LaunchContext::default(),
        })
    }

    #[cfg(unix)]
    fn make_temp_dir(prefix: &str) -> std::path::PathBuf {
        let unique = format!(
            "{prefix}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos()
        );
        let dir = std::env::temp_dir().join(unique);
        fs::create_dir_all(&dir).expect("temp dir should be created");
        dir
    }

    #[cfg(unix)]
    fn write_provider_test_plugin(dir: &Path, plugin_id: &str, command: &str) {
        let plugin_path = dir.join(format!("osp-{plugin_id}"));
        let script = format!(
            r#"#!/bin/sh
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{plugin_id}","plugin_version":"0.1.0","min_osp_version":null,"commands":[{{"name":"{command}","about":"{plugin_id} provider","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"provider":"{plugin_id}"}},"error":null,"messages":[],"meta":{{"format_hint":"table","columns":["provider"]}}}}
JSON
"#
        );
        fs::write(&plugin_path, script).expect("plugin script should be written");
        let mut perms = fs::metadata(&plugin_path)
            .expect("plugin metadata should be readable")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&plugin_path, perms).expect("plugin should be executable");
    }

    fn render_root_help_line(state: &mut AppState, line: &str) -> String {
        let invocation = base_repl_invocation(&state.runtime);
        let mut sink = BufferedUiSink::default();
        let parsed = ReplParsedLine::parse(line, state.runtime.config.resolved())
            .expect("line should parse");

        let shortcut = classify_repl_shortcut(&state.runtime, &state.session, &parsed, &invocation)
            .expect("help shortcut should classify")
            .expect("help shortcut should exist");
        let rendered = execute_repl_shortcut(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            &parsed,
            shortcut,
            line,
            &mut sink,
        )
        .expect("help shortcut should succeed");

        match rendered {
            ReplLineResult::Continue(text) => text,
            other => panic!("unexpected repl result: {other:?}"),
        }
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
            ReplLineResult::Exit(0)
        ));

        state.session.scope.enter("ldap");
        assert_eq!(
            apply_repl_shell_prefix(&state.session.scope, &["user".to_string()]),
            vec!["ldap".to_string(), "user".to_string()]
        );
        assert!(matches!(
            handle_repl_exit_request(&mut state.session),
            ReplLineResult::Continue(text) if text.contains("Leaving ldap shell")
        ));
    }

    #[test]
    fn shortcut_classification_and_execution_cover_help_none_and_shell_entry_error_unit() {
        let mut state = app_state();
        let invocation = base_repl_invocation(&state.runtime);
        let mut sink = BufferedUiSink::default();

        let help = ReplParsedLine::parse("--help", state.runtime.config.resolved())
            .expect("help should parse");
        let help_shortcut =
            classify_repl_shortcut(&state.runtime, &state.session, &help, &invocation)
                .expect("help shortcut should classify")
                .expect("help shortcut should exist");
        assert!(matches!(help_shortcut, ReplShortcutPlan::Help { .. }));
        assert!(matches!(
            execute_repl_shortcut(
                &mut state.runtime,
                &mut state.session,
                &state.clients,
                &help,
                help_shortcut,
                "--help",
                &mut sink,
            )
            .expect("help shortcut should succeed"),
            ReplLineResult::Continue(text) if text.contains("help") || text.contains("config")
        ));

        let ordinary = ReplParsedLine::parse("config show", state.runtime.config.resolved())
            .expect("ordinary command should parse");
        assert!(
            classify_repl_shortcut(&state.runtime, &state.session, &ordinary, &invocation)
                .expect("ordinary command should not shortcut")
                .is_none()
        );

        let shell_entry = ReplParsedLine::parse("ldap", state.runtime.config.resolved())
            .expect("shell entry parses");
        let shell_shortcut =
            classify_repl_shortcut(&state.runtime, &state.session, &shell_entry, &invocation)
                .expect("shell entry should classify")
                .expect("shell entry should exist");
        assert!(matches!(
            shell_shortcut,
            ReplShortcutPlan::ShellEntry { .. }
        ));
        let err = execute_repl_shortcut(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            &shell_entry,
            shell_shortcut,
            "ldap",
            &mut sink,
        )
        .expect_err("missing plugin should reject shell entry");
        assert!(err.to_string().contains("no command provides `ldap`"));

        let mut sink = BufferedUiSink::default();
        let err = enter_repl_shell(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            "ldap",
            &invocation,
            &mut sink,
        )
        .expect_err("direct shell entry should also fail");
        assert!(err.to_string().contains("no command provides `ldap`"));
    }

    #[test]
    fn native_shell_entry_and_scoped_help_render_unit() {
        let native = NativeCommandRegistry::new().with_command(NativeLdapHelpCommand);
        let mut state = app_state_with_native(native);
        let invocation = base_repl_invocation(&state.runtime);
        let mut sink = BufferedUiSink::default();

        let entered = enter_repl_shell(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            "ldap",
            &invocation,
            &mut sink,
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
    fn bare_shellable_command_dispatches_zero_arg_behavior_unit() {
        let native = NativeCommandRegistry::new().with_command(NativeLdapHelpCommand);
        let mut state = app_state_with_native(native);
        let invocation = base_repl_invocation(&state.runtime);

        let parsed =
            ReplParsedLine::parse("ldap", state.runtime.config.resolved()).expect("ldap parses");
        let shortcut = classify_repl_shortcut(&state.runtime, &state.session, &parsed, &invocation)
            .expect("bare command should classify cleanly")
            .expect("bare shellable root should enter shell");
        let mut sink = BufferedUiSink::default();
        let rendered = execute_repl_shortcut(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            &parsed,
            shortcut,
            "ldap",
            &mut sink,
        )
        .expect("bare shellable root should enter shell");

        match rendered {
            ReplLineResult::Continue(text) => assert!(text.contains("Entering ldap shell")),
            other => panic!("unexpected repl result: {other:?}"),
        }
        assert!(!state.session.scope.is_root());
    }

    #[test]
    fn repeating_current_shell_root_renders_help_with_warning_unit() {
        let native = NativeCommandRegistry::new().with_command(NativeLdapHelpCommand);
        let mut state = app_state_with_native(native);
        state.session.scope.enter("ldap");
        let invocation = base_repl_invocation(&state.runtime);
        let parsed =
            ReplParsedLine::parse("ldap", state.runtime.config.resolved()).expect("ldap parses");
        let shortcut = classify_repl_shortcut(&state.runtime, &state.session, &parsed, &invocation)
            .expect("repeated shell root should classify")
            .expect("repeated shell root should become help");
        let mut sink = BufferedUiSink::default();
        let rendered = execute_repl_shortcut(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            &parsed,
            shortcut,
            "ldap",
            &mut sink,
        )
        .expect("repeated shell root should render help");

        match rendered {
            ReplLineResult::Continue(text) => {
                assert!(text.contains("was interpreted as `ldap --help`"));
                assert!(text.contains("hidden `cd ldap`"));
                assert!(text.contains("LDAP HELP"));
            }
            other => panic!("unexpected repl result: {other:?}"),
        }
        assert_eq!(state.session.scope.commands(), &["ldap".to_string()]);
    }

    #[test]
    fn hidden_cd_still_forces_same_named_nested_shell_entry_unit() {
        let native = NativeCommandRegistry::new().with_command(NativeLdapHelpCommand);
        let mut state = app_state_with_native(native);
        state.session.scope.enter("ldap");
        let invocation = base_repl_invocation(&state.runtime);
        let parsed = ReplParsedLine::parse("cd ldap", state.runtime.config.resolved())
            .expect("hidden cd should parse");
        let shortcut = classify_repl_shortcut(&state.runtime, &state.session, &parsed, &invocation)
            .expect("hidden cd should classify")
            .expect("hidden cd should remain available");
        let mut sink = BufferedUiSink::default();
        let rendered = execute_repl_shortcut(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            &parsed,
            shortcut,
            "cd ldap",
            &mut sink,
        )
        .expect("hidden cd should enter a nested shell");

        match rendered {
            ReplLineResult::Continue(text) => assert!(text.contains("Entering ldap shell")),
            other => panic!("unexpected repl result: {other:?}"),
        }
        assert_eq!(
            state.session.scope.commands(),
            &["ldap".to_string(), "ldap".to_string()]
        );
    }

    #[cfg(unix)]
    #[test]
    fn ambiguous_plugin_shell_entry_fails_before_scope_mutation_unit() {
        let root = make_temp_dir("osp-cli-repl-shell-ambiguous-plugin");
        let plugins_dir = root.join("plugins");
        fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");
        write_provider_test_plugin(&plugins_dir, "alpha", "hello");
        write_provider_test_plugin(&plugins_dir, "beta", "hello");
        let plugin_manager = crate::plugin::PluginManager::new(vec![plugins_dir]);
        let mut state = app_state_with_parts(plugin_manager, NativeCommandRegistry::default());
        let invocation = base_repl_invocation(&state.runtime);
        let mut sink = BufferedUiSink::default();

        let err = enter_repl_shell(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            "hello",
            &invocation,
            &mut sink,
        )
        .expect_err("ambiguous plugin shell entry should fail");
        assert!(err.to_string().contains("requires provider selection"));
        assert!(state.session.scope.is_root());

        fs::remove_dir_all(root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn native_and_plugin_shell_collision_fails_before_scope_mutation_unit() {
        let root = make_temp_dir("osp-cli-repl-shell-native-collision");
        let plugins_dir = root.join("plugins");
        fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");
        write_provider_test_plugin(&plugins_dir, "alpha", "ldap");
        let plugin_manager = crate::plugin::PluginManager::new(vec![plugins_dir]);
        let native = NativeCommandRegistry::new().with_command(NativeLdapHelpCommand);
        let mut state = app_state_with_parts(plugin_manager, native);
        let invocation = base_repl_invocation(&state.runtime);
        let mut sink = BufferedUiSink::default();

        let err = enter_repl_shell(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            "ldap",
            &invocation,
            &mut sink,
        )
        .expect_err("native/plugin collision should fail");
        assert!(err.to_string().contains("ambiguous across command sources"));
        assert!(state.session.scope.is_root());

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn root_help_shortcut_supports_dsl_stages_unit() {
        let mut state = app_state();
        let invocation = base_repl_invocation(&state.runtime);
        let mut sink = BufferedUiSink::default();
        let help = ReplParsedLine::parse("help | help", state.runtime.config.resolved())
            .expect("staged help should parse");
        let shortcut = classify_repl_shortcut(&state.runtime, &state.session, &help, &invocation)
            .expect("staged help shortcut should classify")
            .expect("staged help shortcut should exist");
        let rendered = execute_repl_shortcut(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            &help,
            shortcut,
            "help | help",
            &mut sink,
        )
        .expect("staged help shortcut should succeed");

        assert!(matches!(
            rendered,
            ReplLineResult::Continue(text) if text.contains("help") || text.contains("Show this command overview")
        ));
    }

    #[test]
    fn root_help_shortcut_supports_explicit_value_format_with_dsl_unit() {
        let mut state = app_state();
        let rendered = render_root_help_line(&mut state, "--value help | doctor");
        assert!(
            !rendered.contains("[INVOCATION_OPTIONS] COMMAND [ARGS]..."),
            "quick should prune unmatched root siblings in value mode: {rendered:?}"
        );
        assert!(rendered.contains("Run diagnostics checks"));
        assert!(
            !rendered.contains("Show this command overview."),
            "unexpected extra value-mode payload: {rendered:?}"
        );
        assert!(!rendered.contains("Commands"));
    }

    #[test]
    fn root_help_quick_filter_uses_overview_descriptions_not_subcommand_inventory_unit() {
        let mut state = app_state();
        let rendered = render_root_help_line(&mut state, "--json help | doctor");
        let value: serde_json::Value =
            serde_json::from_str(&rendered).expect("staged help json should parse");
        let rows = value
            .as_array()
            .expect("staged help json should be row array");
        assert_eq!(rows.len(), 1);
        assert!(
            rows[0].get("usage").is_none(),
            "filtered root help should not retain unrelated usage envelope",
        );
        let commands = rows[0]["commands"]
            .as_array()
            .expect("help row should contain commands");
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0]["name"], "doctor");
        assert_eq!(commands[0]["short_help"], "Run diagnostics checks");
    }

    #[test]
    fn root_help_shortcut_supports_explicit_formats_with_pipeline_unit() {
        let mut json_state = app_state();
        let json = render_root_help_line(&mut json_state, "--json help | L 1");
        let json_value: serde_json::Value =
            serde_json::from_str(&json).expect("staged help json should parse");
        let json_rows = json_value
            .as_array()
            .expect("staged help json should be row array");
        assert_eq!(json_rows.len(), 1);
        assert!(json_rows[0].get("usage").is_some());

        let mut guide_state = app_state();
        let guide = render_root_help_line(&mut guide_state, "--guide help | L 1");
        assert!(guide.contains("Usage"));
        assert!(guide.contains("Commands"));

        let mut markdown_state = app_state();
        let markdown = render_root_help_line(&mut markdown_state, "--md help | L 1");
        assert!(markdown.contains("## Usage"));
        assert!(markdown.contains("## Commands"));
        assert!(!markdown.contains("- `cd` Enter a shellable command scope."));
        assert!(!markdown.contains("| name"));

        let mut table_state = app_state();
        let table = render_root_help_line(&mut table_state, "--table help | L 1");
        assert!(table.contains("usage") || table.contains("Usage"));
        assert!(table.contains("commands"));

        let mut mreg_state = app_state();
        let mreg = render_root_help_line(&mut mreg_state, "--mreg help | L 1");
        assert!(mreg.contains("usage:") || mreg.contains("Usage"));
        assert!(
            mreg.contains("commands:") || mreg.contains("commands (") || mreg.contains("Commands")
        );
    }
}
