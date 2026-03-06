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

if [ "$1" = "--help" ] || [ "$1" = "-h" ] || [ "$1" = "help" ]; then
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
