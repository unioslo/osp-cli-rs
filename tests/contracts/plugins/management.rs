#[test]
fn plugins_list_and_doctor_contract() {
    let mut list = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    list.args(["plugins", "list"]);
    list.assert().success();

    let mut commands = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    commands.args(["plugins", "commands"]);
    commands.assert().success();

    let mut doctor = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    doctor.args(["plugins", "doctor"]);
    doctor.assert().success();
}

#[test]
fn debug_flag_enables_developer_logs_contract() {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.env_remove("RUST_LOG").args(["-dd", "plugins", "list"]);
    cmd.assert()
        .success()
        .stderr(predicate::str::contains("resolving runtime config"))
        .stderr(predicate::str::contains("osp session initialized"));
}

#[test]
fn plugins_enable_and_disable_contract() {
    let mut home = std::env::temp_dir();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be valid")
        .as_nanos();
    home.push(format!("osp-cli-plugin-test-{nonce}"));
    let dir = make_temp_dir("osp-cli-plugin-toggle");
    let _plugin = write_named_plugin(&dir, "hello", "hello");

    let mut enable = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    enable
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["plugins", "enable", "hello"]);
    enable
        .assert()
        .success()
        .stderr(predicate::str::contains("enabled command: hello"));

    let config_path = home.join(".config").join("osp").join("config.toml");
    let config = parse_toml_file(&config_path);
    assert_eq!(
        config["profile"]["default"]["plugins"]["hello"]["state"]
            .as_str()
            .expect("plugin state should be a string"),
        "enabled"
    );

    let mut disable = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    disable
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["plugins", "disable", "hello"]);
    disable
        .assert()
        .success()
        .stderr(predicate::str::contains("disabled command: hello"));

    let updated = parse_toml_file(&config_path);
    assert_eq!(
        updated["profile"]["default"]["plugins"]["hello"]["state"]
            .as_str()
            .expect("updated plugin state should be a string"),
        "disabled"
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn quiet_hides_success_messages_contract() {
    let mut home = std::env::temp_dir();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be valid")
        .as_nanos();
    home.push(format!("osp-cli-plugin-quiet-test-{nonce}"));
    let dir = make_temp_dir("osp-cli-plugin-quiet-toggle");
    let _plugin = write_named_plugin(&dir, "hello", "hello");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["-q", "plugins", "enable", "hello"]);
    cmd.assert().success().stderr(predicate::str::is_empty());

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn plugins_refresh_reports_success_contract() {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.args(["plugins", "refresh"]);
    cmd.assert()
        .success()
        .stderr(predicate::str::contains("refreshed plugin discovery cache"));
}

#[test]
#[cfg(unix)]
fn profile_override_can_select_unscoped_profile_contract() {
    let home = make_temp_dir("osp-cli-profile-override-home");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"

[profile.uio]
ui.format = "table"

[profile.tsd]
ui.format = "json"
"#,
    );

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--profile", "prod", "plugins", "list"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("No plugins discovered."));

    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn enabling_one_plugin_does_not_disable_other_default_enabled_plugins_contract() {
    let dir = make_temp_dir("osp-cli-plugin-enable-overrides");
    let _alpha = write_named_plugin(&dir, "alpha", "alpha");
    let _beta = write_named_plugin(&dir, "beta", "beta");
    let home = make_temp_dir("osp-cli-plugin-enable-overrides-home");

    let mut enable = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    enable
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["plugins", "enable", "alpha"]);
    enable
        .assert()
        .success()
        .stderr(predicate::str::contains("enabled command: alpha"));

    let mut clear = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    clear
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["plugins", "clear-state", "alpha"]);
    clear
        .assert()
        .success()
        .stderr(predicate::str::contains("cleared command state for alpha"));

    let mut beta = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    beta.envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["beta"]);
    beta.assert()
        .success()
        .stdout(predicate::str::contains("beta-from-plugin"));

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn plugins_enable_with_terminal_scope_and_clear_state_contract() {
    let mut home = std::env::temp_dir();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be valid")
        .as_nanos();
    home.push(format!("osp-cli-plugin-terminal-scope-{nonce}"));
    let dir = make_temp_dir("osp-cli-plugin-terminal-toggle");
    let _plugin = write_named_plugin(&dir, "hello", "hello");

    let mut enable = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    enable
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["plugins", "enable", "hello", "--terminal", "repl"]);
    enable.assert().success();

    let config_path = home.join(".config").join("osp").join("config.toml");
    let config = parse_toml_file(&config_path);
    assert_eq!(
        config["terminal"]["repl"]["profile"]["default"]["plugins"]["hello"]["state"]
            .as_str()
            .expect("terminal-scoped plugin state should be a string"),
        "enabled"
    );

    let mut clear = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    clear
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["plugins", "clear-state", "hello", "--terminal", "repl"]);
    clear
        .assert()
        .success()
        .stderr(predicate::str::contains("cleared command state for hello"));

    let updated = parse_toml_file(&config_path);
    assert!(
        updated
            .get("terminal")
            .and_then(|table| table.get("repl"))
            .and_then(|table| table.get("profile"))
            .and_then(|table| table.get("default"))
            .and_then(|table| table.get("plugins"))
            .and_then(|table| table.get("hello"))
            .is_none(),
        "expected terminal-scoped plugin state to be cleared"
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}
