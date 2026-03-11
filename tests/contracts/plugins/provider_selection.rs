#[cfg(unix)]
#[test]
fn conflicting_providers_are_visible_in_plugin_commands_contract() {
    let dir = make_temp_dir("osp-cli-plugin-conflicts");
    let _alpha = write_provider_plugin(&dir, "alpha-provider", "hello", "alpha");
    let _beta = write_provider_plugin(&dir, "beta-provider", "hello", "beta");
    let home = make_temp_dir("osp-cli-plugin-conflicts-home");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--json", "plugins", "commands"])
        .assert()
        .success()
        .get_output()
        .clone();

    let payload = parse_json_stdout(&output.stdout);
    let row = first_json_row(&payload, "plugins commands conflict view");
    assert_eq!(row["name"], "hello");
    assert_eq!(row["conflicted"], true);
    assert_eq!(row["requires_selection"], true);
    assert_eq!(row["provider"], Value::Null);
    assert_eq!(row["source"], Value::Null);
    let providers = row["providers"]
        .as_array()
        .expect("providers should render as an array");
    assert!(
        providers.contains(&Value::String("alpha-provider (env)".to_string()))
    );
    assert!(
        providers.contains(&Value::String("beta-provider (env)".to_string()))
    );
    assert!(
        output.stderr.is_empty(),
        "stderr should stay empty: {}",
        String::from_utf8_lossy(&output.stderr)
    );

}

#[cfg(unix)]
#[test]
fn provider_selection_can_be_persisted_or_overridden_per_invocation_contract() {
    let dir = make_temp_dir("osp-cli-plugin-select-provider");
    let _alpha = write_provider_plugin(&dir, "alpha-provider", "shared", "alpha");
    let _beta = write_provider_plugin(&dir, "beta-provider", "shared", "beta");
    let home = make_temp_dir("osp-cli-plugin-select-provider-home");

    let mut before = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    before
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["shared"]);
    let before_output = before.assert().failure().get_output().clone();
    assert!(
        before_output.stdout.is_empty(),
        "stdout should stay empty: {}",
        String::from_utf8_lossy(&before_output.stdout)
    );
    assert_snapshot_text!(
        "provider_selection_before_selection_stderr",
        String::from_utf8(before_output.stderr).expect("stderr should be utf-8"),
    );

    let mut oneshot = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    oneshot
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--plugin-provider", "beta-provider", "shared"]);
    oneshot
        .assert()
        .success()
        .stdout(predicate::str::contains("beta-from-plugin"));

    let mut select = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    select
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["plugins", "select-provider", "shared", "beta-provider"]);
    select.assert().success().stderr(predicate::str::contains(
        "selected provider for command shared: beta-provider",
    ));

    let mut commands = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    commands
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--json", "plugins", "commands"]);
    let commands_output = commands.assert().success().get_output().clone();
    let commands_stdout =
        String::from_utf8(commands_output.stdout).expect("stdout should be utf-8");
    let commands_payload = parse_json_stdout(commands_stdout.as_bytes());
    let row = first_json_row(&commands_payload, "plugins commands selected provider view");
    assert_eq!(row["name"], "shared");
    assert_eq!(row["provider"], "beta-provider");
    assert_eq!(row["conflicted"], true);
    assert_eq!(row["requires_selection"], false);
    assert_eq!(row["selected_explicitly"], true);
    assert_eq!(row["source"], "env");
    assert_snapshot_text!("provider_selection_selected_commands_json", commands_stdout);

    let mut after_select = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    after_select
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["shared"]);
    after_select
        .assert()
        .success()
        .stdout(predicate::str::contains("beta-from-plugin"))
        .stderr(predicate::str::contains("multiple plugins").not())
        .stderr(predicate::str::contains("--plugin-provider").not());

    let mut clear = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    clear
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["plugins", "clear-provider", "shared"]);
    clear.assert().success().stderr(predicate::str::contains(
        "cleared provider selection for command shared",
    ));

    let mut after_clear = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    after_clear
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["shared"]);
    let after_clear_output = after_clear.assert().failure().get_output().clone();
    assert!(
        after_clear_output.stdout.is_empty(),
        "stdout should stay empty: {}",
        String::from_utf8_lossy(&after_clear_output.stdout)
    );
    assert_snapshot_text!(
        "provider_selection_after_clear_stderr",
        String::from_utf8(after_clear_output.stderr).expect("stderr should be utf-8"),
    );

}

#[cfg(unix)]
#[test]
fn plugin_provider_override_works_across_discovery_sources_contract() {
    let explicit_dir = make_temp_dir("osp-cli-plugin-precedence-explicit");
    let env_dir = make_temp_dir("osp-cli-plugin-precedence-env");
    let bundled_dir = make_temp_dir("osp-cli-plugin-precedence-bundled");
    let path_dir = make_temp_dir("osp-cli-plugin-precedence-path");
    let home = make_temp_dir("osp-cli-plugin-precedence-home");
    let user_dir = home.join(".config").join("osp").join("plugins");
    std::fs::create_dir_all(&user_dir).expect("user plugin dir should be created");
    write_config(
        &home,
        r#"
[default]
extensions.plugins.discovery.path = true
"#,
    );

    let _explicit = write_provider_plugin(&explicit_dir, "explicit-provider", "ranked", "explicit");
    let _env = write_provider_plugin(&env_dir, "env-provider", "ranked", "env");
    let _bundled = write_provider_plugin(&bundled_dir, "bundled-provider", "ranked", "bundled");
    let _user = write_provider_plugin(&user_dir, "user-provider", "ranked", "user");
    let _path = write_provider_plugin(&path_dir, "path-provider", "ranked", "path");
    write_manifest(
        &bundled_dir,
        r#"
protocol_version = 1

[[plugin]]
id = "bundled-provider"
exe = "osp-bundled-provider"
version = "0.1.0"
enabled_by_default = true
commands = ["ranked"]
"#,
    );
    let path_env = format!("{}:/usr/bin:/bin", path_dir.display());

    let mut commands = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    commands
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &env_dir)
        .env("OSP_BUNDLED_PLUGIN_DIR", &bundled_dir)
        .env("PATH", &path_env)
        .args([
            "--plugin-dir",
            explicit_dir
                .to_str()
                .expect("explicit path should be utf-8"),
            "--json",
            "plugins",
            "commands",
        ]);
    let commands_output = commands.assert().success().get_output().clone();
    let commands_stdout =
        String::from_utf8(commands_output.stdout).expect("stdout should be utf-8");
    let commands_payload = parse_json_stdout(commands_stdout.as_bytes());
    let row = first_json_row(&commands_payload, "plugins commands conflicted precedence view");
    assert_eq!(row["name"], "ranked");
    assert_eq!(row["conflicted"], true);
    assert_eq!(row["requires_selection"], true);
    assert_eq!(row["provider"], Value::Null);
    assert_eq!(row["source"], Value::Null);
    assert_snapshot_text!(
        "provider_override_conflicted_commands_json",
        commands_stdout,
    );

    let mut explicit = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    explicit
        .envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &env_dir)
        .env("OSP_BUNDLED_PLUGIN_DIR", &bundled_dir)
        .env("PATH", &path_env)
        .args([
            "--plugin-dir",
            explicit_dir
                .to_str()
                .expect("explicit path should be utf-8"),
            "--plugin-provider",
            "explicit-provider",
            "ranked",
        ]);
    explicit
        .assert()
        .success()
        .stdout(predicate::str::contains("explicit-from-plugin"));

    let mut env = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    env.envs(crate::test_env::isolated_env(&home))
        .env("OSP_PLUGIN_PATH", &env_dir)
        .env("OSP_BUNDLED_PLUGIN_DIR", &bundled_dir)
        .env("PATH", &path_env)
        .args(["--plugin-provider", "env-provider", "ranked"]);
    env.assert()
        .success()
        .stdout(predicate::str::contains("env-from-plugin"));

    let mut bundled = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    bundled
        .envs(crate::test_env::isolated_env(&home))
        .env_remove("OSP_PLUGIN_PATH")
        .env("OSP_BUNDLED_PLUGIN_DIR", &bundled_dir)
        .env("PATH", &path_env)
        .args(["--plugin-provider", "bundled-provider", "ranked"]);
    bundled
        .assert()
        .success()
        .stdout(predicate::str::contains("bundled-from-plugin"));

    let mut user = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    user.envs(crate::test_env::isolated_env(&home))
        .env_remove("OSP_PLUGIN_PATH")
        .env_remove("OSP_BUNDLED_PLUGIN_DIR")
        .env("PATH", &path_env)
        .args(["--plugin-provider", "user-provider", "ranked"]);
    user.assert()
        .success()
        .stdout(predicate::str::contains("user-from-plugin"));

    std::fs::remove_file(user_dir.join("osp-user-provider"))
        .expect("user plugin should be removable");

    let mut path = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    path.envs(crate::test_env::isolated_env(&home))
        .env_remove("OSP_PLUGIN_PATH")
        .env_remove("OSP_BUNDLED_PLUGIN_DIR")
        .env("PATH", &path_env)
        .args(["--plugin-provider", "path-provider", "ranked"]);
    path.assert()
        .success()
        .stdout(predicate::str::contains("path-from-plugin"));

}
