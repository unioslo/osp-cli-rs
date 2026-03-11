#[cfg(unix)]
fn make_temp_dir(prefix: &str) -> crate::tests::TestTempDir {
    crate::tests::make_temp_dir(prefix)
}

#[cfg(unix)]
fn env_lock() -> &'static Mutex<()> {
    crate::tests::env_lock()
}

#[cfg(unix)]
fn write_named_test_plugin(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    write_named_test_plugin_with_min_version(dir, name, "0.1.0")
}

#[cfg(unix)]
fn write_named_test_plugin_with_min_version(
    dir: &std::path::Path,
    name: &str,
    min_osp_version: &str,
) -> std::path::PathBuf {
    let plugin_path = dir.join(format!("osp-{name}"));
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{name}","plugin_version":"0.1.0","min_osp_version":"{min_osp_version}","commands":[{{"name":"{name}","about":"{name} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"ok"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
        name = name,
        min_osp_version = min_osp_version
    );

    write_executable_script_atomically(&plugin_path, &script);
    plugin_path
}

#[cfg(unix)]
fn write_provider_test_plugin(
    dir: &std::path::Path,
    plugin_id: &str,
    command_name: &str,
) -> std::path::PathBuf {
    let plugin_path = dir.join(format!("osp-{plugin_id}"));
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{plugin_id}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{command_name}","about":"{plugin_id} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"ok"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
        plugin_id = plugin_id,
        command_name = command_name
    );

    write_executable_script_atomically(&plugin_path, &script);
    plugin_path
}

#[cfg(unix)]
fn write_multi_command_plugin(
    dir: &std::path::Path,
    plugin_id: &str,
    commands: &[&str],
) -> std::path::PathBuf {
    let plugin_path = dir.join(format!("osp-{plugin_id}"));
    let describe_commands = commands
        .iter()
        .map(|command| {
            format!(
                r#"{{"name":"{command}","about":"{plugin_id} plugin","args":[],"flags":{{}},"subcommands":[]}}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{plugin_id}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{describe_commands}]}}
JSON
  exit 0
fi

cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"ok"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
        plugin_id = plugin_id,
        describe_commands = describe_commands,
    );

    write_executable_script_atomically(&plugin_path, &script);
    plugin_path
}

#[cfg(unix)]
fn write_path_probe_plugin(
    dir: &std::path::Path,
    name: &str,
    probe_path: &std::path::Path,
) -> std::path::PathBuf {
    let plugin_path = dir.join(format!("osp-{name}"));
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin
if [ "$1" = "--describe" ]; then
  printf 'described\n' >> '{probe_path}'
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{name}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{name}","about":"{name} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"ok"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
        name = name,
        probe_path = probe_path.display(),
    );

    write_executable_script_atomically(&plugin_path, &script);
    plugin_path
}

#[cfg(unix)]
fn write_sleepy_test_plugin(
    dir: &std::path::Path,
    name: &str,
    sleep_on_describe: bool,
) -> std::path::PathBuf {
    let plugin_path = dir.join(format!("osp-{name}"));
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin
if [ "$1" = "--describe" ]; then
  if [ "{sleep_on_describe}" = "true" ]; then
    sleep 1
  fi
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{name}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{name}","about":"{name} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

sleep 1
cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"ok"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
        name = name,
        sleep_on_describe = if sleep_on_describe { "true" } else { "false" }
    );

    write_executable_script_atomically(&plugin_path, &script);
    plugin_path
}

#[cfg(unix)]
fn write_timeout_leak_test_plugin(
    dir: &std::path::Path,
    name: &str,
    marker: &std::path::Path,
) -> std::path::PathBuf {
    let plugin_path = dir.join(format!("osp-{name}"));
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{name}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{name}","about":"{name} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

(sleep 0.2; touch "{marker}") &
sleep 1
"#,
        name = name,
        marker = marker.display(),
    );

    write_executable_script_atomically(&plugin_path, &script);
    plugin_path
}

#[cfg(unix)]
fn write_large_output_test_plugin(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let plugin_path = dir.join(format!("osp-{name}"));
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{name}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{name}","about":"{name} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

printf '{{"protocol_version":1,"ok":true,"data":{{"blob":"'
head -c 131072 /dev/zero | tr '\0' 'x'
printf '"}},"error":null,"meta":{{"format_hint":"table","columns":["blob"]}}}}'
"#,
        name = name,
    );

    write_executable_script_atomically(&plugin_path, &script);
    plugin_path
}

#[cfg(unix)]
fn write_marker_describe_plugin(
    dir: &std::path::Path,
    name: &str,
    marker: &std::path::Path,
) -> std::path::PathBuf {
    let plugin_path = dir.join(format!("osp-{name}"));
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin
if [ "$1" = "--describe" ]; then
  touch "{marker}"
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{name}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{name}","about":"{name} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"ok"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
        marker = marker.display(),
        name = name,
    );

    write_executable_script_atomically(&plugin_path, &script);
    plugin_path
}

#[cfg(unix)]
fn write_executable_script_atomically(path: &std::path::Path, script: &str) {
    use std::fs::File;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    let tmp_path = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp_path);
    let mut file = File::create(&tmp_path).expect("temp plugin should be created");
    file.write_all(script.as_bytes())
        .expect("plugin should be written");
    file.sync_all().expect("temp plugin should be flushed");

    let mut perms = file
        .metadata()
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    file.set_permissions(perms)
        .expect("temp plugin should be executable");
    drop(file);

    // Publish the executable in one rename step so discovery never races a partially
    // written script. This keeps the tests from manufacturing ETXTBSY on CI.
    std::fs::rename(&tmp_path, path).expect("plugin should be installed atomically");
}
