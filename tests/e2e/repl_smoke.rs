#![allow(missing_docs)]

#[cfg(unix)]
use crate::support::{ReplPtyConfig, ReplPtySession};
#[cfg(unix)]
use crate::temp_support::make_temp_dir;
#[cfg(unix)]
use std::io::Write;
#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
#[test]
fn repl_starts_runs_help_and_exits_end_to_end() {
    let mut session = ReplPtySession::spawn(ReplPtyConfig::default());

    let start = session.output_len();
    assert!(
        session.wait_for_output_since(start, "default>", Duration::from_secs(3)),
        "expected prompt output after REPL startup; output:\n{}",
        session.output_snapshot(2000),
    );

    let start = session.output_len();
    session.write_bytes(b"help\r");
    assert!(
        session.wait_for_output_since(start, "Commands", Duration::from_secs(3)),
        "expected help overview after `help`; output:\n{}",
        session.output_snapshot(2000),
    );
    assert!(
        session.wait_for_output_since(start, "config", Duration::from_secs(3)),
        "expected command overview after `help`; output:\n{}",
        session.output_snapshot(2000),
    );

    session.write_bytes(b"exit\r");
    assert!(
        session.wait_for_exit(Duration::from_secs(3)),
        "expected REPL to exit after `exit`; output:\n{}",
        session.output_snapshot(2000),
    );
}

#[cfg(unix)]
#[test]
fn repl_basic_mode_runs_help_and_exit_without_a_tty_end_to_end() {
    let home = make_temp_dir("osp-cli-basic-home");
    let plugins = make_temp_dir("osp-cli-basic-plugins");
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_osp"));

    let output = Command::new(bin)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_CACHE_HOME", home.join(".cache"))
        .env("XDG_STATE_HOME", home.join(".local/state"))
        .env("TERM", "dumb")
        .env("NO_COLOR", "1")
        .env("OSP__REPL__INTRO", "none")
        .env("OSP__REPL__SIMPLE_PROMPT", "true")
        .env("OSP__REPL__HISTORY__ENABLED", "false")
        .env("OSP__REPL__INPUT_MODE", "basic")
        .env("OSP_PLUGIN_PATH", &plugins)
        .env("OSP_BUNDLED_PLUGIN_DIR", &plugins)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            child
                .stdin
                .as_mut()
                .expect("stdin should be piped")
                .write_all(b"help\nexit\n")?;
            child.wait_with_output()
        })
        .expect("basic repl should run");

    assert!(
        output.status.success(),
        "basic repl should exit successfully; stderr:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("default> "));
    assert!(stdout.contains("Commands"));
    assert!(stdout.contains("help"));
    assert!(stdout.contains("exit"));
}
