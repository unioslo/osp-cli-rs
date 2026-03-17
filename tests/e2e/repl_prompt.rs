#![allow(missing_docs)]

#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use crate::support::{
    ReplPtyColorMode, ReplPtyConfig, ReplPtySession, strip_ansi_preserve_newlines,
};

#[cfg(unix)]
fn spawn_repl_with_color(simple_prompt: bool) -> ReplPtySession {
    ReplPtySession::spawn(
        ReplPtyConfig::default()
            .with_color_mode(ReplPtyColorMode::Always)
            .with_simple_prompt(simple_prompt),
    )
}

#[cfg(unix)]
fn spawn_repl_with_color_and_config(simple_prompt: bool, config: &str) -> ReplPtySession {
    ReplPtySession::spawn(
        ReplPtyConfig::default()
            .with_color_mode(ReplPtyColorMode::Always)
            .with_simple_prompt(simple_prompt)
            .with_config(config),
    )
}

#[cfg(unix)]
#[test]
fn repl_prompt_prefix_uses_prompt_text_color() {
    let mut session = spawn_repl_with_color(false);

    let start = session.output_len();
    let expected = "\x1b[38;2;224;222;244m╭─";
    assert!(
        session.wait_for_output_since(start, expected, Duration::from_secs(3)),
        "expected styled prompt prefix in output; output:\n{}",
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
fn repl_prompt_right_user_override_replaces_default_rhs_end_to_end() {
    let config = r#"
[default]
profile.default = "default"
repl.intro = "none"
repl.simple_prompt = true
repl.prompt_right = "rhs\nmarker"
"#;
    let mut session = spawn_repl_with_color_and_config(true, config);

    assert!(
        session.wait_for_plain_output("default>", Duration::from_secs(3)),
        "expected prompt output after REPL startup; output:\n{}",
        session.output_snapshot(2000),
    );

    let plain = strip_ansi_preserve_newlines(&session.output_snapshot(2000));
    assert!(
        plain.contains("rhs\nmarker"),
        "expected user-configured RHS with decoded newline; output:\n{plain}",
    );
    assert!(
        !plain.contains("incognito"),
        "expected configured RHS to replace fallback incognito marker; output:\n{plain}",
    );

    session.write_bytes(b"exit\r\r");
    if !session.wait_for_exit(Duration::from_secs(3)) {
        session.kill();
    }
}

#[cfg(unix)]
#[test]
fn repl_prompt_right_template_expands_dynamic_placeholders_end_to_end() {
    let config = r#"
[default]
profile.default = "default"
repl.intro = "none"
repl.simple_prompt = true
repl.prompt_right = "{incognito} {timing}"
debug.level = 1
"#;
    let mut session = spawn_repl_with_color_and_config(true, config);

    assert!(
        session.wait_for_plain_output("default>", Duration::from_secs(3)),
        "expected prompt output after REPL startup; output:\n{}",
        session.output_snapshot(2000),
    );

    let plain = strip_ansi_preserve_newlines(&session.output_snapshot(2000));
    assert!(
        plain.contains("(⌐■_■)"),
        "expected incognito placeholder in RHS; output:\n{plain}",
    );
    assert!(
        plain.contains("ms"),
        "expected timing placeholder in RHS; output:\n{plain}",
    );

    session.write_bytes(b"exit\r\r");
    if !session.wait_for_exit(Duration::from_secs(3)) {
        session.kill();
    }
}
