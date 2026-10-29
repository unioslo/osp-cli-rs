#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReplLinePlanKind {
    Builtin,
    Blank,
    DslHelp,
    Shortcut,
    Help,
    Invocation,
}

use insta::assert_snapshot;
use std::path::PathBuf;

use super::{
    BangCommand, classify_repl_line_kind, command_side_effects, config_key_change_requires_intro,
    current_history_scope, execute_bang_command, execute_repl_plugin_line, finalize_repl_command,
    handle_repl_exit_request, is_repl_bang_request, leave_repl_shell, parse_bang_command,
    parse_clap_help, parse_repl_builtin, render_repl_command_output, repl_command_spec,
    repl_help_for_scope, run_repl_command,
};
use crate::app::CommandOutput;
use crate::app::{AppSession, AppState, LaunchContext, RuntimeContext, TerminalKind};
use crate::cli::rows::output::rows_to_output_result;
use crate::cli::{
    Commands, ConfigArgs, ConfigCommands, ConfigSetArgs, ConfigUnsetArgs, DebugCompleteArgs,
    HistoryArgs, HistoryCommands, IntroArgs, PluginsArgs, PluginsCommands, ReplArgs, ReplCommands,
    ThemeArgs, ThemeCommands, ThemeUseArgs,
};
use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions};
use crate::core::output::OutputFormat;
use crate::repl::{HistoryConfig, ReplReloadKind, ReplResult, SharedHistory};
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
    let execution = execute_repl_plugin_line(state, history, line, None);
    if execution
        .as_ref()
        .is_ok_and(|executed| executed.exit_code != 0)
    {
        return ObservedDispatchKind::Error;
    }
    match execution.map(|executed| executed.result) {
        Ok(ReplResult::Continue) => ObservedDispatchKind::Blank,
        Ok(ReplResult::Exit(_)) => ObservedDispatchKind::Exit,
        Ok(ReplResult::Print(output)) if output.is_empty() => ObservedDispatchKind::Blank,
        Ok(ReplResult::Print(output)) if output.contains("DSL Help") => {
            ObservedDispatchKind::DslHelp
        }
        Ok(ReplResult::Print(output)) if line.trim() == "help" && output.contains("Commands") => {
            ObservedDispatchKind::ShortcutHelp
        }
        Ok(ReplResult::Print(output))
            if output.contains("unrecognized subcommand")
                || output.contains("unknown argument") =>
        {
            ObservedDispatchKind::InlineHelp
        }
        Ok(ReplResult::ReplaceInput(_)) | Ok(ReplResult::Restart { .. }) => {
            ObservedDispatchKind::Command
        }
        Ok(ReplResult::Print(_)) => ObservedDispatchKind::Command,
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
fn clap_error_helpers_extract_summary_and_body_when_present_or_missing_unit() {
    let cases = [
        (
            "\
error: unknown argument '--wat'\n\
\n\
Usage: osp config show [OPTIONS]\n\
\n\
tip: try --help\n\
For more information, try '--help'.\n",
            Some("unknown argument '--wat'"),
            "Usage: osp config show [OPTIONS]",
        ),
        (
            "\nUsage: osp ldap user\nFor more information, try '--help'.\n",
            None,
            "Usage: osp ldap user",
        ),
        (
            "Inspect and apply themes\n\nUsage: theme <COMMAND>\n",
            Some("Inspect and apply themes"),
            "Usage: theme <COMMAND>",
        ),
    ];

    for (error, expected_summary, expected_body) in cases {
        let parsed = parse_clap_help(error);
        assert_eq!(parsed.summary, expected_summary, "error: {error}");
        assert_eq!(parsed.body, expected_body, "error: {error}");
    }
}

#[test]
fn repl_exit_behaves_differently_for_root_and_shells_unit() {
    let mut root = AppSession::default();
    assert!(matches!(
        handle_repl_exit_request(&mut root),
        ReplResult::Exit(0)
    ));

    let mut nested = AppSession::default();
    nested.scope.enter("ldap");
    assert!(matches!(
        handle_repl_exit_request(&mut nested),
        ReplResult::Restart {
            output: message,
            reload: ReplReloadKind::Completion,
        } if message == "Leaving ldap. Back at root.\n"
    ));
    assert!(nested.scope.is_root());
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
            value: vec!["json".to_string()],
            display_user: None,
            from_file: None,
            scope: crate::cli::ConfigScopeArgs::default(),
            store: crate::cli::ConfigStoreArgs::default(),
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
            scope: crate::cli::ConfigScopeArgs::default(),
            store: crate::cli::ConfigStoreArgs::default(),
            dry_run: true,
        }),
    });
    assert_eq!(
        command_side_effects(&config_unset_dry_run),
        Default::default()
    );

    let plugins_enable = Commands::Plugins(PluginsArgs {
        command: PluginsCommands::Enable {
            name: "orch".to_string(),
        },
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
fn leave_repl_shell_returns_none_at_root_unit() {
    let mut session = AppSession::default();
    assert!(leave_repl_shell(&mut session).is_none());
    assert!(matches!(
        finalize_repl_command(String::new(), true, false),
        ReplResult::Restart {
            output,
            reload: ReplReloadKind::Default
        } if output.is_empty()
    ));
}

#[test]
fn finalize_repl_command_uses_intro_reload_when_requested_unit() {
    assert!(matches!(
        finalize_repl_command("saved\n".to_string(), true, true),
        ReplResult::Restart {
            output,
            reload: ReplReloadKind::WithIntro
        } if output == "saved\n"
    ));
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
        parse_repl_builtin("last").expect("last parses"),
        Some(super::ReplBuiltin::Last)
    ));
    assert!(matches!(
        parse_repl_builtin("last --raw").expect("last raw parses"),
        Some(super::ReplBuiltin::Last)
    ));
    assert!(matches!(
        parse_bang_command("!?ops").expect("contains parses"),
        Some(BangCommand::Contains(term)) if term == "ops"
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
        Some(BangCommand::Absolute {
            id: 7,
            suffix: None
        })
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
fn repl_last_builtin_replays_processed_and_raw_results_unit() {
    let mut state = make_state_with_plugins(empty_plugins());
    let history = test_history();
    state.session.record_success_output(
        "ldap user alice | P name",
        CommandOutput::output(
            rows_to_output_result(vec![crate::row! {
                "name" => "alice",
                "role" => "admin"
            }]),
            None,
        ),
        &["P name".to_string()],
    );

    let processed = execute_repl_plugin_line(&mut state, &history, "last", None)
        .map(|executed| executed.result)
        .expect("last should replay");
    assert!(matches!(
        processed,
        ReplResult::Print(text) if text.contains("alice") && !text.contains("admin")
    ));

    let raw = execute_repl_plugin_line(&mut state, &history, "last --raw", None)
        .map(|executed| executed.result)
        .expect("last raw should replay");
    assert!(matches!(
        raw,
        ReplResult::Print(text) if text.contains("alice") && text.contains("admin")
    ));
}

#[test]
fn bang_execution_and_scope_helpers_cover_help_matches_and_replace_unit() {
    let history = SharedHistory::new(HistoryConfig {
        max_entries: 20,
        enabled: true,
        dedupe: true,
        shell_context: Default::default(),
        ..HistoryConfig::default()
    });
    history
        .with(|store| store.save_command_line("ldap user alice"))
        .expect("history lock should be available")
        .expect("first command saves");
    history
        .with(|store| store.save_command_line("ldap netgroup ops"))
        .expect("history lock should be available")
        .expect("second command saves");
    history
        .with(|store| store.save_command_line("config show"))
        .expect("history lock should be available")
        .expect("third command saves");

    let mut session = AppSession::default();
    session.scope.enter("ldap");
    assert_eq!(current_history_scope(&session).as_deref(), Some("ldap "));
    assert_eq!(
        crate::repl::history_store::strip_shell_prefix("ldap user alice", Some("ldap")),
        "user alice".to_string()
    );
    assert_eq!(
        crate::repl::history_store::strip_shell_prefix("config show", Some("ldap")),
        "config show".to_string()
    );

    assert!(matches!(
        execute_bang_command(&mut session, &history, "!", BangCommand::Prefix(String::new()))
            .expect("empty prefix renders help"),
        ReplResult::Print(help) if help.contains("Bang history shortcuts")
    ));
    assert!(matches!(
        execute_bang_command(
            &mut session,
            &history,
            "!?ops",
            BangCommand::Contains("ops".to_string())
        )
        .expect("contains search should expand"),
        ReplResult::ReplaceInput(value) if value == "netgroup ops"
    ));
    assert!(matches!(
        execute_bang_command(
            &mut session,
            &history,
            "!user",
            BangCommand::Prefix("user".to_string())
        )
        .expect("prefix search should expand"),
        ReplResult::ReplaceInput(value) if value == "user alice"
    ));
    assert!(matches!(
        execute_bang_command(
            &mut session,
            &history,
            "!missing",
            BangCommand::Prefix("missing".to_string())
        )
        .expect_err("missing bang match should report an error").to_string(),
        value if value.contains("no history match")
    ));
    assert!(is_repl_bang_request(" !prefix"));
    assert!(!is_repl_bang_request("help"));
}

#[test]
fn absolute_history_replay_executes_with_suffix_unit() {
    let mut state = make_state_with_plugins(empty_plugins());
    let history = test_history();
    history
        .with(|store| store.save_command_line("config get profile.default"))
        .expect("history lock should be available")
        .expect("history command saves");
    let mut sink = crate::app::BufferedUiSink::default();
    let recalled = execute_repl_plugin_line(&mut state, &history, "!1 --json", Some(&mut sink))
        .map(|executed| executed.result)
        .expect("absolute history replay should execute");

    assert!(matches!(recalled, ReplResult::Print(value) if value.contains("profile.default")));
    assert_eq!(sink.stderr, "config get profile.default --json\n");
}

#[test]
fn absolute_history_replay_executes_suffix_inside_scoped_repl_unit() {
    let mut state = make_state_with_plugins(empty_plugins());
    let history = test_history();
    history
        .with(|store| store.save_command_line("config show"))
        .expect("history lock should be available")
        .expect("root history command saves");
    history
        .with(|store| store.save_command_line("ldap config get profile.default"))
        .expect("history lock should be available")
        .expect("scoped history command saves");
    state.session.scope.enter("ldap");
    let mut sink = crate::app::BufferedUiSink::default();
    let recalled = execute_repl_plugin_line(&mut state, &history, "!1 --json", Some(&mut sink))
        .map(|executed| executed.result)
        .expect("scoped absolute history replay should execute");

    assert!(matches!(recalled, ReplResult::Print(value) if value.contains("profile.default")));
    assert_eq!(sink.stderr, "config get profile.default --json\n");
}

#[test]
fn intro_reload_keys_cover_theme_and_palette_mutations_unit() {
    assert!(config_key_change_requires_intro("theme.name"));
    assert!(config_key_change_requires_intro("palette.custom"));
    assert!(!config_key_change_requires_intro("ui.format"));

    let config_unset = Commands::Config(ConfigArgs {
        command: ConfigCommands::Unset(ConfigUnsetArgs {
            key: "theme.name".to_string(),
            scope: crate::cli::ConfigScopeArgs::default(),
            store: crate::cli::ConfigStoreArgs::default(),
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
    defaults.set("repl.history.path", "/tmp/osp-repl-dispatch-history.jsonl");
    defaults.set("theme.path", Vec::<String>::new());
    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    let config = resolver
        .resolve(ResolveOptions::default().with_terminal("repl"))
        .expect("test config should resolve");

    let settings = RenderSettings::test_plain(OutputFormat::Json);
    crate::app::AppStateBuilder {
        context: RuntimeContext::new(None, TerminalKind::Repl, None),
        config,
        launch: LaunchContext::default(),
        native_commands: crate::native::NativeCommandRegistry::default(),
        ui: crate::app::UiState::new(settings, MessageLevel::Success, 0),
        plugins: Some(plugins),
        themes: Some(crate::ui::theme_catalog::ThemeCatalog::default()),
        session: None,
        definition: Default::default(),
    }
    .build()
}

fn empty_plugins() -> crate::plugin::PluginManager {
    crate::plugin::PluginManager::new(Vec::new())
        .with_bundled_roots(false)
        .with_default_roots(false)
        .with_path_discovery(false)
}

fn test_history() -> SharedHistory {
    SharedHistory::new(HistoryConfig {
        max_entries: 8,
        enabled: true,
        dedupe: true,
        shell_context: Default::default(),
        ..HistoryConfig::default()
    })
}

#[test]
#[cfg_attr(miri, ignore = "insta snapshot test spawns cargo metadata")]
fn root_help_rendering_and_shell_prefix_helpers_cover_root_paths_unit() {
    let mut state = make_state_with_plugins(empty_plugins());
    let invocation = super::base_repl_invocation(&state.runtime);
    let help = repl_help_for_scope(&mut state, &invocation).expect("root help should render");
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
    let mut state = make_state_with_plugins(empty_plugins());
    let invocation = super::base_repl_invocation(&state.runtime);
    let result = run_repl_command(
        &mut state,
        Commands::Intro(IntroArgs::default()),
        &invocation,
        &test_history(),
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
fn staged_repl_line_validation_rejects_invalid_invocations_before_pipeline_execution_unit() {
    let cases = [
        ("intro --wat | config", "--wat"),
        ("help --help | config", "invalid help target: --help"),
    ];

    for (line, needle) in cases {
        let mut state = make_state_with_plugins(empty_plugins());
        state.runtime.ui.render_settings.format = OutputFormat::Table;
        let mut sink = crate::app::BufferedUiSink::default();
        execute_repl_plugin_line(&mut state, &test_history(), line, Some(&mut sink))
            .expect("failure should render through the sink");
        assert!(sink.stderr.contains(needle), "line: {line}");
    }
}

#[test]
fn repl_dispatch_and_classification_cover_representative_line_categories_unit() {
    let mut state = make_state_with_plugins(empty_plugins());
    state.runtime.ui.debug_level = 1;
    let history = test_history();

    assert!(matches!(
        execute_repl_plugin_line(&mut state, &history, "quit", None,)
            .map(|executed| executed.result)
            .expect("quit should exit from the REPL root"),
        ReplResult::Exit(0)
    ));
    assert!(state.session.prompt_timing.badge().is_some());

    assert!(matches!(
        execute_repl_plugin_line(&mut state, &history, "   ", None,)
            .map(|executed| executed.result)
            .expect("blank lines should be ignored"),
        ReplResult::Continue
    ));

    assert!(matches!(
        execute_repl_plugin_line(
            &mut state,
            &history,
            "intro | H", None,).map(|executed| executed.result)
        .expect("dsl help stage should short-circuit normal command execution"),
        ReplResult::Print(output)
            if output.contains("DSL Help")
                && output.contains("Use | H <verb> for details.")
    ));

    assert!(matches!(
        execute_repl_plugin_line(
            &mut state,
            &history,
            "help", None,).map(|executed| executed.result)
        .expect("root help shortcut should render command help"),
        ReplResult::Print(output)
            if output.contains("Commands") && output.contains("config")
    ));

    let dispatch_cases = [
        ("   ", ObservedDispatchKind::Blank),
        ("quit", ObservedDispatchKind::Exit),
        ("intro | H", ObservedDispatchKind::DslHelp),
        ("help", ObservedDispatchKind::ShortcutHelp),
        ("intro", ObservedDispatchKind::Command),
        ("config shwo", ObservedDispatchKind::Error),
        ("intro --wat | config", ObservedDispatchKind::Error),
    ];

    for (line, expected) in dispatch_cases {
        let mut observed_state = make_state_with_plugins(empty_plugins());
        observed_state.runtime.ui.render_settings.format = OutputFormat::Table;
        let observed = observe_dispatch_kind(&mut observed_state, &test_history(), line);
        assert_eq!(observed, expected, "line: {line}");
    }

    let classification_cases = [
        ("   ", ReplLinePlanKind::Blank),
        ("quit", ReplLinePlanKind::Builtin),
        ("help", ReplLinePlanKind::Builtin),
        ("intro | H", ReplLinePlanKind::DslHelp),
        ("nh", ReplLinePlanKind::Invocation),
        ("intro", ReplLinePlanKind::Invocation),
    ];

    for (line, expected) in classification_cases {
        let classified_state = make_state_with_plugins(empty_plugins());
        let observed =
            classify_repl_line_kind(&classified_state.runtime, &classified_state.session, line)
                .expect("classification should succeed");
        assert_eq!(observed, expected, "line: {line}");
    }

    let classified_state = make_state_with_plugins(empty_plugins());
    let err = classify_repl_line_kind(
        &classified_state.runtime,
        &classified_state.session,
        "intro --wat | config",
    )
    .expect_err("invalid staged invocation should fail during classification");
    assert!(
        crate::app::render_report_message(
            &err,
            crate::core::MessageLevel::Info,
            &classified_state.runtime.ui.render_settings
        )
        .contains("--wat")
    );
}

#[test]
fn execute_repl_plugin_line_records_failures_and_inline_help_unit() {
    let mut state = make_state_with_plugins(empty_plugins());
    state.runtime.ui.render_settings.format = OutputFormat::Table;
    state.runtime.ui.debug_level = 1;
    let history = test_history();

    let mut sink = crate::app::BufferedUiSink::default();
    execute_repl_plugin_line(
        &mut state,
        &history,
        "intro --wat | config",
        Some(&mut sink),
    )
    .expect("failure should render through the sink");
    assert!(sink.stderr.contains("--wat"));

    let last_failure = state
        .session
        .last_failure
        .clone()
        .expect("non-bang failures should be recorded");
    assert_eq!(last_failure.command_line, "intro …");
    assert!(last_failure.summary.contains("--wat"));
    assert!(state.session.prompt_timing.badge().is_some());

    let mut sink = crate::app::BufferedUiSink::default();
    execute_repl_plugin_line(&mut state, &history, "config shwo", Some(&mut sink))
        .expect("syntax errors should render through the sink");
    assert!(sink.stderr.contains("Unknown command 'config shwo'"));
    assert!(sink.stderr.contains("error:"));
}

#[test]
fn intro_value_pipeline_prefers_matching_entry_content_unit() {
    let mut state = make_state_with_plugins(empty_plugins());
    let mut invocation = super::base_repl_invocation(&state.runtime);
    invocation.ui.render_settings.format = OutputFormat::Value;
    invocation.ui.render_settings.format_explicit = true;
    let result = run_repl_command(
        &mut state,
        Commands::Intro(IntroArgs::default()),
        &invocation,
        &test_history(),
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
fn theme_list_value_pipeline_renders_selected_field_rhs_unit() {
    let mut state = make_state_with_plugins(empty_plugins());
    state.runtime.themes =
        crate::ui::theme_catalog::load_theme_catalog(state.runtime.config.resolved());
    let mut invocation = super::base_repl_invocation(&state.runtime);
    invocation.ui.render_settings.format = OutputFormat::Value;
    invocation.ui.render_settings.format_explicit = true;
    let result = run_repl_command(
        &mut state,
        Commands::Theme(ThemeArgs {
            command: ThemeCommands::List,
        }),
        &invocation,
        &test_history(),
    )
    .expect("theme list should succeed");

    let mut sink = crate::app::sink::BufferedUiSink::default();
    let rendered = render_repl_command_output(
        &state.runtime,
        &mut state.session,
        "theme list --value | catppuccin | VALUE muted",
        &["catppuccin".to_string(), "VALUE muted".to_string()],
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
        command: PluginsCommands::List,
    }));
    assert!(plugins.supports_dsl);

    let history = repl_command_spec(&Commands::History(HistoryArgs {
        command: HistoryCommands::Clear(crate::cli::HistoryClearArgs {
            pattern: "*".into(),
            ..Default::default()
        }),
    }));
    assert!(!history.supports_dsl);
}

#[test]
fn render_repl_command_output_handles_text_none_and_stderr_unit() {
    use crate::app::sink::BufferedUiSink;

    let mut state = make_state_with_plugins(empty_plugins());
    let invocation = super::base_repl_invocation(&state.runtime);
    let mut sink = BufferedUiSink::default();

    let rendered = render_repl_command_output(
        &state.runtime,
        &mut state.session,
        "doctor last",
        &[],
        CommandOutput {
            exit_code: 0,
            messages: Default::default(),
            stderr_text: "\nwarn\n".to_string(),
            failure_report: None,
            ..CommandOutput::text("hello".to_string())
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
        CommandOutput::exit(0),
        &invocation,
        &mut sink,
    )
    .expect("empty result should render");
    assert!(empty.is_empty());
}

#[test]
fn repl_unrelated_lines_clear_native_pagination_context_unit() {
    let mut state = make_state_with_plugins(empty_plugins());
    state.runtime.ui.render_settings.format = OutputFormat::Table;
    let history = test_history();
    state
        .session
        .native_context
        .set_pagination(crate::native::NativePagination {
            current: vec!["page".to_string()],
            next: Some(vec!["trusted".to_string()]),
        });

    execute_repl_plugin_line(&mut state, &history, "help", None)
        .map(|executed| executed.result)
        .expect("help should render");
    assert_eq!(state.session.native_context.pagination(), None);

    state
        .session
        .native_context
        .set_pagination(crate::native::NativePagination {
            current: vec!["page".to_string()],
            next: Some(vec!["trusted".to_string()]),
        });
    let mut sink = crate::app::BufferedUiSink::default();
    let executed =
        execute_repl_plugin_line(&mut state, &history, "not-a-real-command", Some(&mut sink))
            .expect("failure should render through the sink");
    assert_ne!(executed.exit_code, 0);
    assert!(sink.stderr.contains("not-a-real-command"));
    assert_eq!(state.session.native_context.pagination(), None);
}
