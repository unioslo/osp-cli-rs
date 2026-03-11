use clap::error::ErrorKind;
use insta::assert_snapshot;
use std::path::PathBuf;

use super::{
    BangCommand, ReplLinePlanKind, classify_repl_line_kind, command_side_effects,
    config_key_change_requires_intro, current_history_scope, enter_repl_shell,
    execute_bang_command, execute_repl_plugin_line, finalize_repl_command,
    handle_repl_exit_request, is_repl_bang_request, leave_repl_shell, parse_bang_command,
    parse_clap_help, parse_repl_builtin, render_repl_command_output, renders_repl_inline_help,
    repl_command_spec, repl_help_for_scope, run_repl_command, strip_history_scope,
};
use crate::app::{AppSession, AppState, AppStateInit, LaunchContext, RuntimeContext, TerminalKind};
use crate::app::{CliCommandResult, ReplCommandOutput};
use crate::cli::{
    Commands, ConfigArgs, ConfigCommands, ConfigSetArgs, ConfigUnsetArgs, DebugCompleteArgs,
    HistoryArgs, HistoryCommands, IntroArgs, PluginsArgs, PluginsCommands, ReplArgs, ReplCommands,
    ThemeArgs, ThemeCommands, ThemeUseArgs,
};
use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions};
use crate::core::output::OutputFormat;
use crate::repl::{HistoryConfig, ReplLineResult, ReplReloadKind, SharedHistory};
use crate::ui::RenderSettings;
use crate::ui::messages::MessageLevel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ObservedDispatchKind {
    Blank,
    Exit,
    DslHelp,
    ShortcutHelp,
    Command,
    InlineHelp,
    Error,
}

fn observe_dispatch_kind(
    state: &mut AppState,
    history: &SharedHistory,
    line: &str,
) -> ObservedDispatchKind {
    match execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        history,
        line,
    ) {
        Ok(ReplLineResult::Exit(_)) => ObservedDispatchKind::Exit,
        Ok(ReplLineResult::Continue(output)) if output.is_empty() => ObservedDispatchKind::Blank,
        Ok(ReplLineResult::Continue(output)) if output.contains("DSL Help") => {
            ObservedDispatchKind::DslHelp
        }
        Ok(ReplLineResult::Continue(output))
            if line.trim() == "help" && output.contains("Commands") =>
        {
            ObservedDispatchKind::ShortcutHelp
        }
        Ok(ReplLineResult::Continue(output))
            if output.contains("unrecognized subcommand")
                || output.contains("unknown argument") =>
        {
            ObservedDispatchKind::InlineHelp
        }
        Ok(ReplLineResult::ReplaceInput(_)) | Ok(ReplLineResult::Restart { .. }) => {
            ObservedDispatchKind::Command
        }
        Ok(ReplLineResult::Continue(_)) => ObservedDispatchKind::Command,
        Err(_) => ObservedDispatchKind::Error,
    }
}

fn dispatch_snapshot_settings() -> insta::Settings {
    let mut settings = insta::Settings::clone_current();
    settings.set_snapshot_path(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("repl")
            .join("snapshots"),
    );
    settings
}

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

    let plugins_enable = Commands::Plugins(PluginsArgs {
        command: PluginsCommands::Enable(crate::cli::PluginCommandStateArgs {
            command: "orch".to_string(),
            global: false,
            profile: None,
            terminal: None,
        }),
    });
    let plugins_enable_effects = command_side_effects(&plugins_enable);
    assert!(plugins_enable_effects.restart_repl);
    assert!(!plugins_enable_effects.show_intro_on_reload);

    let plugins_refresh = Commands::Plugins(PluginsArgs {
        command: PluginsCommands::Refresh,
    });
    let plugins_refresh_effects = command_side_effects(&plugins_refresh);
    assert!(plugins_refresh_effects.restart_repl);
    assert!(!plugins_refresh_effects.show_intro_on_reload);
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
    let history = SharedHistory::new(
        HistoryConfig::builder()
            .with_max_entries(20)
            .with_enabled(true)
            .with_dedupe(true)
            .with_profile_scoped(false)
            .with_shell_context(Default::default())
            .build(),
    )
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

fn test_history() -> SharedHistory {
    SharedHistory::new(
        HistoryConfig::builder()
            .with_max_entries(8)
            .with_enabled(true)
            .with_dedupe(true)
            .with_profile_scoped(false)
            .with_shell_context(Default::default())
            .build(),
    )
    .expect("history should initialize")
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
    dispatch_snapshot_settings().bind(|| {
        insta::with_settings!({ snapshot_path => "../snapshots" }, {
            assert_snapshot!("repl_root_help", help);
        });
    });
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
fn intro_pipeline_keeps_filtered_guide_structure_unit() {
    let mut state = make_state_with_plugins(crate::plugin::PluginManager::new(Vec::new()));
    let invocation = super::base_repl_invocation(&state.runtime);
    let result = run_repl_command(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        Commands::Intro(IntroArgs::default()),
        &invocation,
        &test_history(),
        None,
    )
    .expect("intro command should succeed");

    let mut sink = crate::app::sink::BufferedUiSink::default();
    let rendered = render_repl_command_output(
        &state.runtime,
        &mut state.session,
        "intro | show",
        &["show".to_string()],
        result,
        &invocation,
        &mut sink,
    )
    .expect("intro pipeline should render");

    assert!(rendered.contains("Commands"));
    assert!(rendered.contains("help"));
    assert!(!rendered.contains("Sections"));
    assert!(!rendered.contains("Entries"));
}

#[test]
fn staged_command_parse_errors_do_not_become_pipeline_input_unit() {
    let mut state = make_state_with_plugins(crate::plugin::PluginManager::new(Vec::new()));
    let err = execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &test_history(),
        "intro --wat | config",
    )
    .expect_err("invalid staged invocation should fail before piping");

    assert!(err.to_string().contains("--wat"));
}

#[test]
fn staged_invalid_help_alias_does_not_become_pipeline_input_unit() {
    let mut state = make_state_with_plugins(crate::plugin::PluginManager::new(Vec::new()));
    let err = execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &test_history(),
        "help --help | config",
    )
    .expect_err("invalid staged help alias should fail before piping");

    assert!(err.to_string().contains("invalid help target: --help"));
}

#[test]
fn execute_repl_plugin_line_covers_builtin_blank_dsl_and_help_shortcuts_unit() {
    let mut state = make_state_with_plugins(crate::plugin::PluginManager::new(Vec::new()));
    state.runtime.ui.debug_verbosity = 1;
    let history = test_history();

    assert!(matches!(
        execute_repl_plugin_line(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            &history,
            "quit"
        )
        .expect("quit should exit from the REPL root"),
        ReplLineResult::Exit(0)
    ));
    assert!(state.session.prompt_timing.badge().is_some());

    assert!(matches!(
        execute_repl_plugin_line(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            &history,
            "   "
        )
        .expect("blank lines should be ignored"),
        ReplLineResult::Continue(output) if output.is_empty()
    ));

    assert!(matches!(
        execute_repl_plugin_line(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            &history,
            "intro | H"
        )
        .expect("dsl help stage should short-circuit normal command execution"),
        ReplLineResult::Continue(output)
            if output.contains("DSL Help")
                && output.contains("Use | H <verb> for details.")
    ));

    assert!(matches!(
        execute_repl_plugin_line(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            &history,
            "help"
        )
        .expect("root help shortcut should render command help"),
        ReplLineResult::Continue(output)
            if output.contains("Commands") && output.contains("config")
    ));
}

#[test]
fn repl_dispatch_characterization_covers_representative_line_categories_unit() {
    let cases = [
        ("   ", ObservedDispatchKind::Blank),
        ("quit", ObservedDispatchKind::Exit),
        ("intro | H", ObservedDispatchKind::DslHelp),
        ("help", ObservedDispatchKind::ShortcutHelp),
        ("intro", ObservedDispatchKind::Command),
        ("config sho", ObservedDispatchKind::InlineHelp),
        ("intro --wat | config", ObservedDispatchKind::Error),
    ];

    for (line, expected) in cases {
        let mut state = make_state_with_plugins(crate::plugin::PluginManager::new(Vec::new()));
        let observed = observe_dispatch_kind(&mut state, &test_history(), line);
        assert_eq!(observed, expected, "line: {line}");
    }
}

#[test]
fn repl_line_plan_classifier_covers_representative_categories_unit() {
    let cases = [
        ("   ", ReplLinePlanKind::Blank),
        ("quit", ReplLinePlanKind::Builtin),
        ("help", ReplLinePlanKind::Builtin),
        ("intro | H", ReplLinePlanKind::DslHelp),
        ("nh", ReplLinePlanKind::Shortcut),
        ("config sho", ReplLinePlanKind::Help),
        ("intro", ReplLinePlanKind::Invocation),
    ];

    for (line, expected) in cases {
        let state = make_state_with_plugins(crate::plugin::PluginManager::new(Vec::new()));
        let observed = classify_repl_line_kind(&state.runtime, &state.session, line)
            .expect("classification should succeed");
        assert_eq!(observed, expected, "line: {line}");
    }

    let state = make_state_with_plugins(crate::plugin::PluginManager::new(Vec::new()));
    let err = classify_repl_line_kind(&state.runtime, &state.session, "intro --wat | config")
        .expect_err("invalid staged invocation should fail during classification");
    assert!(err.to_string().contains("--wat"));
}

#[test]
fn execute_repl_plugin_line_records_failures_and_inline_help_unit() {
    let mut state = make_state_with_plugins(crate::plugin::PluginManager::new(Vec::new()));
    state.runtime.ui.debug_verbosity = 1;
    let history = test_history();

    let err = execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "intro --wat | config",
    )
    .expect_err("invalid staged invocation should still fail");
    assert!(err.to_string().contains("--wat"));

    let last_failure = state
        .session
        .last_failure
        .clone()
        .expect("non-bang failures should be recorded");
    assert_eq!(last_failure.command_line, "intro --wat | config");
    assert!(last_failure.summary.contains("--wat"));
    assert!(state.session.prompt_timing.badge().is_some());

    assert!(matches!(
        execute_repl_plugin_line(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            &history,
            "config sho"
        )
        .expect("invalid subcommands should render inline help instead of erroring"),
        ReplLineResult::Continue(output)
            if output.contains("unrecognized subcommand")
                && output.contains("config <COMMAND>")
    ));
}

#[test]
fn intro_value_pipeline_prefers_matching_entry_content_unit() {
    let mut state = make_state_with_plugins(crate::plugin::PluginManager::new(Vec::new()));
    let mut invocation = super::base_repl_invocation(&state.runtime);
    invocation.ui.render_settings.format = OutputFormat::Value;
    invocation.ui.render_settings.format_explicit = true;
    let result = run_repl_command(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        Commands::Intro(IntroArgs::default()),
        &invocation,
        &test_history(),
        None,
    )
    .expect("intro command should succeed");

    let mut sink = crate::app::sink::BufferedUiSink::default();
    let rendered = render_repl_command_output(
        &state.runtime,
        &mut state.session,
        "intro --value | config",
        &["config".to_string()],
        result,
        &invocation,
        &mut sink,
    )
    .expect("value pipeline should render");

    assert_eq!(rendered.trim(), "Inspect and edit runtime config");
}

#[test]
fn theme_show_value_pipeline_renders_selected_field_rhs_unit() {
    let mut state = make_state_with_plugins(crate::plugin::PluginManager::new(Vec::new()));
    state.runtime.themes =
        crate::ui::theme_loader::load_theme_catalog(state.runtime.config.resolved());
    let mut invocation = super::base_repl_invocation(&state.runtime);
    invocation.ui.render_settings.format = OutputFormat::Value;
    invocation.ui.render_settings.format_explicit = true;
    let result = run_repl_command(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        Commands::Theme(ThemeArgs {
            command: ThemeCommands::Show(crate::cli::ThemeShowArgs {
                name: Some("catppuccin".to_string()),
            }),
        }),
        &invocation,
        &test_history(),
        None,
    )
    .expect("theme show should succeed");

    let mut sink = crate::app::sink::BufferedUiSink::default();
    let rendered = render_repl_command_output(
        &state.runtime,
        &mut state.session,
        "theme show catppuccin --value | muted",
        &["muted".to_string()],
        result,
        &invocation,
        &mut sink,
    )
    .expect("value pipeline should render");

    assert_eq!(rendered.trim(), "#89b4fa");
}

#[test]
fn repl_command_spec_covers_repl_variant_and_builtin_dsl_matrix_unit() {
    let repl = repl_command_spec(&Commands::Repl(ReplArgs {
        command: ReplCommands::DebugComplete(DebugCompleteArgs {
            line: String::new(),
            menu: crate::cli::DebugMenuArg::Completion,
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

    let root = crate::tests::make_temp_dir("osp-cli-repl-dispatch");
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
        &SharedHistory::new(
            HistoryConfig::builder()
                .with_max_entries(8)
                .with_enabled(true)
                .with_dedupe(true)
                .with_profile_scoped(false)
                .with_shell_context(Default::default())
                .build(),
        )
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
        &SharedHistory::new(
            HistoryConfig::builder()
                .with_max_entries(8)
                .with_enabled(true)
                .with_dedupe(true)
                .with_profile_scoped(false)
                .with_shell_context(Default::default())
                .build(),
        )
        .expect("history should initialize"),
        Some("cache-key"),
    )
    .expect("cached external run should succeed");
    assert_eq!(cached.exit_code, 0);
}
