#![allow(missing_docs)]

#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use crate::support::{ReplPtyColorMode, ReplPtyConfig, ReplPtySession};

#[cfg(unix)]
fn spawn_repl_with_color() -> ReplPtySession {
    ReplPtySession::spawn(ReplPtyConfig::default().with_color_mode(ReplPtyColorMode::Always))
}

#[cfg(unix)]
#[test]
fn repl_highlights_commands_with_success_color() {
    let mut session = spawn_repl_with_color();

    let start = session.output_len();
    session.write_bytes(b"config");

    let expected = "\x1b[38;2;139;213;202mconfig";
    assert!(
        session.wait_for_output_since(start, expected, Duration::from_secs(3)),
        "expected success-colored command in output; output:\n{}",
        session.output_snapshot(2000),
    );

    session.write_bytes(b"\x03");
    session.write_bytes(b"exit\r\r");
    if !session.wait_for_exit(Duration::from_secs(3)) {
        session.kill();
    }
}

#[cfg(unix)]
#[test]
fn repl_help_history_highlights_help_keyword() {
    let mut session = spawn_repl_with_color();

    session.write_bytes(b"help history");

    let start = session.output_len();
    assert!(
        session.wait_for_output_since(
            start,
            "\x1b[38;2;139;213;202mhelp\x1b[0m",
            Duration::from_secs(3)
        ),
        "expected `help` to be highlighted with the command color; output:\n{}",
        session.output_snapshot(4000),
    );

    session.write_bytes(b"\x03");
    session.write_bytes(b"exit\r\r");
    if !session.wait_for_exit(Duration::from_secs(3)) {
        session.kill();
    }
}
