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
            .with_config("[default]\nrepl.shellable_commands = [\"orch\"]\n"),
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
    // Advertise overlapping host and service-root words through the plugin
    // protocol, so shell precedence crosses the real dispatch boundary.
    let path = plugins.join("osp-orch");
    let script = std::fs::read_to_string(&path).unwrap();
    let commands = ["history", "config", "theme", "plugins", "ldap", "lookup"]
        .map(|name| serde_json::json!({"name": name, "subcommands": [{"name": "inspect"}]}));
    std::fs::write(
        &path,
        script.replace(
            "\"subcommands\":[]",
            &format!(
                "\"subcommands\":{}",
                serde_json::to_string(&commands).unwrap()
            ),
        ),
    )
    .unwrap();
    let mut session = ReplPtySession::spawn(
        ReplPtyConfig::default().with_plugins_dir(&plugins).with_config(
            "[default]\nrepl.shellable_commands = [\"orch\", \"ldap\"]\nalias.lookup = \"config show\"\n",
        ),
    );

    assert!(
        session.wait_for_plain_output("default>", HANG_TIMEOUT),
        "expected root prompt before shell entry; output:\n{}",
        session.output_snapshot(2000),
    );

    let entered = session.output_len();
    session.write_bytes(b"orch\r");
    assert!(
        session.wait_for_output_since(entered, "default [orch]>", Duration::from_secs(1)),
        "expected entered shell in prompt; output:\n{}",
        session.output_snapshot(4000),
    );

    for name in ["history", "config", "theme", "plugins", "ldap", "lookup"] {
        let submitted = session.output_len();
        session.write_bytes(format!("{name} inspect\r").as_bytes());
        assert!(
            session.wait_for_output_since(submitted, "orch-from-plugin", HANG_TIMEOUT),
            "shell command {name} did not reach its provider: {}",
            session.output_snapshot(4000)
        );
        assert!(session.wait_for_output_since(submitted, "default [orch]>", HANG_TIMEOUT));
    }

    let left = session.output_len();
    session.write_bytes(b"exit\r");
    assert!(
        session.wait_for_output_since(left, "default>", Duration::from_secs(1)),
        "expected root prompt after shell exit; output:\n{}",
        session.output_snapshot(4000),
    );

    session.write_bytes(b"exit\r\r");
    if !session.wait_for_exit(HANG_TIMEOUT) {
        session.kill();
    }
}
