#![allow(missing_docs)]

#[cfg(unix)]
use crate::support::{ReplPtyConfig, ReplPtySession};
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
type PtySession = ReplPtySession;

#[cfg(unix)]
fn spawn_repl() -> PtySession {
    ReplPtySession::spawn(ReplPtyConfig::default())
}

#[cfg(unix)]
#[test]
fn repl_tab_completes_single_match_and_exits() {
    let mut session = spawn_repl();

    session.write_bytes(b"ex");
    session.write_bytes(b"\t");
    session.write_bytes(b"\t");
    assert!(
        session.wait_for_plain_output("exit default>", Duration::from_secs(5)),
        "expected tab completion to render `exit` in the prompt; output:\n{}",
        session.plain_output_snapshot(4000),
    );
    session.write_bytes(b"\r");
    session.write_bytes(b"\r");

    if !session.wait_for_exit(Duration::from_secs(3)) {
        session.kill();
        panic!("expected repl to exit after completion");
    }
    assert!(
        !session
            .plain_output_snapshot(4000)
            .contains("unrecognized subcommand"),
        "expected tab completion to turn `ex` into `exit`; output:\n{}",
        session.plain_output_snapshot(4000),
    );
}

#[cfg(unix)]
#[test]
fn repl_tab_accepts_single_visible_completion() {
    let mut session = spawn_repl();

    session.write_bytes(b"the");
    session.write_bytes(b"\t");
    session.write_bytes(b" show dracula --value\r");
    assert!(
        session.wait_for_plain_output("theme show dracula --value", Duration::from_secs(5)),
        "expected tab to accept `theme` before the follow-up command text; output:\n{}",
        session.plain_output_snapshot(4000),
    );

    session.write_bytes(b"\x03");
    session.write_bytes(b"exit\r\r");
    if !session.wait_for_exit(Duration::from_secs(3)) {
        session.kill();
    }
}

#[cfg(unix)]
#[test]
fn repl_theme_show_tab_cycles_visible_suggestion_into_prompt_end_to_end() {
    let mut session = spawn_repl();

    let start = session.output_len();
    session.write_bytes(b"theme show ");
    assert!(
        session.wait_for_output_since(start, "theme show ", Duration::from_secs(3)),
        "expected typed input to be echoed before completion; output:\n{}",
        session.output_snapshot(4000),
    );

    session.write_bytes(b"\t");
    assert!(
        session.wait_for_plain_output_since(start, "catppuccin", Duration::from_secs(5)),
        "expected visible theme completion menu for `theme show `; output:\n{}",
        session.plain_output_snapshot(8000),
    );

    let start = session.output_len();
    session.write_bytes(b"\t\t");
    assert!(
        session.wait_for_plain_output_since(start, "theme show dracula", Duration::from_secs(3)),
        "expected third Tab to cycle to the next theme value; output:\n{}",
        session.plain_output_snapshot(4000),
    );

    session.write_bytes(b"\x03");
    session.write_bytes(b"exit\r\r");
    if !session.wait_for_exit(Duration::from_secs(3)) {
        session.kill();
    }
}

#[cfg(unix)]
#[test]
fn repl_close_menu_keeps_typed_input() {
    let mut session = spawn_repl();

    let start = session.output_len();
    session.write_bytes(b"\t");
    assert!(
        session.wait_for_plain_output_since(start, "help", Duration::from_secs(3)),
        "expected visible root completion menu; output:\n{}",
        session.plain_output_snapshot(2000),
    );

    let start = session.output_len();
    session.write_bytes(b"\x1b");
    session.write_bytes(b"co");
    assert!(
        session.wait_for_plain_output_since(start, "co", Duration::from_secs(3)),
        "expected typed input after closing menu; output:\n{}",
        session.plain_output_snapshot(2000),
    );

    session.write_bytes(b"\x03");
    session.write_bytes(b"exit\r\r");
    if !session.wait_for_exit(Duration::from_secs(3)) {
        session.kill();
    }
}
