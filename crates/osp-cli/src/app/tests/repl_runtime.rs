use super::*;

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

#[test]
fn plugin_pipeline_rendering_matches_between_cli_and_repl_unit() {
    let dir = make_temp_dir("osp-cli-plugin-pipeline-parity");
    let _plugin_path = write_pipeline_test_plugin(&dir);
    let mut state = make_test_state(vec![dir.clone()]);
    let history = make_test_history(&mut state);
    let stages = vec!["message".to_string()];

    let dispatch_context =
        super::super::plugin_dispatch_context_for_runtime(&state.runtime, &state.clients, None);
    let response = state
        .clients
        .plugins
        .dispatch("hello", &[], &dispatch_context)
        .expect("plugin dispatch should succeed");
    let prepared = match super::super::prepare_plugin_response(response, &stages)
        .expect("plugin response should prepare")
    {
        super::super::PreparedPluginResponse::Output(prepared) => prepared,
        super::super::PreparedPluginResponse::Failure(failure) => {
            panic!("unexpected plugin failure: {}", failure.report)
        }
    };
    let cli_rendered = render_output(
        &prepared.output,
        &super::super::resolve_effective_render_settings(
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

    let next = super::super::rebuild_repl_state(&state).expect("rebuild should succeed");

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

#[test]
fn rebuild_repl_state_preserves_session_render_defaults_unit() {
    let mut state = make_test_state(Vec::new());
    state.session.config_overrides.set("ui.format", "table");

    let next = super::super::rebuild_repl_state(&state).expect("rebuild should succeed");

    assert_eq!(next.runtime.ui.render_settings.format, OutputFormat::Table);
}

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

#[test]
fn repl_config_unset_rebuilds_runtime_state_unit() {
    let mut state = make_test_state(Vec::new());
    state
        .session
        .config_overrides
        .set_for_profile("default", "ui.format", "table");
    state = super::super::rebuild_repl_state(&state).expect("rebuild should succeed");
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

    let next = super::super::rebuild_repl_state(&state).expect("rebuild should succeed");
    assert_eq!(next.runtime.config.resolved().get_string("ui.format"), None);
    assert_eq!(next.runtime.ui.render_settings.format, OutputFormat::Auto);
}

#[test]
fn repl_config_unset_dry_run_preserves_session_state_unit() {
    let mut state = make_test_state(Vec::new());
    state
        .session
        .config_overrides
        .set_for_profile("default", "ui.format", "table");
    state = super::super::rebuild_repl_state(&state).expect("rebuild should succeed");
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
    state = super::super::rebuild_repl_state(&state).expect("rebuild should succeed");
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

    state = super::super::rebuild_repl_state(&state).expect("rebuild should succeed");
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

    state = super::super::rebuild_repl_state(&state).expect("rebuild should succeed");
    let restored_prompt =
        crate::repl::presentation::build_repl_prompt(repl_view(&state.runtime, &state.session))
            .left;
    assert_eq!(restored_prompt, default_prompt);
}

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

#[test]
fn rebuild_repl_state_preserves_last_failure_unit() {
    let mut state = make_test_state(Vec::new());
    state.record_repl_failure("ldap user nope", "boom", "boom detail");

    let next = super::super::rebuild_repl_state(&state).expect("rebuild should succeed");
    let last = next
        .last_repl_failure()
        .expect("last failure should survive rebuild");

    assert_eq!(last.command_line, "ldap user nope");
    assert_eq!(last.summary, "boom");
    assert_eq!(last.detail, "boom detail");
}

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
