use assert_cmd::Command;
use predicates::prelude::*;

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
    cmd.env_remove("RUST_LOG").args(["-d", "plugins", "list"]);
    cmd.assert()
        .success()
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

    let mut enable = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    enable
        .env("HOME", &home)
        .args(["plugins", "enable", "uio-ldap"]);
    enable
        .assert()
        .success()
        .stderr(predicate::str::contains("enabled plugin: uio-ldap"));

    let state_path = home.join(".config").join("osp").join("plugins.json");
    assert!(state_path.exists(), "plugin state file should be created");

    let mut disable = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    disable
        .env("HOME", &home)
        .args(["plugins", "disable", "uio-ldap"]);
    disable
        .assert()
        .success()
        .stderr(predicate::str::contains("disabled plugin: uio-ldap"));

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

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.env("HOME", &home)
        .args(["-q", "plugins", "enable", "uio-ldap"]);
    cmd.assert().success().stderr(predicate::str::is_empty());

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
fn unknown_domain_command_shows_plugin_hint_contract() {
    let home = make_temp_dir("osp-cli-no-plugin-home");
    let empty_plugins = make_temp_dir("osp-cli-empty-plugins");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .env("OSP_PLUGIN_PATH", &empty_plugins)
        .env("OSP_BUNDLED_PLUGIN_DIR", &empty_plugins)
        .args(["ldap", "user", "oistes"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("no plugin provides command: ldap"))
        .stderr(predicate::str::contains(
            "Hint: run `osp plugins list` and set --plugin-dir or OSP_PLUGIN_PATH",
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
    cmd.env("HOME", &home)
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

#[test]
#[cfg(unix)]
fn profile_override_is_validated_against_config_contract() {
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
    cmd.env("HOME", &home).env("PATH", "/usr/bin:/bin").args([
        "--profile",
        "prod",
        "plugins",
        "list",
    ]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("unknown profile 'prod'"));

    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn external_plugin_dispatch_contract() {
    let dir = make_temp_dir("osp-cli-plugin-exec");
    let _plugin_path = write_hello_plugin(&dir);
    let home = make_temp_dir("osp-cli-plugin-exec-home");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.env("HOME", &home)
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["hello"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("hello-from-plugin"));

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn plugin_dispatch_propagates_runtime_hints_contract() {
    let dir = make_temp_dir("osp-cli-plugin-runtime-hints");
    let _plugin_path = write_hints_plugin(&dir);
    let home = make_temp_dir("osp-cli-plugin-runtime-home");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.env("TERM", "xterm-256color")
        .env("HOME", &home)
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
        ]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"ui_verbosity\": \"trace\""))
        .stdout(predicate::str::contains("\"debug_level\": \"3\""))
        .stdout(predicate::str::contains("\"format\": \"json\""))
        .stdout(predicate::str::contains("\"color\": \"never\""))
        .stdout(predicate::str::contains("\"unicode\": \"always\""))
        .stdout(predicate::str::contains("\"profile\": \"tsd\""))
        .stdout(predicate::str::contains("\"terminal_kind\": \"cli\""))
        .stdout(predicate::str::contains("\"terminal\": \"xterm-256color\""));

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
    cmd.env("HOME", &home)
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["cfg"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains(
            "\"shared_url\": \"https://common.example\"",
        ))
        .stdout(predicate::str::contains(
            "\"endpoint\": \"plugin-endpoint\"",
        ))
        .stdout(predicate::str::contains("\"api_token\": \"token-123\""))
        .stdout(predicate::str::contains("\"enable_cache\": \"true\""))
        .stdout(predicate::str::contains("\"retries\": \"3\""));

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
    cmd.env("HOME", &home)
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["boom"]);
    cmd.assert().failure().stderr(predicate::str::contains(
        "plugin boom exited with status 7: boom-from-stderr",
    ));

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
    cmd.env("HOME", &home)
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["broken"]);
    cmd.assert().failure().stderr(predicate::str::contains(
        "invalid JSON response from plugin broken",
    ));

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn plugins_config_reports_effective_projected_env_contract() {
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
    cmd.env("HOME", &home)
        .args(["--json", "plugins", "config", "cfg"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains(r#""plugin_id": "cfg""#))
        .stdout(predicate::str::contains(
            r#""env": "OSP_PLUGIN_CFG_ENDPOINT""#,
        ))
        .stdout(predicate::str::contains(r#""value": "plugin-endpoint""#))
        .stdout(predicate::str::contains(
            r#""config_key": "extensions.plugins.cfg.env.endpoint""#,
        ))
        .stdout(predicate::str::contains(r#""scope": "plugin""#))
        .stdout(predicate::str::contains(
            r#""env": "OSP_PLUGIN_CFG_SHARED_URL""#,
        ))
        .stdout(predicate::str::contains(
            r#""config_key": "extensions.plugins.env.shared.url""#,
        ))
        .stdout(predicate::str::contains(r#""scope": "shared""#));

    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn multi_command_plugin_receives_selected_command_contract() {
    let dir = make_temp_dir("osp-cli-plugin-multi-command");
    let _plugin_path = write_multi_command_plugin(&dir);
    let home = make_temp_dir("osp-cli-plugin-multi-command-home");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.env("HOME", &home)
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["alpha", "run"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"selected_command\": \"alpha\""))
        .stdout(predicate::str::contains("\"arg0\": \"alpha\""))
        .stdout(predicate::str::contains("\"arg1\": \"run\""));

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn external_plugin_help_is_passed_through_contract() {
    let dir = make_temp_dir("osp-cli-plugin-help");
    let _plugin_path = write_hello_plugin(&dir);
    let home = make_temp_dir("osp-cli-plugin-help-home");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.env("HOME", &home)
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["hello", "--help"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("hello plugin help text"));

    let mut cmd_help_subcommand = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd_help_subcommand
        .env("HOME", &home)
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["hello", "help"]);
    cmd_help_subcommand
        .assert()
        .success()
        .stdout(predicate::str::contains("hello plugin help text"));

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn ignores_non_plugin_extension_files_contract() {
    use std::os::unix::fs::PermissionsExt;

    let dir = make_temp_dir("osp-cli-ignore-script");
    let script_path = dir.join("osp-ignore.sh");
    std::fs::write(&script_path, "#!/usr/bin/env bash\necho should-not-run\n")
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
    cmd.env("HOME", &home)
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
        .env("HOME", &home)
        .env("OSP_BUNDLED_PLUGIN_DIR", &dir)
        .args(["hello"]);
    first.assert().failure().stderr(predicate::str::contains(
        "no plugin provides command: hello",
    ));

    let mut enable = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    enable
        .env("HOME", &home)
        .env("OSP_BUNDLED_PLUGIN_DIR", &dir)
        .args(["plugins", "enable", "hello"]);
    enable
        .assert()
        .success()
        .stderr(predicate::str::contains("enabled plugin: hello"));

    let mut second = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    second
        .env("HOME", &home)
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
fn enabling_one_plugin_does_not_disable_other_default_enabled_plugins_contract() {
    let dir = make_temp_dir("osp-cli-plugin-enable-overrides");
    let _alpha = write_named_plugin(&dir, "alpha", "alpha");
    let _beta = write_named_plugin(&dir, "beta", "beta");
    let home = make_temp_dir("osp-cli-plugin-enable-overrides-home");

    let mut enable = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    enable
        .env("HOME", &home)
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["plugins", "enable", "alpha"]);
    enable
        .assert()
        .success()
        .stderr(predicate::str::contains("enabled plugin: alpha"));

    let mut beta = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    beta.env("HOME", &home)
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["beta"]);
    beta.assert()
        .success()
        .stdout(predicate::str::contains("beta-from-plugin"));

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
    cmd.env("HOME", &home)
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
    cmd.env("HOME", &home)
        .env("OSP_BUNDLED_PLUGIN_DIR", &dir)
        .args(["--json", "plugins", "list"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains(r#""plugin_id": "hello""#))
        .stdout(predicate::str::contains(
            "manifest id mismatch: expected hello, got wrong",
        ));

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
    cmd.env("HOME", &home)
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--json", "plugins", "list"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains(r#""plugin_id": "future""#))
        .stdout(predicate::str::contains(r#""healthy": false"#))
        .stdout(predicate::str::contains("requires osp >="));

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn conflicting_providers_are_visible_in_plugin_commands_contract() {
    let dir = make_temp_dir("osp-cli-plugin-conflicts");
    let _alpha = write_provider_plugin(&dir, "alpha-provider", "hello", "alpha");
    let _beta = write_provider_plugin(&dir, "beta-provider", "hello", "beta");
    let home = make_temp_dir("osp-cli-plugin-conflicts-home");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.env("HOME", &home)
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--json", "plugins", "commands"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains(r#""conflicted": true"#))
        .stdout(predicate::str::contains("alpha-provider (env)"))
        .stdout(predicate::str::contains("beta-provider (env)"));

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn plugin_source_precedence_is_stable_contract() {
    let explicit_dir = make_temp_dir("osp-cli-plugin-precedence-explicit");
    let env_dir = make_temp_dir("osp-cli-plugin-precedence-env");
    let bundled_dir = make_temp_dir("osp-cli-plugin-precedence-bundled");
    let path_dir = make_temp_dir("osp-cli-plugin-precedence-path");
    let home = make_temp_dir("osp-cli-plugin-precedence-home");
    let user_dir = home.join(".config").join("osp").join("plugins");
    std::fs::create_dir_all(&user_dir).expect("user plugin dir should be created");

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
        .env("HOME", &home)
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
    commands
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""name": "ranked""#))
        .stdout(predicate::str::contains(
            r#""provider": "explicit-provider""#,
        ))
        .stdout(predicate::str::contains(r#""source": "explicit""#))
        .stdout(predicate::str::contains(r#""conflicted": true"#))
        .stdout(predicate::str::contains("explicit-provider (explicit)"))
        .stdout(predicate::str::contains("env-provider (env)"))
        .stdout(predicate::str::contains("bundled-provider (bundled)"))
        .stdout(predicate::str::contains("user-provider (user)"))
        .stdout(predicate::str::contains("path-provider (path)"));

    let mut explicit = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    explicit
        .env("HOME", &home)
        .env("OSP_PLUGIN_PATH", &env_dir)
        .env("OSP_BUNDLED_PLUGIN_DIR", &bundled_dir)
        .env("PATH", &path_env)
        .args([
            "--plugin-dir",
            explicit_dir
                .to_str()
                .expect("explicit path should be utf-8"),
            "ranked",
        ]);
    explicit
        .assert()
        .success()
        .stdout(predicate::str::contains("explicit-from-plugin"));

    let mut env = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    env.env("HOME", &home)
        .env("OSP_PLUGIN_PATH", &env_dir)
        .env("OSP_BUNDLED_PLUGIN_DIR", &bundled_dir)
        .env("PATH", &path_env)
        .args(["ranked"]);
    env.assert()
        .success()
        .stdout(predicate::str::contains("env-from-plugin"));

    let mut bundled = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    bundled
        .env("HOME", &home)
        .env_remove("OSP_PLUGIN_PATH")
        .env("OSP_BUNDLED_PLUGIN_DIR", &bundled_dir)
        .env("PATH", &path_env)
        .args(["ranked"]);
    bundled
        .assert()
        .success()
        .stdout(predicate::str::contains("bundled-from-plugin"));

    let mut user = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    user.env("HOME", &home)
        .env_remove("OSP_PLUGIN_PATH")
        .env_remove("OSP_BUNDLED_PLUGIN_DIR")
        .env("PATH", &path_env)
        .args(["ranked"]);
    user.assert()
        .success()
        .stdout(predicate::str::contains("user-from-plugin"));

    std::fs::remove_file(user_dir.join("osp-user-provider"))
        .expect("user plugin should be removable");

    let mut path = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    path.env("HOME", &home)
        .env_remove("OSP_PLUGIN_PATH")
        .env_remove("OSP_BUNDLED_PLUGIN_DIR")
        .env("PATH", &path_env)
        .args(["ranked"]);
    path.assert()
        .success()
        .stdout(predicate::str::contains("path-from-plugin"));

    let _ = std::fs::remove_dir_all(&explicit_dir);
    let _ = std::fs::remove_dir_all(&env_dir);
    let _ = std::fs::remove_dir_all(&bundled_dir);
    let _ = std::fs::remove_dir_all(&path_dir);
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
        .env("HOME", &home)
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--unicode", "never", "counter"]);
    first
        .assert()
        .success()
        .stdout(predicate::str::contains("| 1"));

    let mut second = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    second
        .env("HOME", &home)
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
        .env("HOME", &home)
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--json", "plugins", "list"]);
    first
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""plugin_version": "0.1.1""#));
    assert_eq!(
        std::fs::read_to_string(&describe_count_path)
            .expect("describe count should be written")
            .trim(),
        "1"
    );

    let mut second = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    second
        .env("HOME", &home)
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--json", "plugins", "list"]);
    second
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""plugin_version": "0.1.1""#));
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
        .env("HOME", &home)
        .env("OSP_PLUGIN_PATH", &dir)
        .args(["--json", "plugins", "list"]);
    third
        .assert()
        .success()
        .stdout(predicate::str::contains(r#""plugin_version": "0.1.2""#));
    assert_eq!(
        std::fs::read_to_string(&describe_count_path)
            .expect("describe count should reflect invalidation")
            .trim(),
        "2"
    );

    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
fn write_hello_plugin(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let plugin_path = dir.join("osp-hello");
    let plugin_script = r#"#!/usr/bin/env bash
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{"protocol_version":1,"plugin_id":"hello","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{"name":"hello","about":"hello plugin","args":[],"flags":{},"subcommands":[]}]}
JSON
  exit 0
fi

if [ "$1" = "--help" ] || [ "$1" = "-h" ] || [ "$1" = "help" ] || \
   [ "$2" = "--help" ] || [ "$2" = "-h" ] || [ "$2" = "help" ]; then
  echo "hello plugin help text"
  exit 0
fi

cat <<'JSON'
{"protocol_version":1,"ok":true,"data":{"message":"hello-from-plugin"},"error":null,"meta":{"format_hint":"table","columns":["message"]}}
JSON
"#;

    std::fs::write(&plugin_path, plugin_script).expect("plugin script should be written");
    let mut perms = std::fs::metadata(&plugin_path)
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&plugin_path, perms).expect("script should be executable");
    plugin_path
}

#[cfg(unix)]
fn write_named_plugin(dir: &std::path::Path, name: &str, message: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let plugin_path = dir.join(format!("osp-{name}"));
    let plugin_script = format!(
        r#"#!/usr/bin/env bash
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{name}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{name}","about":"{name} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"{message}-from-plugin"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
        name = name,
        message = message
    );

    std::fs::write(&plugin_path, plugin_script).expect("plugin script should be written");
    let mut perms = std::fs::metadata(&plugin_path)
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&plugin_path, perms).expect("script should be executable");
    plugin_path
}

#[cfg(unix)]
fn write_provider_plugin(
    dir: &std::path::Path,
    plugin_id: &str,
    command_name: &str,
    message: &str,
) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let plugin_path = dir.join(format!("osp-{plugin_id}"));
    let plugin_script = format!(
        r#"#!/usr/bin/env bash
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{plugin_id}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{command_name}","about":"{plugin_id} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"{message}-from-plugin"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
        plugin_id = plugin_id,
        command_name = command_name,
        message = message
    );

    std::fs::write(&plugin_path, plugin_script).expect("plugin script should be written");
    let mut perms = std::fs::metadata(&plugin_path)
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&plugin_path, perms).expect("script should be executable");
    plugin_path
}

#[cfg(unix)]
fn write_plugin_with_min_version(
    dir: &std::path::Path,
    plugin_id: &str,
    min_osp_version: &str,
) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let plugin_path = dir.join(format!("osp-{plugin_id}"));
    let plugin_script = format!(
        r#"#!/usr/bin/env bash
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{plugin_id}","plugin_version":"0.1.0","min_osp_version":"{min_osp_version}","commands":[{{"name":"{plugin_id}","about":"{plugin_id} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"{plugin_id}-from-plugin"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
        plugin_id = plugin_id,
        min_osp_version = min_osp_version
    );

    std::fs::write(&plugin_path, plugin_script).expect("plugin script should be written");
    let mut perms = std::fs::metadata(&plugin_path)
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&plugin_path, perms).expect("script should be executable");
    plugin_path
}

#[cfg(unix)]
fn write_hints_plugin(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let plugin_path = dir.join("osp-hints");
    let plugin_script = r#"#!/usr/bin/env bash
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{"protocol_version":1,"plugin_id":"hints","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{"name":"hints","about":"runtime hints plugin","args":[],"flags":{},"subcommands":[]}]}
JSON
  exit 0
fi

cat <<JSON
{"protocol_version":1,"ok":true,"data":{
  "ui_verbosity":"${OSP_UI_VERBOSITY:-}",
  "debug_level":"${OSP_DEBUG_LEVEL:-}",
  "format":"${OSP_FORMAT:-}",
  "color":"${OSP_COLOR:-}",
  "unicode":"${OSP_UNICODE:-}",
  "profile":"${OSP_PROFILE:-}",
  "terminal_kind":"${OSP_TERMINAL_KIND:-}",
  "terminal":"${OSP_TERMINAL:-}"
},"error":null,"meta":{"format_hint":"table","columns":["ui_verbosity","debug_level","format","color","unicode","profile","terminal_kind","terminal"]}}
JSON
"#;

    std::fs::write(&plugin_path, plugin_script).expect("plugin script should be written");
    let mut perms = std::fs::metadata(&plugin_path)
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&plugin_path, perms).expect("script should be executable");
    plugin_path
}

#[cfg(unix)]
fn write_multi_command_plugin(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let plugin_path = dir.join("osp-multi");
    let plugin_script = r#"#!/usr/bin/env bash
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{"protocol_version":1,"plugin_id":"multi","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{"name":"alpha","about":"alpha command","args":[],"flags":{},"subcommands":[]},{"name":"beta","about":"beta command","args":[],"flags":{},"subcommands":[]}]}
JSON
  exit 0
fi

cat <<JSON
{"protocol_version":1,"ok":true,"data":{"selected_command":"${OSP_COMMAND:-}","arg0":"${1:-}","arg1":"${2:-}"},"error":null,"meta":{"format_hint":"json"}}
JSON
"#;

    std::fs::write(&plugin_path, plugin_script).expect("plugin script should be written");
    let mut perms = std::fs::metadata(&plugin_path)
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&plugin_path, perms).expect("script should be executable");
    plugin_path
}

#[cfg(unix)]
fn write_config_env_plugin(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let plugin_path = dir.join("osp-cfg");
    let plugin_script = r#"#!/usr/bin/env bash
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{"protocol_version":1,"plugin_id":"cfg","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{"name":"cfg","about":"config env plugin","args":[],"flags":{},"subcommands":[]}]}
JSON
  exit 0
fi

cat <<JSON
{"protocol_version":1,"ok":true,"data":{
  "shared_url":"${OSP_PLUGIN_CFG_SHARED_URL:-}",
  "endpoint":"${OSP_PLUGIN_CFG_ENDPOINT:-}",
  "api_token":"${OSP_PLUGIN_CFG_API_TOKEN:-}",
  "enable_cache":"${OSP_PLUGIN_CFG_ENABLE_CACHE:-}",
  "retries":"${OSP_PLUGIN_CFG_RETRIES:-}"
},"error":null,"meta":{"format_hint":"json"}}
JSON
"#;

    std::fs::write(&plugin_path, plugin_script).expect("plugin script should be written");
    let mut perms = std::fs::metadata(&plugin_path)
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&plugin_path, perms).expect("script should be executable");
    plugin_path
}

#[cfg(unix)]
fn write_non_zero_plugin(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let plugin_path = dir.join("osp-boom");
    let plugin_script = r#"#!/usr/bin/env bash
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{"protocol_version":1,"plugin_id":"boom","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{"name":"boom","about":"boom plugin","args":[],"flags":{},"subcommands":[]}]}
JSON
  exit 0
fi

echo "boom-from-stderr" >&2
exit 7
"#;

    std::fs::write(&plugin_path, plugin_script).expect("plugin script should be written");
    let mut perms = std::fs::metadata(&plugin_path)
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&plugin_path, perms).expect("script should be executable");
    plugin_path
}

#[cfg(unix)]
fn write_invalid_json_plugin(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let plugin_path = dir.join("osp-broken");
    let plugin_script = r#"#!/usr/bin/env bash
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{"protocol_version":1,"plugin_id":"broken","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{"name":"broken","about":"broken plugin","args":[],"flags":{},"subcommands":[]}]}
JSON
  exit 0
fi

echo "{ definitely-not-json"
"#;

    std::fs::write(&plugin_path, plugin_script).expect("plugin script should be written");
    let mut perms = std::fs::metadata(&plugin_path)
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&plugin_path, perms).expect("script should be executable");
    plugin_path
}

#[cfg(unix)]
fn write_describe_mismatch_plugin(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let plugin_path = dir.join("osp-hello");
    let plugin_script = r#"#!/usr/bin/env bash
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{"protocol_version":1,"plugin_id":"wrong","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{"name":"hello","about":"mismatch plugin","args":[],"flags":{},"subcommands":[]}]}
JSON
  exit 0
fi

cat <<'JSON'
{"protocol_version":1,"ok":true,"data":{"message":"ignored"},"error":null,"meta":{"format_hint":"table","columns":["message"]}}
JSON
"#;

    std::fs::write(&plugin_path, plugin_script).expect("plugin script should be written");
    let mut perms = std::fs::metadata(&plugin_path)
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&plugin_path, perms).expect("script should be executable");
    plugin_path
}

#[cfg(unix)]
fn write_counter_plugin(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let plugin_path = dir.join("osp-counter");
    let counter_path = dir.join("counter.txt");
    let plugin_script = format!(
        r#"#!/usr/bin/env bash
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"counter","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"counter","about":"counter plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

count=0
if [ -f "{counter_path}" ]; then
  count=$(cat "{counter_path}")
fi
count=$((count+1))
echo "$count" > "{counter_path}"

cat <<JSON
{{"protocol_version":1,"ok":true,"data":{{"count":$count}},"error":null,"meta":{{"format_hint":"table","columns":["count"]}}}}
JSON
"#,
        counter_path = counter_path.display()
    );

    std::fs::write(&plugin_path, plugin_script).expect("plugin script should be written");
    let mut perms = std::fs::metadata(&plugin_path)
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&plugin_path, perms).expect("script should be executable");
    plugin_path
}

#[cfg(unix)]
fn write_describe_counter_plugin(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let plugin_path = dir.join("osp-describe-counter");
    let describe_count_path = dir.join("describe-count.txt");
    let plugin_script = format!(
        r#"#!/usr/bin/env bash
if [ "$1" = "--describe" ]; then
  count=0
  if [ -f "{describe_count_path}" ]; then
    count=$(cat "{describe_count_path}")
  fi
  count=$((count+1))
  echo "$count" > "{describe_count_path}"
  cat <<JSON
{{"protocol_version":1,"plugin_id":"describe-counter","plugin_version":"0.1.$count","min_osp_version":"0.1.0","commands":[{{"name":"describe-counter","about":"describe counter plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"ok"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
        describe_count_path = describe_count_path.display()
    );

    std::fs::write(&plugin_path, plugin_script).expect("plugin script should be written");
    let mut perms = std::fs::metadata(&plugin_path)
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&plugin_path, perms).expect("script should be executable");
    plugin_path
}

#[cfg(unix)]
fn write_manifest(dir: &std::path::Path, manifest: &str) {
    std::fs::write(dir.join("manifest.toml"), manifest).expect("manifest.toml should be written");
}

#[cfg(unix)]
fn write_config(home: &std::path::Path, config: &str) {
    let config_dir = home.join(".config").join("osp");
    std::fs::create_dir_all(&config_dir).expect("config dir should be created");
    std::fs::write(config_dir.join("config.toml"), config).expect("config should be written");
}

#[cfg(unix)]
fn make_temp_dir(prefix: &str) -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be valid")
        .as_nanos();
    dir.push(format!("{prefix}-{nonce}"));
    std::fs::create_dir_all(&dir).expect("temp dir should be created");
    dir
}
