#[test]
fn repl_plugin_error_payload_is_handled_as_error_unit() {
    use std::os::unix::fs::PermissionsExt;

    let dir = make_temp_dir("osp-cli-repl-error-plugin");
    let plugin_path = dir.join("osp-fail");
    std::fs::write(
        &plugin_path,
        r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
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

    let mut state = make_test_state(vec![dir.to_path_buf()]);

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

}

#[test]
fn repl_records_last_rows_and_bounded_cache_unit() {
    use std::os::unix::fs::PermissionsExt;

    let dir = make_temp_dir("osp-cli-repl-session-plugin");
    let plugin_path = dir.join("osp-cache");
    std::fs::write(
        &plugin_path,
        r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
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

    let mut state = make_test_state(vec![dir.to_path_buf()]);
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
        crate::repl::ReplLineResult::Continue(text) => assert!(text.contains("ok")),
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
        crate::repl::ReplLineResult::Continue(text) => assert!(text.contains("ok")),
        other => panic!("unexpected repl result: {other:?}"),
    }

    assert_eq!(state.repl_cache_size(), 1);
    assert!(state.cached_repl_rows("cache first").is_none());
    assert!(state.cached_repl_rows("cache second").is_some());
    assert!(!state.last_repl_rows().is_empty());

}

#[test]
fn repl_cache_reuses_external_result_across_pipelines_unit() {
    use std::os::unix::fs::PermissionsExt;

    let dir = make_temp_dir("osp-cli-repl-cache-plugin");
    let log_path = dir.join("invocations.log");
    let plugin_path = dir.join("osp-slowcache");
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
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

    let mut state = make_test_state(vec![dir.to_path_buf()]);
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
        crate::repl::ReplLineResult::Continue(text) => assert!(text.contains('1')),
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
        crate::repl::ReplLineResult::Continue(text) => assert!(text.contains("cached")),
        other => panic!("unexpected repl result: {other:?}"),
    }

    let log = std::fs::read_to_string(&log_path).expect("invocation log should exist");
    assert_eq!(log.lines().count(), 1);

}

#[test]
fn plugin_pipeline_rendering_matches_between_cli_and_repl_unit() {
    let dir = make_temp_dir("osp-cli-plugin-pipeline-parity");
    let _plugin_path = write_pipeline_test_plugin(&dir);
    let mut state = make_test_state(vec![dir.to_path_buf()]);
    let history = make_test_history(&mut state);
    let stages = vec!["message".to_string()];

    let dispatch_context =
        super::super::plugin_dispatch_context_for_runtime(&state.runtime, &state.clients, None);
    let response = state
        .clients
        .plugins()
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
        &super::super::resolve_render_settings_with_hint(
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
        crate::repl::ReplLineResult::Continue(text) => {
            assert_eq!(text.trim(), cli_rendered.trim());
            assert!(text.contains("hello-from-plugin"));
        }
        other => panic!("unexpected repl result: {other:?}"),
    }

}

#[test]
fn repl_plugin_provider_override_dispatches_selected_provider_unit() {
    let dir = make_temp_dir("osp-cli-repl-provider-override");
    let _alpha = write_provider_test_plugin(&dir, "alpha-provider", "hello", "alpha");
    let _beta = write_provider_test_plugin(&dir, "beta-provider", "hello", "beta");
    let mut state = make_test_state(vec![dir.to_path_buf()]);
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
        crate::repl::ReplLineResult::Continue(text) => {
            assert!(text.contains("beta-from-plugin"));
            assert!(!text.contains("alpha-from-plugin"));
        }
        other => panic!("unexpected repl result: {other:?}"),
    }

}
