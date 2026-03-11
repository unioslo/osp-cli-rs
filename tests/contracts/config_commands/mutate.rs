#[cfg(unix)]
#[test]
fn config_unset_persistent_contract() {
    let home = make_temp_dir("osp-cli-config-unset");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"

[profile.uio]
ui.mode = "plain"
"#,
    );

    let mut unset = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = unset
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "config", "unset", "ui.mode"])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    let row = first_json_row(&payload, "config unset");
    assert_eq!(row["key"], "ui.mode");
    assert_eq!(row["scope"], "profile:uio");
    assert_eq!(row["changed"], true);
    assert_eq!(row["previous"], "plain");

    let mut get = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    get.envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "config", "get", "ui.mode"]);
    get.assert().failure();

    let payload = std::fs::read_to_string(home.join(".config").join("osp").join("config.toml"))
        .expect("config should be readable");
    assert!(!payload.contains("ui.mode"));

    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn config_set_rejects_profile_scoped_default_profile_contract() {
    let home = make_temp_dir("osp-cli-config-set-bootstrap-scope");
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
            "config",
            "set",
            "--profile",
            "work",
            "profile.default",
            "personal",
        ]);
    cmd.assert().failure().stderr(predicate::str::contains(
        "bootstrap-only key profile.default is not allowed",
    ));

    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn config_set_rejects_profile_terminal_scoped_default_profile_contract() {
    let home = make_temp_dir("osp-cli-config-set-bootstrap-profile-terminal-scope");
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
            "config",
            "set",
            "--profile",
            "work",
            "--terminal",
            "repl",
            "profile.default",
            "personal",
        ]);
    cmd.assert().failure().stderr(predicate::str::contains(
        "bootstrap-only key profile.default is not allowed",
    ));

    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn config_set_allows_terminal_scoped_default_profile_contract() {
    let home = make_temp_dir("osp-cli-config-set-bootstrap-terminal-scope");
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
            "--global",
            "--terminal",
            "repl",
            "profile.default",
            "tsd",
        ])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    let row = first_json_row(&payload, "config set terminal-scoped profile.default");
    assert_eq!(row["key"], "profile.default");
    assert_eq!(row["value"], "tsd");
    assert_eq!(row["scope"], "terminal:repl");
    assert_eq!(row["changed"], true);
    assert_eq!(row["previous"], serde_json::Value::Null);

    let payload = std::fs::read_to_string(home.join(".config").join("osp").join("config.toml"))
        .expect("config should be readable");
    assert!(payload.contains("terminal"));
    assert!(payload.contains("repl"));
    assert!(payload.contains("profile"));
    assert!(payload.contains("default = \"tsd\""));

    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn config_unset_allows_terminal_scoped_default_profile_contract() {
    let home = make_temp_dir("osp-cli-config-unset-bootstrap-terminal-scope");
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
        .args([
            "--json",
            "config",
            "unset",
            "--global",
            "--terminal",
            "repl",
            "profile.default",
        ])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    let row = first_json_row(&payload, "config unset terminal-scoped profile.default");
    assert_eq!(row["key"], "profile.default");
    assert_eq!(row["scope"], "terminal:repl");
    assert_eq!(row["changed"], true);
    assert_eq!(row["previous"], "tsd");

    let payload = std::fs::read_to_string(home.join(".config").join("osp").join("config.toml"))
        .expect("config should be readable");
    assert!(!payload.contains("profile.default = \"tsd\""));

    let _ = std::fs::remove_dir_all(&home);
}
