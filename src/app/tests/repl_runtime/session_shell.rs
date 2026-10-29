#[test]
fn repl_builtin_overrides_do_not_mutate_runtime_ui_state_unit() {
    let (mut state, _state_root) = make_test_state(Vec::new());
    let (history, _history_root) = make_test_history(&mut state);
    let original_message = state.runtime.ui.verbosity;
    let original_error_detail = state.runtime.ui.verbosity;
    let original_debug = state.runtime.ui.debug_level;

    repl_dispatch::execute_repl_plugin_line(
        &mut state,
        &history,
        "-q config get missing.key", None,).map(|executed| executed.result)
    .expect("quiet config get should complete");
    assert_eq!(state.runtime.ui.verbosity, original_message);
    assert_eq!(state.runtime.ui.verbosity, original_error_detail);
    assert_eq!(state.runtime.ui.debug_level, original_debug);

    repl_dispatch::execute_repl_plugin_line(
        &mut state,
        &history,
        "-v doctor last", None,).map(|executed| executed.result)
    .expect("doctor last with verbosity override should complete");
    assert_eq!(state.runtime.ui.verbosity, original_message);
    assert_eq!(state.runtime.ui.verbosity, original_error_detail);
    assert_eq!(state.runtime.ui.debug_level, original_debug);
}

#[test]
fn repl_exit_is_host_owned_at_root_but_leaves_shell_in_scope_unit() {
    let (mut state, _state_root) = make_test_state(Vec::new());
    let (history, _history_root) = make_test_history(&mut state);

    let root_exit = repl_dispatch::execute_repl_plugin_line(
        &mut state,
        &history,
        "exit", None,).map(|executed| executed.result)
    .expect("root exit should be handled by host dispatch");
    assert_eq!(root_exit, crate::repl::ReplResult::Exit(0));

    state.session.scope.enter("orch");
    let shell_exit = repl_dispatch::execute_repl_plugin_line(
        &mut state,
        &history,
        "exit", None,).map(|executed| executed.result)
    .expect("shell exit should leave the current shell");
    match shell_exit {
        crate::repl::ReplResult::Restart {
            output: text,
            reload: crate::repl::ReplReloadKind::Completion,
        } => {
            assert!(text.contains("Leaving orch."));
        }
        other => panic!("unexpected repl result: {other:?}"),
    }
    assert!(state.session.scope.is_root());
}

#[test]
fn repl_failure_is_cached_for_doctor_last_unit() {
    let (mut state, _state_root) = make_test_state(Vec::new());
    state.runtime.ui.render_settings.format = OutputFormat::Table;
    let (history, _history_root) = make_test_history(&mut state);

    let mut sink = BufferedUiSink::default();
    repl_dispatch::execute_repl_plugin_line(
        &mut state,
        &history,
        "missing", Some(&mut sink),)
    .expect("failure should render through the sink");
    let err_text = &sink.stderr;
    assert!(err_text.contains("Unknown command 'missing'"));

    let last = state
        .last_repl_failure()
        .expect("last failure should be recorded");
    assert_eq!(last.command_line, "missing");
    assert!(!last.summary.trim().is_empty());
    assert!(last.detail(crate::core::MessageLevel::Info).contains("Unknown command 'missing'"));

    state.runtime.ui.render_settings.format = OutputFormat::Json;
    let rendered = doctor_cmd::run_doctor_command(
        &state.runtime, &state.session, &state.clients, &state.runtime.ui,
        crate::cli::DoctorArgs {
            command: Some(crate::cli::DoctorCommands::Last),
        },
    )
    .expect("doctor last should render");
    match rendered.body {
        crate::app::CommandBody::Json(json) => {
            assert_eq!(json["status"], "error");
            assert_eq!(json["command"], "missing");
        }
        other => panic!("expected doctor JSON output, got {other:?}"),
    }
}

#[test]
fn rebuild_repl_state_preserves_last_failure_unit() {
    if crate::tests::isolated_test() { return; }
    with_test_xdg_env(|| {
        let (mut state, _state_root) = make_test_state(Vec::new());
        state.record_repl_failure("ldap user nope", "boom", "boom detail");

        let next = super::super::rebuild_repl_state(&state).expect("rebuild should succeed");
        let last = next
            .last_repl_failure()
            .expect("last failure should survive rebuild");

        assert_eq!(last.command_line, "ldap user nope");
        assert_eq!(last.summary, "boom");
        assert_eq!(last.detail(crate::core::MessageLevel::Info), "boom detail");
    });
}

#[cfg_attr(miri, ignore = "plugin filesystem/process integration test")]
#[test]
fn repl_bang_executes_last_command_unit() {
    use std::os::unix::fs::PermissionsExt;

    let dir = make_temp_dir("osp-cli-repl-bang-plugin");
    let plugin_path = dir.join("osp-cache");
    std::fs::write(
        &plugin_path,
        r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
if [ "$1" = "--osp-describe" ]; then
  cat <<'JSON'
{"about":"cache plugin","subcommands":[],"args":[],"flags":{},"output":"json"}
JSON
  exit 0
fi
printf '{"protocol_version":1,"ok":true,"data":{"message":"ok","arg":"%s"},"error":null,"meta":{"format_hint":"table","columns":["message","arg"]}}\n' "$1"
"#,
    )
    .expect("plugin script should be written");
    let mut perms = std::fs::metadata(&plugin_path)
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&plugin_path, perms).expect("script should be executable");

    let (mut state, _state_root) = make_test_state(vec![dir.to_path_buf()]);
    let (history, _history_root) = make_test_history(&mut state);

    repl_dispatch::execute_repl_plugin_line(
        &mut state,
        &history,
        "cache first", None,).map(|executed| executed.result)
    .expect("seed command should succeed");
    history
        .with(|store| store.save_command_line("cache first"))
        .expect("history lock should be available")
        .expect("history seed should save");
    let expanded = repl_dispatch::execute_repl_plugin_line(
        &mut state,
        &history,
        "!!", None,).map(|executed| executed.result)
    .expect("bang expansion should succeed");
    match expanded {
        crate::repl::ReplResult::Print(text) => {
            assert!(text.contains("first"), "repeated command output: {text}");
        }
        other => panic!("unexpected repl result: {other:?}"),
    }
}

#[cfg_attr(miri, ignore = "plugin filesystem/process integration test")]
#[test]
fn repl_bang_contains_search_executes_matching_command_unit() {
    use std::os::unix::fs::PermissionsExt;

    let dir = make_temp_dir("osp-cli-repl-bang-contains-plugin");
    let plugin_path = dir.join("osp-cache");
    std::fs::write(
        &plugin_path,
        r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
if [ "$1" = "--osp-describe" ]; then
  cat <<'JSON'
{"about":"cache plugin","subcommands":[],"args":[],"flags":{},"output":"json"}
JSON
  exit 0
fi
printf '{"protocol_version":1,"ok":true,"data":{"message":"ok","arg":"%s"},"error":null,"meta":{"format_hint":"table","columns":["message","arg"]}}\n' "$1"
"#,
    )
    .expect("plugin script should be written");
    let mut perms = std::fs::metadata(&plugin_path)
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&plugin_path, perms).expect("script should be executable");

    let (mut state, _state_root) = make_test_state(vec![dir.to_path_buf()]);
    let (history, _history_root) = make_test_history(&mut state);

    repl_dispatch::execute_repl_plugin_line(
        &mut state,
        &history,
        "cache alpha", None,).map(|executed| executed.result)
    .expect("first seed command should succeed");
    history
        .with(|store| store.save_command_line("cache alpha"))
        .expect("history lock should be available")
        .expect("history seed should save");
    repl_dispatch::execute_repl_plugin_line(
        &mut state,
        &history,
        "cache beta", None,).map(|executed| executed.result)
    .expect("second seed command should succeed");
    history
        .with(|store| store.save_command_line("cache beta"))
        .expect("history lock should be available")
        .expect("history seed should save");
    let expanded = repl_dispatch::execute_repl_plugin_line(
        &mut state,
        &history,
        "!?alpha", None,).map(|executed| executed.result)
    .expect("contains bang expansion should succeed");
    match expanded {
        crate::repl::ReplResult::Print(text) => {
            assert!(text.contains("alpha"), "replayed command output: {text}");
        }
        other => panic!("unexpected repl result: {other:?}"),
    }
}

#[test]
fn repl_bang_contains_search_respects_shell_scope_unit() {
    let (mut state, _state_root) = make_test_state(Vec::new());
    let (history, _history_root) = make_test_history(&mut state);

    history
        .with(|store| store.save_command_line("config show"))
        .expect("history lock should be available")
        .expect("root history seed should save");

    state.session.scope.enter("orch");
    state.sync_history_shell_context();
    history
        .with(|store| store.save_command_line("config get profile.default"))
        .expect("history lock should be available")
        .expect("scoped history seed should save");

    let scoped_hit = repl_dispatch::execute_repl_plugin_line(
        &mut state,
        &history,
        "!?profile", None,).map(|executed| executed.result)
    .expect("scoped bang search should succeed");
    match scoped_hit {
        crate::repl::ReplResult::Print(text) => assert!(text.contains("profile.default")),
        other => panic!("unexpected repl result: {other:?}"),
    }

    let mut sink = crate::app::BufferedUiSink::default();
    repl_dispatch::execute_repl_plugin_line(
        &mut state,
        &history,
        "!?show", Some(&mut sink),)
    .expect("cross-shell bang miss should render feedback");
    assert!(sink.stdout.contains("no history match for: !?show"));
}
