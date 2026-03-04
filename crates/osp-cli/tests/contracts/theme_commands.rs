use assert_cmd::Command;
use predicates::prelude::*;

#[cfg(unix)]
#[test]
fn theme_list_contract() {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.env("PATH", "/usr/bin:/bin")
        .args(["--json", "theme", "list"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"name\": \"dracula\""))
        .stdout(predicate::str::contains("\"name\": \"rose-pine-moon\""));
}

#[cfg(unix)]
#[test]
fn theme_show_contract() {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.env("PATH", "/usr/bin:/bin")
        .args(["--json", "theme", "show", "dracula"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"name\": \"dracula\""))
        .stdout(predicate::str::contains("\"accent\": \"#bd93f9\""));
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
    cmd.env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "--theme", "dracula", "theme", "show"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"name\": \"dracula\""));

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
    cmd.env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .args(["--json", "theme", "show"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("\"name\": \"nord\""));

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
