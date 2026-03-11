#[cfg(unix)]
#[test]
fn config_explain_reports_winner_and_candidates_contract() {
    let home = make_temp_dir("osp-cli-config-explain");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"
ui.mode = "rich"

[profile.uio]
ui.mode = "plain"
"#,
    );

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .env("OSP__UI__MODE", "auto")
        .args(["--json", "config", "explain", "ui.mode"])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    assert_eq!(payload["source"], "env");
    let candidates = payload["candidates"]
        .as_array()
        .expect("candidates should be an array");
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate["source"] == "env" && candidate["winner"] == true)
    );
    assert_eq!(
        candidates
            .iter()
            .filter(|candidate| candidate["winner"] == true)
            .count(),
        1
    );
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

}

#[cfg(unix)]
#[test]
fn config_explain_json_stdout_is_machine_parseable_contract() {
    let home = make_temp_dir("osp-cli-config-explain-parseable");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"
ui.mode = "plain"
"#,
    );

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "config", "explain", "ui.mode"])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    assert_eq!(payload["key"], "ui.mode");
    assert_eq!(payload["value"], "plain");
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

}

#[cfg(unix)]
#[test]
fn config_explain_reports_presentation_seeded_values_contract() {
    let home = make_temp_dir("osp-cli-config-explain-presentation");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"
"#,
    );

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args([
            "--json",
            "--presentation",
            "austere",
            "config",
            "explain",
            "ui.chrome.frame",
        ])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    assert_eq!(payload["value"], "none");
    assert_eq!(payload["presentation"]["preset"], "austere");
    assert_eq!(payload["presentation"]["preset_source"], "session");
    assert_eq!(payload["presentation"]["seeded_value"], "none");
    assert_eq!(payload["presentation"]["seeded_value_type"], "string");
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

}

#[cfg(unix)]
#[test]
fn config_set_explain_json_keeps_messages_off_stdout_contract() {
    let home = make_temp_dir("osp-cli-config-set-explain-json");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"
"#,
    );

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args([
            "--json",
            "config",
            "set",
            "--session",
            "ui.mode",
            "plain",
            "--explain",
        ])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    assert_eq!(payload["key"], "ui.mode");
    assert_eq!(payload["value"], "plain");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("set value for ui.mode"),
        "expected success message on stderr, got: {}",
        String::from_utf8_lossy(&output.stderr)
    );

}

#[cfg(unix)]
#[test]
fn config_explain_profile_default_uses_bootstrap_view_contract() {
    let home = make_temp_dir("osp-cli-config-explain-default-profile");
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
        .args(["--json", "config", "explain", "profile.default"])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    assert_eq!(payload["key"], "profile.default");
    assert_eq!(payload["phase"], "bootstrap");
    assert_eq!(payload["active_profile_source"], "profile.default");
    assert_eq!(
        payload["bootstrap_scope_policy"],
        "global and terminal-only; profile scopes are ignored during bootstrap"
    );
    assert_eq!(payload["value"], "uio");
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

}

#[cfg(unix)]
#[test]
fn config_explain_profile_default_reports_override_source_contract() {
    let home = make_temp_dir("osp-cli-config-explain-default-profile-override");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"

[profile.uio]
ui.mode = "plain"

[profile.tsd]
ui.mode = "rich"
"#,
    );

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "tsd", "config", "explain", "profile.default"])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    assert_eq!(payload["phase"], "bootstrap");
    assert_eq!(payload["active_profile"], "tsd");
    assert_eq!(payload["active_profile_source"], "override");
    assert_eq!(payload["value"], "uio");
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

}

#[cfg(unix)]
#[test]
fn config_explain_profile_active_reports_runtime_phase_contract() {
    let home = make_temp_dir("osp-cli-config-explain-active-profile");
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
        .args(["--json", "config", "explain", "profile.active"])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    assert_eq!(payload["key"], "profile.active");
    assert_eq!(payload["phase"], "runtime");
    assert_eq!(payload["value"], "uio");
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

}

#[cfg(unix)]
#[test]
fn config_explain_reports_interpolation_trace_contract() {
    let home = make_temp_dir("osp-cli-config-explain-interpolation");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"
base.dir = "/etc/osp"
ui.prompt = "${profile.active}:${extensions.uio.ldap.url}:${base.dir}"

[profile.uio]
extensions.uio.ldap.url = "ldaps://ldap.uio.no"
"#,
    );

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "config", "explain", "ui.prompt"])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    assert_eq!(
        payload["interpolation"]["template"],
        "${profile.active}:${extensions.uio.ldap.url}:${base.dir}"
    );
    let steps = payload["interpolation"]["steps"]
        .as_array()
        .expect("interpolation steps should be an array");
    assert!(
        steps
            .iter()
            .any(|step| step["placeholder"] == "extensions.uio.ldap.url")
    );
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

}

#[cfg(unix)]
#[test]
fn config_explain_redacts_secrets_unless_flag_contract() {
    let home = make_temp_dir("osp-cli-config-explain-secret");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"
extensions.uio.ldap.bind_password = "file-secret"
"#,
    );

    let mut redacted = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let redacted_output = redacted
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args([
            "--json",
            "config",
            "explain",
            "extensions.uio.ldap.bind_password",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let redacted_payload = parse_json_stdout(&redacted_output.stdout);
    assert_eq!(redacted_payload["value"], "[REDACTED]");
    assert_eq!(redacted_payload["value_type"], "string");

    let mut clear = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let clear_output = clear
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args([
            "--json",
            "config",
            "explain",
            "extensions.uio.ldap.bind_password",
            "--show-secrets",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let clear_payload = parse_json_stdout(&clear_output.stdout);
    assert_eq!(clear_payload["value"], "file-secret");

}

#[cfg(unix)]
#[test]
fn config_explain_redacts_secrets_source_even_without_sensitive_key_contract() {
    let home = make_temp_dir("osp-cli-config-explain-secret-source");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"
"#,
    );
    write_secrets(
        &home,
        r#"
[default]
extensions.demo.potato = "sekrit"
"#,
    );

    let mut redacted = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let redacted_output = redacted
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "config", "explain", "extensions.demo.potato"])
        .assert()
        .success()
        .get_output()
        .clone();
    let redacted_payload = parse_json_stdout(&redacted_output.stdout);
    assert_eq!(redacted_payload["value"], "[REDACTED]");

    let mut clear = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let clear_output = clear
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args([
            "--json",
            "config",
            "explain",
            "extensions.demo.potato",
            "--show-secrets",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let clear_payload = parse_json_stdout(&clear_output.stdout);
    assert_eq!(clear_payload["value"], "sekrit");

    let mut get_redacted = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let get_redacted_output = get_redacted
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "config", "get", "extensions.demo.potato"])
        .assert()
        .success()
        .get_output()
        .clone();
    let get_redacted_payload = parse_json_stdout(&get_redacted_output.stdout);
    let row = first_json_row(&get_redacted_payload, "config get redacted secret");
    assert_eq!(row["key"], "extensions.demo.potato");
    assert_eq!(row["value"], "[REDACTED]");

}

#[cfg(unix)]
#[test]
fn config_explain_missing_key_writes_suggestions_to_stderr_contract() {
    let home = make_temp_dir("osp-cli-config-explain-missing-key");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"
ui.mode = "plain"
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
            "explain",
            "ui.m",
        ]);
    let output = cmd.assert().failure().get_output().clone();
    assert!(
        output.stdout.is_empty(),
        "stdout should stay empty: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_snapshot_text!(
        "config_explain_missing_key_grouped_stderr",
        String::from_utf8(output.stderr).expect("stderr should be utf-8"),
    );

}

#[cfg(unix)]
#[test]
fn config_explain_human_output_contract() {
    let home = make_temp_dir("osp-cli-config-explain-human");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"
ui.mode = "rich"

[profile.uio]
ui.mode = "plain"
"#,
    );

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args([
            "--mode",
            "rich",
            "--color",
            "never",
            "--unicode",
            "never",
            "config",
            "explain",
            "ui.mode",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    assert!(
        output.stderr.is_empty(),
        "stderr should stay empty: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let home_text = home.display().to_string();
    assert_snapshot_text_with!(
        "config_explain_human_stdout",
        String::from_utf8(output.stdout).expect("stdout should be utf-8"),
        &[(&home_text, "<HOME>")],
    );

}

#[cfg(unix)]
#[test]
fn config_explain_missing_key_keeps_stdout_clean_contract() {
    let home = make_temp_dir("osp-cli-config-explain-missing-key");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"
ui.format = "json"
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
            "explain",
            "ui.formt",
        ]);
    cmd.assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("config key not found: ui.formt"));

}
