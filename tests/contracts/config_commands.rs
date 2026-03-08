use crate::{assert_snapshot_text, assert_snapshot_text_with};
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
    cmd.envs(crate::test_env::isolated_env(&home))
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
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "config", "get", "ui.mode", "--sources"]);
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
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "config", "get", "profile.default", "--sources"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"key\": \"profile.default\""))
        .stdout(predicate::str::contains("\"value\": \"uio\""))
        .stdout(predicate::str::contains("\"source\": \"file\""));

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
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "config", "get", "alias.me", "--sources"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"key\": \"alias.me\""))
        .stdout(predicate::str::contains(
            "\"value\": \"ldap user ${user.name}\"",
        ))
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
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "config", "unset", "ui.mode"]);
    unset
        .assert()
        .success()
        .stdout(predicate::str::contains("\"changed\": true"))
        .stdout(predicate::str::contains("\"previous\": \"plain\""));

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
    cmd.envs(crate::test_env::isolated_env(&home))
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
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "tsd", "config", "get", "ui.format"]);
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
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "tsd", "config", "explain", "ui.format"]);
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
    cmd.envs(crate::test_env::isolated_env(&home))
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

    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn config_explain_reports_presentation_seeded_effective_values_contract() {
    let home = make_temp_dir("osp-cli-config-explain-presentation");
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
            "--json",
            "--presentation",
            "austere",
            "config",
            "explain",
            "ui.help.layout",
        ]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"value\": \"full\""))
        .stdout(predicate::str::contains("\"presentation\""))
        .stdout(predicate::str::contains("\"preset\": \"austere\""))
        .stdout(predicate::str::contains("\"preset_source\": \"session\""))
        .stdout(predicate::str::contains("\"effective_value\": \"minimal\""));

    let _ = std::fs::remove_dir_all(&home);
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
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "config", "explain", "profile.default"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"key\": \"profile.default\""))
        .stdout(predicate::str::contains("\"phase\": \"bootstrap\""))
        .stdout(predicate::str::contains(
            "\"active_profile_source\": \"profile.default\"",
        ))
        .stdout(predicate::str::contains(
            "\"bootstrap_scope_policy\": \"global and terminal-only; profile scopes are ignored during bootstrap\"",
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
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "tsd", "config", "explain", "profile.default"]);
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
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "config", "explain", "profile.active"]);
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
    cmd.envs(crate::test_env::isolated_env(&home))
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
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "--no-config-file", "config", "get", "ui.mode"]);
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
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "config", "explain", "ui.prompt"]);
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
        .envs(crate::test_env::isolated_env(&home))
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
    clear
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args([
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
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "config", "explain", "extensions.demo.potato"]);
    redacted
        .assert()
        .success()
        .stdout(predicate::str::contains("[REDACTED]"));

    let mut clear = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    clear
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args([
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
        .envs(crate::test_env::isolated_env(&home))
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

    let _ = std::fs::remove_dir_all(&home);
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
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "config", "get", "ui.format", "--sources"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"source\": \"file\""))
        .stdout(predicate::str::contains("\"value\": \"table\""));

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

    let _ = std::fs::remove_dir_all(&home);
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

    let _ = std::fs::remove_dir_all(&home);
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
    cmd.envs(crate::test_env::isolated_env(&home))
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
        ]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"changed\": true"))
        .stdout(predicate::str::contains("\"scope\": \"terminal:repl\""));

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
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args([
            "--json",
            "config",
            "unset",
            "--global",
            "--terminal",
            "repl",
            "profile.default",
        ]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"changed\": true"))
        .stdout(predicate::str::contains("\"scope\": \"terminal:repl\""));

    let payload = std::fs::read_to_string(home.join(".config").join("osp").join("config.toml"))
        .expect("config should be readable");
    assert!(!payload.contains("profile.default = \"tsd\""));

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

#[cfg(unix)]
fn parse_json_stdout(stdout: &[u8]) -> serde_json::Value {
    serde_json::from_slice(stdout).unwrap_or_else(|err| {
        panic!(
            "stdout should be valid json: {err}\n{}",
            String::from_utf8_lossy(stdout)
        )
    })
}
