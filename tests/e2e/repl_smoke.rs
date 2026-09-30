#![allow(missing_docs)]

#[cfg(unix)]
use crate::support::{ReplPtyConfig, ReplPtySession};
#[cfg(unix)]
use crate::temp_support::make_temp_dir;
#[cfg(unix)]
use std::io::Write;
#[cfg(unix)]
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

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

    let start = session.output_len();
    session.write_bytes(b"config set --session repl.history.enabled true\r");
    assert!(
        session.wait_for_plain_output_since(start, "for this session only", Duration::from_secs(3)),
        "history capture should be enabled for this session; output:\n{}",
        session.output_snapshot(4000),
    );
    assert!(session.wait_for_plain_output_since(start, "default>", Duration::from_secs(3)));

    // Recall executes the selected command; cancelling keeps the unfinished input.
    for (query, command, expected_id, expected_name) in [
        (None, "theme show dracula --json", "dracula", "Dracula"),
        (Some("dracula"), "", "dracula", "Dracula"),
        (
            Some("theme show "),
            "rose-pine-moon --json",
            "rose-pine-moon",
            "Rose Pine Moon",
        ),
    ] {
        if let Some(query) = query {
            session.type_text(query);
            let start = session.output_len();
            session.write_bytes(b"\x12");
            assert!(
                session.wait_for_plain_output_since(
                    start,
                    "(reverse-i-search)>",
                    Duration::from_secs(3)
                ),
                "history picker should display the current query; output:\n{}",
                session.output_snapshot(8000),
            );
            assert!(
                session.wait_for_plain_output_since(start, "--json", Duration::from_secs(3)),
                "history entry should be visible; output:\n{}",
                session.output_snapshot(12000)
            );
            let start = session.output_len();
            session.write_bytes(if command.is_empty() { b"\r" } else { b"\x03" });
            assert!(
                session.wait_for_output_since(start, "\x1b[?25h", Duration::from_secs(3)),
                "editor should resume after the history picker; output:\n{}",
                session.output_snapshot(8000),
            );
        }
        if !command.is_empty() {
            session.type_text(command);
        }
        let start = session.output_len();
        session.write_bytes(b"\r");
        assert!(
            session.wait_for_plain_output_since(start, "} ]", Duration::from_secs(3)),
            "selected or retained input should produce the theme result; output:\n{}",
            session.output_snapshot(8000),
        );
        let output = crate::support::strip_ansi_preserve_newlines(&session.output_since(start));
        let json_start = output
            .find("[\n")
            .expect("theme result should be a JSON array");
        let result = serde_json::Deserializer::from_str(&output[json_start..])
            .into_iter::<serde_json::Value>()
            .next()
            .expect("theme result should exist")
            .expect("theme result should parse");
        assert_eq!(result[0]["id"], expected_id);
        assert_eq!(result[0]["name"], expected_name);
        assert!(session.wait_for_plain_output_since(start, "default>", Duration::from_secs(3)));
    }

    session.write_bytes(b"exit\r");
    assert!(
        session.wait_for_exit(Duration::from_secs(3)),
        "expected REPL to exit after `exit`; output:\n{}",
        session.output_snapshot(2000),
    );
}

#[cfg(unix)]
#[test]
fn repl_builtin_commands_and_bare_help_return_without_a_refresh_wait() {
    let mut session = ReplPtySession::spawn(ReplPtyConfig::default());
    assert!(
        session.wait_for_plain_output("default>", Duration::from_secs(10)),
        "expected prompt after startup; output:\n{}",
        session.output_snapshot(2000),
    );

    let started = Instant::now();
    let output_start = session.output_len();
    session.write_bytes(b"theme list\r");
    assert!(
        session.wait_for_output_since(output_start, "Catppuccin", Duration::from_secs(1)),
        "theme list stalled; output:\n{}",
        session.output_snapshot(4000),
    );
    assert!(started.elapsed() < Duration::from_secs(1));

    let started = Instant::now();
    let output_start = session.output_len();
    session.write_bytes(b"theme\r");
    assert!(
        session.wait_for_output_since(
            output_start,
            "Inspect and change output themes",
            Duration::from_secs(1),
        ),
        "bare theme help stalled; output:\n{}",
        session.output_snapshot(4000),
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    let output = session.output_since(output_start);
    assert!(
        output.contains("Usage"),
        "expected styled theme help: {output}"
    );
    assert!(!output.contains("Try: use `theme --help`"));

    session.write_bytes(b"exit\r");
    if !session.wait_for_exit(Duration::from_secs(1)) {
        session.kill();
    }
}

#[cfg(unix)]
#[test]
fn repl_without_cursor_position_reports_falls_back_without_blocking() {
    let started = Instant::now();
    let mut session =
        ReplPtySession::spawn(ReplPtyConfig::default().without_cursor_position_reports());
    assert!(
        session.wait_for_plain_output("default>", Duration::from_secs(1)),
        "expected prompt after cursor-probe fallback; output:\n{}",
        session.output_snapshot(2000),
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(
        session
            .output_snapshot(2000)
            .contains("using basic input mode")
    );

    let output_start = session.output_len();
    let started = Instant::now();
    session.write_bytes(b"theme list\r");
    assert!(
        session.wait_for_output_since(output_start, "Catppuccin", Duration::from_secs(1)),
        "theme list stalled after cursor-probe fallback; output:\n{}",
        session.output_snapshot(4000),
    );
    assert!(started.elapsed() < Duration::from_secs(1));

    session.write_bytes(b"exit\r");
    if !session.wait_for_exit(Duration::from_secs(1)) {
        session.kill();
    }
}

#[cfg(unix)]
#[test]
fn repl_basic_mode_runs_help_and_exit_without_a_tty_end_to_end() {
    let home = make_temp_dir("osp-cli-basic-home");
    let plugins = make_temp_dir("osp-cli-basic-plugins");

    let output = Command::new(assert_cmd::cargo::cargo_bin!("osp"))
        .env_clear()
        .envs(crate::test_env::isolated_env(home.path()))
        .env("PATH", "/usr/bin:/bin")
        .env("LANG", "C.UTF-8")
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
