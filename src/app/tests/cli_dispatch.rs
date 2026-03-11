use super::*;

fn dispatch_plan_for(args: &[&str], profile_names: &[&str]) -> crate::app::dispatch::DispatchPlan {
    let mut cli = Cli::parse_from(args);
    build_dispatch_plan(&mut cli, &profiles(profile_names)).expect("dispatch plan should parse")
}

fn assert_external_tokens(action: RunAction, expected: &[&str]) {
    match action {
        RunAction::External(tokens) => assert_eq!(
            tokens,
            expected
                .iter()
                .map(|value| (*value).to_string())
                .collect::<Vec<_>>()
        ),
        _ => panic!("expected external action"),
    }
}

#[test]
fn cli_scan_extracts_invocation_flags_without_polluting_clap_unit() {
    let argv = [
        OsString::from("osp"),
        OsString::from("--json"),
        OsString::from("--mode"),
        OsString::from("plain"),
        OsString::from("--color=never"),
        OsString::from("--ascii"),
        OsString::from("-q"),
        OsString::from("config"),
        OsString::from("show"),
    ];

    let scanned = scan_cli_argv(&argv).expect("argv scan should succeed");

    assert_eq!(
        scanned.argv,
        vec![
            OsString::from("osp"),
            OsString::from("config"),
            OsString::from("show"),
        ]
    );
    assert_eq!(scanned.invocation.format, Some(OutputFormat::Json));
    assert_eq!(scanned.invocation.mode, Some(RenderMode::Plain));
    assert_eq!(scanned.invocation.color, Some(ColorMode::Never));
    assert_eq!(scanned.invocation.unicode, Some(UnicodeMode::Never));
    assert_eq!(scanned.invocation.quiet, 1);
}

#[test]
fn invocation_ui_overlays_runtime_defaults_per_command_unit() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    let config = resolver
        .resolve(ResolveOptions::default().with_terminal("cli"))
        .expect("config should resolve");
    let ui = crate::app::UiState::builder(RenderSettings::test_plain(OutputFormat::Table))
        .with_message_verbosity(MessageLevel::Success)
        .with_debug_verbosity(1)
        .build();
    let invocation = InvocationOptions {
        format: Some(OutputFormat::Json),
        mode: Some(RenderMode::Rich),
        color: Some(ColorMode::Always),
        unicode: Some(UnicodeMode::Never),
        verbose: 2,
        quiet: 1,
        debug: 3,
        cache: false,
        plugin_provider: Some("beta".to_string()),
    };

    let resolved = resolve_invocation_ui(&config, &ui, &invocation);

    assert_eq!(resolved.ui.render_settings.format, OutputFormat::Json);
    assert!(resolved.ui.render_settings.format_explicit);
    assert_eq!(resolved.ui.render_settings.mode, RenderMode::Rich);
    assert_eq!(resolved.ui.render_settings.color, ColorMode::Always);
    assert_eq!(resolved.ui.render_settings.unicode, UnicodeMode::Never);
    assert_eq!(resolved.ui.message_verbosity, MessageLevel::Info);
    assert_eq!(resolved.ui.debug_verbosity, 3);
    assert_eq!(resolved.plugin_provider.as_deref(), Some("beta"));
}

#[test]
fn cli_cache_flag_is_rejected_outside_repl_unit() {
    let err = super::run_from(["osp", "--cache", "config", "show"])
        .expect_err("cache should be rejected outside repl");

    assert!(
        err.to_string()
            .contains("`--cache` is only available inside the interactive REPL")
    );
}

#[test]
fn cli_presentation_flag_sets_session_override_unit() {
    let cli = Cli::parse_from(["osp", "--presentation", "compact"]);

    let layer = build_cli_session_layer(
        &cli,
        None,
        TerminalKind::Repl,
        RuntimeLoadOptions::default(),
    )
    .expect("session layer should build")
    .expect("presentation flag should create session overrides");

    assert_eq!(
        layer_value(&layer, "ui.presentation"),
        Some(&ConfigValue::from("compact"))
    );
}

#[test]
fn cli_gammel_og_bitter_flag_maps_to_austere_unit() {
    let cli = Cli::parse_from(["osp", "--gammel-og-bitter"]);

    let layer = build_cli_session_layer(
        &cli,
        None,
        TerminalKind::Repl,
        RuntimeLoadOptions::default(),
    )
    .expect("session layer should build")
    .expect("legacy presentation flag should create session overrides");

    assert_eq!(
        layer_value(&layer, "ui.presentation"),
        Some(&ConfigValue::from("austere"))
    );
}

#[test]
fn cli_runtime_load_options_follow_disable_flags_unit() {
    let cli = Cli::parse_from(["osp", "--no-env", "--no-config"]);
    let options = cli.runtime_load_options();
    assert!(!options.include_env);
    assert!(!options.include_config_file);
}

#[test]
fn render_settings_with_hint_use_plugin_hint_only_when_auto_unit() {
    let base = RenderSettings::test_plain(OutputFormat::Auto);
    let hinted = resolve_render_settings_with_hint(&base, Some(OutputFormat::Table));
    assert_eq!(hinted.format, OutputFormat::Table);

    let pinned = resolve_render_settings_with_hint(
        &RenderSettings {
            format: OutputFormat::Json,
            ..base
        },
        Some(OutputFormat::Table),
    );
    assert_eq!(pinned.format, OutputFormat::Json);
}

#[test]
fn positional_profile_dispatch_matrix_covers_repl_external_and_builtin_routes_unit() {
    let plan = dispatch_plan_for(&["osp", "tsd"], &["uio", "tsd"]);
    assert_eq!(plan.profile_override.as_deref(), Some("tsd"));
    assert!(matches!(plan.action, RunAction::Repl));

    let plan = dispatch_plan_for(&["osp", "tsd", "ldap", "user", "oistes"], &["uio", "tsd"]);
    assert_eq!(plan.profile_override.as_deref(), Some("tsd"));
    assert_external_tokens(plan.action, &["ldap", "user", "oistes"]);

    let plan = dispatch_plan_for(&["osp", "tsd", "plugins", "list"], &["uio", "tsd"]);
    assert_eq!(plan.profile_override.as_deref(), Some("tsd"));
    assert!(matches!(
        plan.action,
        RunAction::Plugins(crate::cli::PluginsArgs {
            command: PluginsCommands::List
        })
    ));

    let plan = dispatch_plan_for(&["osp", "prod", "ldap", "user", "oistes"], &["uio", "tsd"]);
    assert_eq!(plan.profile_override, None);
    assert_external_tokens(plan.action, &["prod", "ldap", "user", "oistes"]);
}

#[test]
fn explicit_profile_precedence_and_parity_cover_external_and_builtin_paths_unit() {
    let positional = dispatch_plan_for(&["osp", "tsd", "plugins", "list"], &["uio", "tsd"]);
    let explicit = dispatch_plan_for(
        &["osp", "--profile", "tsd", "plugins", "list"],
        &["uio", "tsd"],
    );
    assert_eq!(positional.profile_override, explicit.profile_override);
    assert!(matches!(positional.action, RunAction::Plugins(_)));
    assert!(matches!(explicit.action, RunAction::Plugins(_)));

    let positional = dispatch_plan_for(&["osp", "tsd", "ldap", "user", "oistes"], &["uio", "tsd"]);
    let explicit = dispatch_plan_for(
        &["osp", "--profile", "tsd", "ldap", "user", "oistes"],
        &["uio", "tsd"],
    );
    assert_eq!(positional.profile_override, explicit.profile_override);
    match (positional.action, explicit.action) {
        (RunAction::External(left), RunAction::External(right)) => assert_eq!(left, right),
        _ => panic!("expected external action on both plans"),
    }

    let plan = dispatch_plan_for(
        &["osp", "--profile", "uio", "tsd", "plugins", "list"],
        &["uio", "tsd"],
    );
    assert_eq!(plan.profile_override.as_deref(), Some("uio"));
    assert_external_tokens(plan.action, &["tsd", "plugins", "list"]);

    let normalized = dispatch_plan_for(&["osp", "--profile", "TSD"], &["tsd"]);
    assert_eq!(normalized.profile_override.as_deref(), Some("tsd"));
}

#[test]
fn direct_plugins_command_keeps_clap_action_unit() {
    let mut cli = Cli::parse_from(["osp", "plugins", "doctor"]);
    let plan = build_dispatch_plan(&mut cli, &profiles(&["uio", "tsd"]))
        .expect("dispatch plan should parse");

    assert_eq!(plan.profile_override, None);
    assert!(matches!(
        plan.action,
        RunAction::Plugins(crate::cli::PluginsArgs {
            command: PluginsCommands::Doctor
        })
    ));
    assert!(matches!(cli.command, None | Some(Commands::Plugins(_))));
}

#[test]
fn positional_profile_with_config_uses_clap_parser_unit() {
    let mut cli = Cli::parse_from(["osp", "tsd", "config", "show", "--sources"]);
    let plan = build_dispatch_plan(&mut cli, &profiles(&["uio", "tsd"]))
        .expect("dispatch plan should parse");

    assert_eq!(plan.profile_override.as_deref(), Some("tsd"));
    match plan.action {
        RunAction::Config(args) => {
            assert!(matches!(
                args.command,
                ConfigCommands::Show(crate::cli::ConfigShowArgs {
                    sources: true,
                    raw: false,
                })
            ));
        }
        _ => panic!("expected config action"),
    }
}

#[test]
fn repl_dsl_capability_is_declared_per_command_unit() {
    let plugins_list = Commands::Plugins(crate::cli::PluginsArgs {
        command: PluginsCommands::List,
    });
    let plugins_enable = Commands::Plugins(crate::cli::PluginsArgs {
        command: PluginsCommands::Enable(crate::cli::PluginCommandStateArgs {
            command: "ldap".to_string(),
            global: false,
            profile: None,
            terminal: None,
        }),
    });
    let theme_show = Commands::Theme(crate::cli::ThemeArgs {
        command: ThemeCommands::Show(crate::cli::ThemeShowArgs { name: None }),
    });
    let theme_use = Commands::Theme(crate::cli::ThemeArgs {
        command: ThemeCommands::Use(crate::cli::ThemeUseArgs {
            name: "nord".to_string(),
        }),
    });
    let config_show = Commands::Config(crate::cli::ConfigArgs {
        command: ConfigCommands::Show(crate::cli::ConfigShowArgs {
            sources: false,
            raw: false,
        }),
    });
    let config_set = Commands::Config(crate::cli::ConfigArgs {
        command: ConfigCommands::Set(crate::cli::ConfigSetArgs {
            key: "ui.mode".to_string(),
            value: "plain".to_string(),
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
    let history_list = Commands::History(crate::cli::HistoryArgs {
        command: crate::cli::HistoryCommands::List,
    });
    let history_prune = Commands::History(crate::cli::HistoryArgs {
        command: crate::cli::HistoryCommands::Prune(crate::cli::HistoryPruneArgs { keep: 5 }),
    });

    assert!(repl::repl_command_spec(&plugins_list).supports_dsl);
    assert!(!repl::repl_command_spec(&plugins_enable).supports_dsl);
    assert!(repl::repl_command_spec(&theme_show).supports_dsl);
    assert!(!repl::repl_command_spec(&theme_use).supports_dsl);
    assert!(repl::repl_command_spec(&config_show).supports_dsl);
    assert!(!repl::repl_command_spec(&config_set).supports_dsl);
    assert!(repl::repl_command_spec(&history_list).supports_dsl);
    assert!(!repl::repl_command_spec(&history_prune).supports_dsl);
}

#[test]
fn external_inline_builtin_reuses_repl_dsl_policy_unit() {
    let mut state = make_completion_state(None);
    let command = Commands::Config(crate::cli::ConfigArgs {
        command: ConfigCommands::Set(crate::cli::ConfigSetArgs {
            key: "ui.mode".to_string(),
            value: "plain".to_string(),
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

    let err = match run_inline_builtin_command(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        None,
        command,
        &["uid".to_string()],
    ) {
        Ok(_) => panic!("expected DSL rejection"),
        Err(err) => err,
    };
    assert_eq!(
        err.to_string(),
        "`config` does not support DSL pipeline stages"
    );
}
