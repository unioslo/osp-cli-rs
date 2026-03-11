#[cfg(unix)]
use assert_cmd::Command;
#[cfg(unix)]
use serde_json::Value;
#[cfg(unix)]
use std::path::{Path, PathBuf};

#[cfg(unix)]
pub(crate) fn osp_command(home: &Path) -> Command {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("TERM", "xterm-256color")
        .env("LANG", "C.UTF-8")
        .env("NO_COLOR", "1");
    for (key, value) in crate::test_env::isolated_env(home) {
        cmd.env(key, value);
    }
    cmd
}

#[cfg(unix)]
pub(crate) fn write_config(home: &Path, config: &str) {
    let config_dir = home.join(".config").join("osp");
    std::fs::create_dir_all(&config_dir).expect("config dir should be created");
    std::fs::write(config_dir.join("config.toml"), config).expect("config should be written");
}

#[cfg(unix)]
pub(crate) fn parse_json_stdout(stdout: &[u8]) -> Value {
    serde_json::from_slice(stdout).unwrap_or_else(|err| {
        panic!(
            "stdout should be valid json: {err}\n{}",
            String::from_utf8_lossy(stdout)
        )
    })
}

#[cfg(unix)]
pub(crate) fn first_json_row<'a>(payload: &'a Value, context: &str) -> &'a Value {
    payload
        .as_array()
        .unwrap_or_else(|| panic!("{context} should render a JSON array"))
        .first()
        .unwrap_or_else(|| panic!("{context} should render at least one row"))
}

#[cfg(unix)]
pub(crate) fn stderr_utf8(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).expect("stderr should be utf-8")
}

#[cfg(unix)]
pub(crate) fn write_executable_script(path: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::write(path, script).expect("script should be written");
    let mut perms = std::fs::metadata(path)
        .expect("script metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).expect("script should be executable");
}

#[cfg(unix)]
pub(crate) fn write_table_plugin(
    dir: &Path,
    plugin_id: &str,
    command_name: &str,
    message: &str,
) -> PathBuf {
    let plugin_path = dir.join(format!("osp-{plugin_id}"));
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{plugin_id}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{command_name}","about":"{plugin_id} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"{message}"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
        plugin_id = plugin_id,
        command_name = command_name,
        message = message,
    );
    write_executable_script(&plugin_path, &script);
    plugin_path
}

#[cfg(unix)]
pub(crate) fn write_nonzero_plugin(
    dir: &Path,
    plugin_id: &str,
    command_name: &str,
    status_code: i32,
    stderr: &str,
) -> PathBuf {
    let plugin_path = dir.join(format!("osp-{plugin_id}"));
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{plugin_id}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{command_name}","about":"{plugin_id} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

echo "{stderr}" >&2
exit {status_code}
"#,
        plugin_id = plugin_id,
        command_name = command_name,
        stderr = stderr,
        status_code = status_code,
    );
    write_executable_script(&plugin_path, &script);
    plugin_path
}

#[cfg(unix)]
pub(crate) fn write_timeout_plugin(dir: &Path, plugin_id: &str, command_name: &str) -> PathBuf {
    let plugin_path = dir.join(format!("osp-{plugin_id}"));
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{plugin_id}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{command_name}","about":"{plugin_id} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

sleep 1
cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"late"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
        plugin_id = plugin_id,
        command_name = command_name,
    );
    write_executable_script(&plugin_path, &script);
    plugin_path
}
