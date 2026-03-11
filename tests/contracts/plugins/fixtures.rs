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
fn write_message_plugin(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let plugin_path = dir.join("osp-messageful");
    let plugin_script = r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{"protocol_version":1,"plugin_id":"messageful","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{"name":"messageful","about":"messageful plugin","args":[],"flags":{},"subcommands":[]}]}
JSON
  exit 0
fi

cat <<'JSON'
{"protocol_version":1,"ok":true,"data":{"message":"json-from-plugin"},"error":null,"meta":{"format_hint":"json"},"messages":[{"level":"warning","text":"plugin-warning-line"}]}
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
fn write_help_stderr_plugin(dir: &std::path::Path) -> std::path::PathBuf {
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

if [ "$1" = "--help" ] || [ "$1" = "-h" ] || [ "$1" = "help" ] || \
   [ "$2" = "--help" ] || [ "$2" = "-h" ] || [ "$2" = "help" ]; then
  echo "hello plugin help text"
  echo "plugin help note" >&2
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
        r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
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

fn write_provider_plugin(
    dir: &std::path::Path,
    plugin_id: &str,
    command_name: &str,
    message: &str,
) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let plugin_path = dir.join(format!("osp-{plugin_id}"));
    let plugin_script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
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
        r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
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
    let plugin_script = r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
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
    let plugin_script = r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
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
    let plugin_script = r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
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
    let plugin_script = r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
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
    let plugin_script = r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
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

fn write_describe_mismatch_plugin(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let plugin_path = dir.join("osp-hello");
    let plugin_script = r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
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
        r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
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
        r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
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
fn parse_json_stdout(stdout: &[u8]) -> serde_json::Value {
    serde_json::from_slice(stdout).unwrap_or_else(|err| {
        panic!(
            "stdout should be valid json: {err}\n{}",
            String::from_utf8_lossy(stdout)
        )
    })
}

#[cfg(unix)]
fn parse_toml_file(path: &std::path::Path) -> toml::Value {
    let raw = std::fs::read_to_string(path).unwrap_or_else(|err| {
        panic!("{} should be readable: {err}", path.display())
    });
    toml::from_str(&raw).unwrap_or_else(|err| {
        panic!(
            "{} should be valid toml: {err}\n{raw}",
            path.display()
        )
    })
}

#[cfg(unix)]
fn first_json_row<'a>(payload: &'a serde_json::Value, context: &str) -> &'a serde_json::Value {
    payload
        .as_array()
        .unwrap_or_else(|| panic!("{context} should render a JSON array"))
        .first()
        .unwrap_or_else(|| panic!("{context} should render at least one row"))
}
use crate::temp_support::make_temp_dir;
