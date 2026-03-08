mod builtins;
mod command;
mod shell;

use crate::repl::{ReplLineResult, SharedHistory};
use miette::Result;
use std::time::Instant;

use crate::app::sink::{StdIoUiSink, UiSink};
use crate::app::{AppClients, AppRuntime, AppSession};
use crate::app::{ResolvedInvocation, resolve_invocation_ui};

use super::{ReplViewContext, completion, input};

use builtins::{is_repl_bang_request, maybe_execute_repl_builtin};
use command::{
    ParsedReplDispatch, finalize_repl_command, parse_repl_invocation, render_repl_command_output,
    run_repl_command,
};
use shell::maybe_handle_repl_shortcuts;

#[cfg(test)]
use builtins::{
    BangCommand, ReplBuiltin, current_history_scope, execute_bang_command, parse_bang_command,
    parse_repl_builtin, strip_history_scope,
};
pub(crate) use command::repl_command_spec;
#[cfg(test)]
use command::{
    command_side_effects, config_key_change_requires_intro, parse_clap_help,
    renders_repl_inline_help,
};
#[cfg(test)]
pub(crate) use shell::apply_repl_shell_prefix;
#[cfg(test)]
pub(crate) use shell::leave_repl_shell;
#[cfg(test)]
use shell::{enter_repl_shell, handle_repl_exit_request, repl_help_for_scope};

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

fn base_repl_invocation(runtime: &AppRuntime) -> ResolvedInvocation {
    resolve_invocation_ui(&runtime.ui, &Default::default())
}

#[cfg(test)]
mod tests {
    use crate::core::output::OutputFormat;
    use crate::repl::{HistoryConfig, ReplLineResult, ReplReloadKind, SharedHistory};
    use crate::ui::RenderSettings;
    use crate::ui::messages::MessageLevel;
    use clap::error::ErrorKind;
    use insta::assert_snapshot;

    use super::{
        BangCommand, command_side_effects, config_key_change_requires_intro, current_history_scope,
        enter_repl_shell, execute_bang_command, finalize_repl_command, handle_repl_exit_request,
        is_repl_bang_request, leave_repl_shell, parse_bang_command, parse_clap_help,
        parse_repl_builtin, render_repl_command_output, renders_repl_inline_help,
        repl_command_spec, repl_help_for_scope, run_repl_command, strip_history_scope,
    };
    use crate::app::{
        AppSession, AppState, AppStateInit, LaunchContext, RuntimeContext, TerminalKind,
    };
    use crate::app::{CliCommandResult, ReplCommandOutput};
    use crate::cli::{
        Commands, ConfigArgs, ConfigCommands, ConfigSetArgs, ConfigUnsetArgs, DebugCompleteArgs,
        HistoryArgs, HistoryCommands, PluginsArgs, PluginsCommands, ReplArgs, ReplCommands,
        ThemeArgs, ThemeCommands, ThemeUseArgs,
    };
    use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions};

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

    fn make_state_with_plugins(plugins: crate::plugin::PluginManager) -> AppState {
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
            native_commands: crate::native::NativeCommandRegistry::default(),
            themes: crate::ui::theme_loader::ThemeCatalog::default(),
            launch: LaunchContext::default(),
        })
    }

    #[test]
    fn root_help_rendering_and_shell_prefix_helpers_cover_root_paths_unit() {
        let mut state = make_state_with_plugins(crate::plugin::PluginManager::new(Vec::new()));
        let invocation = super::base_repl_invocation(&state.runtime);
        let help = repl_help_for_scope(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            &invocation,
        )
        .expect("root help should render");
        assert!(help.contains("help"));
        assert_snapshot!("repl_root_help", help);
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
        use crate::app::sink::BufferedUiSink;

        let mut state = make_state_with_plugins(crate::plugin::PluginManager::new(Vec::new()));
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
            r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
if [ "$1" = "--describe" ]; then
  printf '%s\n' '{"protocol_version":1,"plugin_id":"cache","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{"name":"cache","about":"cache plugin","args":[],"flags":{},"subcommands":[]}]}'
  exit 0
fi
printf '%s\n' '{"protocol_version":1,"ok":true,"data":{"message":"ok"},"error":null,"meta":{"format_hint":"table","columns":["message"]}}'
"#,
        )
        .expect("plugin script should be written");
        let mut perms = std::fs::metadata(&plugin_path)
            .expect("plugin metadata should be readable")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&plugin_path, perms).expect("plugin should be executable");

        let mut state =
            make_state_with_plugins(crate::plugin::PluginManager::new(vec![plugins_dir.clone()]));
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
