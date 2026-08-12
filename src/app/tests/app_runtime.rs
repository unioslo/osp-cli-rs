use super::*;
use miette::WrapErr;
use std::sync::Mutex;

fn env_lock() -> &'static Mutex<()> {
    crate::tests::env_lock()
}

fn with_test_xdg_env<T>(run: impl FnOnce() -> T) -> T {
    let _guard = env_lock().lock().expect("env lock should not be poisoned");
    let original_home = std::env::var("HOME").ok();
    let original_xdg_config_home = std::env::var("XDG_CONFIG_HOME").ok();
    let original_xdg_cache_home = std::env::var("XDG_CACHE_HOME").ok();
    let original_xdg_data_home = std::env::var("XDG_DATA_HOME").ok();

    unsafe {
        std::env::set_var("HOME", "/tmp/osp-app-runtime-home");
        std::env::set_var("XDG_CONFIG_HOME", "/tmp/osp-app-runtime-xdg/config");
        std::env::set_var("XDG_CACHE_HOME", "/tmp/osp-app-runtime-xdg/cache");
        std::env::set_var("XDG_DATA_HOME", "/tmp/osp-app-runtime-xdg/data");
    }

    let output = run();

    match original_home {
        Some(value) => unsafe { std::env::set_var("HOME", value) },
        None => unsafe { std::env::remove_var("HOME") },
    }
    match original_xdg_config_home {
        Some(value) => unsafe { std::env::set_var("XDG_CONFIG_HOME", value) },
        None => unsafe { std::env::remove_var("XDG_CONFIG_HOME") },
    }
    match original_xdg_cache_home {
        Some(value) => unsafe { std::env::set_var("XDG_CACHE_HOME", value) },
        None => unsafe { std::env::remove_var("XDG_CACHE_HOME") },
    }
    match original_xdg_data_home {
        Some(value) => unsafe { std::env::set_var("XDG_DATA_HOME", value) },
        None => unsafe { std::env::remove_var("XDG_DATA_HOME") },
    }

    output
}

struct SessionGuardedNativeCommand;

struct AuthenticatedBuiltinRecovery;

impl crate::app::CommandAccessRecovery for AuthenticatedBuiltinRecovery {
    fn try_recover(
        &self,
        request: &crate::app::AccessRecoveryRequest,
        runtime: &mut crate::app::AppRuntime,
        _session: &mut crate::app::AppSession,
    ) -> miette::Result<crate::app::AccessRecoveryOutcome> {
        if request.command_kind != crate::app::CommandAccessKind::Builtin
            || request.command != "config"
        {
            return Ok(crate::app::AccessRecoveryOutcome::NoChange);
        }

        runtime.auth_mut().set_policy_context(
            crate::core::command_policy::CommandPolicyContext::default().authenticated(true),
        );
        Ok(crate::app::AccessRecoveryOutcome::Recovered)
    }
}

impl crate::NativeCommand for SessionGuardedNativeCommand {
    fn command(&self) -> clap::Command {
        clap::Command::new("secure").about("Guarded test command")
    }

    fn auth(&self) -> Option<crate::core::plugin::DescribeCommandAuthV1> {
        Some(crate::core::plugin::DescribeCommandAuthV1 {
            visibility: Some(crate::core::plugin::DescribeVisibilityModeV1::Public),
            run_session: Some(crate::core::plugin::DescribeSessionRequirementsV1 {
                auth_strength: Some(crate::core::plugin::DescribeAuthStrengthV1::Strong),
                credentials: vec![
                    crate::core::plugin::DescribeCredentialRequirementV1::Fresh {
                        service: "osp".to_string(),
                        min_ttl_seconds: 900,
                    },
                ],
            }),
            ..crate::core::plugin::DescribeCommandAuthV1::default()
        })
    }

    fn execute(
        &self,
        args: &[String],
        _context: &crate::NativeCommandContext<'_>,
    ) -> anyhow::Result<crate::NativeCommandOutcome> {
        if args.iter().any(|arg| arg == "--help") {
            return Ok(crate::NativeCommandOutcome::Help(
                "Usage: osp secure".to_string(),
            ));
        }

        Ok(crate::NativeCommandOutcome::Exit(0))
    }
}

#[cfg_attr(miri, ignore = "public app bootstrap integration test")]
#[test]
fn app_help_entrypoints_share_exit_and_sink_routing_invariants_unit() {
    with_test_xdg_env(|| {
        let app = crate::app::App::builder().build();
        let mut help_sink = BufferedUiSink::default();

        assert_eq!(
            app.run_with_sink(["osp", "--defaults-only", "--help"], &mut help_sink)
                .expect("app help with sink should render"),
            0
        );
        assert_eq!(
            app.run_process_with_sink(["osp", "--defaults-only", "--help"], &mut help_sink),
            0
        );
        assert!(!help_sink.stdout.is_empty());
        assert!(help_sink.stderr.is_empty());

        let mut sink = BufferedUiSink::default();
        let exit = super::run_from_with_sink(["osp", "--defaults-only", "--help"], &mut sink)
            .expect("help should render");
        assert_eq!(exit, 0);
        assert!(!sink.stdout.is_empty());
        assert!(sink.stderr.is_empty());

        let mut runner_sink = BufferedUiSink::default();
        let mut runner = crate::app::App::builder().build_with_sink(&mut runner_sink);
        assert_eq!(
            runner
                .run_from(["osp", "--defaults-only", "--help"])
                .expect("runner help should render"),
            0
        );
    });
}

#[cfg_attr(miri, ignore = "public app bootstrap integration test")]
#[test]
fn app_public_entrypoints_cover_owned_runner_and_free_process_wrappers_unit() {
    with_test_xdg_env(|| {
        let app = crate::app::App::builder().build();
        assert_eq!(
            app.run_from(["osp", "--defaults-only", "--help"])
                .expect("app run_from help should render"),
            0
        );
        assert_eq!(app.run_process(["osp", "--defaults-only", "--help"]), 0);

        let mut runner_sink = BufferedUiSink::default();
        let mut runner = crate::app::App::builder().build_with_sink(&mut runner_sink);
        assert_eq!(runner.run_process(["osp", "--defaults-only", "--help"]), 0);
        drop(runner);
        assert!(!runner_sink.stdout.is_empty());
        assert!(runner_sink.stderr.is_empty());

        assert_eq!(
            crate::app::run_process(["osp", "--defaults-only", "--help"]),
            0
        );

        let mut sink = BufferedUiSink::default();
        assert_eq!(
            crate::app::run_process_with_sink(["osp", "--defaults-only", "--help"], &mut sink),
            0
        );
        assert!(!sink.stdout.is_empty());
        assert!(sink.stderr.is_empty());
    });
}

#[cfg_attr(miri, ignore = "public app bootstrap integration test")]
#[test]
fn app_builder_product_defaults_flow_through_public_builder_surface_unit() {
    with_test_xdg_env(|| {
        let mut product_defaults = crate::config::ConfigLayer::default();
        product_defaults.set("extensions.site.enabled", true);
        product_defaults.set("theme.path", Vec::<String>::new());

        let mut sink = BufferedUiSink::default();
        let exit = crate::app::App::builder()
            .with_product_defaults(product_defaults)
            .build_with_sink(&mut sink)
            .run_process([
                "osp",
                "--defaults-only",
                "--json",
                "config",
                "get",
                "extensions.site.enabled",
            ]);

        assert_eq!(exit, 0);
        let payload: serde_json::Value =
            serde_json::from_str(&sink.stdout).expect("config get should render JSON");
        let rows = payload
            .as_array()
            .expect("config get JSON should be an array");
        assert!(rows.iter().any(|row| {
            row["key"] == "extensions.site.enabled" && row["value"] == serde_json::json!(true)
        }));
        assert!(sink.stderr.is_empty());
    });
}

#[cfg_attr(miri, ignore = "public app bootstrap integration test")]
#[test]
fn app_builder_policy_context_flows_through_public_builder_surface_unit() {
    with_test_xdg_env(|| {
        let registry =
            crate::NativeCommandRegistry::new().with_command(SessionGuardedNativeCommand);
        let mut denied_sink = BufferedUiSink::default();
        let denied_exit = crate::app::App::builder()
            .with_native_commands(registry.clone())
            .build_with_sink(&mut denied_sink)
            .run_process(["osp", "--defaults-only", "secure"]);

        assert_ne!(denied_exit, 0);
        assert!(
            denied_sink
                .stderr
                .contains("requires strong authentication")
        );

        let mut allowed_sink = BufferedUiSink::default();
        let allowed_exit = crate::app::App::builder()
            .with_policy_context(
                crate::core::command_policy::CommandPolicyContext::default()
                    .authenticated(true)
                    .with_auth_strength(crate::core::command_policy::AuthStrength::Strong)
                    .with_credential(
                        "osp",
                        crate::core::command_policy::CredentialState::valid_for(900),
                    ),
            )
            .with_native_commands(registry)
            .build_with_sink(&mut allowed_sink)
            .run_process(["osp", "--defaults-only", "secure"]);

        assert_eq!(allowed_exit, 0);
        assert!(allowed_sink.stdout.is_empty());
        assert!(allowed_sink.stderr.is_empty());
    });
}

#[cfg_attr(miri, ignore = "public app bootstrap integration test")]
#[test]
fn app_builder_builtin_policy_flows_through_public_builder_surface_unit() {
    with_test_xdg_env(|| {
        let mut builtin_policy = crate::core::command_policy::CommandPolicyRegistry::new();
        builtin_policy.register(
            crate::core::command_policy::CommandPolicy::new(
                crate::core::command_policy::CommandPath::new(["config"]),
            )
            .visibility(crate::core::command_policy::VisibilityMode::Authenticated),
        );
        let mut denied_sink = BufferedUiSink::default();
        let denied_exit = crate::app::App::builder()
            .with_builtin_policy(builtin_policy.clone())
            .build_with_sink(&mut denied_sink)
            .run_process(["osp", "--defaults-only", "config", "get", "theme.name"]);

        assert_ne!(denied_exit, 0);
        assert!(denied_sink.stderr.contains("requires authentication"));

        let mut allowed_sink = BufferedUiSink::default();
        let allowed_exit = crate::app::App::builder()
            .with_builtin_policy(builtin_policy)
            .with_policy_context(
                crate::core::command_policy::CommandPolicyContext::default().authenticated(true),
            )
            .build_with_sink(&mut allowed_sink)
            .run_process(["osp", "--defaults-only", "config", "get", "theme.name"]);

        assert_eq!(allowed_exit, 0);
        assert!(allowed_sink.stdout.contains("theme.name"));
        assert!(allowed_sink.stderr.is_empty());
    });
}

#[cfg_attr(miri, ignore = "public app bootstrap integration test")]
#[test]
fn app_builder_access_recovery_retries_denied_builtin_command_unit() {
    with_test_xdg_env(|| {
        let mut builtin_policy = crate::core::command_policy::CommandPolicyRegistry::new();
        builtin_policy.register(
            crate::core::command_policy::CommandPolicy::new(
                crate::core::command_policy::CommandPath::new(["config"]),
            )
            .visibility(crate::core::command_policy::VisibilityMode::Authenticated),
        );

        let mut sink = BufferedUiSink::default();
        let exit = crate::app::App::builder()
            .with_builtin_policy(builtin_policy)
            .with_access_recovery(AuthenticatedBuiltinRecovery)
            .build_with_sink(&mut sink)
            .run_process(["osp", "--defaults-only", "config", "get", "theme.name"]);

        assert_eq!(exit, 0);
        assert!(sink.stdout.contains("theme.name"));
        assert!(sink.stderr.is_empty());
    });
}

#[cfg_attr(miri, ignore = "public app bootstrap integration test")]
#[test]
fn free_process_wrapper_renders_usage_errors_to_bound_sink_unit() {
    with_test_xdg_env(|| {
        let mut sink = BufferedUiSink::default();
        let exit = crate::app::run_process_with_sink(["osp", "--definitely-not-a-flag"], &mut sink);

        assert_eq!(exit, EXIT_CODE_USAGE);
        assert!(sink.stdout.is_empty());
        assert!(
            sink.stderr.contains("definitely-not-a-flag")
                || sink.stderr.contains("unexpected argument")
        );
    });
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
fn bootstrap_error_detail_handles_non_utf8_short_flags_and_double_dash_unit() {
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

    assert_eq!(super::bootstrap_error_detail(&args), ErrorDetail::Normal);
}

#[cfg_attr(
    miri,
    ignore = "forensic backtrace rendering uses std backtrace display"
)]
#[test]
fn error_rendering_prioritizes_actionable_details_across_levels_unit() {
    let settings = RenderSettings::test_plain(OutputFormat::Guide);
    let report = enrich_dispatch_error(PluginDispatchError::NonZeroExit {
        plugin_id: "ldap".to_string(),
        status_code: 7,
        stderr: "backend exploded".to_string(),
    });

    let rendered = render_report_message(&report, ErrorDetail::Terse, &settings);
    assert!(rendered.contains("plugin ldap exited with status 7: backend exploded"));
    assert!(rendered.contains("Hint:"));
    assert!(rendered.contains("run again with -v, -vv, or -vvv"));

    let rendered = render_report_message(&report, ErrorDetail::Normal, &settings);
    assert!(rendered.contains("plugin ldap exited with status 7"));
    assert!(rendered.contains("plugin command failed"));
    assert!(rendered.contains("backend exploded"));
    assert!(!rendered.contains("Hint:"));

    let rendered = render_report_message(&report, ErrorDetail::Debug, &settings);
    assert!(rendered.contains("plugin ldap exited with status 7"));
    assert!(rendered.contains("plugin command failed"));
    assert!(rendered.contains("backend exploded"));
    assert!(!rendered.contains("Hint:"));
    assert!(!rendered.contains("Stack backtrace"));

    let rendered = render_report_message(&report, ErrorDetail::Forensic, &settings);
    assert!(rendered.contains("plugin command failed"));
    assert!(rendered.contains("backend exploded"));
    assert!(rendered.contains("Stack backtrace"));

    let report = Err::<(), _>(miette::miette!("unknown theme: missing-theme"))
        .wrap_err("failed to derive host runtime inputs for startup")
        .expect_err("wrapped error should stay an error");
    let rendered = render_report_message(&report, ErrorDetail::Terse, &settings);
    assert!(rendered.contains("unknown theme: missing-theme"));
    assert!(!rendered.starts_with("failed to derive host runtime inputs for startup"));
    assert!(rendered.contains("run again with -v, -vv, or -vvv"));
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

#[cfg_attr(miri, ignore = "public app bootstrap integration test")]
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
    let plugins =
        crate::plugin::PluginManager::new(vec![std::path::PathBuf::from("/tmp/osp-plugin-a")])
            .with_roots(
                Some(std::path::PathBuf::from("/tmp/osp-config")),
                Some(std::path::PathBuf::from("/tmp/osp-cache")),
            )
            .with_bundled_roots(false)
            .with_default_roots(false);

    let state = crate::app::AppStateBuilder::new(
        crate::app::RuntimeContext::new(None, crate::app::TerminalKind::Cli, None),
        config,
        ui,
    )
    .with_launch(launch)
    .with_plugins(plugins)
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

#[cfg_attr(miri, ignore = "public app bootstrap integration test")]
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
fn app_session_cache_helpers_cover_public_session_surface_unit() {
    let mut neutral_session = crate::app::AppSession::default();
    neutral_session.scope.enter("theme");
    assert_eq!(
        neutral_session.scope.help_tokens(),
        vec!["theme".to_string(), "--help".to_string()]
    );

    let mut overrides = ConfigLayer::default();
    overrides.set("extensions.site.enabled", true);
    let history_shell = HistoryShellContext::default();

    let built_session = crate::app::AppSession::with_cache_limit(0)
        .with_prompt_prefix("builder")
        .with_history_enabled(false)
        .with_history_shell(history_shell.clone())
        .with_config_overrides(overrides.clone());
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

    let cached = StructuredCommandOutput {
        source_guide: None,
        output: rows_to_output_result(vec![crate::row! { "value" => "cached" }]),
        format_hint: None,
    };
    session.record_cached_command("   ", &cached);
    assert!(session.cached_command("missing").is_none());
    session.record_cached_command("ldap user alice", &cached);
    session.record_cached_command("ldap user bob", &cached);
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
        data: serde_json::json!({
            "code": "vm_ambiguous",
            "candidates": [{"name": "db01.uio.no", "provider": "vmware"}]
        }),
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
    assert_eq!(failure.report.to_string(), "NOT_FOUND: missing user");
    assert_eq!(
        failure
            .output
            .document
            .as_ref()
            .map(|document| document.value.clone()),
        None
    );
    assert_eq!(
        crate::core::output_model::output_items_to_value(&failure.output.items),
        serde_json::json!({
            "code": "vm_ambiguous",
            "candidates": [{"name": "db01.uio.no", "provider": "vmware"}]
        })
    );

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
            column_labels: Vec::new(),
            row_path: None,
            preserve_json_document: false,
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
    let Some(ReplCommandOutput::Output(output)) = result.output.as_ref() else {
        panic!("structured failure data should remain renderable");
    };
    assert_eq!(
        crate::core::output_model::output_items_to_value(&output.output.items),
        serde_json::json!({})
    );
    assert_eq!(
        result.failure_report.as_ref().map(ToString::to_string),
        Some("NOT_FOUND: missing user".to_string())
    );
    assert!(!result.messages.is_empty());
}

#[test]
fn exit_code_classification_distinguishes_usage_config_and_plugin_unit() {
    with_test_xdg_env(|| {
        let clap_report = super::run_from(["osp", "--defaults-only", "--definitely-not-a-flag"])
            .expect_err("parse should fail");
        assert_eq!(classify_exit_code(&clap_report), EXIT_CODE_USAGE);

        let mut invalid_session = ConfigLayer::default();
        invalid_session.set("ui.message.verbosity", "definitely-invalid");
        let config_report = super::resolve_runtime_config(
            RuntimeConfigRequest::new(None, Some("cli"))
                .with_runtime_load(crate::config::RuntimeLoadOptions::defaults_only())
                .with_session_layer(Some(invalid_session)),
        )
        .expect_err("config resolution should fail");
        assert_eq!(classify_exit_code(&config_report), EXIT_CODE_CONFIG);

        let plugin_report = enrich_dispatch_error(PluginDispatchError::CommandNotFound {
            command: "ldap".to_string(),
        });
        assert_eq!(classify_exit_code(&plugin_report), EXIT_CODE_PLUGIN);
    });
}
