use crate::assert_snapshot_text;
use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;

fn first_row<'a>(payload: &'a Value, context: &str) -> &'a Value {
    payload
        .as_array()
        .unwrap_or_else(|| panic!("{context} should render a JSON array"))
        .first()
        .unwrap_or_else(|| panic!("{context} should render at least one row"))
}

#[cfg(unix)]
#[test]
fn theme_list_contract() {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "theme", "list"])
        .assert()
        .success()
        .get_output()
        .clone();

    assert!(
        output.stderr.is_empty(),
        "stderr should stay empty: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let payload: Value =
        serde_json::from_slice(&output.stdout).expect("theme list stdout should be valid json");
    let rows = payload
        .as_array()
        .expect("theme list should render a JSON array");
    assert!(
        rows.iter().any(|row| {
            row.get("id") == Some(&Value::String("dracula".to_string()))
                && row.get("name") == Some(&Value::String("Dracula".to_string()))
        }),
        "expected dracula theme in payload: {payload}"
    );
    assert!(
        rows.iter().any(|row| {
            row.get("id") == Some(&Value::String("rose-pine-moon".to_string()))
                && row.get("name") == Some(&Value::String("Rose Pine Moon".to_string()))
        }),
        "expected rose-pine-moon theme in payload: {payload}"
    );
}

#[cfg(unix)]
#[test]
fn theme_list_human_rich_snapshot_contract() {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .env("PATH", "/usr/bin:/bin")
        .args([
            "--mode",
            "rich",
            "--color",
            "never",
            "--unicode",
            "never",
            "theme",
            "list",
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
    assert_snapshot_text!(
        "theme_list_human_rich_stdout",
        String::from_utf8(output.stdout).expect("stdout should be utf-8"),
    );
}

#[cfg(unix)]
#[test]
fn theme_show_contract() {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "theme", "show", "dracula"])
        .assert()
        .success()
        .get_output()
        .clone();

    assert!(
        output.stderr.is_empty(),
        "stderr should stay empty: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let payload: Value =
        serde_json::from_slice(&output.stdout).expect("theme show stdout should be valid json");
    let row = first_row(&payload, "theme show");
    assert_eq!(row.get("id"), Some(&Value::String("dracula".to_string())));
    assert_eq!(row.get("name"), Some(&Value::String("Dracula".to_string())));
    assert_eq!(
        row.get("accent"),
        Some(&Value::String("#bd93f9".to_string()))
    );
}

#[cfg(unix)]
#[test]
fn theme_show_plain_snapshot_contract() {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .env("PATH", "/usr/bin:/bin")
        .args([
            "--mode",
            "plain",
            "--color",
            "never",
            "--unicode",
            "never",
            "theme",
            "show",
            "dracula",
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
    assert_snapshot_text!(
        "theme_show_plain_stdout",
        String::from_utf8(output.stdout).expect("stdout should be utf-8"),
    );
}

#[cfg(unix)]
#[test]
fn cli_theme_override_contract() {
    let home = make_temp_dir("osp-cli-theme-override");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"
theme.name = "nord"
"#,
    );

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "--theme", "dracula", "theme", "show"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert!(
        output.stderr.is_empty(),
        "stderr should stay empty: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: Value =
        serde_json::from_slice(&output.stdout).expect("theme show stdout should be valid json");
    let row = first_row(&payload, "theme show override");
    assert_eq!(row.get("id"), Some(&Value::String("dracula".to_string())));

    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn config_theme_seed_contract() {
    let home = make_temp_dir("osp-cli-theme-config-seed");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"
theme.name = "nord"
"#,
    );

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "theme", "show"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert!(
        output.stderr.is_empty(),
        "stderr should stay empty: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: Value =
        serde_json::from_slice(&output.stdout).expect("theme show stdout should be valid json");
    let row = first_row(&payload, "theme show from config");
    assert_eq!(row.get("id"), Some(&Value::String("nord".to_string())));

    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn custom_theme_inherits_custom_base_contract() {
    let home = make_temp_dir("osp-cli-theme-custom-base");
    write_config(
        &home,
        r#"
[default]
profile.default = "uio"
theme.name = "brand-child"
"#,
    );

    let theme_dir = home.join(".config").join("osp").join("themes");
    std::fs::create_dir_all(&theme_dir).expect("theme dir should be created");
    std::fs::write(
        theme_dir.join("brand-base.toml"),
        r##"
base = "nord"

[palette]
accent = "#123456"
"##,
    )
    .expect("base theme should be written");
    std::fs::write(
        theme_dir.join("brand-child.toml"),
        r##"
base = "brand-base"

[palette]
warning = "#abcdef"
"##,
    )
    .expect("child theme should be written");

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    let output = cmd
        .envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "theme", "show"])
        .assert()
        .success()
        .get_output()
        .clone();
    assert!(
        output.stderr.is_empty(),
        "stderr should stay empty: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: Value =
        serde_json::from_slice(&output.stdout).expect("theme show stdout should be valid json");
    let row = first_row(&payload, "custom theme show");
    assert_eq!(
        row.get("id"),
        Some(&Value::String("brand-child".to_string()))
    );
    assert_eq!(
        row.get("base"),
        Some(&Value::String("brand-base".to_string()))
    );
    assert_eq!(
        row.get("source"),
        Some(&Value::String("custom".to_string()))
    );
    assert_eq!(
        row.get("accent"),
        Some(&Value::String("#123456".to_string()))
    );
    assert_eq!(
        row.get("warning"),
        Some(&Value::String("#abcdef".to_string()))
    );
    assert_eq!(row.get("text"), Some(&Value::String("#d8dee9".to_string())));

    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(unix)]
#[test]
fn unknown_theme_fails_fast_contract() {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.env("PATH", "/usr/bin:/bin")
        .args(["--theme", "missing-theme", "theme", "list"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("unknown theme: missing-theme"));
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
