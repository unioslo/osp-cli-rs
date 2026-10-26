use crate::repl::{ReplReloadKind, ReplResult};
use miette::{Result, miette};

use super::command::run_repl_external_command;
use crate::app;
use crate::app::dispatch::{
    ExternalPathAccessRequirement, before_external_run, ensure_external_access,
    resolve_external_command_source,
};
use crate::app::sink::UiSink;
use crate::app::{AppRuntime, AppSession, CommandOutput};
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
    // re-entering the same shell instead of answering "what does this subcommand do?".
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

    if let Some(command) = parsed.shell_entry_command(&session.scope, runtime.config.resolved()) {
        return Ok(Some(ReplShortcutPlan::ShellEntry {
            command: command.to_string(),
            invocation: base_invocation.clone(),
        }));
    }

    Ok(None)
}

pub(super) fn execute_repl_shortcut(
    state: &mut crate::app::AppState,
    parsed: &input::ReplParsedLine,
    shortcut: ReplShortcutPlan,
    line: &str,
    sink: &mut dyn UiSink,
) -> Result<ReplResult> {
    match shortcut {
        ReplShortcutPlan::Help { invocation } => {
            repl_help_result(state, parsed, &invocation, line, sink)
        }
        ReplShortcutPlan::CurrentShellHelp {
            command,
            invocation,
            warn_on_repeat,
        } => {
            let mut rendered =
                render_repl_help_for_scope(state, &invocation, line, &parsed.stages, sink)?;
            if warn_on_repeat {
                rendered = format!("{}{}", repeated_shell_root_help_warning(&command), rendered);
            }
            Ok(ReplResult::Print(rendered))
        }
        ReplShortcutPlan::ShellEntry {
            command,
            invocation,
        } => {
            let entered = enter_repl_shell(state, &command, &invocation, sink)?;
            Ok(ReplResult::Restart {
                output: entered,
                reload: ReplReloadKind::Completion,
            })
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
If you meant help explicitly, use `help {command}` or `{command} --help`.\n\n"
    )
}

fn repl_help_result(
    state: &mut crate::app::AppState,
    parsed: &input::ReplParsedLine,
    invocation: &ResolvedInvocation,
    line: &str,
    sink: &mut dyn UiSink,
) -> Result<ReplResult> {
    Ok(ReplResult::Print(render_repl_help_for_scope(
        state,
        invocation,
        line,
        &parsed.stages,
        sink,
    )?))
}

pub(super) fn handle_repl_exit_request(session: &mut AppSession) -> ReplResult {
    match session.request_repl_exit() {
        crate::app::session::ReplExitTransition::ExitRoot => ReplResult::Exit(0),
        crate::app::session::ReplExitTransition::LeftShell { frame } => ReplResult::Restart {
            output: render_repl_shell_leave_message(&frame),
            reload: ReplReloadKind::Completion,
        },
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) fn apply_repl_shell_prefix(
    scope: &crate::app::ReplScopeStack,
    tokens: &[String],
) -> Vec<String> {
    scope.prefixed_tokens(tokens)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) fn leave_repl_shell(session: &mut AppSession) -> Option<String> {
    match session.request_repl_exit() {
        crate::app::session::ReplExitTransition::ExitRoot => None,
        crate::app::session::ReplExitTransition::LeftShell { frame } => {
            Some(render_repl_shell_leave_message(&frame))
        }
    }
}

fn render_repl_shell_leave_message(frame: &crate::app::ReplScopeFrame) -> String {
    format!("Leaving {} shell. Back at root.\n", frame.command())
}

pub(super) fn enter_repl_shell(
    state: &mut crate::app::AppState,
    command: &str,
    invocation: &ResolvedInvocation,
    sink: &mut dyn UiSink,
) -> Result<String> {
    let path = crate::core::command_policy::CommandPath::new([command]);
    before_external_run(
        &mut state.runtime,
        &path,
        ExternalPathAccessRequirement::Runnable,
        "command",
    )?;
    state.reload_config_after_access_hook()?;
    ensure_external_access(
        &state.runtime,
        &path,
        ExternalPathAccessRequirement::Runnable,
        "command",
    )?;
    let catalog = app::authorized_command_catalog_for(
        &state.runtime.auth,
        &state.clients,
        state.clients.native_commands().catalog(),
    );
    resolve_external_command_source(
        &state.runtime.auth,
        &state.clients,
        command,
        &[],
        &invocation.ui.render_settings.runtime.command_scope,
    )?;
    ensure_shell_entry_dispatchable(
        &catalog,
        command,
        &invocation.ui.render_settings.runtime.command_scope,
    )?;

    // Shells are root commands: entering one switches from the current shell.
    let mut out = state
        .session
        .scope
        .commands()
        .last()
        .map(|current| format!("Leaving {current} shell. "))
        .unwrap_or_default();
    state.session.enter_repl_scope(command.to_string());
    out.push_str(&format!(
        "Entering {command} shell. Type `..` or `exit` to leave.\n"
    ));
    if let Ok(help) = render_repl_help_for_scope(state, invocation, "", &[], sink) {
        out.push_str(&help);
    }
    Ok(out)
}

fn ensure_shell_entry_dispatchable(
    catalog: &[crate::plugin::CommandCatalogEntry],
    command: &str,
    _scope: &crate::core::CommandScope,
) -> Result<()> {
    let matching = catalog
        .iter()
        .filter(|entry| entry.name == command)
        .collect::<Vec<_>>();
    let Some(_) = matching.first().copied() else {
        return Err(miette!("no command provides `{command}`"));
    };

    Ok(())
}

pub(super) fn render_repl_help_for_scope(
    state: &mut crate::app::AppState,
    invocation: &ResolvedInvocation,
    line: &str,
    stages: &[String],
    sink: &mut dyn UiSink,
) -> Result<String> {
    let result = repl_help_command_result_for_scope(state, invocation)?;
    super::command::render_repl_command_output(
        &state.runtime,
        &mut state.session,
        line,
        stages,
        result,
        invocation,
        sink,
    )
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
pub(super) fn repl_help_for_scope(
    state: &mut crate::app::AppState,
    invocation: &ResolvedInvocation,
) -> Result<String> {
    let mut sink = crate::app::sink::BufferedUiSink::default();
    render_repl_help_for_scope(state, invocation, "", &[], &mut sink)
}

fn repl_help_command_result_for_scope(
    state: &mut crate::app::AppState,
    invocation: &ResolvedInvocation,
) -> Result<CommandOutput> {
    if state.session.scope.is_root() {
        let prepared =
            lifecycle::prepare_repl_surface_state(&state.runtime, &state.session, &state.clients)?;
        let mut document =
            presentation::build_repl_help_view(&prepared.surface, invocation.help_level);
        let actions = lifecycle::resolved_product_actions(&state.runtime, &state.clients)?;
        for action in actions {
            if let Some(key) = action.key {
                document
                    .blocks
                    .push(crate::core::document::DocBlock::Paragraph {
                        text: format!("`F{key}` {}", action.label),
                    });
            }
        }
        return Ok(CommandOutput::output(
            presentation::repl_document_output(&document),
            None,
        ));
    }

    let tokens = state.session.scope.help_tokens();
    run_repl_external_command(state, tokens, invocation)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::{
        ReplShortcutPlan, apply_repl_shell_prefix, classify_repl_shortcut, enter_repl_shell,
        execute_repl_shortcut, handle_repl_exit_request, repl_help_for_scope,
    };
    use crate::app::sink::BufferedUiSink;
    use crate::app::{AppState, LaunchContext, RuntimeContext, TerminalKind};
    use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions};
    use crate::core::output::OutputFormat;
    use crate::core::plugin::{DescribeCommandAuthV1, DescribeVisibilityModeV1};
    use crate::native::{
        NativeCommand, NativeCommandContext, NativeCommandOutcome, NativeCommandRegistry,
    };
    use crate::repl::dispatch::base_repl_invocation;
    use crate::repl::input::ReplParsedLine;
    use crate::repl::{ReplReloadKind, ReplResult};
    use crate::ui::RenderSettings;
    use crate::ui::messages::MessageLevel;
    use clap::Command;
    #[cfg(unix)]
    #[cfg(unix)]
    #[cfg(unix)]
    #[cfg(all(unix, miri))]
    use std::sync::atomic::{AtomicU64, Ordering};
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
                ..DescribeCommandAuthV1::default()
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
            crate::plugin::PluginManager::new(Vec::new())
                .with_bundled_roots(false)
                .with_default_roots(false),
            NativeCommandRegistry::default(),
        )
    }

    fn app_state_with_native(native_commands: NativeCommandRegistry) -> AppState {
        app_state_with_parts(
            crate::plugin::PluginManager::new(Vec::new())
                .with_bundled_roots(false)
                .with_default_roots(false),
            native_commands,
        )
    }

    fn app_state_with_parts(
        plugins: crate::plugin::PluginManager,
        native_commands: NativeCommandRegistry,
    ) -> AppState {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        defaults.set("repl.history.path", "/tmp/osp-repl-shell-history.jsonl");
        defaults.set("repl.shellable_commands", vec!["ldap".to_string()]);
        defaults.set("theme.path", Vec::<String>::new());
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        let config = resolver
            .resolve(ResolveOptions::default().with_terminal("repl"))
            .expect("test config should resolve");

        crate::app::AppStateBuilder {
            context: RuntimeContext::new(None, TerminalKind::Repl, None),
            config,
            launch: LaunchContext::default(),
            native_commands,
            ui: crate::app::UiState::new(
                RenderSettings::test_plain(OutputFormat::Json),
                MessageLevel::Success,
                0,
            ),
            plugins: Some(plugins),
            themes: Some(crate::ui::theme_catalog::ThemeCatalog::default()),
            session: None,
            definition: Default::default(),
        }
        .build()
    }
    fn render_root_help_line(state: &mut AppState, line: &str) -> String {
        let invocation = base_repl_invocation(&state.runtime);
        let mut sink = BufferedUiSink::default();
        let parsed = ReplParsedLine::parse(line, state.runtime.config.resolved())
            .expect("line should parse");

        let shortcut = classify_repl_shortcut(&state.runtime, &state.session, &parsed, &invocation)
            .expect("help shortcut should classify")
            .expect("help shortcut should exist");
        let rendered = execute_repl_shortcut(state, &parsed, shortcut, line, &mut sink)
            .expect("help shortcut should succeed");

        match rendered {
            ReplResult::Print(text) => text,
            other => panic!("unexpected repl result: {other:?}"),
        }
    }

    #[test]
    fn command_scope_reaches_repl_hint_rendering() {
        let mut registry = NativeCommandRegistry::default();
        registry.register(NativeLdapHelpCommand);
        let mut state = app_state_with_native(registry);
        let history = crate::repl::SharedHistory::new(crate::repl::HistoryConfig {
            enabled: false,
            ..Default::default()
        });
        let hint = |state: &mut AppState, line: &str| {
            let mut sink = BufferedUiSink::default();
            crate::repl::dispatch::execute_repl_plugin_line(state, &history, line, Some(&mut sink))
                .expect("dispatch should render the usage error");
            let output: serde_json::Value = serde_json::from_str(&sink.stdout).unwrap();
            output["error"]["details"]["action"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert!(hint(&mut state, "ldap usr").contains("Did you mean ldap user?"));
        state.session.scope.enter("ldap");
        assert!(hint(&mut state, "usr").contains("Did you mean user?"));
    }

    #[test]
    fn shell_helpers_cover_prefix_exit_and_root_help_unit() {
        let mut state = app_state();
        let invocation = base_repl_invocation(&state.runtime);
        let help = repl_help_for_scope(&mut state, &invocation).expect("root help should render");
        assert!(help.contains("help") || help.contains("config"));

        assert_eq!(
            apply_repl_shell_prefix(&state.session.scope, &["user".to_string()]),
            vec!["user".to_string()]
        );
        assert!(matches!(
            handle_repl_exit_request(&mut state.session),
            ReplResult::Exit(0)
        ));

        state.session.scope.enter("ldap");
        assert_eq!(
            apply_repl_shell_prefix(&state.session.scope, &["user".to_string()]),
            vec!["ldap".to_string(), "user".to_string()]
        );
        assert!(matches!(
            handle_repl_exit_request(&mut state.session),
            ReplResult::Restart {
                output: text,
                reload: ReplReloadKind::Completion,
            } if text.contains("Leaving ldap shell")
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
                &mut state,
                &help,
                help_shortcut,
                "--help",
                &mut sink,
            )
            .expect("help shortcut should succeed"),
            ReplResult::Print(text) if text.contains("help") || text.contains("config")
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
        let err =
            execute_repl_shortcut(&mut state, &shell_entry, shell_shortcut, "ldap", &mut sink)
                .expect_err("missing plugin should reject shell entry");
        assert!(err.to_string().contains("ldap"));

        let mut sink = BufferedUiSink::default();
        let err = enter_repl_shell(&mut state, "ldap", &invocation, &mut sink)
            .expect_err("direct shell entry should also fail");
        assert!(err.to_string().contains("ldap"));
    }

    #[test]
    fn native_shell_entry_and_scoped_help_render_unit() {
        let native = NativeCommandRegistry::new().with_command(NativeLdapHelpCommand);
        let mut state = app_state_with_native(native);
        let invocation = base_repl_invocation(&state.runtime);
        let mut sink = BufferedUiSink::default();

        let entered = enter_repl_shell(&mut state, "ldap", &invocation, &mut sink)
            .expect("native ldap shell should enter");
        assert!(entered.contains("Entering ldap shell"));
        assert!(state.session.scope.commands().last().is_some());

        let scoped_help =
            repl_help_for_scope(&mut state, &invocation).expect("scoped help should render");
        assert!(scoped_help.contains("Directory lookup"));
        assert!(scoped_help.contains("user"));
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
        let rendered = execute_repl_shortcut(&mut state, &parsed, shortcut, "ldap", &mut sink)
            .expect("bare shellable root should enter shell");

        match rendered {
            ReplResult::Restart {
                output: text,
                reload: ReplReloadKind::Completion,
            } => assert!(text.contains("Entering ldap shell")),
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
        let rendered = execute_repl_shortcut(&mut state, &parsed, shortcut, "ldap", &mut sink)
            .expect("repeated shell root should render help");

        match rendered {
            ReplResult::Print(text) => {
                assert!(text.contains("was interpreted as `ldap --help`"));
                assert!(text.contains("Directory lookup"));
            }
            other => panic!("unexpected repl result: {other:?}"),
        }
        assert_eq!(state.session.scope.commands(), &["ldap".to_string()]);
    }

    #[test]
    fn entering_another_shell_replaces_the_current_scope_unit() {
        let native = NativeCommandRegistry::new().with_command(NativeLdapHelpCommand);
        let mut state = app_state_with_native(native);
        state.session.scope.enter("orch");
        let invocation = base_repl_invocation(&state.runtime);
        let parsed = ReplParsedLine::parse("ldap", state.runtime.config.resolved())
            .expect("shell root should parse");
        let shortcut = classify_repl_shortcut(&state.runtime, &state.session, &parsed, &invocation)
            .expect("shell root should classify")
            .expect("shell root should remain available");
        let mut sink = BufferedUiSink::default();
        let rendered = execute_repl_shortcut(&mut state, &parsed, shortcut, "ldap", &mut sink)
            .expect("shell root should switch shells");

        match rendered {
            ReplResult::Restart {
                output: text,
                reload: ReplReloadKind::Completion,
            } => assert!(text.contains("Entering ldap shell")),
            other => panic!("unexpected repl result: {other:?}"),
        }
        assert_eq!(state.session.scope.commands(), &["ldap".to_string()]);
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
        let rendered = execute_repl_shortcut(&mut state, &help, shortcut, "help | help", &mut sink)
            .expect("staged help shortcut should succeed");

        assert!(matches!(
            rendered,
            ReplResult::Print(text) if text.contains("help") || text.contains("Show this command overview")
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
        assert_eq!(rows[0]["name"], "doctor");
        assert_eq!(rows[0]["description"], "Run diagnostics checks");
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
        assert!(json_rows[0].get("text").is_some());

        let mut guide_state = app_state();
        let guide = render_root_help_line(&mut guide_state, "--format auto help | L 1");
        assert!(guide.contains("Usage"));
        assert!(!guide.contains("Commands"));

        let mut markdown_state = app_state();
        let markdown = render_root_help_line(&mut markdown_state, "--md help | L 1");
        assert!(markdown.contains("## Usage"));
        assert!(!markdown.contains("## Commands"));
        assert!(!markdown.contains("- `cd` Enter a shellable command scope."));
        assert!(!markdown.contains("| name"));

        let mut table_state = app_state();
        let table = render_root_help_line(&mut table_state, "--table help | L 1");
        assert!(table.contains("text"));
        assert!(table.contains("COMMAND"));

        let mut mreg_state = app_state();
        let mreg = render_root_help_line(&mut mreg_state, "--mreg help | L 1");
        assert!(mreg.contains("usage:") || mreg.contains("Usage"));
        assert!(!mreg.contains("Commands"));
    }
}
