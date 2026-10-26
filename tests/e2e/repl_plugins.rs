#![allow(missing_docs)]

#[cfg(unix)]
use crate::support::{HANG_TIMEOUT, ReplPtyConfig, ReplPtySession, write_executable_script};
#[cfg(unix)]
use crate::temp_support::make_temp_dir;
#[cfg(unix)]
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
fn spawn_repl(plugins: &Path) -> ReplPtySession {
    ReplPtySession::spawn(
        ReplPtyConfig::default()
            .with_plugins_dir(plugins)
            .with_config("[default]\nrepl.shellable_commands = [\"orch\", \"ldap\"]\n"),
    )
}

#[cfg(unix)]
fn write_provider_plugin(
    dir: &Path,
    plugin_id: &str,
    _command_name: &str,
    message: &str,
) -> PathBuf {
    let plugin_path = dir.join(format!("osp-{plugin_id}"));
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
if [ "$1" = "--osp-describe" ]; then
  cat <<'JSON'
{{"about":"{plugin_id} plugin","args":[],"flags":{{}},"subcommands":[],"output":"json"}}
JSON
  exit 0
fi

cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"{message}-from-plugin"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
        plugin_id = plugin_id,
        message = message
    );

    write_executable_script(&plugin_path, &script);
    plugin_path
}
#[cfg(unix)]
#[test]
fn repl_shell_entry_and_exit_refresh_the_prompt_scope() {
    if crate::isolation::isolated_test() {
        return;
    }
    let plugins = make_temp_dir("osp-cli-pty-shell-prompt-plugins");
    let _plugin = write_provider_plugin(&plugins, "orch", "orch", "orch");
    let _ldap = write_provider_plugin(&plugins, "ldap", "ldap", "ldap");
    let mut session = spawn_repl(&plugins);

    assert!(
        session.wait_for_plain_output("default>", HANG_TIMEOUT),
        "expected root prompt before shell entry; output:\n{}",
        session.output_snapshot(2000),
    );

    for shell in ["orch", "ldap"] {
        for leave in ["exit", "quit", "  ..  "] {
            let entered = session.output_len();
            session.write_bytes(format!("..\r{shell}\r").as_bytes());
            assert!(
                session.wait_for_output_since(
                    entered,
                    &format!("default [{shell}]>"),
                    Duration::from_secs(1),
                ),
                "expected shell prompt after harmless root navigation; output:\n{}",
                session.output_snapshot(4000),
            );

            let left = session.output_len();
            session.write_bytes(format!("{leave}\r").as_bytes());
            assert!(
                session.wait_for_output_since(left, "default>", Duration::from_secs(1)),
                "expected root prompt after `{leave}` in `{shell}`; output:\n{}",
                session.output_snapshot(4000),
            );
        }
    }

    session.write_bytes(b"exit\r\r");
    assert!(session.wait_for_exit(HANG_TIMEOUT), "root exit should quit");
}
