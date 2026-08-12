#![allow(missing_docs)]

#[cfg(unix)]
use crate::support::{
    osp_command, stderr_utf8, write_config, write_executable_script, write_nonzero_plugin,
    write_table_plugin, write_timeout_plugin,
};
#[cfg(unix)]
use crate::temp_support::make_temp_dir;

#[cfg(unix)]
#[test]
fn external_plugin_happy_path_runs_through_real_binary_process() {
    let home = make_temp_dir("osp-e2e-plugin-happy-home");
    let plugins = make_temp_dir("osp-e2e-plugin-happy-plugins");
    let _plugin = write_table_plugin(&plugins, "hello", "hello", "hello-from-plugin");

    let output = osp_command(home.path())
        .env("OSP_PLUGIN_PATH", plugins.path())
        .args(["hello"])
        .assert()
        .success()
        .get_output()
        .clone();

    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf-8");
    assert!(stdout.contains("hello-from-plugin"));
    assert!(
        output.stderr.is_empty(),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(unix)]
#[test]
fn external_plugin_nonzero_exit_surfaces_stderr_and_nonzero_status() {
    let home = make_temp_dir("osp-e2e-plugin-nonzero-home");
    let plugins = make_temp_dir("osp-e2e-plugin-nonzero-plugins");
    let _plugin = write_nonzero_plugin(&plugins, "boom", "boom", 7, "backend exploded");

    let output = osp_command(home.path())
        .env("OSP_PLUGIN_PATH", plugins.path())
        .args(["boom"])
        .assert()
        .failure()
        .code(1)
        .get_output()
        .clone();

    assert!(
        output.stdout.is_empty(),
        "stdout should stay empty: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = stderr_utf8(output.stderr);
    assert!(stderr.contains("plugin boom exited with status 7"));
    assert!(stderr.contains("backend exploded"));
}

#[cfg(unix)]
#[test]
fn external_plugin_timeout_surfaces_timeout_to_the_user() {
    let home = make_temp_dir("osp-e2e-plugin-timeout-home");
    let plugins = make_temp_dir("osp-e2e-plugin-timeout-plugins");
    let _plugin = write_timeout_plugin(&plugins, "hang", "hang");
    write_config(
        home.path(),
        r#"
[default]
profile.default = "default"
extensions.plugins.timeout_ms = 50
"#,
    );

    let output = osp_command(home.path())
        .env("OSP_PLUGIN_PATH", plugins.path())
        .args(["hang"])
        .assert()
        .failure()
        .code(1)
        .get_output()
        .clone();

    assert!(
        output.stdout.is_empty(),
        "stdout should stay empty: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = stderr_utf8(output.stderr);
    assert!(stderr.contains("plugin hang timed out after 50 ms"));
    assert!(stderr.contains("extensions.plugins.timeout_ms"));
}

#[cfg(unix)]
#[test]
fn uncached_path_plugin_auth_is_enforced_before_first_dispatch() {
    let home = make_temp_dir("osp-e2e-plugin-path-auth-home");
    let plugins = make_temp_dir("osp-e2e-plugin-path-auth-plugins");
    let execution_marker = home.path().join("plugin-executed");
    let plugin_path = plugins.path().join("osp-guarded");
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"guarded","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"guarded","about":"guarded plugin","auth":{{"visibility":"authenticated"}},"args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

touch "{marker}"
cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"should-not-run"}},"error":null,"meta":{{"format_hint":"table"}}}}
JSON
"#,
        marker = execution_marker.display(),
    );
    write_executable_script(&plugin_path, &script);
    write_config(
        home.path(),
        r#"
[default]
profile.default = "default"
extensions.plugins.discovery.path = true
"#,
    );
    let path = std::env::join_paths([
        plugins.path(),
        std::path::Path::new("/usr/bin"),
        std::path::Path::new("/bin"),
    ])
    .expect("plugin PATH should join");

    let output = osp_command(home.path())
        .env("PATH", path)
        .args(["guarded"])
        .assert()
        .failure()
        .get_output()
        .clone();

    assert!(
        !execution_marker.exists(),
        "plugin execution must not start before policy allows it"
    );
    let stderr = stderr_utf8(output.stderr);
    assert!(stderr.contains("unknown command guarded"), "{stderr}");
}

#[cfg(unix)]
#[test]
fn visible_but_denied_plugin_help_is_available_while_execution_is_blocked() {
    let home = make_temp_dir("osp-e2e-plugin-visible-help-home");
    let plugins = make_temp_dir("osp-e2e-plugin-visible-help-plugins");
    let execution_marker = home.path().join("plugin-executed");
    let plugin_path = plugins.path().join("osp-guarded-help");
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"guarded-help","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"guarded-help","about":"guarded help plugin","auth":{{"visibility":"authenticated"}},"args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

if [ "$2" = "--help" ] || [ "$2" = "help" ]; then
  echo "Usage: osp guarded-help"
  exit 0
fi

touch "{marker}"
cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"should-not-run"}},"error":null,"meta":{{"format_hint":"table"}}}}
JSON
"#,
        marker = execution_marker.display(),
    );
    write_executable_script(&plugin_path, &script);

    let help = osp_command(home.path())
        .env("OSP_PLUGIN_PATH", plugins.path())
        .args(["guarded-help", "--help"])
        .assert()
        .success()
        .get_output()
        .clone();
    let help_stdout = String::from_utf8(help.stdout).expect("help stdout should be UTF-8");
    assert!(help_stdout.contains("osp guarded-help"), "{help_stdout}");

    let denied = osp_command(home.path())
        .env("OSP_PLUGIN_PATH", plugins.path())
        .args(["guarded-help"])
        .assert()
        .failure()
        .get_output()
        .clone();
    assert!(
        stderr_utf8(denied.stderr).contains("requires authentication"),
        "real execution should still require runnability"
    );
    assert!(!execution_marker.exists());
}
