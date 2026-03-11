#[cfg(unix)]
#[test]
fn config_show_contract() {
    let home = make_temp_dir("osp-cli-config-show");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"
ui.format = "table"
extensions.feature.flag = "on"

[profile.uio]
ui.mode = "plain"

[profile.tsd]
ui.format = "json"
"#,
    );

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "config", "show"])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    let rows = payload
        .as_array()
        .expect("config show should render a JSON array");
    assert!(
        rows.iter().any(|row| {
            row["key"] == "ui.format" && row["value"] == "table"
        }),
        "expected ui.format row in payload: {payload}"
    );
    assert!(
        rows.iter()
            .all(|row| row["key"] != "profile.default"),
        "config show should omit bootstrap-only profile.default row: {payload}"
    );
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn config_get_with_sources_contract() {
    let home = make_temp_dir("osp-cli-config-get");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"

[profile.uio]
ui.mode = "plain"
"#,
    );

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "config", "get", "ui.mode", "--sources"])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    let row = first_json_row(&payload, "config get --sources");
    assert_eq!(row["key"], "ui.mode");
    assert_eq!(row["value"], "plain");
    assert_eq!(row["source"], "file");
    assert_eq!(row["scope_profile"], "uio");
    assert_eq!(row["scope_terminal"], serde_json::Value::Null);
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn config_get_profile_default_uses_bootstrap_view_contract() {
    let home = make_temp_dir("osp-cli-config-get-default-profile");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"

[terminal.repl]
profile.default = "tsd"
"#,
    );

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "config", "get", "profile.default", "--sources"])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    let row = first_json_row(&payload, "config get profile.default --sources");
    assert_eq!(row["key"], "profile.default");
    assert_eq!(row["value"], "uio");
    assert_eq!(row["source"], "file");
    assert_eq!(row["scope_profile"], serde_json::Value::Null);
    assert_eq!(row["scope_terminal"], serde_json::Value::Null);
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn config_get_alias_uses_alias_namespace_contract() {
    let home = make_temp_dir("osp-cli-config-get-alias");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"
alias.me = "ldap user ${user.name}"
user.name = "tester"
"#,
    );

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "config", "get", "alias.me", "--sources"])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    let row = first_json_row(&payload, "config get alias.me --sources");
    assert_eq!(row["key"], "alias.me");
    assert_eq!(row["value"], "ldap user ${user.name}");
    assert_eq!(row["source"], "file");
    assert_eq!(row["scope_profile"], serde_json::Value::Null);
    assert_eq!(row["scope_terminal"], serde_json::Value::Null);
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn config_doctor_contract() {
    let home = make_temp_dir("osp-cli-config-doctor");
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
    let output = cmd
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "config", "doctor"])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    let row = first_json_row(&payload, "config doctor");
    assert_eq!(row["status"], "ok");
    assert_eq!(row["active_profile"], "uio");
    assert_eq!(row["theme_issue_count"], 0);
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn config_get_missing_key_writes_grouped_error_to_stderr_contract() {
    let home = make_temp_dir("osp-cli-config-missing-key");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"
"#,
    );

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args([
            "--mode",
            "rich",
            "--color",
            "never",
            "--unicode",
            "never",
            "config",
            "get",
            "missing.key",
        ]);
    let output = cmd.assert().failure().get_output().clone();
    assert!(
        output.stdout.is_empty(),
        "stdout should stay empty: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_snapshot_text!(
        "config_get_missing_key_grouped_stderr",
        String::from_utf8(output.stderr).expect("stderr should be utf-8"),
    );

    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn config_get_missing_key_honors_rich_color_and_unicode_contract() {
    let home = make_temp_dir("osp-cli-config-missing-key-rich");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"
"#,
    );

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args([
            "--mode",
            "rich",
            "--color",
            "always",
            "--unicode",
            "always",
            "config",
            "get",
            "missing.key",
        ]);
    cmd.assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("─ Errors "))
        .stderr(predicate::str::contains("\x1b["));

    let _ = std::fs::remove_dir_all(&home);
}
