#![allow(missing_docs)]

#[cfg(unix)]
use crate::support::{ReplPtyConfig, ReplPtySession};
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
type PtySession = ReplPtySession;

#[cfg(unix)]
fn spawn_repl() -> PtySession {
    let session = ReplPtySession::spawn(
        ReplPtyConfig::default().with_config("[default]\nui.format = \"json\"\n"),
    );
    assert!(
        session.wait_for_output_since(0, "\x1b[?25h", Duration::from_secs(3)),
        "expected editor startup repaint; output:\n{}",
        session.output_snapshot(4000),
    );
    session
}

#[cfg(unix)]
fn json_result(session: &ReplPtySession, start: usize) -> serde_json::Value {
    assert!(
        session.wait_for_plain_output_since(start, "} ]", Duration::from_secs(3)),
        "expected completed JSON command result; output:\n{}",
        session.output_snapshot(8000),
    );
    let output = crate::support::strip_ansi_preserve_newlines(&session.output_since(start));
    let json_start = output
        .find("[\n")
        .expect("JSON array should start on its own line");
    serde_json::Deserializer::from_str(&output[json_start..])
        .into_iter::<serde_json::Value>()
        .next()
        .expect("JSON result should exist")
        .expect("command result should parse")
}

#[cfg(unix)]
#[test]
fn repl_tab_completes_single_match_and_exits() {
    let mut session = spawn_repl();

    session.type_text("ex");
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

    session.type_text("the");
    let menu = session.output_len();
    session.write_bytes(b"\t");
    assert!(
        session.wait_for_plain_output_since(menu, "theme", Duration::from_secs(3)),
        "expected the single theme completion",
    );
    let selected = session.output_len();
    session.write_bytes(b"\t");
    assert!(
        session.wait_for_output_since(selected, "\x1b[?25h", Duration::from_secs(3)),
        "expected selected completion to repaint the input",
    );
    let accepted = session.output_len();
    session.write_bytes(b"\r");
    assert!(
        session.wait_for_output_since(accepted, "\x1b[?25h", Duration::from_secs(3)),
        "expected accepted completion to repaint the input",
    );
    session.type_text(" show dracula");
    let start = session.output_len();
    session.write_bytes(b"\r");
    let result = json_result(&session, start);
    assert_eq!(result[0]["id"], "dracula");
    assert_eq!(result[0]["name"], "Dracula");

    session.write_bytes(b"exit\r\r");
    assert!(
        session.wait_for_exit(Duration::from_secs(3)),
        "REPL should exit"
    );
}

#[cfg(unix)]
#[test]
fn repl_theme_show_tab_cycles_visible_suggestion_into_prompt_end_to_end() {
    let mut session = spawn_repl();

    session.type_text("theme show ");

    let start = session.output_len();
    session.write_bytes(b"\t");
    assert!(
        session.wait_for_plain_output_since(start, "catppuccin", Duration::from_secs(5)),
        "expected visible theme completion menu for `theme show `; output:\n{}",
        session.plain_output_snapshot(8000),
    );

    let mut selections = Vec::new();
    for _ in 0..2 {
        let start = session.output_len();
        session.write_bytes(b"\t");
        assert!(
            session.wait_for_output_since(start, "\x1b[?25h", Duration::from_secs(3)),
            "expected completion selection repaint; output:\n{}",
            session.output_snapshot(8000),
        );
        let output = crate::support::strip_terminal_noise(&session.output_since(start));
        let selected = output
            .rsplit_once("theme show ")
            .expect("selected theme should appear in the input")
            .1
            .split_whitespace()
            .next()
            .expect("theme selection should not be empty")
            .to_string();
        selections.push(selected);
    }
    assert_ne!(
        selections[0], selections[1],
        "Tab should move the selection"
    );

    let accepted = session.output_len();
    session.write_bytes(b"\r");
    assert!(
        session.wait_for_output_since(accepted, "\x1b[?25h", Duration::from_secs(3)),
        "expected accepted completion to repaint the input",
    );
    let start = session.output_len();
    session.write_bytes(b"\r");
    let result = json_result(&session, start);
    assert_eq!(result[0]["id"], selections[1]);

    session.write_bytes(b"exit\r\r");
    assert!(
        session.wait_for_exit(Duration::from_secs(3)),
        "REPL should exit"
    );
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
    assert!(
        session.wait_for_output_since(start, "\x1b[?25h", Duration::from_secs(3)),
        "expected menu to close before typing; output:\n{}",
        session.plain_output_snapshot(2000),
    );

    session.type_text("co");
    session.type_text("nfig get theme.name");
    let start = session.output_len();
    session.write_bytes(b"\r");
    let result = json_result(&session, start);
    assert_eq!(result[0]["key"], "theme.name");
    assert_eq!(result[0]["value"], "rose-pine-moon");

    session.write_bytes(b"exit\r\r");
    assert!(
        session.wait_for_exit(Duration::from_secs(3)),
        "REPL should exit"
    );
}
