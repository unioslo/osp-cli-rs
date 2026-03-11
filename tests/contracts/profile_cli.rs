#[cfg(unix)]
use crate::temp_support::make_temp_dir;
use assert_cmd::Command;
use predicates::prelude::*;

#[cfg(unix)]
#[test]
fn positional_profile_routes_to_plugin_command_contract() {
    let home = make_temp_dir("osp-cli-profile-home");
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

    let plugin_dir = make_temp_dir("osp-cli-profile-plugin");
    write_hello_plugin(&plugin_dir);

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.envs(crate::test_env::isolated_env(&home))
        .env("PATH", "/usr/bin:/bin")
        .env("OSP_PLUGIN_PATH", &plugin_dir)
        .args(["tsd", "hello"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("hello-from-plugin"));
}

#[cfg(unix)]
#[test]
fn positional_profile_routes_to_builtin_plugins_command_contract() {
    let home = make_temp_dir("osp-cli-profile-home-builtins");
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
        .args(["tsd", "plugins", "list"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("No plugins discovered."));
}

#[cfg(unix)]
#[test]
fn unknown_first_token_is_treated_as_command_contract() {
    let home = make_temp_dir("osp-cli-profile-home-unknown");
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
        .args(["prod", "plugins", "list"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("no plugin provides command: prod"));
}

#[cfg(unix)]
#[test]
fn explicit_profile_overrides_positional_profile_contract() {
    let home = make_temp_dir("osp-cli-profile-home-explicit");
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
        .args(["--profile", "uio", "tsd", "plugins", "list"]);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains("no plugin provides command: tsd"));
}

#[cfg(unix)]
fn write_config(home: &std::path::Path, config: &str) {
    let config_dir = home.join(".config").join("osp");
    std::fs::create_dir_all(&config_dir).expect("config dir should be created");
    std::fs::write(config_dir.join("config.toml"), config).expect("config should be written");
}

#[cfg(unix)]
fn write_hello_plugin(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let plugin_path = dir.join("osp-hello");
    let plugin_script = r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{"protocol_version":1,"plugin_id":"hello","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{"name":"hello","about":"hello plugin","args":[],"flags":{},"subcommands":[]}]}
JSON
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
