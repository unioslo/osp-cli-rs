use super::command_output::parse_output_format_hint;
use super::help::parse_help_render_overrides;
use super::{
    EXIT_CODE_CONFIG, EXIT_CODE_PLUGIN, EXIT_CODE_USAGE, PluginConfigEntry, PluginConfigScope,
    ReplCommandOutput, RunAction, RuntimeConfigRequest, build_cli_session_layer,
    build_dispatch_plan, classify_exit_code, collect_plugin_config_env, config_value_to_plugin_env,
    enrich_dispatch_error, is_sensitive_key, plugin_config_env_name, plugin_path_discovery_enabled,
    plugin_process_timeout, render_report_message, resolve_invocation_ui,
    resolve_render_settings_with_hint, run_inline_builtin_command,
};
use crate::app::sink::BufferedUiSink;
use crate::app::{AppState, AppStateInit, LaunchContext, RuntimeContext, TerminalKind};
use crate::cli::commands::doctor as doctor_cmd;
use crate::cli::invocation::{InvocationOptions, scan_cli_argv};
use crate::cli::{Cli, Commands, ConfigCommands, PluginsCommands, ThemeCommands};
use crate::config::{ConfigLayer, ConfigResolver, ConfigValue, ResolveOptions, RuntimeLoadOptions};
use crate::core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use crate::core::plugin::{
    ResponseErrorV1, ResponseMessageLevelV1, ResponseMessageV1, ResponseMetaV1, ResponseV1,
};
use crate::plugin::{
    CommandCatalogEntry, DEFAULT_PLUGIN_PROCESS_TIMEOUT_MS, PluginDispatchError, PluginManager,
    PluginSource,
};
use crate::repl;
use crate::repl::{HistoryConfig, HistoryShellContext, SharedHistory};
use crate::repl::{completion, dispatch as repl_dispatch, help as repl_help, surface};
use crate::ui::document::Block;
use crate::ui::messages::{MessageBuffer, MessageLevel};
use crate::ui::presentation::build_presentation_defaults_layer;
use crate::ui::{RenderSettings, render_output};
use clap::Parser;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;

mod presentation;
mod repl_completion;
#[cfg(unix)]
mod repl_runtime;

fn profiles(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| name.to_string()).collect()
}

fn repl_view<'a>(
    runtime: &'a crate::app::AppRuntime,
    session: &'a crate::app::AppSession,
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
    let options = ResolveOptions::default().with_terminal("repl");
    let base = resolver
        .resolve(options.clone())
        .expect("base test config should resolve");
    resolver.set_presentation(build_presentation_defaults_layer(&base));
    let config = resolver
        .resolve(options)
        .expect("test config should resolve");

    let settings = RenderSettings::test_plain(OutputFormat::Json);

    AppState::new(AppStateInit {
        context: RuntimeContext::new(None, TerminalKind::Repl, None),
        config,
        render_settings: settings,
        message_verbosity: MessageLevel::Success,
        debug_verbosity: 0,
        plugins: PluginManager::new(Vec::new()),
        themes: crate::ui::theme_loader::ThemeCatalog::default(),
        launch: LaunchContext::default(),
    })
}

fn test_config(entries: &[(&str, &str)]) -> crate::config::ResolvedConfig {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    for (key, value) in entries {
        defaults.set(*key, *value);
    }

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    let options = ResolveOptions::default().with_terminal("cli");
    let base = resolver
        .resolve(options.clone())
        .expect("base test config should resolve");
    resolver.set_presentation(build_presentation_defaults_layer(&base));
    resolver
        .resolve(options)
        .expect("test config should resolve")
}

fn synced_render_settings(config: &crate::config::ResolvedConfig) -> RenderSettings {
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
        crate::ui::presentation::help_layout(&config),
    )
}

fn render_message_snapshot(entries: &[(&str, &str)]) -> String {
    let config = test_config(entries);
    let settings = synced_render_settings(&config);
    let resolved = settings.resolve_render_settings();
    let mut messages = MessageBuffer::default();
    messages.error("bad");
    messages.warning("careful");
    messages.render_grouped_with_options(crate::ui::messages::GroupedRenderOptions {
        max_level: MessageLevel::Warning,
        color: resolved.color,
        unicode: resolved.unicode,
        width: resolved.width,
        theme: &resolved.theme,
        layout: crate::ui::presentation::message_layout(&config),
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
    render_output(
        &crate::cli::rows::output::rows_to_output_result(rows),
        &settings,
    )
}

#[test]
fn app_builder_and_runner_delegate_to_host_paths_unit() {
    let app = crate::app::AppBuilder::new().build();
    let mut sink = BufferedUiSink::default();

    assert_eq!(
        app.run_from(["osp", "--help"])
            .expect("app help should render"),
        0
    );
    assert_eq!(
        app.run_with_sink(["osp", "--help"], &mut sink)
            .expect("app help with sink should render"),
        0
    );
    assert_eq!(app.run_process_with_sink(["osp", "--help"], &mut sink), 0);

    let mut runner = crate::app::AppBuilder::new().build_with_sink(&mut sink);
    assert_eq!(
        runner
            .run_from(["osp", "--help"])
            .expect("runner help should render"),
        0
    );
    assert_eq!(runner.run_process(["osp", "--help"]), 0);
}

#[test]
fn bootstrap_message_verbosity_handles_non_utf8_short_flags_and_double_dash_unit() {
    let mut args = vec![
        OsString::from("osp"),
        OsString::from("--verbose"),
        OsString::from("-qv"),
    ];
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        args.push(OsString::from_vec(vec![0xFF]));
    }
    args.extend([
        OsString::from("--"),
        OsString::from("-vvv"),
        OsString::from("--quiet"),
    ]);

    assert_eq!(
        super::bootstrap_message_verbosity(&args),
        MessageLevel::Info
    );
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
fn run_from_with_sink_routes_help_to_stdout_unit() {
    let mut sink = BufferedUiSink::default();

    let exit = super::run_from_with_sink(["osp", "--help"], &mut sink).expect("help should render");

    assert_eq!(exit, 0);
    assert!(!sink.stdout.is_empty());
    assert!(sink.stdout.contains("osp [OPTIONS]"));
    assert!(sink.stderr.is_empty());
}

#[test]
fn run_cli_command_routes_messages_stdout_and_stderr_through_sink_unit() {
    let config = test_config(&[]);
    let ui = crate::app::UiState {
        render_settings: RenderSettings::test_plain(OutputFormat::Value),
        message_verbosity: MessageLevel::Success,
        debug_verbosity: 0,
    };
    let runtime = super::CommandRenderRuntime::new(&config, &ui);
    let mut sink = BufferedUiSink::default();
    let mut messages = MessageBuffer::default();
    messages.success("done");

    let exit = super::run_cli_command(
        &runtime,
        super::CliCommandResult {
            exit_code: 7,
            messages,
            output: Some(super::ReplCommandOutput::Text("payload\n".to_string())),
            stderr_text: Some("warn\n".to_string()),
            failure_report: None,
        },
        &mut sink,
    )
    .expect("command output should render");

    assert_eq!(exit, 7);
    assert_eq!(sink.stdout, "payload\n");
    assert!(sink.stderr.contains("done"));
    assert!(sink.stderr.contains("warn"));
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
fn prepare_plugin_response_keeps_protocol_failures_in_messages_unit() {
    let response = ResponseV1 {
        protocol_version: 1,
        ok: false,
        data: serde_json::json!({}),
        error: Some(ResponseErrorV1 {
            code: "NOT_FOUND".to_string(),
            message: "missing user".to_string(),
            details: serde_json::json!({}),
        }),
        messages: vec![ResponseMessageV1 {
            level: ResponseMessageLevelV1::Warning,
            text: "queried fallback backend".to_string(),
        }],
        meta: ResponseMetaV1::default(),
    };

    let prepared = super::command_output::prepare_plugin_response(response, &[])
        .expect("protocol failure should still parse");

    let super::command_output::PreparedPluginResponse::Failure(failure) = prepared else {
        panic!("expected failure response");
    };

    let rendered = failure.messages.render_grouped(MessageLevel::Trace);
    assert!(rendered.contains("queried fallback backend"));
    assert!(rendered.contains("NOT_FOUND: missing user"));
    assert_eq!(failure.report, "NOT_FOUND: missing user");
}

#[test]
fn prepare_plugin_response_drops_format_hint_after_pipeline_unit() {
    let response = ResponseV1 {
        protocol_version: 1,
        ok: true,
        data: serde_json::json!([{"uid": "alice"}]),
        error: None,
        messages: Vec::new(),
        meta: ResponseMetaV1 {
            format_hint: Some("table".to_string()),
            columns: Some(vec!["uid".to_string()]),
            column_align: Vec::new(),
        },
    };

    let prepared = super::command_output::prepare_plugin_response(response, &["P uid".to_string()])
        .expect("pipeline should apply");

    let super::command_output::PreparedPluginResponse::Output(output) = prepared else {
        panic!("expected successful output response");
    };

    assert!(output.format_hint.is_none());
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
        completion: crate::completion::CommandSpec {
            name: "orch".to_string(),
            tooltip: Some("Provision orchestrator resources".to_string()),
            subcommands: vec![
                crate::completion::CommandSpec::new("provision"),
                crate::completion::CommandSpec::new("status"),
            ],
            ..crate::completion::CommandSpec::default()
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
        completion: crate::completion::CommandSpec {
            name: "orch".to_string(),
            tooltip: Some("Provision orchestrator resources".to_string()),
            subcommands: vec![
                crate::completion::CommandSpec::new("provision")
                    .arg(
                        crate::completion::ArgNode::named("guest")
                            .tooltip("Guest name for the provision request"),
                    )
                    .arg(
                        crate::completion::ArgNode::named("image")
                            .tooltip("Base image to provision")
                            .suggestions([
                                crate::completion::SuggestionEntry::from("ubuntu"),
                                crate::completion::SuggestionEntry::from("alma"),
                            ]),
                    )
                    .flag(
                        "--provider",
                        crate::completion::FlagNode::new().suggestions([
                            crate::completion::SuggestionEntry::from("vmware"),
                            crate::completion::SuggestionEntry::from("nrec"),
                        ]),
                    )
                    .flag(
                        "--os",
                        crate::completion::FlagNode {
                            suggestions: vec![
                                crate::completion::SuggestionEntry::from("rhel"),
                                crate::completion::SuggestionEntry::from("alma"),
                            ],
                            suggestions_by_provider: BTreeMap::from([
                                (
                                    "vmware".to_string(),
                                    vec![crate::completion::SuggestionEntry::from("rhel")],
                                ),
                                (
                                    "nrec".to_string(),
                                    vec![crate::completion::SuggestionEntry::from("alma")],
                                ),
                            ]),
                            ..crate::completion::FlagNode::default()
                        },
                    ),
                crate::completion::CommandSpec::new("status"),
            ],
            ..crate::completion::CommandSpec::default()
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
        completion: crate::completion::CommandSpec {
            name: "hello".to_string(),
            tooltip: Some("hello plugin".to_string()),
            ..crate::completion::CommandSpec::default()
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
fn plugin_path_discovery_defaults_off_and_respects_config_unit() {
    assert!(!plugin_path_discovery_enabled(&test_config(&[])));
    assert!(plugin_path_discovery_enabled(&test_config(&[(
        "extensions.plugins.discovery.path",
        "true",
    )])));
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
fn invocation_ui_overlays_runtime_defaults_per_command_unit() {
    let ui = crate::app::UiState {
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

    let resolved = resolve_invocation_ui(&ui, &invocation);

    assert_eq!(resolved.ui.render_settings.format, OutputFormat::Json);
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
        themes: crate::ui::theme_loader::ThemeCatalog::default(),
        launch,
    })
}

#[cfg(unix)]
fn write_pipeline_test_plugin(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let plugin_path = dir.join("osp-hello");
    std::fs::write(
            &plugin_path,
            r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
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
        r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
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
