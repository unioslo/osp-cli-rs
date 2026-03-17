#![allow(missing_docs)]

#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use crate::support::{ReplPtyColorMode, ReplPtyConfig, ReplPtySession};

#[cfg(unix)]
#[test]
fn repl_startup_intro_uses_the_configured_rich_tty_path() {
    let config = r#"
[default]
profile.default = "default"
ui.presentation = "expressive"
repl.intro = "full"
repl.simple_prompt = true
user.display_name = "demo"
"#;
    let mut session = ReplPtySession::spawn(
        ReplPtyConfig::default()
            .with_color_mode(ReplPtyColorMode::Always)
            .with_config(config)
            .with_intro_override(None),
    );

    assert!(
        session.wait_for_plain_output("default>", Duration::from_secs(3)),
        "expected REPL prompt after startup; output:\n{}",
        session.output_snapshot(4000),
    );

    let output = session.output_snapshot(4000);
    let plain = session.plain_output_snapshot(4000);
    assert!(
        output.contains("\x1b["),
        "expected ANSI-colored intro output on startup; output:\n{output}",
    );
    assert!(
        plain.contains("demo"),
        "expected config-driven intro content before the prompt; output:\n{plain}",
    );

    session.write_bytes(b"exit\r");
    assert!(
        session.wait_for_exit(Duration::from_secs(3)),
        "expected REPL to exit after `exit`; output:\n{}",
        session.output_snapshot(4000),
    );
}
