use assert_cmd::Command;
use predicates::prelude::*;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

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
    cmd.env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "config", "show"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"key\": \"ui.format\""))
        .stdout(predicate::str::contains("\"key\": \"profile.default\"").not());

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
    cmd.env("HOME", &home).env("PATH", "/usr/bin:/bin").args([
        "--json",
        "config",
        "get",
        "ui.mode",
        "--sources",
    ]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"source\": \"file\""))
        .stdout(predicate::str::contains("\"value\": \"plain\""));

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
    cmd.env("HOME", &home).env("PATH", "/usr/bin:/bin").args([
        "--json",
        "config",
        "get",
        "profile.default",
        "--sources",
    ]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"key\": \"profile.default\""))
        .stdout(predicate::str::contains("\"value\": \"uio\""))
        .stdout(predicate::str::contains("\"source\": \"file\""));

    let _ = std::fs::remove_dir_all(&home);
}

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
    unset
        .env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "config", "unset", "ui.mode"]);
    unset
        .assert()
        .success()
        .stdout(predicate::str::contains("\"changed\": true"))
        .stdout(predicate::str::contains("\"previous\": \"plain\""));

    let mut get = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    get.env("HOME", &home)
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
    cmd.env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "config", "doctor"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"status\": \"ok\""))
        .stdout(predicate::str::contains("\"active_profile\": \"uio\""));

    let _ = std::fs::remove_dir_all(&home);
}

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
    cmd.env("HOME", &home).env("PATH", "/usr/bin:/bin").args([
        "--json",
        "tsd",
        "config",
        "get",
        "ui.format",
    ]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"value\": \"json\""));

    let _ = std::fs::remove_dir_all(&home);
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
    cmd.env("HOME", &home).env("PATH", "/usr/bin:/bin").args([
        "--json",
        "tsd",
        "config",
        "explain",
        "ui.format",
    ]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"active_profile\": \"tsd\""))
        .stdout(predicate::str::contains("\"value\": \"json\""));

    let _ = std::fs::remove_dir_all(&home);
}

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
    cmd.env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .env("OSP__UI__MODE", "auto")
        .args(["--json", "config", "explain", "ui.mode"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"source\": \"env\""))
        .stdout(predicate::str::contains("\"candidates\""))
        .stdout(predicate::str::contains("\"winner\": true"));

    let _ = std::fs::remove_dir_all(&home);
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
    cmd.env("HOME", &home).env("PATH", "/usr/bin:/bin").args([
        "--json",
        "config",
        "explain",
        "profile.default",
    ]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"key\": \"profile.default\""))
        .stdout(predicate::str::contains("\"phase\": \"bootstrap\""))
        .stdout(predicate::str::contains(
            "\"active_profile_source\": \"profile.default\"",
        ))
        .stdout(predicate::str::contains("\"value\": \"uio\""));

    let _ = std::fs::remove_dir_all(&home);
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
    cmd.env("HOME", &home).env("PATH", "/usr/bin:/bin").args([
        "--json",
        "tsd",
        "config",
        "explain",
        "profile.default",
    ]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"phase\": \"bootstrap\""))
        .stdout(predicate::str::contains("\"active_profile\": \"tsd\""))
        .stdout(predicate::str::contains(
            "\"active_profile_source\": \"override\"",
        ))
        .stdout(predicate::str::contains("\"value\": \"uio\""));

    let _ = std::fs::remove_dir_all(&home);
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
    cmd.env("HOME", &home).env("PATH", "/usr/bin:/bin").args([
        "--json",
        "config",
        "explain",
        "profile.active",
    ]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"key\": \"profile.active\""))
        .stdout(predicate::str::contains("\"phase\": \"runtime\""))
        .stdout(predicate::str::contains("\"value\": \"uio\""));

    let _ = std::fs::remove_dir_all(&home);
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
    cmd.env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .env("OSP__UI__MODE", "auto")
        .args(["--json", "--no-env", "config", "explain", "ui.mode"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"value\": \"plain\""))
        .stdout(predicate::str::contains("\"source\": \"file\""))
        .stdout(predicate::str::contains("OSP__UI__MODE").not());

    let _ = std::fs::remove_dir_all(&home);
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
    cmd.env("HOME", &home).env("PATH", "/usr/bin:/bin").args([
        "--json",
        "--no-config-file",
        "config",
        "get",
        "ui.mode",
    ]);
    cmd.assert().failure();

    let _ = std::fs::remove_dir_all(&home);
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
    cmd.env("HOME", &home).env("PATH", "/usr/bin:/bin").args([
        "--json",
        "config",
        "explain",
        "ui.prompt",
    ]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"interpolation\""))
        .stdout(predicate::str::contains(
            "\"placeholder\": \"extensions.uio.ldap.url\"",
        ))
        .stdout(predicate::str::contains(
            "\"template\": \"${profile.active}:${extensions.uio.ldap.url}:${base.dir}\"",
        ));

    let _ = std::fs::remove_dir_all(&home);
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
    redacted
        .env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .args([
            "--json",
            "config",
            "explain",
            "extensions.uio.ldap.bind_password",
        ]);
    redacted
        .assert()
        .success()
        .stdout(predicate::str::contains("[REDACTED]"))
        .stdout(predicate::str::contains("\"value_type\": \"string\""));

    let mut clear = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    clear.env("HOME", &home).env("PATH", "/usr/bin:/bin").args([
        "--json",
        "config",
        "explain",
        "extensions.uio.ldap.bind_password",
        "--show-secrets",
    ]);
    clear
        .assert()
        .success()
        .stdout(predicate::str::contains("\"value\": \"file-secret\""));

    let _ = std::fs::remove_dir_all(&home);
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
    redacted
        .env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "config", "explain", "extensions.demo.potato"]);
    redacted
        .assert()
        .success()
        .stdout(predicate::str::contains("[REDACTED]"));

    let mut clear = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    clear.env("HOME", &home).env("PATH", "/usr/bin:/bin").args([
        "--json",
        "config",
        "explain",
        "extensions.demo.potato",
        "--show-secrets",
    ]);
    clear
        .assert()
        .success()
        .stdout(predicate::str::contains("\"value\": \"sekrit\""));

    let mut get_redacted = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    get_redacted
        .env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "config", "get", "extensions.demo.potato"]);
    get_redacted
        .assert()
        .success()
        .stdout(predicate::str::contains("[REDACTED]"));

    let _ = std::fs::remove_dir_all(&home);
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
        .env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "tsd", "config", "get", "ui.mode"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let mut explicit = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let explicit_out = explicit
        .env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "--profile", "tsd", "config", "get", "ui.mode"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    assert_eq!(positional_out, explicit_out);

    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn launch_json_flag_seeds_session_format_contract() {
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
    cmd.env("HOME", &home).env("PATH", "/usr/bin:/bin").args([
        "--json",
        "config",
        "get",
        "ui.format",
        "--sources",
    ]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"source\": \"session\""))
        .stdout(predicate::str::contains("\"value\": \"json\""));

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
    cmd.env("HOME", &home).env("PATH", "/usr/bin:/bin").args([
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
    cmd.assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("- Errors "))
        .stderr(predicate::str::contains(
            "config key not found: missing.key",
        ))
        .stderr(predicate::str::contains("\x1b[").not())
        .stderr(predicate::str::contains("──").not());

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
    cmd.env("HOME", &home).env("PATH", "/usr/bin:/bin").args([
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
    cmd.env("HOME", &home).env("PATH", "/usr/bin:/bin").args([
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
    cmd.env("HOME", &home).env("PATH", "/usr/bin:/bin").args([
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

#[cfg(unix)]
fn write_config(home: &std::path::Path, config: &str) {
    let config_dir = home.join(".config").join("osp");
    std::fs::create_dir_all(&config_dir).expect("config dir should be created");
    std::fs::write(config_dir.join("config.toml"), config).expect("config should be written");
}

#[cfg(unix)]
fn write_secrets(home: &std::path::Path, secrets: &str) {
    let config_dir = home.join(".config").join("osp");
    std::fs::create_dir_all(&config_dir).expect("config dir should be created");
    let secrets_path = config_dir.join("secrets.toml");
    std::fs::write(&secrets_path, secrets).expect("secrets should be written");
    let mut perms = std::fs::metadata(&secrets_path)
        .expect("secrets metadata")
        .permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(&secrets_path, perms).expect("secrets permissions");
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
