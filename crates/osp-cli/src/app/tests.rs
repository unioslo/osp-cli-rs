use super::command_output::parse_output_format_hint;
use super::help::parse_help_render_overrides;
use super::{
    PluginConfigEntry, PluginConfigScope, ReplCommandOutput, RunAction, build_cli_session_layer,
    build_dispatch_plan, collect_plugin_config_env, config_value_to_plugin_env, doctor_cmd,
    is_sensitive_key, plugin_config_env_name, resolve_effective_render_settings,
    run_inline_builtin_command,
};
use crate::cli::{Cli, Commands, ConfigCommands, PluginsCommands, ThemeCommands};
use crate::plugin_manager::{CommandCatalogEntry, PluginManager, PluginSource};
use crate::repl;
use crate::repl::{completion, dispatch as repl_dispatch, help as repl_help, surface};
use crate::state::{AppState, AppStateInit, LaunchContext, RuntimeContext, TerminalKind};
use clap::Parser;
use osp_config::{ConfigLayer, ConfigResolver, ConfigValue, ResolveOptions, RuntimeLoadOptions};
use osp_core::output::OutputFormat;
use osp_repl::{HistoryConfig, HistoryShellContext, SharedHistory};
use osp_ui::messages::MessageLevel;
use osp_ui::{RenderSettings, render_output};
use std::collections::BTreeSet;
use std::ffi::OsString;

fn profiles(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| name.to_string()).collect()
}

fn repl_view<'a>(
    runtime: &'a crate::state::AppRuntime,
    session: &'a crate::state::AppSession,
) -> repl::ReplViewContext<'a> {
    repl::ReplViewContext::from_parts(runtime, session)
}

fn make_completion_state(auth_visible_builtins: Option<&str>) -> AppState {
    make_completion_state_with_entries(auth_visible_builtins, &[])
}

fn make_completion_state_with_entries(
    auth_visible_builtins: Option<&str>,
    entries: &[(&str, &str)],
) -> AppState {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    if let Some(allowlist) = auth_visible_builtins {
        defaults.set("auth.visible.builtins", allowlist);
    }
    for (key, value) in entries {
        defaults.set(*key, *value);
    }
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
        plugins: PluginManager::new(Vec::new()),
        themes: crate::theme_loader::ThemeCatalog::default(),
        launch: LaunchContext::default(),
    })
}

fn sample_catalog() -> Vec<CommandCatalogEntry> {
    vec![CommandCatalogEntry {
        name: "orch".to_string(),
        about: "Provision orchestrator resources".to_string(),
        subcommands: vec!["provision".to_string(), "status".to_string()],
        completion: osp_completion::CommandSpec {
            name: "orch".to_string(),
            tooltip: Some("Provision orchestrator resources".to_string()),
            subcommands: vec![
                osp_completion::CommandSpec::new("provision"),
                osp_completion::CommandSpec::new("status"),
            ],
            ..osp_completion::CommandSpec::default()
        },
        provider: "mock-provider".to_string(),
        providers: vec!["mock-provider (explicit)".to_string()],
        conflicted: false,
        source: PluginSource::Explicit,
    }]
}

#[test]
fn theme_slug_is_rendered_as_title_case_display_name_unit() {
    assert_eq!(repl::theme_display_name("rose-pine-moon"), "Rose Pine Moon");
    assert_eq!(repl::theme_display_name("dracula"), "Dracula");
}

#[test]
fn plugin_format_hint_parser_supports_known_values_unit() {
    assert_eq!(
        parse_output_format_hint(Some("table")),
        Some(OutputFormat::Table)
    );
    assert_eq!(
        parse_output_format_hint(Some("mreg")),
        Some(OutputFormat::Mreg)
    );
    assert_eq!(
        parse_output_format_hint(Some("markdown")),
        Some(OutputFormat::Markdown)
    );
    assert_eq!(parse_output_format_hint(Some("unknown")), None);
}

#[test]
fn plugin_config_env_name_normalizes_extension_keys_unit() {
    assert_eq!(
        plugin_config_env_name("api.token"),
        Some("OSP_PLUGIN_CFG_API_TOKEN".to_string())
    );
    assert_eq!(
        plugin_config_env_name("nested-value/path"),
        Some("OSP_PLUGIN_CFG_NESTED_VALUE_PATH".to_string())
    );
    assert_eq!(plugin_config_env_name("..."), None);
}

#[test]
fn plugin_config_env_serializes_lists_and_secrets_unit() {
    assert_eq!(
        config_value_to_plugin_env(&ConfigValue::List(vec![
            ConfigValue::String("alpha".to_string()),
            ConfigValue::Integer(2),
            ConfigValue::Bool(true),
        ])),
        r#"["alpha",2,true]"#
    );
    assert_eq!(
        config_value_to_plugin_env(&ConfigValue::String("sekrit".to_string()).into_secret()),
        "sekrit"
    );
}

#[test]
fn plugin_config_env_collects_shared_and_plugin_specific_entries_unit() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    defaults.set(
        "extensions.plugins.env.shared.url",
        "https://common.example",
    );
    defaults.set("extensions.plugins.env.endpoint", "shared");
    defaults.set("extensions.plugins.cfg.env.endpoint", "plugin");
    defaults.set("extensions.plugins.cfg.env.api.token", "token-123");
    defaults.set("extensions.plugins.other.env.endpoint", "other");
    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    let config = resolver
        .resolve(ResolveOptions::default())
        .expect("test config should resolve");

    let env = collect_plugin_config_env(&config);

    assert_eq!(
        env.shared,
        vec![
            PluginConfigEntry {
                env_key: "OSP_PLUGIN_CFG_ENDPOINT".to_string(),
                value: "shared".to_string(),
                config_key: "extensions.plugins.env.endpoint".to_string(),
                scope: PluginConfigScope::Shared,
            },
            PluginConfigEntry {
                env_key: "OSP_PLUGIN_CFG_SHARED_URL".to_string(),
                value: "https://common.example".to_string(),
                config_key: "extensions.plugins.env.shared.url".to_string(),
                scope: PluginConfigScope::Shared,
            },
        ]
    );
    assert_eq!(
        env.by_plugin_id.get("cfg"),
        Some(&vec![
            PluginConfigEntry {
                env_key: "OSP_PLUGIN_CFG_API_TOKEN".to_string(),
                value: "token-123".to_string(),
                config_key: "extensions.plugins.cfg.env.api.token".to_string(),
                scope: PluginConfigScope::Plugin,
            },
            PluginConfigEntry {
                env_key: "OSP_PLUGIN_CFG_ENDPOINT".to_string(),
                value: "plugin".to_string(),
                config_key: "extensions.plugins.cfg.env.endpoint".to_string(),
                scope: PluginConfigScope::Plugin,
            },
        ])
    );
    assert_eq!(
        env.by_plugin_id.get("other"),
        Some(&vec![PluginConfigEntry {
            env_key: "OSP_PLUGIN_CFG_ENDPOINT".to_string(),
            value: "other".to_string(),
            config_key: "extensions.plugins.other.env.endpoint".to_string(),
            scope: PluginConfigScope::Plugin,
        }])
    );
}

#[test]
fn plugin_dispatch_context_refreshes_cached_plugin_env_after_config_change() {
    let mut state =
        make_completion_state_with_entries(None, &[("extensions.plugins.env.endpoint", "before")]);
    let before = super::plugin_dispatch_context_for_runtime(&state.runtime, &state.clients, None);
    assert_eq!(
        before.shared_env,
        vec![("OSP_PLUGIN_CFG_ENDPOINT".to_string(), "before".to_string(),)]
    );

    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    defaults.set("extensions.plugins.env.endpoint", "after");
    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    let updated = resolver
        .resolve(ResolveOptions::default().with_terminal("repl"))
        .expect("test config should resolve");
    assert!(state.runtime.config.replace_resolved(updated));

    let after = super::plugin_dispatch_context_for_runtime(&state.runtime, &state.clients, None);
    assert_eq!(
        after.shared_env,
        vec![("OSP_PLUGIN_CFG_ENDPOINT".to_string(), "after".to_string(),)]
    );
}

fn layer_value<'a>(layer: &'a ConfigLayer, key: &str) -> Option<&'a ConfigValue> {
    layer
        .entries()
        .iter()
        .find(|entry| entry.key == key)
        .map(|entry| &entry.value)
}

#[test]
fn cli_launch_render_flags_seed_session_layer_unit() {
    let cli = Cli::parse_from([
        "osp", "--json", "--mode", "plain", "--color", "never", "--ascii",
    ]);

    let layer = build_cli_session_layer(
        &cli,
        None,
        TerminalKind::Repl,
        RuntimeLoadOptions::default(),
    )
    .expect("session layer should build")
    .expect("launch flags should create session overrides");

    assert_eq!(
        layer_value(&layer, "ui.format"),
        Some(&ConfigValue::from("json"))
    );
    assert_eq!(
        layer_value(&layer, "ui.mode"),
        Some(&ConfigValue::from("plain"))
    );
    assert_eq!(
        layer_value(&layer, "ui.color.mode"),
        Some(&ConfigValue::from("never"))
    );
    assert_eq!(
        layer_value(&layer, "ui.unicode.mode"),
        Some(&ConfigValue::from("never"))
    );
}

#[test]
fn cli_launch_quiet_adjusts_session_base_verbosity_unit() {
    let cli = Cli::parse_from(["osp", "-q"]);

    let layer = build_cli_session_layer(
        &cli,
        None,
        TerminalKind::Repl,
        RuntimeLoadOptions::default(),
    )
    .expect("session layer should build")
    .expect("quiet flag should create session overrides");

    assert_eq!(
        layer_value(&layer, "ui.verbosity.level"),
        Some(&ConfigValue::from("warning"))
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
fn effective_settings_use_plugin_hint_only_when_auto_unit() {
    let base = RenderSettings::test_plain(OutputFormat::Auto);
    let hinted = resolve_effective_render_settings(&base, Some(OutputFormat::Table));
    assert_eq!(hinted.format, OutputFormat::Table);

    let pinned = resolve_effective_render_settings(
        &RenderSettings {
            format: OutputFormat::Json,
            ..base
        },
        Some(OutputFormat::Table),
    );
    assert_eq!(pinned.format, OutputFormat::Json);
}

#[test]
fn positional_profile_only_routes_to_repl_unit() {
    let mut cli = Cli::parse_from(["osp", "tsd"]);
    let plan = build_dispatch_plan(&mut cli, &profiles(&["uio", "tsd"]))
        .expect("dispatch plan should parse");

    assert_eq!(plan.profile_override.as_deref(), Some("tsd"));
    assert!(matches!(plan.action, RunAction::Repl));
}

#[test]
fn positional_profile_with_command_routes_external_unit() {
    let mut cli = Cli::parse_from(["osp", "tsd", "ldap", "user", "oistes"]);
    let plan = build_dispatch_plan(&mut cli, &profiles(&["uio", "tsd"]))
        .expect("dispatch plan should parse");

    assert_eq!(plan.profile_override.as_deref(), Some("tsd"));
    match plan.action {
        RunAction::External(tokens) => {
            assert_eq!(
                tokens,
                vec!["ldap".to_string(), "user".to_string(), "oistes".to_string()]
            );
        }
        _ => panic!("expected external action"),
    }
}

#[test]
fn positional_profile_with_plugins_routes_builtin_unit() {
    let mut cli = Cli::parse_from(["osp", "tsd", "plugins", "list"]);
    let plan = build_dispatch_plan(&mut cli, &profiles(&["uio", "tsd"]))
        .expect("dispatch plan should parse");

    assert_eq!(plan.profile_override.as_deref(), Some("tsd"));
    match plan.action {
        RunAction::Plugins(args) => {
            assert!(matches!(args.command, PluginsCommands::List));
        }
        _ => panic!("expected plugins action"),
    }
}

#[test]
fn positional_profile_plugins_matches_explicit_profile_unit() {
    let mut positional = Cli::parse_from(["osp", "tsd", "plugins", "list"]);
    let positional_plan = build_dispatch_plan(&mut positional, &profiles(&["uio", "tsd"]))
        .expect("positional dispatch plan should parse");

    let mut explicit = Cli::parse_from(["osp", "--profile", "tsd", "plugins", "list"]);
    let explicit_plan = build_dispatch_plan(&mut explicit, &profiles(&["uio", "tsd"]))
        .expect("explicit dispatch plan should parse");

    assert_eq!(
        positional_plan.profile_override,
        explicit_plan.profile_override
    );
    assert!(matches!(positional_plan.action, RunAction::Plugins(_)));
    assert!(matches!(explicit_plan.action, RunAction::Plugins(_)));
}

#[test]
fn positional_profile_external_matches_explicit_profile_unit() {
    let mut positional = Cli::parse_from(["osp", "tsd", "ldap", "user", "oistes"]);
    let positional_plan = build_dispatch_plan(&mut positional, &profiles(&["uio", "tsd"]))
        .expect("positional dispatch plan should parse");

    let mut explicit = Cli::parse_from(["osp", "--profile", "tsd", "ldap", "user", "oistes"]);
    let explicit_plan = build_dispatch_plan(&mut explicit, &profiles(&["uio", "tsd"]))
        .expect("explicit dispatch plan should parse");

    assert_eq!(
        positional_plan.profile_override,
        explicit_plan.profile_override
    );
    match (positional_plan.action, explicit_plan.action) {
        (RunAction::External(left), RunAction::External(right)) => assert_eq!(left, right),
        _ => panic!("expected external action on both plans"),
    }
}

#[test]
fn unknown_first_token_is_command_unit() {
    let mut cli = Cli::parse_from(["osp", "prod", "ldap", "user", "oistes"]);
    let plan = build_dispatch_plan(&mut cli, &profiles(&["uio", "tsd"]))
        .expect("dispatch plan should parse");

    assert_eq!(plan.profile_override, None);
    match plan.action {
        RunAction::External(tokens) => {
            assert_eq!(
                tokens,
                vec![
                    "prod".to_string(),
                    "ldap".to_string(),
                    "user".to_string(),
                    "oistes".to_string()
                ]
            );
        }
        _ => panic!("expected external action"),
    }
}

#[test]
fn explicit_profile_overrides_positional_unit() {
    let mut cli = Cli::parse_from(["osp", "--profile", "uio", "tsd", "plugins", "list"]);
    let plan = build_dispatch_plan(&mut cli, &profiles(&["uio", "tsd"]))
        .expect("dispatch plan should parse");

    assert_eq!(plan.profile_override.as_deref(), Some("uio"));
    match plan.action {
        RunAction::External(tokens) => {
            assert_eq!(
                tokens,
                vec!["tsd".to_string(), "plugins".to_string(), "list".to_string()]
            );
        }
        _ => panic!("expected external action"),
    }
}

#[test]
fn explicit_profile_is_normalized_unit() {
    let mut cli = Cli::parse_from(["osp", "--profile", "TSD"]);
    let plan =
        build_dispatch_plan(&mut cli, &profiles(&["tsd"])).expect("dispatch plan should parse");
    assert_eq!(plan.profile_override.as_deref(), Some("tsd"));
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
        command: PluginsCommands::Enable(crate::cli::PluginToggleArgs {
            plugin_id: "uio-ldap".to_string(),
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

#[test]
fn repl_prompt_template_substitutes_profile_and_indicator_unit() {
    let rendered = repl::render_prompt_template(
        "╭─{user}@{domain} {indicator}\n╰─{profile}> ",
        "oistes",
        "uio.no",
        "uio",
        "[orch]",
    );
    assert!(rendered.contains("oistes@uio.no [orch]"));
    assert!(rendered.contains("╰─uio> "));
}

#[test]
fn repl_prompt_template_appends_indicator_when_missing_placeholder_unit() {
    let rendered = repl::render_prompt_template("{profile}>", "oistes", "uio.no", "tsd", "[shell]");
    assert_eq!(rendered, "tsd> [shell]");
}

#[test]
fn repl_help_alias_rewrites_to_command_help_unit() {
    let state = make_completion_state(None);
    let rewritten = repl::ReplParsedLine::parse("help ldap user", state.runtime.config.resolved())
        .expect("help alias should parse");
    assert_eq!(
        rewritten.dispatch_tokens,
        vec!["ldap".to_string(), "user".to_string(), "--help".to_string()]
    );
}

#[test]
fn repl_help_alias_preserves_existing_help_flag_unit() {
    let state = make_completion_state(None);
    let rewritten =
        repl::ReplParsedLine::parse("help ldap --help", state.runtime.config.resolved())
            .expect("help alias should parse");
    assert_eq!(
        rewritten.dispatch_tokens,
        vec!["ldap".to_string(), "--help".to_string()]
    );
}

#[test]
fn repl_help_alias_skips_bare_help_unit() {
    let state = make_completion_state(None);
    let parsed = repl::ReplParsedLine::parse("help", state.runtime.config.resolved())
        .expect("bare help should parse");
    assert_eq!(parsed.command_tokens, vec!["help".to_string()]);
    assert_eq!(parsed.dispatch_tokens, vec!["help".to_string()]);
}

#[test]
fn repl_shellable_commands_include_ldap_unit() {
    assert!(repl::is_repl_shellable_command("ldap"));
    assert!(repl::is_repl_shellable_command("LDAP"));
    assert!(!repl::is_repl_shellable_command("theme"));
}

#[test]
fn repl_shell_prefix_applies_once_unit() {
    let mut stack = crate::state::ReplScopeStack::default();
    stack.enter("ldap");
    let bare = repl::apply_repl_shell_prefix(&stack, &["user".to_string(), "oistes".to_string()]);
    assert_eq!(
        bare,
        vec!["ldap".to_string(), "user".to_string(), "oistes".to_string()]
    );

    let already_prefixed = repl::apply_repl_shell_prefix(
        &stack,
        &["ldap".to_string(), "user".to_string(), "oistes".to_string()],
    );
    assert_eq!(
        already_prefixed,
        vec!["ldap".to_string(), "user".to_string(), "oistes".to_string()]
    );
}

#[test]
fn repl_shell_leave_message_unit() {
    let mut state = make_completion_state(None);
    state.session.scope.enter("ldap");
    let message = repl_dispatch::leave_repl_shell(&mut state.session).expect("shell should leave");
    assert_eq!(message, "Leaving ldap shell. Back at root.\n");
    assert!(state.session.scope.is_root());
}

#[test]
fn repl_shell_enter_only_from_root_unit() {
    let mut state = make_completion_state(None);
    let ldap = repl::ReplParsedLine::parse("ldap", state.runtime.config.resolved())
        .expect("ldap should parse");
    assert_eq!(ldap.shell_entry_command(&state.session.scope), Some("ldap"));
    state.session.scope.enter("ldap");
    let mreg = repl::ReplParsedLine::parse("mreg", state.runtime.config.resolved())
        .expect("mreg should parse");
    assert_eq!(mreg.shell_entry_command(&state.session.scope), Some("mreg"));
    assert_eq!(ldap.shell_entry_command(&state.session.scope), None);
}

#[test]
fn repl_partial_root_completion_does_not_enter_shell_unit() {
    let state = make_completion_state(None);
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = osp_completion::CompletionEngine::new(tree);

    let (_, suggestions) = engine.complete("or", 2);
    assert!(suggestions.into_iter().any(|entry| matches!(
        entry,
        osp_completion::SuggestionOutput::Item(item) if item.text == "orch"
    )));

    let parsed = repl::ReplParsedLine::parse("or", state.runtime.config.resolved())
        .expect("partial command should parse");
    assert_eq!(parsed.shell_entry_command(&state.session.scope), None);
}

#[test]
fn repl_shell_scoped_completion_and_dispatch_prefix_align_unit() {
    let mut state = make_completion_state(None);
    state.session.scope.enter("orch");
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = osp_completion::CompletionEngine::new(tree);

    let (_, suggestions) = engine.complete("prov", 4);
    assert!(suggestions.into_iter().any(|entry| matches!(
        entry,
        osp_completion::SuggestionOutput::Item(item) if item.text == "provision"
    )));

    let parsed =
        repl::ReplParsedLine::parse("provision --os alma", state.runtime.config.resolved())
            .expect("scoped command should parse");
    assert_eq!(
        parsed.prefixed_tokens(&state.session.scope),
        vec![
            "orch".to_string(),
            "provision".to_string(),
            "--os".to_string(),
            "alma".to_string()
        ]
    );
}

#[test]
fn repl_alias_partial_completion_does_not_trigger_shell_entry_unit() {
    let state = make_completion_state_with_entries(
        None,
        &[("alias.ops", "orch provision --provider vmware")],
    );
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = osp_completion::CompletionEngine::new(tree);

    let (_, suggestions) = engine.complete("op", 2);
    assert!(suggestions.into_iter().any(|entry| matches!(
        entry,
        osp_completion::SuggestionOutput::Item(item) if item.text == "ops"
    )));

    let parsed = repl::ReplParsedLine::parse("op", state.runtime.config.resolved())
        .expect("partial alias should parse");
    assert_eq!(parsed.shell_entry_command(&state.session.scope), None);
}

#[test]
fn repl_help_chrome_replaces_clap_headings_unit() {
    let state = make_completion_state(None);
    let raw =
        "Usage: config <COMMAND>\n\nCommands:\n  show\n\nOptions:\n  -h, --help  Print help\n";
    let rendered =
        repl_help::render_repl_help_with_chrome(repl_view(&state.runtime, &state.session), raw);
    assert!(rendered.contains("  Usage: config <COMMAND>"));
    assert!(rendered.contains("Commands:"));
    assert!(rendered.contains("Options:"));
}

#[test]
fn repl_help_chrome_passthrough_without_known_sections_unit() {
    let state = make_completion_state(None);
    let raw = "custom help text";
    assert_eq!(
        repl_help::render_repl_help_with_chrome(repl_view(&state.runtime, &state.session), raw,),
        raw
    );
}

#[test]
fn help_render_overrides_parse_long_flags_unit() {
    let args = vec![
        OsString::from("osp"),
        OsString::from("--profile"),
        OsString::from("tsd"),
        OsString::from("--theme=dracula"),
        OsString::from("--mode"),
        OsString::from("plain"),
        OsString::from("--color=always"),
        OsString::from("--unicode"),
        OsString::from("never"),
        OsString::from("--no-env"),
        OsString::from("--no-config-file"),
        OsString::from("--ascii"),
    ];

    let parsed = parse_help_render_overrides(&args);
    assert_eq!(parsed.profile.as_deref(), Some("tsd"));
    assert_eq!(parsed.theme.as_deref(), Some("dracula"));
    assert_eq!(parsed.mode, Some(osp_core::output::RenderMode::Plain));
    assert_eq!(parsed.color, Some(osp_core::output::ColorMode::Always));
    assert_eq!(parsed.unicode, Some(osp_core::output::UnicodeMode::Never));
    assert!(parsed.no_env);
    assert!(parsed.no_config_file);
    assert!(parsed.ascii_legacy);
}

#[test]
fn help_render_overrides_skips_next_flag_value_unit() {
    let args = vec![
        OsString::from("osp"),
        OsString::from("--mode"),
        OsString::from("--profile"),
        OsString::from("tsd"),
    ];
    let parsed = parse_help_render_overrides(&args);
    assert_eq!(parsed.mode, None);
    assert_eq!(parsed.profile.as_deref(), Some("tsd"));
}

#[test]
fn help_chrome_uses_unicode_dividers_when_enabled_unit() {
    let state = make_completion_state(None);
    let mut resolved = state.runtime.ui.render_settings.resolve_render_settings();
    resolved.unicode = true;
    let rendered = repl_help::render_help_with_chrome(
        "Usage: osp [OPTIONS]\n\nCommands:\n  help\n\nOptions:\n  -h, --help\n",
        &resolved,
    );
    assert!(rendered.contains("Usage: osp [OPTIONS]"));
    assert!(rendered.contains("Commands:"));
    assert!(rendered.contains("Options:"));
}

#[test]
fn sensitive_key_detection_handles_common_variants_unit() {
    assert!(is_sensitive_key("auth.api_key"));
    assert!(is_sensitive_key("ssh.private_key"));
    assert!(is_sensitive_key("oauth.access_token"));
    assert!(is_sensitive_key("client_secret"));
    assert!(is_sensitive_key("bearer_token"));
    assert!(!is_sensitive_key("ui.keybinding"));
    assert!(!is_sensitive_key("monkey.business"));
}

#[test]
fn repl_completion_tree_contains_builtin_and_plugin_commands_unit() {
    let state = make_completion_state(None);
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);

    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    assert!(tree.root.children.contains_key("help"));
    assert!(tree.root.children.contains_key("exit"));
    assert!(tree.root.children.contains_key("quit"));
    assert!(tree.root.children.contains_key("plugins"));
    assert!(tree.root.children.contains_key("theme"));
    assert!(tree.root.children.contains_key("config"));
    assert!(tree.root.children.contains_key("history"));
    assert!(tree.root.children.contains_key("orch"));
    assert!(
        tree.root.children["orch"]
            .children
            .contains_key("provision")
    );
    assert_eq!(
        tree.root.children["orch"].tooltip.as_deref(),
        Some("Provision orchestrator resources")
    );
    assert!(tree.pipe_verbs.contains_key("F"));
}

#[test]
fn repl_completion_tree_injects_config_set_schema_keys_unit() {
    let state = make_completion_state(None);
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);

    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let set_node = &tree.root.children["config"].children["set"];
    let ui_mode = &set_node.children["ui.mode"];
    assert!(ui_mode.value_key);
    assert!(ui_mode.children.contains_key("auto"));
    assert!(ui_mode.children.contains_key("plain"));
    assert!(ui_mode.children.contains_key("rich"));

    let repl_intro = &set_node.children["repl.intro"];
    assert!(repl_intro.children.contains_key("true"));
    assert!(repl_intro.children.contains_key("false"));
}

#[test]
fn repl_completion_tree_respects_builtin_visibility_unit() {
    let state = make_completion_state(Some("theme"));
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);

    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    assert!(tree.root.children.contains_key("theme"));
    assert!(!tree.root.children.contains_key("config"));
    assert!(!tree.root.children.contains_key("plugins"));
    assert!(!tree.root.children.contains_key("history"));
}

#[test]
fn repl_completion_tree_roots_to_active_shell_scope_unit() {
    let mut state = make_completion_state(None);
    state.session.scope.enter("orch");
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);

    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    assert!(!tree.root.children.contains_key("orch"));
    assert!(tree.root.children.contains_key("provision"));
    assert!(tree.root.children.contains_key("help"));
    assert!(tree.root.children.contains_key("exit"));
    assert!(tree.root.children.contains_key("quit"));
}

#[test]
fn repl_surface_drives_overview_and_completion_visibility_unit() {
    let state = make_completion_state(Some("theme config"));
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);

    let names = surface
        .overview_entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names[..2], ["exit", "help"]);
    assert!(names.contains(&"theme"));
    assert!(names.contains(&"config"));
    assert!(names.contains(&"orch"));
    assert!(!names.contains(&"plugins"));
    assert!(!names.contains(&"history"));
    assert!(surface.root_words.contains(&"theme".to_string()));
    assert!(surface.root_words.contains(&"config".to_string()));
    assert!(surface.root_words.contains(&"orch".to_string()));
}

#[cfg(unix)]
#[test]
fn repl_plugin_error_payload_is_handled_as_error_unit() {
    use std::os::unix::fs::PermissionsExt;

    let dir = make_temp_dir("osp-cli-repl-error-plugin");
    let plugin_path = dir.join("osp-fail");
    std::fs::write(
            &plugin_path,
            r#"#!/usr/bin/env bash
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{"protocol_version":1,"plugin_id":"fail","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{"name":"fail","about":"fail","subcommands":[],"args":[],"flags":{}}]}
JSON
  exit 0
fi
cat <<'JSON'
{"protocol_version":1,"ok":false,"data":{},"error":{"code":"MOCK_ERR","message":"mock failure","details":{}},"meta":{}}
JSON
"#,
        )
        .expect("plugin script should be written");
    let mut perms = std::fs::metadata(&plugin_path)
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&plugin_path, perms).expect("script should be executable");

    let mut state = make_test_state(vec![dir.clone()]);

    let history = make_test_history(&mut state);
    let err = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "fail",
    )
    .expect_err("response ok=false should become repl error");
    assert!(err.to_string().contains("MOCK_ERR: mock failure"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn repl_records_last_rows_and_bounded_cache_unit() {
    use std::os::unix::fs::PermissionsExt;

    let dir = make_temp_dir("osp-cli-repl-session-plugin");
    let plugin_path = dir.join("osp-cache");
    std::fs::write(
            &plugin_path,
            r#"#!/usr/bin/env bash
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{"protocol_version":1,"plugin_id":"cache","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{"name":"cache","about":"cache plugin","subcommands":[],"args":[],"flags":{}}]}
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
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&plugin_path, perms).expect("script should be executable");

    let mut state = make_test_state(vec![dir.clone()]);
    state.session.max_cached_results = 1;

    let history = make_test_history(&mut state);
    let first = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "cache first",
    )
    .expect("first command should succeed");
    match first {
        osp_repl::ReplLineResult::Continue(text) => assert!(text.contains("ok")),
        other => panic!("unexpected repl result: {other:?}"),
    }

    let second = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "cache second",
    )
    .expect("second command should succeed");
    match second {
        osp_repl::ReplLineResult::Continue(text) => assert!(text.contains("ok")),
        other => panic!("unexpected repl result: {other:?}"),
    }

    assert_eq!(state.repl_cache_size(), 1);
    assert!(state.cached_repl_rows("cache first").is_none());
    assert!(state.cached_repl_rows("cache second").is_some());
    assert!(!state.last_repl_rows().is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn plugin_pipeline_rendering_matches_between_cli_and_repl_unit() {
    let dir = make_temp_dir("osp-cli-plugin-pipeline-parity");
    let _plugin_path = write_pipeline_test_plugin(&dir);
    let mut state = make_test_state(vec![dir.clone()]);
    let history = make_test_history(&mut state);
    let stages = vec!["message".to_string()];

    let dispatch_context =
        super::plugin_dispatch_context_for_runtime(&state.runtime, &state.clients, None);
    let response = state
        .clients
        .plugins
        .dispatch("hello", &[], &dispatch_context)
        .expect("plugin dispatch should succeed");
    let prepared = match super::prepare_plugin_response(response, &stages)
        .expect("plugin response should prepare")
    {
        super::PreparedPluginResponse::Output(prepared) => prepared,
        super::PreparedPluginResponse::Failure(failure) => {
            panic!("unexpected plugin failure: {}", failure.report)
        }
    };
    let cli_rendered = render_output(
        &prepared.output,
        &super::resolve_effective_render_settings(
            &state.runtime.ui.render_settings,
            prepared.format_hint,
        ),
    );

    let repl_rendered = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "hello | message",
    )
    .expect("repl command should succeed");
    match repl_rendered {
        osp_repl::ReplLineResult::Continue(text) => {
            assert_eq!(text.trim(), cli_rendered.trim());
            assert!(text.contains("hello-from-plugin"));
        }
        other => panic!("unexpected repl result: {other:?}"),
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn rebuild_repl_state_preserves_session_defaults_and_shell_context_unit() {
    let mut state = make_test_state(Vec::new());
    state
        .session
        .config_overrides
        .set("user.name", "launch-user");
    state
        .session
        .config_overrides
        .set("ui.verbosity.level", "trace");
    state.session.config_overrides.set("debug.level", 2i64);
    state.session.config_overrides.set("ui.format", "json");
    state.session.config_overrides.set("theme.name", "dracula");
    state.session.scope.enter("orch");

    state.session.history_shell = HistoryShellContext::default();
    state.sync_history_shell_context();

    let next = super::rebuild_repl_state(&state).expect("rebuild should succeed");

    assert_eq!(
        next.runtime.config.resolved().get_string("user.name"),
        Some("launch-user")
    );
    assert_eq!(next.runtime.ui.message_verbosity, MessageLevel::Trace);
    assert_eq!(next.runtime.ui.debug_verbosity, 2);
    assert_eq!(next.runtime.ui.render_settings.format, OutputFormat::Json);
    assert_eq!(next.runtime.ui.render_settings.theme_name, "dracula");
    assert_eq!(next.session.scope.commands(), vec!["orch".to_string()]);
    assert_eq!(
        next.session.history_shell.prefix(),
        Some("orch ".to_string())
    );
}

#[cfg(unix)]
#[test]
fn rebuild_repl_state_preserves_session_render_defaults_unit() {
    let mut state = make_test_state(Vec::new());
    state.session.config_overrides.set("ui.format", "table");

    let next = super::rebuild_repl_state(&state).expect("rebuild should succeed");

    assert_eq!(next.runtime.ui.render_settings.format, OutputFormat::Table);
}

#[cfg(unix)]
#[test]
fn repl_reload_intent_matches_command_scope_unit() {
    let mut state = make_test_state(Vec::new());
    state.runtime.themes = crate::theme_loader::load_theme_catalog(state.runtime.config.resolved());
    let history = make_test_history(&mut state);

    let theme_result = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "theme use dracula",
    )
    .expect("theme use should succeed");
    assert!(matches!(
        theme_result,
        osp_repl::ReplLineResult::Restart {
            reload: osp_repl::ReplReloadKind::WithIntro,
            ..
        }
    ));

    let format_result = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "config set ui.format json",
    )
    .expect("config set should succeed");
    assert!(matches!(
        format_result,
        osp_repl::ReplLineResult::Restart {
            reload: osp_repl::ReplReloadKind::Default,
            ..
        }
    ));

    let color_result = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "config set color.prompt.text '#ffffff'",
    )
    .expect("color config set should succeed");
    assert!(matches!(
        color_result,
        osp_repl::ReplLineResult::Restart {
            reload: osp_repl::ReplReloadKind::WithIntro,
            ..
        }
    ));

    let unset_format_result = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "config unset ui.format",
    )
    .expect("config unset should succeed");
    assert!(matches!(
        unset_format_result,
        osp_repl::ReplLineResult::Restart {
            reload: osp_repl::ReplReloadKind::Default,
            ..
        }
    ));

    let unset_color_result = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "config unset color.prompt.text",
    )
    .expect("color config unset should succeed");
    assert!(matches!(
        unset_color_result,
        osp_repl::ReplLineResult::Restart {
            reload: osp_repl::ReplReloadKind::WithIntro,
            ..
        }
    ));

    let dry_run_unset_result = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "config unset ui.format --dry-run",
    )
    .expect("dry-run config unset should succeed");
    assert!(matches!(
        dry_run_unset_result,
        osp_repl::ReplLineResult::Continue(_)
    ));
}

#[cfg(unix)]
#[test]
fn repl_config_unset_rebuilds_runtime_state_unit() {
    let mut state = make_test_state(Vec::new());
    state
        .session
        .config_overrides
        .set_for_profile("default", "ui.format", "table");
    state = super::rebuild_repl_state(&state).expect("rebuild should succeed");
    assert_eq!(state.runtime.ui.render_settings.format, OutputFormat::Table);

    let history = make_test_history(&mut state);
    let result = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "config unset ui.format",
    )
    .expect("config unset should succeed");
    assert!(matches!(
        result,
        osp_repl::ReplLineResult::Restart {
            reload: osp_repl::ReplReloadKind::Default,
            ..
        }
    ));
    assert_eq!(
        layer_value(&state.session.config_overrides, "ui.format"),
        None
    );

    let next = super::rebuild_repl_state(&state).expect("rebuild should succeed");
    assert_eq!(next.runtime.config.resolved().get_string("ui.format"), None);
    assert_eq!(next.runtime.ui.render_settings.format, OutputFormat::Auto);
}

#[cfg(unix)]
#[test]
fn repl_config_unset_dry_run_preserves_session_state_unit() {
    let mut state = make_test_state(Vec::new());
    state
        .session
        .config_overrides
        .set_for_profile("default", "ui.format", "table");
    state = super::rebuild_repl_state(&state).expect("rebuild should succeed");
    assert_eq!(state.runtime.ui.render_settings.format, OutputFormat::Table);

    let history = make_test_history(&mut state);
    let result = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "config unset ui.format --dry-run",
    )
    .expect("dry-run config unset should succeed");
    assert!(matches!(result, osp_repl::ReplLineResult::Continue(_)));
    assert_eq!(
        layer_value(&state.session.config_overrides, "ui.format"),
        Some(&ConfigValue::from("table"))
    );
    assert_eq!(state.runtime.ui.render_settings.format, OutputFormat::Table);
}

#[cfg(unix)]
#[test]
fn repl_exit_is_host_owned_at_root_but_leaves_shell_in_scope_unit() {
    let mut state = make_test_state(Vec::new());
    let history = make_test_history(&mut state);

    let root_exit = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "exit",
    )
    .expect("root exit should be handled by host dispatch");
    assert_eq!(root_exit, osp_repl::ReplLineResult::Exit(0));

    state.session.scope.enter("orch");
    let shell_exit = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "exit",
    )
    .expect("shell exit should leave the current shell");
    match shell_exit {
        osp_repl::ReplLineResult::Continue(text) => {
            assert!(text.contains("Leaving orch shell"));
        }
        other => panic!("unexpected repl result: {other:?}"),
    }
    assert!(state.session.scope.is_root());
}

#[cfg(unix)]
#[test]
fn repl_failure_is_cached_for_doctor_last_unit() {
    let mut state = make_test_state(Vec::new());
    let history = make_test_history(&mut state);

    let err = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "missing",
    )
    .expect_err("unknown command should fail");
    assert!(
        err.to_string()
            .contains("no plugin provides command: missing")
    );

    let last = state
        .last_repl_failure()
        .expect("last failure should be recorded");
    assert_eq!(last.command_line, "missing");
    assert!(last.summary.contains("no plugin provides command: missing"));

    let rendered = doctor_cmd::run_doctor_repl_command(
        doctor_cmd::DoctorCommandContext {
            config: crate::cli::commands::config::ConfigReadContext {
                context: &state.runtime.context,
                config: state.runtime.config.resolved(),
                ui: &state.runtime.ui,
                themes: &state.runtime.themes,
                session_layer: &state.session.config_overrides,
                runtime_load: state.runtime.launch.runtime_load,
            },
            plugins: crate::cli::commands::plugins::PluginsCommandContext {
                config: state.runtime.config.resolved(),
                config_state: Some(&state.runtime.config),
                ui: &state.runtime.ui,
                auth: &state.runtime.auth,
                clients: Some(&state.clients),
                plugin_manager: &state.clients.plugins,
            },
            ui: &state.runtime.ui,
            auth: &state.runtime.auth,
            themes: &state.runtime.themes,
            last_failure: state.session.last_failure.as_ref(),
        },
        crate::cli::DoctorArgs {
            command: Some(crate::cli::DoctorCommands::Last),
        },
        MessageLevel::Success,
    )
    .expect("doctor last should render");
    match rendered {
        ReplCommandOutput::Text(text) => {
            assert!(text.contains("\"status\": \"error\""));
            assert!(text.contains("\"command\": \"missing\""));
        }
        ReplCommandOutput::Output { .. } => panic!("unexpected doctor output variant"),
    }
}

#[cfg(unix)]
#[test]
fn rebuild_repl_state_preserves_last_failure_unit() {
    let mut state = make_test_state(Vec::new());
    state.record_repl_failure("ldap user nope", "boom", "boom detail");

    let next = super::rebuild_repl_state(&state).expect("rebuild should succeed");
    let last = next
        .last_repl_failure()
        .expect("last failure should survive rebuild");

    assert_eq!(last.command_line, "ldap user nope");
    assert_eq!(last.summary, "boom");
    assert_eq!(last.detail, "boom detail");
}

#[cfg(unix)]
#[test]
fn repl_bang_expands_last_visible_command_unit() {
    use std::os::unix::fs::PermissionsExt;

    let dir = make_temp_dir("osp-cli-repl-bang-plugin");
    let plugin_path = dir.join("osp-cache");
    std::fs::write(
            &plugin_path,
            r#"#!/usr/bin/env bash
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{"protocol_version":1,"plugin_id":"cache","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{"name":"cache","about":"cache plugin","subcommands":[],"args":[],"flags":{}}]}
JSON
  exit 0
fi
printf '{"protocol_version":1,"ok":true,"data":{"message":"ok","arg":"%s"},"error":null,"meta":{"format_hint":"table","columns":["message","arg"]}}\n' "$2"
"#,
        )
        .expect("plugin script should be written");
    let mut perms = std::fs::metadata(&plugin_path)
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&plugin_path, perms).expect("script should be executable");

    let mut state = make_test_state(vec![dir.clone()]);
    let history = make_test_history(&mut state);

    repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "cache first",
    )
    .expect("seed command should succeed");
    history
        .save_command_line("cache first")
        .expect("history seed should save");
    let cache_size_before = state.repl_cache_size();
    let expanded = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "!!",
    )
    .expect("bang expansion should succeed");
    match expanded {
        osp_repl::ReplLineResult::ReplaceInput(text) => {
            assert_eq!(text, "cache first");
        }
        other => panic!("unexpected repl result: {other:?}"),
    }
    assert_eq!(state.repl_cache_size(), cache_size_before);

    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn repl_bang_contains_search_expands_matching_command_unit() {
    use std::os::unix::fs::PermissionsExt;

    let dir = make_temp_dir("osp-cli-repl-bang-contains-plugin");
    let plugin_path = dir.join("osp-cache");
    std::fs::write(
            &plugin_path,
            r#"#!/usr/bin/env bash
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{"protocol_version":1,"plugin_id":"cache","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{"name":"cache","about":"cache plugin","subcommands":[],"args":[],"flags":{}}]}
JSON
  exit 0
fi
printf '{"protocol_version":1,"ok":true,"data":{"message":"ok","arg":"%s"},"error":null,"meta":{"format_hint":"table","columns":["message","arg"]}}\n' "$2"
"#,
        )
        .expect("plugin script should be written");
    let mut perms = std::fs::metadata(&plugin_path)
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&plugin_path, perms).expect("script should be executable");

    let mut state = make_test_state(vec![dir.clone()]);
    let history = make_test_history(&mut state);

    repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "cache alpha",
    )
    .expect("first seed command should succeed");
    history
        .save_command_line("cache alpha")
        .expect("history seed should save");
    repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "cache beta",
    )
    .expect("second seed command should succeed");
    history
        .save_command_line("cache beta")
        .expect("history seed should save");
    let cache_size_before = state.repl_cache_size();
    let expanded = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "!?alpha",
    )
    .expect("contains bang expansion should succeed");
    match expanded {
        osp_repl::ReplLineResult::ReplaceInput(text) => {
            assert_eq!(text, "cache alpha");
        }
        other => panic!("unexpected repl result: {other:?}"),
    }
    assert_eq!(state.repl_cache_size(), cache_size_before);

    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
fn make_test_history(state: &mut AppState) -> SharedHistory {
    let history_dir = make_temp_dir("osp-cli-test-history");
    let history_path = history_dir.join("history.jsonl");
    let history_shell = state.session.history_shell.clone();
    state.sync_history_shell_context();

    let history_config = HistoryConfig {
        path: Some(history_path),
        max_entries: 128,
        enabled: true,
        dedupe: true,
        profile_scoped: true,
        exclude_patterns: Vec::new(),
        profile: Some(state.runtime.config.resolved().active_profile().to_string()),
        terminal: Some(
            state
                .runtime
                .context
                .terminal_kind()
                .as_config_terminal()
                .to_string(),
        ),
        shell_context: history_shell,
    }
    .normalized();

    SharedHistory::new(history_config).expect("history should init")
}

#[cfg(unix)]
fn make_test_state(plugin_dirs: Vec<std::path::PathBuf>) -> AppState {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    let config = resolver
        .resolve(ResolveOptions::default().with_terminal("repl"))
        .expect("test config should resolve");

    let settings = RenderSettings::test_plain(OutputFormat::Json);

    let config_root = make_temp_dir("osp-cli-test-config");
    let cache_root = make_temp_dir("osp-cli-test-cache");
    let launch = LaunchContext {
        plugin_dirs: plugin_dirs.clone(),
        config_root: Some(config_root.clone()),
        cache_root: Some(cache_root.clone()),
        runtime_load: RuntimeLoadOptions::default(),
    };

    AppState::new(AppStateInit {
        context: RuntimeContext::new(None, TerminalKind::Repl, None),
        config,
        render_settings: settings,
        message_verbosity: MessageLevel::Success,
        debug_verbosity: 0,
        plugins: PluginManager::new(plugin_dirs).with_roots(Some(config_root), Some(cache_root)),
        themes: crate::theme_loader::ThemeCatalog::default(),
        launch,
    })
}

#[cfg(unix)]
fn write_pipeline_test_plugin(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let plugin_path = dir.join("osp-hello");
    std::fs::write(
            &plugin_path,
            r#"#!/usr/bin/env bash
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{"protocol_version":1,"plugin_id":"hello","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{"name":"hello","about":"hello plugin","args":[],"flags":{},"subcommands":[]}]}
JSON
  exit 0
fi

cat <<'JSON'
{"protocol_version":1,"ok":true,"data":{"message":"hello-from-plugin"},"error":null,"meta":{"format_hint":"table","columns":["message"]}}
JSON
"#,
        )
        .expect("plugin script should be written");
    let mut perms = std::fs::metadata(&plugin_path)
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&plugin_path, perms).expect("script should be executable");
    plugin_path
}

#[cfg(unix)]
fn make_temp_dir(prefix: &str) -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be valid")
        .as_nanos();
    dir.push(format!("{prefix}-{nonce}"));
    std::fs::create_dir_all(&dir).expect("temp dir should be created");
    dir
}
