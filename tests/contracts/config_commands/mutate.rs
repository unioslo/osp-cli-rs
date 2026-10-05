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
            "--json",
            "config",
            "set",
            "--profile-all",
            "--terminal",
            "cli",
            "repl.history.exclude",
            "['help', 'config show']",
        ];
        if preview {
            args.push("--dry-run");
        }
        let output = run(&args);
        let payload = parse_json_stdout(&output.stdout);
        let rows = payload
            .as_array()
            .expect("each profile should have a write result");
        assert_eq!(
            rows.iter()
                .map(|row| row["scope"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec![
                "profile:default terminal:cli",
                "profile:tsd terminal:cli",
                "profile:uio terminal:cli"
            ],
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
        let persisted: toml::Value =
            toml::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        if preview {
            assert_eq!(persisted, stored);
        } else {
            for profile in ["default", "uio", "tsd"] {
                assert_eq!(
                    persisted["terminal"]["cli"]["profile"][profile]["repl"]["history"]["exclude"]
                        .as_array()
                        .unwrap(),
                    &vec![
                        toml::Value::String("help".into()),
                        toml::Value::String("config show".into())
                    ],
                );
            }
        }
    }
    for profile in ["default", "uio", "tsd"] {
        let output = run(&[
            "--json",
            "--profile",
            profile,
            "config",
            "explain",
            "repl.history.exclude",
        ]);
        let explain = parse_json_stdout(&output.stdout);
        assert_eq!(explain["value"], exclusions);
        assert_eq!(explain["value_type"], "list");
        assert_eq!(explain["source"], "file");
        assert_eq!(explain["scope"], format!("profile:{profile} terminal:cli"));
    }
    let preview = run(&[
        "--plain",
        "config",
        "set",
        "repl.history.exclude",
        "[]",
        "--profile-all",
        "--terminal",
        "cli",
        "--dry-run",
        "--explain",
    ]);
    assert!(
        String::from_utf8_lossy(&preview.stdout)
            .contains("value: [\"help\",\"config show\"] (list)")
    );
    assert!(String::from_utf8_lossy(&preview.stderr).contains("would set"));

    for preview in [true, false] {
        let mut args = vec![
            "--json",
            "config",
            "unset",
            "repl.history.exclude",
            "--profile-all",
            "--terminal",
            "cli",
        ];
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
        assert_eq!(
            first_json_row(&payload, "history exclusion reload")["value"],
            if preview {
                exclusions.clone()
            } else {
                serde_json::json!([])
            }
        );
    }

    let baseline = run(&["--json", "--profile", "tsd", "config", "get", "ui.width"]);
    let baseline_width =
        first_json_row(&parse_json_stdout(&baseline.stdout), "baseline width")["value"].clone();
    let target = run(&[
        "--json",
        "config",
        "set",
        "config.default-target",
        "tsd",
        "--global",
    ]);
    assert_eq!(
        first_json_row(
            &parse_json_stdout(&target.stdout),
            "default profile write target"
        )["value"],
        "tsd"
    );
    let scoped = run(&[
        "--json",
        "config",
        "set",
        "ui.width",
        "112",
        "--terminal",
        "cli",
    ]);
    let scoped_payload = parse_json_stdout(&scoped.stdout);
    let scoped_row = first_json_row(&scoped_payload, "default target profile-terminal edit");
    assert_eq!(scoped_row["key"], "ui.width");
    assert_eq!(scoped_row["value"], 112);
    assert_eq!(scoped_row["scope"], "profile:tsd terminal:cli");
    assert_eq!(scoped_row["store"], "config");
    assert_eq!(scoped_row["path"], config_path.to_str().unwrap());
    let stored: toml::Value =
        toml::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(
        stored["terminal"]["cli"]["profile"]["tsd"]["ui"]["width"].as_integer(),
        Some(112)
    );
    let explained = run(&[
        "--json",
        "--profile",
        "tsd",
        "config",
        "explain",
        "ui.width",
    ]);
    let explained = parse_json_stdout(&explained.stdout);
    assert_eq!(explained["value"], 112);
    assert_eq!(explained["value_type"], "integer");
    assert_eq!(explained["source"], "file");
    assert_eq!(explained["scope"], "profile:tsd terminal:cli");
    assert_eq!(explained["origin"], config_path.to_str().unwrap());

    run(&[
        "--json",
        "config",
        "set",
        "config.default-target",
        "session",
        "--global",
    ]);
    run(&[
        "--json",
        "config",
        "set",
        "profile.default",
        "tsd",
        "--global",
    ]);
    let session = run(&[
        "--json",
        "config",
        "set",
        "ui.width",
        "96",
        "--terminal",
        "cli",
        "--explain",
    ]);
    let session_payload = parse_json_stdout(&session.stdout);
    assert_eq!(session_payload["value"], 96);
    assert_eq!(session_payload["value_type"], "integer");
    assert_eq!(session_payload["source"], "session");
    assert_eq!(session_payload["scope"], "profile:tsd terminal:cli");
    assert!(
        String::from_utf8_lossy(&session.stderr)
            .contains("config set ui.width 96 --permanent --profile tsd --terminal cli")
    );
    let fresh = run(&[
        "--json",
        "--profile",
        "tsd",
        "config",
        "get",
        "ui.width",
        "--sources",
    ]);
    let fresh_payload = parse_json_stdout(&fresh.stdout);
    let fresh_row = first_json_row(&fresh_payload, "fresh process persisted width");
    assert_eq!(fresh_row["value"], 112);
    assert_eq!(fresh_row["source"], "file");
    assert_eq!(fresh_row["scope_profile"], "tsd");
    assert_eq!(fresh_row["scope_terminal"], "cli");

    let saved = run(&[
        "--json",
        "config",
        "set",
        "ui.width",
        "96",
        "--permanent",
        "--profile",
        "tsd",
        "--terminal",
        "cli",
    ]);
    let saved_payload = parse_json_stdout(&saved.stdout);
    let saved_row = first_json_row(&saved_payload, "suggested permanent width edit");
    assert_eq!(saved_row["store"], "config");
    assert_eq!(saved_row["scope"], "profile:tsd terminal:cli");
    assert_eq!(saved_row["value"], 96);
    assert_eq!(saved_row["previous"], 112);
    let persisted = run(&["--json", "--profile", "tsd", "config", "get", "ui.width"]);
    assert_eq!(
        first_json_row(&parse_json_stdout(&persisted.stdout), "saved width reload")["value"],
        96
    );
    let unset = run(&[
        "--json",
        "config",
        "unset",
        "ui.width",
        "--permanent",
        "--profile",
        "tsd",
        "--terminal",
        "cli",
    ]);
    let unset_payload = parse_json_stdout(&unset.stdout);
    let unset_row = first_json_row(&unset_payload, "named-profile override cleanup");
    assert_eq!(unset_row["changed"], true);
    assert_eq!(unset_row["previous"], 96);
    assert_eq!(unset_row["scope"], "profile:tsd terminal:cli");
    let restored = run(&["--json", "--profile", "tsd", "config", "get", "ui.width"]);
    assert_eq!(
        first_json_row(
            &parse_json_stdout(&restored.stdout),
            "baseline width restored"
        )["value"],
        baseline_width
    );
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
