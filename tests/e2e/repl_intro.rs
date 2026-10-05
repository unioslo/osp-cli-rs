#![allow(missing_docs)]

#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use crate::support::{
    ReplPtyColorMode, ReplPtyConfig, ReplPtySession, strip_ansi_preserve_newlines,
};

#[cfg(unix)]
#[test]
fn repl_startup_intro_uses_the_configured_rich_tty_path() {
    let config = r#"
[default]
profile.default = "default"
ui.presentation = "expressive"
repl.intro = "full"
repl.simple_prompt = true
user.full_name = "demo operator"
domain = "example.uio.no"
theme.name = "dracula"
repl.intro_tips = ["Inspect ownership before making changes."]
repl.intro_template.full = """
## Operator desk
Welcome {{ user.full_name }}.
Theme: {{ theme_display }}
Tip: {{ tip }}

## Session facts
```osp
{"profile":"{{ profile }}","theme":"{{ theme }}","operator":"{{ display_name }}"}
```
```osp
[{"profile":"{{ profile }}","theme":"{{ theme }}","operator":"UiO support desk","domain":"{{ domain }}"}]
```

## Pipes
Export selected fields with `P key,value`.

{{ help }}
"""
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

    let output = session.output_snapshot(10_000);
    let plain = session.plain_output_snapshot(10_000);
    assert!(
        output.contains("\x1b["),
        "expected ANSI-colored intro output on startup; output:\n{output}",
    );
    assert!(
        plain.contains("demo"),
        "expected config-driven intro content before the prompt; output:\n{plain}",
    );
    let pipes = plain
        .split("Pipes")
        .nth(1)
        .expect("startup should show pipe hints")
        .split("Usage:")
        .next()
        .unwrap();
    assert!(
        pipes.lines().count() <= 6,
        "startup should show a cheat sheet, not a manual:\n{plain}"
    );
    assert!(plain.contains("Theme:") && plain.contains("Commands"));
    for authored_fact in [
        "Operator desk",
        "Welcome demo operator.",
        "Theme: Dracula",
        "Tip: Inspect ownership before making changes.",
        "Session facts",
        "UiO support desk",
        "example.uio.no",
        "dracula",
    ] {
        assert!(
            plain.contains(authored_fact),
            "missing {authored_fact:?}:\n{plain}"
        );
    }

    session.type_text("config set --session theme.name nord");
    let changed = session.output_len();
    session.write_bytes(b"\r");
    assert!(
        session.wait_for_plain_output_since(changed, "Theme: Nord", Duration::from_secs(3)),
        "theme change should rebuild the authored intro; output:\n{}",
        session.output_snapshot(12_000),
    );
    assert!(
        session.wait_for_plain_output_since(changed, "default>", Duration::from_secs(3)),
        "rebuilt intro should return to the editor; output:\n{}",
        session.output_snapshot(12_000),
    );
    let rebuilt = strip_ansi_preserve_newlines(&session.output_since(changed));
    let intro = rebuilt.split_once("Operator desk").unwrap().1;
    for authored_fact in [
        "Welcome demo operator.",
        "Theme: Nord",
        "Tip: Inspect ownership before making changes.",
        "Session facts",
        "UiO support desk",
        "example.uio.no",
        "Commands",
    ] {
        assert!(
            intro.contains(authored_fact),
            "missing rebuilt {authored_fact:?}:\n{intro}"
        );
    }

    session.type_text("config get theme.name --sources --json");
    let closed = session.output_len();
    session.write_bytes(b"\x1b");
    assert!(
        session.wait_for_output_since(closed, "\x1b[?25h", Duration::from_secs(3)),
        "closing the flag menu should repaint the retained command",
    );
    let inspected = session.output_len();
    session.write_bytes(b"\r");
    assert!(
        session.wait_for_plain_output_since(inspected, "} ]", Duration::from_secs(3)),
        "rebuilt editor should execute the config inspection; output:\n{}",
        session.output_snapshot(12_000),
    );
    let rendered = strip_ansi_preserve_newlines(&session.output_since(inspected));
    let start = rendered.find("[\n").expect("config result should be JSON");
    let document = serde_json::Deserializer::from_str(&rendered[start..])
        .into_iter::<serde_json::Value>()
        .next()
        .unwrap()
        .expect("config result should parse");
    assert_eq!(document[0]["key"], "theme.name");
    assert_eq!(document[0]["value"], "nord");
    assert_eq!(document[0]["source"], "session");
    assert_eq!(document[0]["scope_profile"], "default");
    assert!(session.wait_for_plain_output_since(inspected, "default>", Duration::from_secs(3)));

    session.write_bytes(b"exit\r");
    assert!(
        session.wait_for_exit(Duration::from_secs(3)),
        "expected REPL to exit after `exit`; output:\n{}",
        session.output_snapshot(4000),
    );
}
