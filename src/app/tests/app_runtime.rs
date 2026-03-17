use super::*;
use miette::WrapErr;

#[test]
fn app_help_entrypoints_and_sink_routing_render_help_unit() {
    let app = crate::app::App::builder().build();
    let mut help_sink = BufferedUiSink::default();

    assert_eq!(
        app.run_from(["osp", "--help"])
            .expect("app help should render"),
        0
    );
    assert_eq!(
        app.run_with_sink(["osp", "--help"], &mut help_sink)
            .expect("app help with sink should render"),
        0
    );
    assert_eq!(
        app.run_process_with_sink(["osp", "--help"], &mut help_sink),
        0
    );
    assert!(help_sink.stdout.contains("osp [OPTIONS]"));
    assert!(help_sink.stderr.is_empty());

    let mut sink = BufferedUiSink::default();
    let exit = super::run_from_with_sink(["osp", "--help"], &mut sink).expect("help should render");
    assert_eq!(exit, 0);
    assert!(!sink.stdout.is_empty());
    assert!(sink.stdout.contains("osp [OPTIONS]"));
    assert!(sink.stderr.is_empty());

    let mut runner_sink = BufferedUiSink::default();
    let mut runner = crate::app::App::builder().build_with_sink(&mut runner_sink);
    assert_eq!(
        runner
            .run_from(["osp", "--help"])
            .expect("runner help should render"),
        0
    );
    assert_eq!(runner.run_process(["osp", "--help"]), 0);
}

#[test]
fn native_commands_project_into_auth_catalog_unit() {
    let state = make_completion_state_with_entries_and_native(
        None,
        &[("auth.visible.plugins", "ldap")],
        test_native_registry(),
    );
    let catalog = super::authorized_command_catalog_for(&state.runtime.auth, &state.clients)
        .expect("catalog should render");
    assert!(catalog.iter().any(|entry| entry.name == "ldap"));
    assert!(
        state
            .runtime
            .auth
            .external_command_access("ldap")
            .is_visible()
    );
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
fn error_rendering_prioritizes_actionable_details_across_levels_unit() {
    let report = enrich_dispatch_error(PluginDispatchError::NonZeroExit {
        plugin_id: "ldap".to_string(),
        status_code: 7,
        stderr: "backend exploded".to_string(),
    });

    let rendered = render_report_message(&report, MessageLevel::Success);
    assert!(rendered.contains("plugin ldap exited with status 7: backend exploded"));
    assert!(rendered.contains("Hint:"));

    let rendered = render_report_message(&report, MessageLevel::Info);
    assert!(rendered.contains("plugin ldap exited with status 7"));
    assert!(rendered.contains("plugin command failed"));
    assert!(rendered.contains("backend exploded"));

    let report = Err::<(), _>(miette::miette!("unknown theme: missing-theme"))
        .wrap_err("failed to derive host runtime inputs for startup")
        .expect_err("wrapped error should stay an error");
    let rendered = render_report_message(&report, MessageLevel::Success);
    assert!(rendered.contains("unknown theme: missing-theme"));
    assert!(!rendered.starts_with("failed to derive host runtime inputs for startup"));
}

#[test]
fn run_cli_command_routes_messages_stdout_and_stderr_through_sink_unit() {
    let config = test_config(&[]);
    let ui = crate::app::UiState::new(
        RenderSettings::test_plain(OutputFormat::Value),
        MessageLevel::Success,
        0,
    );
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
fn run_cli_command_with_ui_builds_runtime_from_config_and_ui_unit() {
    let config = test_config(&[]);
    let ui = crate::app::UiState::new(
        RenderSettings::test_plain(OutputFormat::Value),
        MessageLevel::Success,
        0,
    );
    let mut sink = BufferedUiSink::default();

    let exit = super::run_cli_command_with_ui(
        &config,
        &ui,
        super::CliCommandResult {
            exit_code: 3,
            messages: MessageBuffer::default(),
            output: Some(super::ReplCommandOutput::Text("payload\n".to_string())),
            stderr_text: None,
            failure_report: None,
        },
        &mut sink,
    )
    .expect("command output should render");

    assert_eq!(exit, 3);
    assert_eq!(sink.stdout, "payload\n");
    assert!(sink.stderr.is_empty());
}

#[test]
fn render_messages_for_ui_uses_ui2_as_the_runtime_owner_unit() {
    let config = test_config(&[]);
    let mut settings = RenderSettings::test_plain(OutputFormat::Value);
    settings.mode = RenderMode::Rich;
    settings.color = ColorMode::Always;
    settings.runtime.stdout_is_tty = true;
    settings.theme_name = "dracula".to_string();
    let ui = crate::app::UiState::new(settings, MessageLevel::Trace, 0);
    let mut messages = MessageBuffer::default();
    messages.warning("careful");

    let rendered =
        crate::ui::render_messages(&config, &ui.render_settings, &messages, MessageLevel::Trace);

    assert!(rendered.contains("careful"));
    assert!(rendered.contains("Warnings"));
    assert!(rendered.contains('\u{1b}'));
}

#[test]
fn render_messages_for_ui_supports_plain_layout_in_ui2_unit() {
    let mut resolver = crate::config::ConfigResolver::default();
    let mut defaults = crate::config::ConfigLayer::default();
    defaults.set("profile.default", "default");
    resolver.set_defaults(defaults);

    let options = crate::config::ResolveOptions::default().with_terminal("cli");
    let base = resolver
        .resolve(options.clone())
        .expect("base test config should resolve");
    resolver.set_presentation(crate::ui::build_presentation_defaults_layer(&base));

    let mut session = crate::config::ConfigLayer::default();
    session.set("ui.messages.layout", "plain");
    resolver.set_session(session);

    let config = resolver
        .resolve(options)
        .expect("plain layout override should resolve");
    let mut settings = RenderSettings::test_plain(OutputFormat::Value);
    settings.mode = RenderMode::Rich;
    settings.color = ColorMode::Always;
    settings.runtime.stdout_is_tty = true;
    settings.theme_name = "dracula".to_string();
    let ui = crate::app::UiState::new(settings, MessageLevel::Trace, 0);
    let mut messages = MessageBuffer::default();
    messages.warning("careful");

    let rendered =
        crate::ui::render_messages(&config, &ui.render_settings, &messages, MessageLevel::Trace);

    assert!(!rendered.contains("Warnings"));
    assert!(rendered.contains("careful"));
    assert!(rendered.contains("  careful"));
}

#[test]
fn state_and_client_builders_produce_coherent_embedder_state_unit() {
    let config = test_config(&[]);
    let ui = crate::app::UiState::new(
        RenderSettings::test_plain(OutputFormat::Json),
        MessageLevel::Trace,
        2,
    );
    let launch = crate::app::LaunchContext::default()
        .with_plugin_dir("/tmp/osp-plugin-a")
        .with_config_root(Some(std::path::PathBuf::from("/tmp/osp-config")))
        .with_cache_root(Some(std::path::PathBuf::from("/tmp/osp-cache")));
    let session = crate::app::AppSession::with_cache_limit(5).with_prompt_prefix("osp-dev");

    let state = crate::app::AppStateBuilder::new(
        crate::app::RuntimeContext::new(None, crate::app::TerminalKind::Cli, None),
        config,
        ui,
    )
    .with_launch(launch)
    .with_session(session)
    .with_native_commands(test_native_registry())
    .build();

    assert_eq!(state.runtime.ui.message_verbosity, MessageLevel::Trace);
    assert_eq!(state.runtime.ui.debug_verbosity, 2);
    assert_eq!(state.session.prompt_prefix, "osp-dev");
    assert_eq!(state.session.max_cached_results, 5);
    assert_eq!(
        state.clients.plugins().explicit_dirs(),
        &[std::path::PathBuf::from("/tmp/osp-plugin-a")]
    );
    assert_eq!(
        state.clients.plugins().config_root(),
        Some(std::path::Path::new("/tmp/osp-config"))
    );
    assert_eq!(
        state.clients.plugins().cache_root(),
        Some(std::path::Path::new("/tmp/osp-cache"))
    );
    assert!(state.clients.native_commands().command("ldap").is_some());

    let config = test_config(&[
        ("ui.message.verbosity", "trace"),
        ("debug.level", "2"),
        ("theme.name", "dracula"),
    ]);
    let context = crate::app::RuntimeContext::new(
        None,
        crate::app::TerminalKind::Cli,
        Some("xterm-256color".to_string()),
    );
    let ui = crate::app::UiState::from_resolved_config(&context, &config)
        .expect("ui state should derive from resolved config");
    assert_eq!(ui.message_verbosity, MessageLevel::Trace);
    assert_eq!(ui.debug_verbosity, 2);
    assert_eq!(ui.render_settings.theme_name, "dracula");

    let state = crate::app::AppState::from_resolved_config(context, config)
        .expect("app state should derive from resolved config");
    assert_eq!(state.runtime.ui.message_verbosity, MessageLevel::Trace);
    assert_eq!(state.runtime.ui.debug_verbosity, 2);
    assert_eq!(state.runtime.ui.render_settings.theme_name, "dracula");
    assert!(state.clients.plugins().explicit_dirs().is_empty());

    let clients = crate::app::AppClients::new(
        crate::plugin::PluginManager::new(vec![std::path::PathBuf::from("/tmp/osp-plugin-a")]),
        test_native_registry(),
    );
    assert_eq!(
        clients.plugins().explicit_dirs(),
        &[std::path::PathBuf::from("/tmp/osp-plugin-a")]
    );
    assert!(clients.native_commands().command("ldap").is_some());
}

#[test]
fn state_builder_from_host_inputs_preserves_derived_plugin_and_theme_state_unit() {
    let config = test_config(&[
        ("ui.message.verbosity", "trace"),
        ("debug.level", "2"),
        ("theme.name", "dracula"),
    ]);
    let context = crate::app::RuntimeContext::new(
        None,
        crate::app::TerminalKind::Cli,
        Some("xterm-256color".to_string()),
    );
    let launch = crate::app::LaunchContext::default().with_plugin_dir("/tmp/osp-plugin-a");
    let host_inputs = crate::app::assembly::ResolvedHostInputs::derive(
        &context,
        &config,
        &launch,
        crate::app::assembly::RenderSettingsSeed::DefaultAuto,
        None,
        None,
        None,
    )
    .expect("host inputs should derive");

    let state = crate::app::AppStateBuilder::from_host_inputs(context, config, host_inputs)
        .with_launch(launch)
        .build();

    assert_eq!(state.runtime.ui.message_verbosity, MessageLevel::Trace);
    assert_eq!(state.runtime.ui.debug_verbosity, 2);
    assert_eq!(state.runtime.ui.render_settings.theme_name, "dracula");
    assert_eq!(
        state.clients.plugins().explicit_dirs(),
        &[std::path::PathBuf::from("/tmp/osp-plugin-a")]
    );
}

#[test]
fn app_builder_product_defaults_flow_through_host_bootstrap_unit() {
    let mut product_defaults = ConfigLayer::default();
    product_defaults.set("extensions.site.enabled", true);
    product_defaults.set_for_terminal("cli", "extensions.site.banner", "cli-wrapper");
    product_defaults.set_for_terminal("repl", "extensions.site.banner", "repl-wrapper");

    let app = crate::app::App::builder()
        .with_native_commands(product_defaults_registry())
        .with_product_defaults(product_defaults)
        .build();

    let mut status_sink = BufferedUiSink::default();
    let status_exit = app.run_process_with_sink(["osp", "site-status"], &mut status_sink);
    assert_eq!(status_exit, 0);
    assert!(status_sink.stdout.contains("site_enabled=true"));
    assert!(status_sink.stdout.contains("site_banner=cli-wrapper"));
    assert!(status_sink.stdout.contains("active_profile=default"));

    let mut config_sink = BufferedUiSink::default();
    let config_exit = app.run_process_with_sink(
        ["osp", "--json", "config", "get", "extensions.site.banner"],
        &mut config_sink,
    );
    assert_eq!(config_exit, 0);
    assert!(config_sink.stdout.contains("\"extensions.site.banner\""));
    assert!(config_sink.stdout.contains("\"cli-wrapper\""));

    let mut explain_sink = BufferedUiSink::default();
    let explain_exit = app.run_process_with_sink(
        ["osp", "config", "explain", "extensions.site.banner"],
        &mut explain_sink,
    );
    assert_eq!(explain_exit, 0);
    assert!(explain_sink.stdout.contains("key: extensions.site.banner"));
    assert!(explain_sink.stdout.contains("cli-wrapper"));
}

#[test]
fn app_session_builders_and_cache_helpers_cover_public_session_surface_unit() {
    let mut neutral_session = crate::app::AppSession::builder().build();
    neutral_session.scope.enter("theme");
    assert_eq!(
        neutral_session.scope.help_tokens(),
        vec!["theme".to_string(), "--help".to_string()]
    );

    let mut overrides = ConfigLayer::default();
    overrides.set("extensions.site.enabled", true);
    let history_shell = HistoryShellContext::default();

    let built_session = crate::app::AppSessionBuilder::default()
        .with_prompt_prefix("builder")
        .with_history_enabled(false)
        .with_history_shell(history_shell.clone())
        .with_cache_limit(0)
        .with_config_overrides(overrides.clone())
        .build();
    assert_eq!(built_session.prompt_prefix, "builder");
    assert!(!built_session.history_enabled);
    assert_eq!(built_session.max_cached_results, 1);
    assert_eq!(
        built_session
            .config_overrides
            .entries()
            .iter()
            .filter(|entry| entry.key == "extensions.site.enabled")
            .count(),
        1
    );

    let mut session = crate::app::AppSession::with_cache_limit(0)
        .with_prompt_prefix("demo")
        .with_history_enabled(false)
        .with_history_shell(history_shell)
        .with_config_overrides(overrides);
    assert_eq!(session.prompt_prefix, "demo");
    assert!(!session.history_enabled);
    assert_eq!(session.max_cached_results, 1);

    session.record_result("   ", vec![serde_json::Map::new()]);
    assert!(session.last_rows.is_empty());

    let mut row = serde_json::Map::new();
    row.insert("uid".to_string(), serde_json::json!("alice"));
    session.record_result("ldap user alice", vec![row.clone()]);
    assert_eq!(
        session.cached_rows("ldap user alice").unwrap()[0]["uid"],
        "alice"
    );

    session.record_failure("   ", "ignored", "ignored");
    assert!(session.last_failure.is_none());
    session.record_failure("ldap user alice", "boom", "boom detail");
    assert_eq!(
        session
            .last_failure
            .as_ref()
            .expect("failure should be recorded")
            .summary,
        "boom"
    );

    session.record_cached_command("   ", &super::CliCommandResult::text("ignored"));
    assert!(session.cached_command("missing").is_none());
    session.record_cached_command("ldap user alice", &super::CliCommandResult::text("first"));
    session.record_cached_command("ldap user bob", &super::CliCommandResult::text("second"));
    assert!(session.cached_command("ldap user alice").is_none());
    assert!(session.cached_command("ldap user bob").is_some());

    session.record_prompt_timing(1, std::time::Duration::from_millis(10), None, None, None);
    assert!(session.prompt_timing.badge().is_some());
    session.record_prompt_timing(0, std::time::Duration::from_millis(0), None, None, None);
    assert!(session.prompt_timing.badge().is_none());

    let default_session = crate::app::AppSession::default();
    assert_eq!(
        default_session.max_cached_results,
        crate::config::DEFAULT_SESSION_CACHE_MAX_RESULTS as usize
    );
}

#[test]
fn prepare_plugin_response_handles_failures_and_pipeline_hints_unit() {
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
fn prepared_plugin_response_maps_into_cli_command_result_unit() {
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
    let result = CliCommandResult::from_prepared_plugin_response(prepared);

    assert_eq!(result.exit_code, 1);
    assert!(result.output.is_none());
    assert_eq!(
        result.failure_report.as_deref(),
        Some("NOT_FOUND: missing user")
    );
    assert!(!result.messages.is_empty());
}

#[test]
fn exit_code_classification_distinguishes_usage_config_and_plugin_unit() {
    let clap_report =
        super::run_from(["osp", "--definitely-not-a-flag"]).expect_err("parse should fail");
    assert_eq!(classify_exit_code(&clap_report), EXIT_CODE_USAGE);

    let mut invalid_session = ConfigLayer::default();
    invalid_session.set("ui.message.verbosity", "definitely-invalid");
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
