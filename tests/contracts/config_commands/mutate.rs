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

[profile.work]
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

[profile.work]
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
repl.history.exclude = []

[profile.uio]
[profile.tsd]
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
    let stored: toml::Value = toml::from_str(&payload).expect("stored config should parse");
    assert_eq!(
        stored["terminal"]["repl"]["profile"]["default"].as_str(),
        Some("tsd")
    );
    assert_eq!(
        stored["default"]["profile"]["default"].as_str(),
        Some("uio")
    );

    let run = |args: &[&str]| {
        Command::new(assert_cmd::cargo::cargo_bin!("osp"))
            .envs(crate::test_env::isolated_env(&home))
            .env("PATH", "/usr/bin:/bin")
            .args(args)
            .assert()
            .success()
            .get_output()
            .clone()
    };
    let config_path = home.join(".config/osp/config.toml");
    let exclusions = serde_json::json!(["help", "config show"]);
    for preview in [true, false] {
        let mut args = vec![
            "--json", "config", "set", "--profile-all", "--terminal", "cli",
            "repl.history.exclude", "['help', 'config show']",
        ];
        if preview {
            args.push("--dry-run");
        }
        let output = run(&args);
        let payload = parse_json_stdout(&output.stdout);
        let rows = payload.as_array().expect("each profile should have a write result");
        assert_eq!(
            rows.iter().map(|row| row["scope"].as_str().unwrap()).collect::<Vec<_>>(),
            vec!["profile:default terminal:cli", "profile:tsd terminal:cli", "profile:uio terminal:cli"],
        );
        for row in rows {
            assert_eq!(row["key"], "repl.history.exclude");
            assert_eq!(row["value"], exclusions);
            assert_eq!(row["store"], "config");
            assert_eq!(row["dry_run"], preview);
            assert_eq!(row["changed"], true);
            assert_eq!(row["previous"], serde_json::Value::Null);
            assert_eq!(row["path"], config_path.to_str().unwrap());
        }
        let persisted: toml::Value = toml::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        if preview {
            assert_eq!(persisted, stored);
        } else {
            for profile in ["default", "uio", "tsd"] {
                assert_eq!(
                    persisted["terminal"]["cli"]["profile"][profile]["repl"]["history"]["exclude"].as_array().unwrap(),
                    &vec![toml::Value::String("help".into()), toml::Value::String("config show".into())],
                );
            }
        }
    }
    for profile in ["default", "uio", "tsd"] {
        let output = run(&["--json", "--profile", profile, "config", "explain", "repl.history.exclude"]);
        let explain = parse_json_stdout(&output.stdout);
        assert_eq!(explain["value"], exclusions);
        assert_eq!(explain["value_type"], "list");
        assert_eq!(explain["source"], "file");
        assert_eq!(explain["scope"], format!("profile:{profile} terminal:cli"));
    }
    let preview = run(&["--plain", "config", "set", "repl.history.exclude", "[]", "--profile-all", "--terminal", "cli", "--dry-run", "--explain"]);
    assert!(String::from_utf8_lossy(&preview.stdout).contains("value: [\"help\",\"config show\"] (list)"));
    assert!(String::from_utf8_lossy(&preview.stderr).contains("would set"));

    for preview in [true, false] {
        let mut args = vec!["--json", "config", "unset", "repl.history.exclude", "--profile-all", "--terminal", "cli"];
        if preview {
            args.push("--dry-run");
        }
        let output = run(&args);
        let payload = parse_json_stdout(&output.stdout);
        for row in payload.as_array().unwrap() {
            assert_eq!(row["previous"], exclusions);
            assert_eq!(row["changed"], true);
            assert_eq!(row["dry_run"], preview);
        }
        let output = run(&["--json", "config", "get", "repl.history.exclude"]);
        let payload = parse_json_stdout(&output.stdout);
        assert_eq!(first_json_row(&payload, "history exclusion reload")["value"], if preview { exclusions.clone() } else { serde_json::json!([]) });
    }

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
    let stored: toml::Value = toml::from_str(&payload).expect("stored config should parse");
    assert!(
        stored
            .get("terminal")
            .and_then(|terminal| terminal.get("repl"))
            .and_then(|repl| repl.get("profile"))
            .and_then(|profile| profile.get("default"))
            .is_none()
    );
    assert_eq!(
        stored["default"]["profile"]["default"].as_str(),
        Some("uio")
    );

}
