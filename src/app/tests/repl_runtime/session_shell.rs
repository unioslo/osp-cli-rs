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
    assert_eq!(root_exit, crate::repl::ReplLineResult::Exit(0));

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
        crate::repl::ReplLineResult::Continue(text) => {
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
                config_overrides: &state.session.config_overrides,
                product_defaults: state.runtime.product_defaults(),
                runtime_load: state.runtime.launch.runtime_load,
            },
            plugins: crate::cli::commands::plugins::PluginsCommandContext {
                context: &state.runtime.context,
                config: state.runtime.config.resolved(),
                config_state: Some(&state.runtime.config),
                auth: &state.runtime.auth,
                clients: Some(&state.clients),
                plugin_manager: state.clients.plugins(),
                product_defaults: state.runtime.product_defaults(),
                runtime_load: state.runtime.launch.runtime_load,
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
        Some(ReplCommandOutput::Json(json)) => {
            assert_eq!(json["status"], "error");
            assert_eq!(json["command"], "missing");
        }
        Some(ReplCommandOutput::Output(_)) => panic!("unexpected doctor output variant"),
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
        r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
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

    let mut state = make_test_state(vec![dir.to_path_buf()]);
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
        crate::repl::ReplLineResult::ReplaceInput(text) => {
            assert_eq!(text, "cache first");
        }
        other => panic!("unexpected repl result: {other:?}"),
    }
    assert_eq!(state.repl_cache_size(), cache_size_before);

}

#[test]
fn repl_bang_contains_search_expands_matching_command_unit() {
    use std::os::unix::fs::PermissionsExt;

    let dir = make_temp_dir("osp-cli-repl-bang-contains-plugin");
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
printf '{"protocol_version":1,"ok":true,"data":{"message":"ok","arg":"%s"},"error":null,"meta":{"format_hint":"table","columns":["message","arg"]}}\n' "$2"
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
        crate::repl::ReplLineResult::ReplaceInput(text) => {
            assert_eq!(text, "cache alpha");
        }
        other => panic!("unexpected repl result: {other:?}"),
    }
    assert_eq!(state.repl_cache_size(), cache_size_before);

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
        crate::repl::ReplLineResult::ReplaceInput(text) => assert_eq!(text, "status"),
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
        crate::repl::ReplLineResult::Continue(text) => {
            assert_eq!(text, "No history match for: !?config\n");
        }
        other => panic!("unexpected repl result: {other:?}"),
    }
}
