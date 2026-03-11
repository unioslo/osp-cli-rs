#[cfg(unix)]
#[test]
fn external_plugin_dispatch_contract() {
    let dir = make_temp_dir("osp-cli-plugin-exec");
    let _plugin_path = write_hello_plugin(&dir);
    let home = make_temp_dir("osp-cli-plugin-exec-home");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["hello"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("hello-from-plugin"));

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn plugin_dispatch_propagates_runtime_hints_contract() {
    let dir = make_temp_dir("osp-cli-plugin-runtime-hints");
    let _plugin_path = write_hints_plugin(&dir);
    let home = make_temp_dir("osp-cli-plugin-runtime-home");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .env("TERM", "xterm-256color")
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args([
            "--profile",
            "tsd",
            "-vv",
            "-ddd",
            "--json",
            "--color",
            "never",
            "--unicode",
            "always",
            "hints",
        ])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    let row = first_json_row(&payload, "plugin runtime hints");
    assert_eq!(row["ui_verbosity"], "trace");
    assert_eq!(row["debug_level"], "3");
    assert_eq!(row["format"], "json");
    assert_eq!(row["color"], "never");
    assert_eq!(row["unicode"], "always");
    assert_eq!(row["profile"], "tsd");
    assert_eq!(row["terminal_kind"], "cli");
    assert_eq!(row["terminal"], "xterm-256color");

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn plugin_dispatch_propagates_config_env_contract() {
    let dir = make_temp_dir("osp-cli-plugin-config-env");
    let _plugin_path = write_config_env_plugin(&dir);
    let home = make_temp_dir("osp-cli-plugin-config-env-home");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"
extensions.plugins.env.shared.url = "https://common.example"
extensions.plugins.env.endpoint = "shared-endpoint"
extensions.plugins.cfg.env.endpoint = "plugin-endpoint"
extensions.plugins.cfg.env.api.token = "token-123"
extensions.plugins.cfg.env.enable_cache = true
extensions.plugins.cfg.env.retries = 3
"#,
    );

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--json", "cfg"])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    let row = first_json_row(&payload, "plugin config env");
    assert_eq!(row["shared_url"], "https://common.example");
    assert_eq!(row["endpoint"], "plugin-endpoint");
    assert_eq!(row["api_token"], "token-123");
    assert_eq!(row["enable_cache"], "true");
    assert_eq!(row["retries"], "3");
    assert!(
        output.stderr.is_empty(),
        "stderr should stay empty: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn plugin_non_zero_exit_surfaces_stderr_contract() {
    let dir = make_temp_dir("osp-cli-plugin-non-zero-exit");
    let _plugin_path = write_non_zero_plugin(&dir);
    let home = make_temp_dir("osp-cli-plugin-non-zero-exit-home");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["boom"]);
    let output = cmd.assert().failure().get_output().clone();
    assert!(
        output.stdout.is_empty(),
        "stdout should stay empty: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_snapshot_text!(
        "plugin_non_zero_exit_stderr",
        String::from_utf8(output.stderr).expect("stderr should be utf-8"),
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn plugin_invalid_json_response_surfaces_contract() {
    let dir = make_temp_dir("osp-cli-plugin-invalid-json");
    let _plugin_path = write_invalid_json_plugin(&dir);
    let home = make_temp_dir("osp-cli-plugin-invalid-json-home");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["broken"]);
    cmd.assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains(
            "invalid JSON response from plugin broken",
        ));

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn plugin_messages_stay_on_stderr_when_data_is_json_contract() {
    let dir = make_temp_dir("osp-cli-plugin-messages-json");
    let _plugin_path = write_message_plugin(&dir);
    let home = make_temp_dir("osp-cli-plugin-messages-json-home");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--json", "messageful"]);
    let output = cmd.assert().success().get_output().clone();
    let stdout = String::from_utf8(output.stdout.clone()).expect("stdout should be utf-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be utf-8");
    let payload = parse_json_stdout(stdout.as_bytes());
    let row = first_json_row(&payload, "plugin json message output");
    assert_eq!(row["message"], "json-from-plugin");
    assert!(!stdout.contains("plugin-warning-line"));
    assert_snapshot_text!("plugin_messages_json_stdout", stdout);
    assert_snapshot_text!("plugin_messages_json_stderr", stderr);

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn plugins_config_reports_projected_env_contract() {
    let home = make_temp_dir("osp-cli-plugin-config-view-home");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"
extensions.plugins.env.endpoint = "shared-endpoint"
extensions.plugins.env.shared.url = "https://common.example"
extensions.plugins.cfg.env.endpoint = "plugin-endpoint"
extensions.plugins.cfg.env.retries = 3
"#,
    );

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .envs(crate::test_env::isolated_env(&home))
        .args(["--json", "plugins", "config", "cfg"])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    let rows = payload
        .as_array()
        .expect("plugins config should render a JSON array");
    assert!(
        rows.iter().any(|row| {
            row["plugin_id"] == "cfg"
                && row["env"] == "OSP_PLUGIN_CFG_ENDPOINT"
                && row["value"] == "plugin-endpoint"
                && row["config_key"] == "extensions.plugins.cfg.env.endpoint"
                && row["scope"] == "plugin"
        }),
        "expected plugin-scoped endpoint row in payload: {payload}"
    );
    assert!(
        rows.iter().any(|row| {
            row["plugin_id"] == "cfg"
                && row["env"] == "OSP_PLUGIN_CFG_SHARED_URL"
                && row["value"] == "https://common.example"
                && row["config_key"] == "extensions.plugins.env.shared.url"
                && row["scope"] == "shared"
        }),
        "expected shared env row in payload: {payload}"
    );
    assert!(
        output.stderr.is_empty(),
        "stderr should stay empty: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn multi_command_plugin_receives_selected_command_contract() {
    let dir = make_temp_dir("osp-cli-plugin-multi-command");
    let _plugin_path = write_multi_command_plugin(&dir);
    let home = make_temp_dir("osp-cli-plugin-multi-command-home");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--json", "alpha", "run"])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    let row = first_json_row(&payload, "multi-command plugin dispatch");
    assert_eq!(row["selected_command"], "alpha");
    assert_eq!(row["arg0"], "alpha");
    assert_eq!(row["arg1"], "run");
    assert!(
        output.stderr.is_empty(),
        "stderr should stay empty: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn oneshot_dispatch_does_not_use_repl_session_cache_contract() {
    let dir = make_temp_dir("osp-cli-plugin-counter");
    let _plugin_path = write_counter_plugin(&dir);
    let home = make_temp_dir("osp-cli-plugin-counter-home");

    let mut first = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    first
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--unicode", "never", "counter"]);
    first
        .assert()
        .success()
        .stdout(predicate::str::contains("| 1"));

    let mut second = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    second
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--unicode", "never", "counter"]);
    second
        .assert()
        .success()
        .stdout(predicate::str::contains("| 2"));

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn describe_cache_is_reused_and_invalidated_contract() {
    let dir = make_temp_dir("osp-cli-plugin-describe-cache");
    let plugin_path = write_describe_counter_plugin(&dir);
    let home = make_temp_dir("osp-cli-plugin-describe-cache-home");
    let describe_count_path = dir.join("describe-count.txt");

    let mut first = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    first
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--json", "plugins", "list"]);
    let first_output = first
        .assert()
        .success()
        .get_output()
        .clone();
    let first_payload = parse_json_stdout(&first_output.stdout);
    assert!(
        first_payload
            .as_array()
            .expect("plugins list should render a JSON array")
            .iter()
            .any(|row| row["plugin_version"] == "0.1.1"),
        "expected plugin_version 0.1.1 in payload: {first_payload}"
    );
    assert_eq!(
        std::fs::read_to_string(&describe_count_path)
            .expect("describe count should be written")
            .trim(),
        "1"
    );

    let mut second = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    second
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--json", "plugins", "list"]);
    let second_output = second
        .assert()
        .success()
        .get_output()
        .clone();
    let second_payload = parse_json_stdout(&second_output.stdout);
    assert!(
        second_payload
            .as_array()
            .expect("plugins list should render a JSON array")
            .iter()
            .any(|row| row["plugin_version"] == "0.1.1"),
        "expected cached plugin_version 0.1.1 in payload: {second_payload}"
    );
    assert_eq!(
        std::fs::read_to_string(&describe_count_path)
            .expect("describe count should still be readable")
            .trim(),
        "1"
    );

    let mut script =
        std::fs::read_to_string(&plugin_path).expect("plugin script should be readable");
    script.push_str("\n# cache invalidation\n");
    std::fs::write(&plugin_path, script).expect("plugin script should be updated");

    let mut third = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    third
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--json", "plugins", "list"]);
    let third_output = third
        .assert()
        .success()
        .get_output()
        .clone();
    let third_payload = parse_json_stdout(&third_output.stdout);
    assert!(
        third_payload
            .as_array()
            .expect("plugins list should render a JSON array")
            .iter()
            .any(|row| row["plugin_version"] == "0.1.2"),
        "expected invalidated plugin_version 0.1.2 in payload: {third_payload}"
    );
    assert_eq!(
        std::fs::read_to_string(&describe_count_path)
            .expect("describe count should reflect invalidation")
            .trim(),
        "2"
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}
