#[test]
#[cfg(unix)]
fn unknown_domain_command_shows_plugin_hint_contract() {
    let home = make_temp_dir("osp-cli-no-plugin-home");
    let empty_plugins = make_temp_dir("osp-cli-empty-plugins");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .env("OSP_PLUGIN_PATH", &empty_plugins)
        .env("OSP_BUNDLED_PLUGIN_DIR", &empty_plugins)
        .args(["ldap", "user", "oistes"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("no plugin provides command: ldap"))
        .stderr(predicate::str::contains(
            "Hint: run osp plugins list and set --plugin-dir or OSP_PLUGIN_PATH",
        ));

    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&empty_plugins);
}

#[test]
#[cfg(unix)]
fn errors_remain_visible_at_double_quiet_contract() {
    let home = make_temp_dir("osp-cli-no-plugin-home-quiet");
    let empty_plugins = make_temp_dir("osp-cli-empty-plugins-quiet");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .env("OSP_PLUGIN_PATH", &empty_plugins)
        .env("OSP_BUNDLED_PLUGIN_DIR", &empty_plugins)
        .args(["-qq", "ldap", "user", "oistes"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("no plugin provides command: ldap"));

    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&empty_plugins);
}

#[cfg(unix)]
#[test]
fn external_plugin_help_is_passed_through_contract() {
    let dir = make_temp_dir("osp-cli-plugin-help");
    let _plugin_path = write_hello_plugin(&dir);
    let home = make_temp_dir("osp-cli-plugin-help-home");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["hello", "--help"]);
    let help_flag = cmd.assert().success().get_output().clone();
    let help_flag_stdout =
        String::from_utf8(help_flag.stdout).expect("help stdout should be utf-8");
    assert_snapshot_text!("external_plugin_help_stdout", help_flag_stdout.clone());

    let mut cmd_help_subcommand = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd_help_subcommand
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["hello", "help"]);
    let help_subcommand = cmd_help_subcommand.assert().success().get_output().clone();
    let help_subcommand_stdout =
        String::from_utf8(help_subcommand.stdout).expect("help stdout should be utf-8");
    assert_eq!(help_subcommand_stdout, help_flag_stdout);

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn external_plugin_help_keeps_raw_stderr_contract() {
    let dir = make_temp_dir("osp-cli-plugin-help-stderr");
    let _plugin_path = write_help_stderr_plugin(&dir);
    let home = make_temp_dir("osp-cli-plugin-help-stderr-home");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["hello", "--help"]);
    let output = cmd.assert().success().get_output().clone();
    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be utf-8");
    assert_snapshot_text!("external_plugin_help_stderr_stdout", stdout);
    assert_snapshot_text!("external_plugin_help_stderr_stderr", stderr);

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn ignores_non_plugin_extension_files_contract() {
    use std::os::unix::fs::PermissionsExt;

    let dir = make_temp_dir("osp-cli-ignore-script");
    let script_path = dir.join("osp-ignore.sh");
    std::fs::write(&script_path, "#!/bin/sh\necho should-not-run\n")
        .expect("script should be written");
    let mut perms = std::fs::metadata(&script_path)
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&script_path, perms).expect("script should be executable");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.env("PATH", "/usr/bin:/bin")
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["plugins", "list"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("osp-ignore.sh").not());

    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn bundled_plugin_requires_manifest_contract() {
    let dir = make_temp_dir("osp-cli-plugin-bundled-missing-manifest");
    let _plugin_path = write_hello_plugin(&dir);
    let home = make_temp_dir("osp-cli-plugin-home");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("OSP_BUNDLED_PLUGIN_DIR", &dir)
        .args(["plugins", "list"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("bundled manifest.toml not found"))
        .stdout(predicate::str::contains("healthy:        false"))
        .stdout(predicate::str::contains("source:         bundled"));

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn bundled_manifest_controls_default_enable_contract() {
    let dir = make_temp_dir("osp-cli-plugin-bundled-manifest");
    let _plugin_path = write_hello_plugin(&dir);
    write_manifest(
        &dir,
        r#"
protocol_version = 1

[[plugin]]
id = "hello"
exe = "osp-hello"
version = "0.1.0"
enabled_by_default = false
commands = ["hello"]
"#,
    );
    let home = make_temp_dir("osp-cli-plugin-home");

    let mut first = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    first
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_BUNDLED_PLUGIN_DIR", &dir)
        .args(["hello"]);
    first.assert().failure().stderr(predicate::str::contains(
        "no plugin provides command: hello",
    ));

    let mut enable = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    enable
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_BUNDLED_PLUGIN_DIR", &dir)
        .args(["plugins", "enable", "hello"]);
    enable
        .assert()
        .success()
        .stderr(predicate::str::contains("enabled command: hello"));

    let mut second = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    second
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_BUNDLED_PLUGIN_DIR", &dir)
        .args(["hello"]);
    second
        .assert()
        .success()
        .stdout(predicate::str::contains("hello-from-plugin"));

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn bundled_manifest_mismatch_marks_plugin_unhealthy_contract() {
    let dir = make_temp_dir("osp-cli-plugin-bundled-manifest-mismatch");
    let _plugin_path = write_hello_plugin(&dir);
    write_manifest(
        &dir,
        r#"
protocol_version = 1

[[plugin]]
id = "hello"
exe = "osp-hello"
version = "0.1.0"
enabled_by_default = true
commands = ["ldap"]
"#,
    );
    let home = make_temp_dir("osp-cli-plugin-home");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("OSP_BUNDLED_PLUGIN_DIR", &dir)
        .args(["plugins", "list"]);
    cmd.assert().success().stdout(predicate::str::contains(
        "manifest commands mismatch for hello",
    ));

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn bundled_manifest_id_stays_visible_when_describe_id_mismatches_contract() {
    let dir = make_temp_dir("osp-cli-plugin-bundled-id-mismatch");
    let _plugin_path = write_describe_mismatch_plugin(&dir);
    write_manifest(
        &dir,
        r#"
protocol_version = 1

[[plugin]]
id = "hello"
exe = "osp-hello"
version = "0.1.0"
enabled_by_default = true
commands = ["hello"]
"#,
    );
    let home = make_temp_dir("osp-cli-plugin-bundled-id-mismatch-home");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_BUNDLED_PLUGIN_DIR", &dir)
        .args(["--json", "plugins", "list"])
        .assert()
        .success()
        .get_output()
        .clone();
    let payload = parse_json_stdout(&output.stdout);
    assert!(
        payload
            .as_array()
            .expect("plugins list should render a JSON array")
            .iter()
            .any(|row| {
                row["plugin_id"] == "hello"
                    && row["issue"]
                        .as_str()
                        .is_some_and(|issue| issue.contains("manifest id mismatch: expected hello, got wrong"))
            }),
        "expected manifest id mismatch row in payload: {payload}"
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn plugin_min_osp_version_mismatch_marks_plugin_unhealthy_contract() {
    let dir = make_temp_dir("osp-cli-plugin-min-osp-version");
    let _plugin_path = write_plugin_with_min_version(&dir, "future", "9.9.9");
    let home = make_temp_dir("osp-cli-plugin-min-osp-version-home");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--json", "plugins", "list"])
        .assert()
        .success()
        .get_output()
        .clone();
    let payload = parse_json_stdout(&output.stdout);
    assert!(
        payload
            .as_array()
            .expect("plugins list should render a JSON array")
            .iter()
            .any(|row| {
                row["plugin_id"] == "future"
                    && row["healthy"] == false
                    && row["issue"]
                        .as_str()
                        .is_some_and(|issue| issue.contains("requires osp >="))
            }),
        "expected min-version failure row in payload: {payload}"
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}
