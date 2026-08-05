use crate::temp_support::make_temp_dir;
use crate::test_env::isolated_env;
use assert_cmd::Command;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

fn run_osp(args: &[&str], config_toml: Option<&str>) -> std::process::Output {
    run_osp_with_plugin_path(args, config_toml, None)
}

fn normalized_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn run_osp_with_plugin_path(
    args: &[&str],
    config_toml: Option<&str>,
    plugin_path: Option<&Path>,
) -> std::process::Output {
    let home = make_temp_dir("osp-cli-error-reporting");
    if let Some(config_toml) = config_toml {
        let config_dir = home.path().join(".config").join("osp");
        std::fs::create_dir_all(&config_dir).expect("config dir should be created");
        std::fs::write(config_dir.join("config.toml"), config_toml)
            .expect("config should be written");
    }

    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("TERM", "xterm-256color")
        .env("LANG", "C.UTF-8");
    for (key, value) in isolated_env(home.path()) {
        cmd.env(key, value);
    }
    if let Some(plugin_path) = plugin_path {
        cmd.env("OSP_PLUGIN_PATH", plugin_path);
    }

    cmd.args(args).output().expect("osp command should run")
}

#[cfg(unix)]
fn write_non_zero_plugin(dir: &Path) -> std::path::PathBuf {
    let plugin_path = dir.join("osp-boom");
    std::fs::write(
        &plugin_path,
        r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{"protocol_version":1,"plugin_id":"boom","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{"name":"boom","about":"boom plugin","args":[],"flags":{},"subcommands":[]}]}
JSON
  exit 0
fi

echo "boom-from-stderr" >&2
exit 7
"#,
    )
    .expect("plugin script should be written");
    let mut perms = std::fs::metadata(&plugin_path)
        .expect("plugin metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&plugin_path, perms).expect("plugin should be executable");
    plugin_path
}

#[test]
fn default_usage_failure_stays_terse_with_hint_contract() {
    let output = run_osp(&["--definitely-not-a-flag"], None);

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unexpected argument '--definitely-not-a-flag' found"));
    assert!(stderr.contains("Hint: use --help to inspect accepted flags and subcommands"));
    assert!(stderr.contains("run again with -v, -vv, or -vvv"));
}

#[test]
fn verbose_usage_failure_shows_parse_context_without_terse_hint_contract() {
    let output = run_osp(&["-v", "--definitely-not-a-flag"], None);

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unexpected argument '--definitely-not-a-flag' found"));
    assert!(stderr.contains("failed to parse CLI arguments"));
    assert!(!stderr.contains("Hint: use --help to inspect accepted flags and subcommands"));
}

#[test]
fn forensic_config_failure_renders_full_host_context_contract() {
    let output = run_osp(
        &["-vvv", "config", "get", "ui.message.verbosity"],
        Some(
            r#"[default]
ui.message.verbosity = "definitely-invalid"
"#,
        ),
    );

    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&output.stderr);
    let normalized = normalized_whitespace(&stderr);
    assert!(normalized.contains("failed to resolve initial config for startup"));
    assert!(normalized.contains("config resolution failed"));
    assert!(
        normalized.contains("invalid value for key ui.message.verbosity"),
        "{normalized}"
    );
    assert!(!stderr.contains("Hint:"));
}

#[test]
fn forensic_runtime_failure_uses_configured_color_and_stacktrace_contract() {
    let output = run_osp(
        &["-vvv", "config", "get", "ui.message.verbosity"],
        Some(
            r#"[default]
ui.color.mode = "always"
theme.name = "missing-theme"
"#,
        ),
    );

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    let normalized = normalized_whitespace(&stderr);
    assert!(stderr.contains("failed to derive host runtime inputs for startup"));
    assert!(stderr.contains("unknown theme: missing-theme"));
    assert!(normalized.contains("Stack backtrace"));
    assert!(stderr.contains("\u{1b}["));
}

#[test]
fn debug_runtime_failure_shows_context_without_stacktrace_contract() {
    let output = run_osp(
        &["-vv", "config", "get", "ui.message.verbosity"],
        Some(
            r#"[default]
theme.name = "missing-theme"
"#,
        ),
    );

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("failed to derive host runtime inputs for startup"));
    assert!(stderr.contains("unknown theme: missing-theme"));
    assert!(!stderr.contains("Stack backtrace"));
}

#[cfg(unix)]
#[test]
fn debug_plugin_failure_shows_context_without_stacktrace_contract() {
    let dir = make_temp_dir("osp-cli-error-reporting-plugin");
    let _plugin = write_non_zero_plugin(dir.path());

    let output = run_osp_with_plugin_path(&["-vv", "boom"], None, Some(dir.path()));

    assert_eq!(output.status.code(), Some(4));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("plugin command failed"));
    assert!(stderr.contains("plugin boom exited with status 7"));
    assert!(stderr.contains("boom-from-stderr"));
    assert!(!stderr.contains("Stack backtrace"));
}

#[test]
fn debug_toml_parse_failure_shows_snippet_without_stacktrace_contract() {
    let output = run_osp(
        &["-vv", "config", "get", "ui.message.verbosity"],
        Some("not = [valid\n"),
    );

    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&output.stderr);
    let normalized = normalized_whitespace(&stderr);
    assert!(normalized.contains("failed to parse TOML"));
    assert!(normalized.contains("Begin snippet"));
    assert!(normalized.contains("invalid TOML starts here"));
    assert!(!stderr.contains("Stack backtrace"));
}

#[test]
fn forensic_toml_parse_failure_shows_snippet_and_stacktrace_contract() {
    let output = run_osp(
        &["-vvv", "config", "get", "ui.message.verbosity"],
        Some("not = [valid\n"),
    );

    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&output.stderr);
    let normalized = normalized_whitespace(&stderr);
    assert!(normalized.contains("failed to parse TOML"));
    assert!(normalized.contains("Begin snippet"));
    assert!(normalized.contains("invalid TOML starts here"));
    assert!(normalized.contains("Stack backtrace"));
}
