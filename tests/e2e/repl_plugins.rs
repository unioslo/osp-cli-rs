#![allow(missing_docs)]

#[cfg(unix)]
use crate::support::{ReplPtyConfig, ReplPtySession, write_executable_script};
#[cfg(unix)]
use crate::temp_support::make_temp_dir;
#[cfg(unix)]
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

#[cfg(unix)]
fn spawn_repl(plugins: &Path) -> ReplPtySession {
    ReplPtySession::spawn(
        ReplPtyConfig::default()
            .with_plugins_dir(plugins)
            .with_config("[default]\nrepl.shellable_commands = [\"orch\"]\n"),
    )
}

#[cfg(unix)]
fn write_provider_plugin(
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
{{"protocol_version":1,"ok":true,"data":{{"message":"{message}-from-plugin"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
        plugin_id = plugin_id,
        command_name = command_name,
        message = message
    );

    write_executable_script(&plugin_path, &script);
    plugin_path
}

#[cfg(unix)]
#[test]
fn repl_requires_explicit_provider_selection_for_conflicted_commands() {
    let plugins = make_temp_dir("osp-cli-pty-conflict-plugins");
    let _alpha = write_provider_plugin(&plugins, "alpha-provider", "hello", "alpha");
    let _beta = write_provider_plugin(&plugins, "beta-provider", "hello", "beta");
    let mut session = spawn_repl(&plugins);

    assert!(
        session.wait_for_output_since(0, "> ", Duration::from_secs(3)),
        "expected prompt before dispatch; output:\n{}",
        session.output_snapshot(2000),
    );

    let start = session.output_len();
    session.write_bytes(b"hello\r");
    assert!(
        session.wait_for_output_since(
            start,
            "command `hello` requires provider selection",
            Duration::from_secs(3)
        ),
        "expected provider-selection error in repl output; output:\n{}",
        session.output_snapshot(4000),
    );
    assert!(
        session.wait_for_output_since(start, "--plugin-provider", Duration::from_secs(3)),
        "expected provider-selection hint in repl output; output:\n{}",
        session.output_snapshot(4000),
    );
    assert!(
        !session.output_snapshot(4000).contains("alpha-from-plugin"),
        "did not expect implicit provider execution; output:\n{}",
        session.output_snapshot(4000),
    );

    session.write_bytes(b"exit\r\r");
    if !session.wait_for_exit(Duration::from_secs(3)) {
        session.kill();
    }
}

#[cfg(unix)]
#[test]
fn repl_shell_entry_and_exit_refresh_the_prompt_scope() {
    let plugins = make_temp_dir("osp-cli-pty-shell-prompt-plugins");
    let _plugin = write_provider_plugin(&plugins, "orch-provider", "orch", "orch");
    let mut session = spawn_repl(&plugins);

    assert!(
        session.wait_for_plain_output("default>", Duration::from_secs(10)),
        "expected root prompt before shell entry; output:\n{}",
        session.output_snapshot(2000),
    );

    let entered = session.output_len();
    let started = Instant::now();
    session.write_bytes(b"orch\r");
    assert!(
        session.wait_for_output_since(entered, "default [orch]>", Duration::from_secs(1)),
        "expected entered shell in prompt; output:\n{}",
        session.output_snapshot(4000),
    );
    assert!(started.elapsed() < Duration::from_secs(1));

    let left = session.output_len();
    let started = Instant::now();
    session.write_bytes(b"exit\r");
    assert!(
        session.wait_for_output_since(left, "default>", Duration::from_secs(1)),
        "expected root prompt after shell exit; output:\n{}",
        session.output_snapshot(4000),
    );
    assert!(started.elapsed() < Duration::from_secs(1));

    session.write_bytes(b"exit\r\r");
    if !session.wait_for_exit(Duration::from_secs(3)) {
        session.kill();
    }
}
