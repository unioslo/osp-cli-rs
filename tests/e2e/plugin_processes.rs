#![allow(missing_docs)]

#[cfg(unix)]
use crate::support::{
    osp_command, stderr_utf8, write_config, write_nonzero_plugin, write_table_plugin,
    write_timeout_plugin,
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
        .code(4)
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
        .code(4)
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
