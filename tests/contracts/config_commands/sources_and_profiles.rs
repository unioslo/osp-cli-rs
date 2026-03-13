#[cfg(unix)]
#[test]
fn positional_profile_with_config_get_contract() {
    let home = make_temp_dir("osp-cli-config-profile");
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
        .args(["--json", "tsd", "config", "get", "ui.format"])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    let row = first_json_row(&payload, "positional profile config get");
    assert_eq!(row["key"], "ui.format");
    assert_eq!(row["value"], "json");
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

}

#[cfg(unix)]
#[test]
fn positional_profile_with_config_explain_contract() {
    let home = make_temp_dir("osp-cli-config-profile-explain");
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
        .args(["--json", "tsd", "config", "explain", "ui.format"])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    assert_eq!(payload["active_profile"], "tsd");
    assert_eq!(payload["value"], "json");
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

}

#[cfg(unix)]
#[test]
fn no_env_ignores_environment_overrides_contract() {
    let home = make_temp_dir("osp-cli-config-no-env");
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
        .args(["--json", "--no-env", "config", "explain", "ui.mode"])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    assert_eq!(payload["value"], "plain");
    assert_eq!(payload["source"], "file");
    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf-8");
    assert!(!stdout.contains("OSP__UI__MODE"));
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

}

#[cfg(unix)]
#[test]
fn no_config_file_ignores_file_values_contract() {
    let home = make_temp_dir("osp-cli-config-no-config-file");
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
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "--no-config-file", "config", "get", "ui.mode"]);
    cmd.assert().failure();

}

#[cfg(unix)]
#[test]
fn defaults_only_ignores_file_and_environment_bootstrap_contract() {
    let home = make_temp_dir("osp-cli-config-defaults-only");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"
theme.name = "dracula"
"#,
    );

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .env("OSP__THEME__NAME", "nord")
        .args(["--json", "--defaults-only", "config", "explain", "theme.name"])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    assert_eq!(payload["value"], osp_cli::ui::DEFAULT_THEME_NAME);
    assert_eq!(payload["source"], "defaults");
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn positional_and_explicit_profile_resolve_equivalent_config_contract() {
    let home = make_temp_dir("osp-cli-config-profile-equivalent");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"

[profile.uio]
ui.mode = "plain"

[profile.tsd]
ui.mode = "rich"

[terminal.cli.profile.tsd]
ui.mode = "plain"
"#,
    );

    let mut positional = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let positional_out = positional
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "tsd", "config", "get", "ui.mode"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let mut explicit = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let explicit_out = explicit
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "--profile", "tsd", "config", "get", "ui.mode"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(positional_out, explicit_out);

}

#[cfg(unix)]
#[test]
fn launch_json_flag_formats_output_without_mutating_config_contract() {
    let home = make_temp_dir("osp-cli-config-launch-json");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"
ui.format = "table"
"#,
    );

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "config", "get", "ui.format", "--sources"])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    let row = first_json_row(&payload, "launch --json config get --sources");
    assert_eq!(row["key"], "ui.format");
    assert_eq!(row["value"], "table");
    assert_eq!(row["source"], "file");
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

}
