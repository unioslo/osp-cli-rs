use super::*;
use miette::WrapErr;

#[test]
fn app_help_entrypoints_and_sink_routing_render_help_unit() {
    let app = crate::app::AppBuilder::new().build();
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
    let mut runner = crate::app::AppBuilder::new().build_with_sink(&mut runner_sink);
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
    let ui = crate::app::UiState::builder(RenderSettings::test_plain(OutputFormat::Value))
        .with_message_verbosity(MessageLevel::Success)
        .build();
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
fn state_and_client_builders_produce_coherent_embedder_state_unit() {
    let config = test_config(&[]);
    let ui = crate::app::UiState::builder(RenderSettings::test_plain(OutputFormat::Json))
        .with_message_verbosity(MessageLevel::Trace)
        .with_debug_verbosity(2)
        .build();
    let launch = crate::app::LaunchContext::builder()
        .with_plugin_dir("/tmp/osp-plugin-a")
        .with_config_root(Some(std::path::PathBuf::from("/tmp/osp-config")))
        .with_cache_root(Some(std::path::PathBuf::from("/tmp/osp-cache")))
        .build();
    let session = crate::app::AppSession::builder()
        .with_prompt_prefix("osp-dev")
        .with_cache_limit(5)
        .build();

    let state = crate::app::AppState::builder(
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

    let clients = crate::app::AppClients::builder()
        .with_plugins(crate::plugin::PluginManager::new(vec![
            std::path::PathBuf::from("/tmp/osp-plugin-a"),
        ]))
        .with_native_commands(test_native_registry())
        .build();
    assert_eq!(
        clients.plugins().explicit_dirs(),
        &[std::path::PathBuf::from("/tmp/osp-plugin-a")]
    );
    assert!(clients.native_commands().command("ldap").is_some());
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
