use super::command_output::parse_output_format_hint;
use super::help::parse_help_render_overrides;
use super::{
    EXIT_CODE_CONFIG, EXIT_CODE_PLUGIN, EXIT_CODE_USAGE, PluginConfigEntry, PluginConfigScope,
    ReplCommandOutput, RunAction, RuntimeConfigRequest, build_cli_session_layer,
    build_dispatch_plan, classify_exit_code, collect_plugin_config_env, config_value_to_plugin_env,
    doctor_cmd, enrich_dispatch_error, is_sensitive_key, plugin_config_env_name,
    plugin_process_timeout, render_report_message, resolve_effective_invocation,
    resolve_effective_render_settings, run_inline_builtin_command,
};
use crate::cli::{Cli, Commands, ConfigCommands, PluginsCommands, ThemeCommands};
use crate::invocation::{InvocationOptions, scan_cli_argv};
use crate::plugin_manager::{
    CommandCatalogEntry, DEFAULT_PLUGIN_PROCESS_TIMEOUT_MS, PluginDispatchError, PluginManager,
    PluginSource,
};
use crate::repl;
use crate::repl::{completion, dispatch as repl_dispatch, help as repl_help, surface};
use crate::state::{AppState, AppStateInit, LaunchContext, RuntimeContext, TerminalKind};
use clap::Parser;
use osp_config::{ConfigLayer, ConfigResolver, ConfigValue, ResolveOptions, RuntimeLoadOptions};
use osp_core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use osp_repl::{HistoryConfig, HistoryShellContext, SharedHistory};
use osp_ui::document::Block;
use osp_ui::messages::{MessageBuffer, MessageLevel};
use osp_ui::{RenderSettings, render_output};
use std::collections::{BTreeMap, BTreeSet};
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

fn test_config(entries: &[(&str, &str)]) -> osp_config::ResolvedConfig {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    for (key, value) in entries {
        defaults.set(*key, *value);
    }

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    resolver
        .resolve(ResolveOptions::default().with_terminal("cli"))
        .expect("test config should resolve")
}

fn synced_render_settings(config: &osp_config::ResolvedConfig) -> RenderSettings {
    let mut settings = RenderSettings::test_plain(OutputFormat::Table);
    crate::cli::apply_render_settings_from_config(&mut settings, config);
    settings.width = Some(18);
    settings
}

fn render_help_snapshot(entries: &[(&str, &str)]) -> String {
    let config = test_config(entries);
    let settings = synced_render_settings(&config);
    repl_help::render_help_with_chrome(
        "Usage: osp [OPTIONS]\n\nCommands:\n  help\n\nOptions:\n  -h, --help\n",
        &settings.resolve_render_settings(),
        crate::ui_presentation::effective_help_layout(&config),
    )
}

fn render_message_snapshot(entries: &[(&str, &str)]) -> String {
    let config = test_config(entries);
    let settings = synced_render_settings(&config);
    let resolved = settings.resolve_render_settings();
    let mut messages = MessageBuffer::default();
    messages.error("bad");
    messages.warning("careful");
    messages.render_grouped_with_options(osp_ui::messages::GroupedRenderOptions {
        max_level: MessageLevel::Warning,
        color: resolved.color,
        unicode: resolved.unicode,
        width: resolved.width,
        theme: &resolved.theme,
        layout: crate::ui_presentation::effective_message_layout(&config),
        chrome_frame: resolved.chrome_frame,
        style_overrides: resolved.style_overrides.clone(),
    })
}

fn render_prompt_snapshot(entries: &[(&str, &str)]) -> String {
    let state = make_completion_state_with_entries(None, entries);
    crate::repl::presentation::build_repl_prompt(repl_view(&state.runtime, &state.session)).left
}

fn render_table_snapshot(entries: &[(&str, &str)]) -> String {
    let config = test_config(entries);
    let mut settings = crate::cli::default_render_settings();
    settings.runtime.stdout_is_tty = true;
    settings.runtime.terminal = Some("xterm-256color".to_string());
    settings.runtime.locale_utf8 = Some(true);
    settings.color = ColorMode::Never;
    crate::cli::apply_render_settings_from_config(&mut settings, &config);
    settings.format = OutputFormat::Table;
    settings.width = Some(24);

    let rows = vec![
        crate::row! { "uid" => "alice", "count" => 2 },
        crate::row! { "uid" => "bob", "count" => 15 },
    ];
    render_output(&crate::rows::output::rows_to_output_result(rows), &settings)
}

#[test]
fn default_plugin_error_render_preserves_primary_detail_unit() {
    let report = enrich_dispatch_error(PluginDispatchError::NonZeroExit {
        plugin_id: "ldap".to_string(),
        status_code: 7,
        stderr: "backend exploded".to_string(),
    });

    let rendered = render_report_message(&report, MessageLevel::Success);

    assert!(rendered.contains("plugin ldap exited with status 7: backend exploded"));
    assert!(rendered.contains("Hint:"));
}

#[test]
fn verbose_plugin_error_render_includes_detail_chain_unit() {
    let report = enrich_dispatch_error(PluginDispatchError::NonZeroExit {
        plugin_id: "ldap".to_string(),
        status_code: 7,
        stderr: "backend exploded".to_string(),
    });

    let rendered = render_report_message(&report, MessageLevel::Info);

    assert!(rendered.contains("plugin ldap exited with status 7"));
    assert!(rendered.contains("plugin command failed"));
    assert!(rendered.contains("backend exploded"));
}

#[test]
fn exit_code_classification_distinguishes_usage_config_and_plugin_unit() {
    let clap_report =
        super::run_from(["osp", "--definitely-not-a-flag"]).expect_err("parse should fail");
    assert_eq!(classify_exit_code(&clap_report), EXIT_CODE_USAGE);

    let mut invalid_session = ConfigLayer::default();
    invalid_session.set("ui.verbosity.level", "definitely-invalid");
    let config_report = super::resolve_runtime_config(
        RuntimeConfigRequest::new(None, Some("cli")).with_session_layer(Some(invalid_session)),
    )
    .expect_err("config resolution should fail");
    assert_eq!(classify_exit_code(&config_report), EXIT_CODE_CONFIG);

    let plugin_report = enrich_dispatch_error(PluginDispatchError::CommandNotFound {
        command: "ldap".to_string(),
    });
    assert_eq!(classify_exit_code(&plugin_report), EXIT_CODE_PLUGIN);
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
        provider: Some("mock-provider".to_string()),
        providers: vec!["mock-provider (explicit)".to_string()],
        conflicted: false,
        requires_selection: false,
        selected_explicitly: false,
        source: Some(PluginSource::Explicit),
    }]
}

fn sample_catalog_with_provision_context() -> Vec<CommandCatalogEntry> {
    vec![CommandCatalogEntry {
        name: "orch".to_string(),
        about: "Provision orchestrator resources".to_string(),
        subcommands: vec!["provision".to_string(), "status".to_string()],
        completion: osp_completion::CommandSpec {
            name: "orch".to_string(),
            tooltip: Some("Provision orchestrator resources".to_string()),
            subcommands: vec![
                osp_completion::CommandSpec::new("provision")
                    .arg(
                        osp_completion::ArgNode::named("guest")
                            .tooltip("Guest name for the provision request"),
                    )
                    .arg(
                        osp_completion::ArgNode::named("image")
                            .tooltip("Base image to provision")
                            .suggestions([
                                osp_completion::SuggestionEntry::from("ubuntu"),
                                osp_completion::SuggestionEntry::from("alma"),
                            ]),
                    )
                    .flag(
                        "--provider",
                        osp_completion::FlagNode::new().suggestions([
                            osp_completion::SuggestionEntry::from("vmware"),
                            osp_completion::SuggestionEntry::from("nrec"),
                        ]),
                    )
                    .flag(
                        "--os",
                        osp_completion::FlagNode {
                            suggestions: vec![
                                osp_completion::SuggestionEntry::from("rhel"),
                                osp_completion::SuggestionEntry::from("alma"),
                            ],
                            suggestions_by_provider: BTreeMap::from([
                                (
                                    "vmware".to_string(),
                                    vec![osp_completion::SuggestionEntry::from("rhel")],
                                ),
                                (
                                    "nrec".to_string(),
                                    vec![osp_completion::SuggestionEntry::from("alma")],
                                ),
                            ]),
                            ..osp_completion::FlagNode::default()
                        },
                    ),
                osp_completion::CommandSpec::new("status"),
            ],
            ..osp_completion::CommandSpec::default()
        },
        provider: Some("mock-provider".to_string()),
        providers: vec!["mock-provider (explicit)".to_string()],
        conflicted: false,
        requires_selection: false,
        selected_explicitly: false,
        source: Some(PluginSource::Explicit),
    }]
}

fn sample_conflicted_catalog() -> Vec<CommandCatalogEntry> {
    vec![CommandCatalogEntry {
        name: "hello".to_string(),
        about: "hello plugin".to_string(),
        subcommands: Vec::new(),
        completion: osp_completion::CommandSpec {
            name: "hello".to_string(),
            tooltip: Some("hello plugin".to_string()),
            ..osp_completion::CommandSpec::default()
        },
        provider: None,
        providers: vec![
            "alpha-provider (env)".to_string(),
            "beta-provider (user)".to_string(),
        ],
        conflicted: true,
        requires_selection: true,
        selected_explicitly: false,
        source: None,
    }]
}

#[test]
fn theme_slug_is_rendered_as_title_case_display_name_unit() {
    assert_eq!(repl::theme_display_name("rose-pine-moon"), "Rose Pine Moon");
    assert_eq!(repl::theme_display_name("dracula"), "Dracula");
}

#[test]
fn repl_prompt_right_shows_ascii_incognito_marker_unit() {
    let settings = RenderSettings::test_plain(OutputFormat::Table);
    let resolved = settings.resolve_render_settings();
    let timing = crate::state::DebugTimingState::default();

    let rendered = repl::render_repl_prompt_right_for_test(&resolved, false, &timing);

    assert!(rendered.contains("incognito"));
}

#[test]
fn repl_prompt_right_includes_timing_breakdown_at_debug_three_unit() {
    let settings = RenderSettings::test_plain(OutputFormat::Table);
    let resolved = settings.resolve_render_settings();
    let timing = crate::state::DebugTimingState::default();
    timing.set(crate::state::DebugTimingBadge {
        level: 3,
        summary: crate::app::TimingSummary {
            total: std::time::Duration::from_millis(321),
            parse: Some(std::time::Duration::from_millis(4)),
            execute: Some(std::time::Duration::from_millis(300)),
            render: Some(std::time::Duration::from_millis(17)),
        },
    });

    let rendered = repl::render_repl_prompt_right_for_test(&resolved, true, &timing);

    assert!(rendered.contains("321.0ms"));
    assert!(rendered.contains("p4.0ms"));
    assert!(rendered.contains("e300.0ms"));
    assert!(rendered.contains("r17.0ms"));
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
fn plugin_process_timeout_reads_config_override_unit() {
    let config = test_config(&[("extensions.plugins.timeout_ms", "250")]);
    assert_eq!(
        plugin_process_timeout(&config),
        std::time::Duration::from_millis(250)
    );

    let fallback = test_config(&[]);
    assert_eq!(
        plugin_process_timeout(&fallback),
        std::time::Duration::from_millis(DEFAULT_PLUGIN_PROCESS_TIMEOUT_MS as u64)
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
fn effective_invocation_overlays_runtime_defaults_per_command_unit() {
    let ui = crate::state::UiState {
        render_settings: RenderSettings::test_plain(OutputFormat::Table),
        message_verbosity: MessageLevel::Success,
        debug_verbosity: 1,
    };
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

    let effective = resolve_effective_invocation(&ui, &invocation);

    assert_eq!(effective.ui.render_settings.format, OutputFormat::Json);
    assert_eq!(effective.ui.render_settings.mode, RenderMode::Rich);
    assert_eq!(effective.ui.render_settings.color, ColorMode::Always);
    assert_eq!(effective.ui.render_settings.unicode, UnicodeMode::Never);
    assert_eq!(effective.ui.message_verbosity, MessageLevel::Info);
    assert_eq!(effective.ui.debug_verbosity, 3);
    assert_eq!(effective.plugin_provider.as_deref(), Some("beta"));
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
fn repl_ui_projection_supports_flag_prefixed_help_and_completion_unit() {
    let state = make_completion_state(None);
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = osp_completion::CompletionEngine::new(tree);

    let projected = crate::repl::input::project_repl_ui_line(
        "--json help orch prov",
        state.runtime.config.resolved(),
    )
    .expect("projection should succeed");

    let (_, suggestions) = engine.complete(&projected, projected.len());
    assert!(suggestions.into_iter().any(|entry| matches!(
        entry,
        osp_completion::SuggestionOutput::Item(item) if item.text == "provision"
    )));
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
fn repl_structural_alias_exposes_underlying_subcommands_unit() {
    let state = make_completion_state_with_entries(None, &[("alias.ops", "orch")]);
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = osp_completion::CompletionEngine::new(tree);

    let (_, suggestions) = engine.complete("ops prov", "ops prov".len());
    assert!(suggestions.into_iter().any(|entry| matches!(
        entry,
        osp_completion::SuggestionOutput::Item(item) if item.text == "provision"
    )));
}

#[test]
fn repl_alias_with_prefilled_positional_args_inherits_target_flags_unit() {
    let state = make_completion_state_with_entries(None, &[("alias.me", "orch provision guest")]);
    let catalog = sample_catalog_with_provision_context();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = osp_completion::CompletionEngine::new(tree.clone());

    let alias_node = tree
        .root
        .children
        .get("me")
        .expect("alias node should exist");
    assert_eq!(alias_node.prefilled_positionals, vec!["guest".to_string()]);

    let (_, suggestions) = engine.complete("me --", "me --".len());
    let values = suggestions
        .into_iter()
        .filter_map(|entry| match entry {
            osp_completion::SuggestionOutput::Item(item) => Some(item.text),
            osp_completion::SuggestionOutput::PathSentinel => None,
        })
        .collect::<Vec<_>>();

    assert!(values.contains(&"--provider".to_string()));
    assert!(values.contains(&"--os".to_string()));
}

#[test]
fn repl_alias_prefilled_context_filters_provider_scoped_values_unit() {
    let state = make_completion_state_with_entries(
        None,
        &[("alias.me", "orch provision guest --provider vmware")],
    );
    let catalog = sample_catalog_with_provision_context();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = osp_completion::CompletionEngine::new(tree);

    let (_, suggestions) = engine.complete("me --os ", "me --os ".len());
    let values = suggestions
        .into_iter()
        .filter_map(|entry| match entry {
            osp_completion::SuggestionOutput::Item(item) => Some(item.text),
            osp_completion::SuggestionOutput::PathSentinel => None,
        })
        .collect::<Vec<_>>();

    assert!(values.contains(&"rhel".to_string()));
    assert!(!values.contains(&"alma".to_string()));
}

#[test]
fn repl_alias_placeholder_keeps_following_arg_slot_open_unit() {
    let state =
        make_completion_state_with_entries(None, &[("alias.me", "orch provision guest ${1}")]);
    let catalog = sample_catalog_with_provision_context();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = osp_completion::CompletionEngine::new(tree);

    let (_, suggestions) = engine.complete("me ", "me ".len());
    let values = suggestions
        .into_iter()
        .filter_map(|entry| match entry {
            osp_completion::SuggestionOutput::Item(item) => Some(item.text),
            osp_completion::SuggestionOutput::PathSentinel => None,
        })
        .collect::<Vec<_>>();

    assert!(values.contains(&"ubuntu".to_string()));
    assert!(values.contains(&"alma".to_string()));
}

#[test]
fn repl_help_chrome_replaces_clap_headings_unit() {
    let state = make_completion_state(None);
    let raw =
        "Usage: config <COMMAND>\n\nCommands:\n  show\n\nOptions:\n  -h, --help  Print help\n";
    let rendered =
        repl_help::render_repl_help_with_chrome(repl_view(&state.runtime, &state.session), raw);
    assert!(rendered.contains("Usage"));
    assert!(rendered.contains("config <COMMAND>"));
    assert!(rendered.contains("Commands"));
    assert!(rendered.contains("Options"));
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
fn austere_repl_intro_is_minimal_single_line_unit() {
    let state = make_completion_state_with_entries(None, &[("ui.presentation", "austere")]);
    let rendered =
        crate::repl::presentation::render_repl_intro(repl_view(&state.runtime, &state.session));

    assert_eq!(
        rendered,
        format!(
            "\nWelcome anonymous. v{}. Commands: help, config, theme, plugins. See help for more.\n\n",
            env!("CARGO_PKG_VERSION")
        )
    );
}

#[test]
fn compact_repl_intro_is_minimal_single_line_unit() {
    let state = make_completion_state_with_entries(None, &[("ui.presentation", "compact")]);
    let rendered =
        crate::repl::presentation::render_repl_intro(repl_view(&state.runtime, &state.session));

    assert_eq!(
        rendered,
        format!(
            "\nWelcome anonymous. v{}. Commands: help, config, theme, plugins. See help for more.\n\n",
            env!("CARGO_PKG_VERSION")
        )
    );
}

#[test]
fn presentation_profiles_shape_help_output_snapshot_unit() {
    assert_eq!(
        render_help_snapshot(&[("ui.presentation", "expressive")]),
        "- Usage ----------\n  osp [OPTIONS]\n------------------\n\n- Commands -------\n  help\n------------------\n\n- Options --------\n  -h, --help\n------------------\n"
    );
    assert_eq!(
        render_help_snapshot(&[("ui.presentation", "compact")]),
        "- Usage ----------\n  osp [OPTIONS]\n\n- Commands -------\n  help\n\n- Options --------\n  -h, --help\n"
    );
    assert_eq!(
        render_help_snapshot(&[("ui.presentation", "austere")]),
        "Usage:\n  osp [OPTIONS]\nCommands:\n  help\nOptions:\n  -h, --help\n"
    );
}

#[test]
fn presentation_profiles_shape_message_output_snapshot_unit() {
    assert_eq!(
        render_message_snapshot(&[("ui.presentation", "expressive")]),
        "- Errors ---------\n- bad\n------------------\n\n- Warnings -------\n- careful\n------------------\n"
    );
    assert_eq!(
        render_message_snapshot(&[("ui.presentation", "compact")]),
        "- Errors ---------\n- bad\n\n- Warnings -------\n- careful\n"
    );
    assert_eq!(
        render_message_snapshot(&[("ui.presentation", "austere")]),
        "error: bad\nwarning: careful\n"
    );
}

#[test]
fn presentation_profiles_shape_prompt_output_snapshot_unit() {
    assert_eq!(
        render_prompt_snapshot(&[("ui.presentation", "expressive")]),
        "╭─anonymous@local \n╰─default> "
    );
    assert_eq!(
        render_prompt_snapshot(&[("ui.presentation", "compact")]),
        "default> "
    );
    assert_eq!(
        render_prompt_snapshot(&[("ui.presentation", "austere")]),
        "default> "
    );
}

#[test]
fn presentation_profiles_shape_table_output_snapshot_unit() {
    assert_eq!(
        render_table_snapshot(&[("ui.presentation", "expressive")]),
        "╭━━━━━━━┳━━━━━━━╮\n┃ uid   ┃ count ┃\n┡━━━━━━━╇━━━━━━━┩\n│ alice │ 2     │\n│ bob   │ 15    │\n╰───────┴───────╯\n"
    );
    assert_eq!(
        render_table_snapshot(&[("ui.presentation", "compact")]),
        "+--------+--------+\n| uid    | count  |\n+--------+--------+\n| alice  | 2      |\n| bob    | 15     |\n+--------+--------+\n"
    );
    assert_eq!(
        render_table_snapshot(&[("ui.presentation", "austere")]),
        "+--------+--------+\n| uid    | count  |\n+--------+--------+\n| alice  | 2      |\n| bob    | 15     |\n+--------+--------+\n"
    );
}

#[test]
fn help_render_overrides_parse_long_flags_unit() {
    let args = vec![
        OsString::from("osp"),
        OsString::from("--profile"),
        OsString::from("tsd"),
        OsString::from("--theme=dracula"),
        OsString::from("--presentation"),
        OsString::from("compact"),
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
    assert_eq!(
        parsed.presentation,
        Some(crate::ui_presentation::UiPresentation::Compact)
    );
    assert_eq!(parsed.mode, Some(osp_core::output::RenderMode::Plain));
    assert_eq!(parsed.color, Some(osp_core::output::ColorMode::Always));
    assert_eq!(parsed.unicode, Some(osp_core::output::UnicodeMode::Never));
    assert!(parsed.no_env);
    assert!(parsed.no_config_file);
    assert!(parsed.ascii_legacy);
}

#[test]
fn help_render_overrides_parse_gammel_og_bitter_alias_unit() {
    let args = vec![OsString::from("osp"), OsString::from("--gammel-og-bitter")];

    let parsed = parse_help_render_overrides(&args);
    assert!(parsed.gammel_og_bitter);
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
        crate::ui_presentation::HelpLayout::Full,
    );
    assert!(rendered.contains("Usage"));
    assert!(rendered.contains("osp [OPTIONS]"));
    assert!(rendered.contains("Commands"));
    assert!(rendered.contains("Options"));
}

#[test]
fn austere_help_layout_collapses_footer_spacing_unit() {
    let state = make_completion_state_with_entries(None, &[("ui.presentation", "austere")]);
    let raw = "Usage: osp [OPTIONS]\n\nOptions:\n  -h, --help\n\nUse `osp plugins commands` to list plugin-provided commands.\n";
    let rendered =
        repl_help::render_repl_help_with_chrome(repl_view(&state.runtime, &state.session), raw);

    assert!(rendered.contains("Options"));
    assert!(rendered.contains("Use `osp plugins commands`"));
    assert!(!rendered.contains("\n\nUse `osp plugins commands`"));
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

#[test]
fn compact_repl_surface_omits_options_overview_and_prioritizes_builtins_unit() {
    let state =
        make_completion_state_with_entries(Some("theme config"), &[("ui.presentation", "compact")]);
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);

    let names = surface
        .overview_entries
        .iter()
        .map(|entry| entry.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names[..4], ["exit", "help", "theme", "config"]);
    assert!(!names.contains(&"options"));
    let orch_index = names
        .iter()
        .position(|name| *name == "orch")
        .expect("orch should be present");
    let config_index = names
        .iter()
        .position(|name| *name == "config")
        .expect("config should be present");
    assert!(config_index < orch_index);
}

#[test]
fn compact_root_completion_suggestions_prioritize_core_commands_unit() {
    let state = make_completion_state_with_entries(None, &[("ui.presentation", "compact")]);
    let catalog = sample_catalog();
    let surface = surface::build_repl_surface(repl_view(&state.runtime, &state.session), &catalog);
    let tree =
        completion::build_repl_completion_tree(repl_view(&state.runtime, &state.session), &surface);
    let engine = osp_completion::CompletionEngine::new(tree);

    let (_, suggestions) = engine.complete("", 0);
    let labels = suggestions
        .into_iter()
        .filter_map(|entry| match entry {
            osp_completion::SuggestionOutput::Item(item) => Some(item.text),
            _ => None,
        })
        .take(6)
        .collect::<Vec<_>>();

    assert_eq!(
        labels[..6],
        ["help", "exit", "quit", "config", "theme", "plugins"]
    );
}

#[test]
fn repl_surface_exposes_selected_provider_for_conflicts_unit() {
    let state = make_completion_state(None);
    let surface = surface::build_repl_surface(
        repl_view(&state.runtime, &state.session),
        &sample_conflicted_catalog(),
    );

    let overview = surface
        .overview_entries
        .iter()
        .find(|entry| entry.name == "hello")
        .expect("hello overview should exist");
    assert!(overview.summary.contains("provider selection required"));
    assert!(overview.summary.contains("--plugin-provider"));
    assert!(overview.summary.contains("beta-provider (user)"));

    let spec = surface
        .specs
        .iter()
        .find(|entry| entry.name == "hello")
        .expect("hello command spec should exist");
    let tooltip = spec.tooltip.as_deref().expect("tooltip should exist");
    assert!(tooltip.contains("provider selection required"));
    assert!(tooltip.contains("beta-provider (user)"));
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
fn repl_cache_reuses_external_result_across_pipelines_unit() {
    use std::os::unix::fs::PermissionsExt;

    let dir = make_temp_dir("osp-cli-repl-cache-plugin");
    let log_path = dir.join("invocations.log");
    let plugin_path = dir.join("osp-slowcache");
    let script = format!(
        r#"#!/usr/bin/env bash
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"slowcache","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"slowcache","about":"cache plugin","subcommands":[],"args":[],"flags":{{}}}}]}}
JSON
  exit 0
fi

printf 'run\n' >> "{log_path}"
count=$(wc -l < "{log_path}" | tr -d ' ')
cat <<JSON
{{"protocol_version":1,"ok":true,"data":{{"message":"cached","counter":$count}},"error":null,"meta":{{"format_hint":"table","columns":["message","counter"]}}}}
JSON
"#,
        log_path = log_path.display(),
    );

    std::fs::write(&plugin_path, script).expect("plugin script should be written");
    let mut perms = std::fs::metadata(&plugin_path)
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&plugin_path, perms).expect("script should be executable");

    let mut state = make_test_state(vec![dir.clone()]);
    let history = make_test_history(&mut state);

    let first = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "slowcache --cache | counter",
    )
    .expect("first cached command should succeed");
    match first {
        osp_repl::ReplLineResult::Continue(text) => assert!(text.contains('1')),
        other => panic!("unexpected repl result: {other:?}"),
    }

    let second = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "slowcache --cache | message",
    )
    .expect("second cached command should succeed");
    match second {
        osp_repl::ReplLineResult::Continue(text) => assert!(text.contains("cached")),
        other => panic!("unexpected repl result: {other:?}"),
    }

    let log = std::fs::read_to_string(&log_path).expect("invocation log should exist");
    assert_eq!(log.lines().count(), 1);

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
fn repl_plugin_provider_override_dispatches_selected_provider_unit() {
    let dir = make_temp_dir("osp-cli-repl-provider-override");
    let _alpha = write_provider_test_plugin(&dir, "alpha-provider", "hello", "alpha");
    let _beta = write_provider_test_plugin(&dir, "beta-provider", "hello", "beta");
    let mut state = make_test_state(vec![dir.clone()]);
    let history = make_test_history(&mut state);

    let err = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "hello",
    )
    .expect_err("ambiguous plugin command should fail");
    assert!(format!("{err:#}").contains("provided by multiple plugins"));

    let repl_rendered = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "--plugin-provider beta-provider hello",
    )
    .expect("repl command should honor one-shot provider override");
    match repl_rendered {
        osp_repl::ReplLineResult::Continue(text) => {
            assert!(text.contains("beta-from-plugin"));
            assert!(!text.contains("alpha-from-plugin"));
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
fn repl_config_prompt_color_change_rebuilds_deterministically_unit() {
    let mut state = make_test_state(Vec::new());
    state
        .session
        .config_overrides
        .set("ui.color.mode", "always");
    state.session.config_overrides.set("ui.mode", "rich");
    state
        .session
        .config_overrides
        .set("repl.simple_prompt", true);
    state = super::rebuild_repl_state(&state).expect("rebuild should succeed");
    assert!(
        state
            .runtime
            .ui
            .render_settings
            .resolve_render_settings()
            .color
    );

    let default_prompt =
        crate::repl::presentation::build_repl_prompt(repl_view(&state.runtime, &state.session))
            .left;
    assert!(default_prompt.contains("\x1b["));

    let history = make_test_history(&mut state);
    let result = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "config set color.prompt.text white",
    )
    .expect("prompt color config set should succeed");
    assert!(matches!(
        result,
        osp_repl::ReplLineResult::Restart {
            reload: osp_repl::ReplReloadKind::WithIntro,
            ..
        }
    ));

    state = super::rebuild_repl_state(&state).expect("rebuild should succeed");
    let white_prompt =
        crate::repl::presentation::build_repl_prompt(repl_view(&state.runtime, &state.session))
            .left;
    assert!(white_prompt.contains("\x1b[37mdefault"));

    let history = make_test_history(&mut state);
    let result = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "config unset color.prompt.text",
    )
    .expect("prompt color config unset should succeed");
    assert!(matches!(
        result,
        osp_repl::ReplLineResult::Restart {
            reload: osp_repl::ReplReloadKind::WithIntro,
            ..
        }
    ));

    state = super::rebuild_repl_state(&state).expect("rebuild should succeed");
    let restored_prompt =
        crate::repl::presentation::build_repl_prompt(repl_view(&state.runtime, &state.session))
            .left;
    assert_eq!(restored_prompt, default_prompt);
}

#[cfg(unix)]
#[test]
fn repl_builtin_overrides_do_not_mutate_runtime_ui_state_unit() {
    let mut state = make_test_state(Vec::new());
    let history = make_test_history(&mut state);
    let original_message = state.runtime.ui.message_verbosity;
    let original_debug = state.runtime.ui.debug_verbosity;

    repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "-q config get missing.key",
    )
    .expect("quiet config get should complete");
    assert_eq!(state.runtime.ui.message_verbosity, original_message);
    assert_eq!(state.runtime.ui.debug_verbosity, original_debug);

    repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "-d doctor last",
    )
    .expect("doctor last with debug override should complete");
    assert_eq!(state.runtime.ui.message_verbosity, original_message);
    assert_eq!(state.runtime.ui.debug_verbosity, original_debug);
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
    let err_text = err.to_string();
    assert!(err_text.contains("plugin command failed"));

    let last = state
        .last_repl_failure()
        .expect("last failure should be recorded");
    assert_eq!(last.command_line, "missing");
    assert!(last.summary.contains("plugin command failed"));

    let rendered = doctor_cmd::run_doctor_command(
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
    )
    .expect("doctor last should render");
    match rendered.output {
        Some(ReplCommandOutput::Document(document)) => {
            let Some(Block::Json(json)) = document.blocks.first() else {
                panic!("expected doctor last json document");
            };
            assert_eq!(json.payload["status"], "error");
            assert_eq!(json.payload["command"], "missing");
        }
        Some(ReplCommandOutput::Output { .. }) => panic!("unexpected doctor output variant"),
        Some(ReplCommandOutput::Text(_)) => panic!("unexpected doctor output variant"),
        None => panic!("expected doctor output"),
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
#[test]
fn repl_bang_contains_search_respects_shell_scope_unit() {
    let mut state = make_test_state(Vec::new());
    let history = make_test_history(&mut state);

    history
        .save_command_line("config show")
        .expect("root history seed should save");

    state.session.scope.enter("orch");
    state.sync_history_shell_context();
    history
        .save_command_line("status")
        .expect("scoped history seed should save");

    let scoped_hit = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "!?status",
    )
    .expect("scoped bang search should succeed");
    match scoped_hit {
        osp_repl::ReplLineResult::ReplaceInput(text) => assert_eq!(text, "status"),
        other => panic!("unexpected repl result: {other:?}"),
    }

    let scoped_miss = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "!?config",
    )
    .expect("cross-shell bang miss should render feedback");
    match scoped_miss {
        osp_repl::ReplLineResult::Continue(text) => {
            assert_eq!(text, "No history match for: !?config\n");
        }
        other => panic!("unexpected repl result: {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn repl_invalid_subcommand_renders_inline_help_unit() {
    let mut state = make_test_state(Vec::new());
    let history = make_test_history(&mut state);

    let rendered = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "config sho",
    )
    .expect("invalid subcommand should stay inside repl help flow");

    match rendered {
        osp_repl::ReplLineResult::Continue(text) => {
            assert!(text.contains("unrecognized subcommand"));
            assert!(text.contains("config <COMMAND>"));
            assert!(!text.contains("For more information, try '--help'."));
        }
        other => panic!("unexpected repl result: {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn repl_flag_prefixed_help_alias_dispatches_to_command_help_unit() {
    let mut state = make_test_state(Vec::new());
    let history = make_test_history(&mut state);

    let rendered = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "-q help config",
    )
    .expect("flag-prefixed help alias should stay in help flow");

    match rendered {
        osp_repl::ReplLineResult::Continue(text) => {
            assert!(text.contains("config <COMMAND>"));
            assert!(text.contains("Common Invocation Options"));
        }
        other => panic!("unexpected repl result: {other:?}"),
    }
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
fn write_provider_test_plugin(
    dir: &std::path::Path,
    plugin_id: &str,
    command_name: &str,
    message: &str,
) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let plugin_path = dir.join(format!("osp-{plugin_id}"));
    let script = format!(
        r#"#!/usr/bin/env bash
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{plugin_id}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{command_name}","about":"{plugin_id} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"{message}-from-plugin"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
        plugin_id = plugin_id,
        command_name = command_name,
        message = message,
    );
    std::fs::write(&plugin_path, script).expect("plugin script should be written");
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
