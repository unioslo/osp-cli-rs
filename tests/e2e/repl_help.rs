#![allow(missing_docs)]

#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use crate::support::{ReplPtyColorMode, ReplPtyConfig, ReplPtySession};

#[cfg(unix)]
fn run_repl_command(command: &str, colored: bool) -> (String, String) {
    let color_mode = if colored {
        ReplPtyColorMode::Always
    } else {
        ReplPtyColorMode::Plain
    };
    let mut session = ReplPtySession::spawn(ReplPtyConfig::default().with_color_mode(color_mode));

    let start = session.output_len();
    assert!(
        session.wait_for_plain_output_since(start, "default>", Duration::from_secs(3)),
        "expected prompt output after REPL startup; output:\n{}",
        session.output_snapshot(4000),
    );

    let start = session.output_len();
    session.write_bytes(format!("{command}\r").as_bytes());
    let expected = if command == "config sho" {
        "unrecognized subcommand"
    } else if matches!(command, "help help" | "help --help") {
        "invalid help target"
    } else {
        "Usage"
    };
    assert!(
        session.wait_for_plain_output_since(start, expected, Duration::from_secs(3)),
        "expected output from `{command}`; output:\n{}",
        session.output_snapshot(4000),
    );
    let raw = session.output_since(start);
    let plain = crate::support::strip_terminal_noise(&raw);

    session.write_bytes(b"exit\r\r");
    assert!(
        session.wait_for_exit(Duration::from_secs(3)),
        "expected REPL to exit after `{command}`; output:\n{}",
        session.output_snapshot(4000),
    );

    (raw, plain)
}

#[cfg(unix)]
fn help_body(text: &str) -> String {
    text.find("Usage")
        .map(|idx| text[idx..].to_string())
        .unwrap_or_else(|| text.to_string())
}

#[cfg(unix)]
#[test]
fn repl_help_alias_and_verbose_help_route_to_the_canonical_help_surface_end_to_end() {
    let (_, alias_plain) = run_repl_command("help history", false);
    let (_, canonical_plain) = run_repl_command("history --help", false);
    assert_eq!(help_body(&alias_plain), help_body(&canonical_plain));

    let (_, alias_verbose_plain) = run_repl_command("help history -v", false);
    let (_, canonical_verbose_plain) = run_repl_command("history --help -v", false);
    assert_eq!(
        help_body(&alias_verbose_plain),
        help_body(&canonical_verbose_plain)
    );
}

#[cfg(unix)]
#[test]
fn repl_help_alias_rejects_invalid_targets_end_to_end() {
    for command in ["help help", "help --help"] {
        let (_, plain) = run_repl_command(command, false);
        assert!(
            plain.contains("invalid help target"),
            "command={command} output={plain:?}"
        );
        assert!(
            plain.contains("default>"),
            "command={command} output={plain:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn repl_invalid_subcommand_suggests_the_valid_command_end_to_end() {
    let (_, plain) = run_repl_command("config sho", false);
    assert!(
        plain.contains("unrecognized subcommand"),
        "output={plain:?}"
    );
    assert!(plain.contains("Try: use `config show`"), "output={plain:?}");
    assert!(
        !plain.contains("For more information, try '--help'."),
        "output={plain:?}"
    );
}

#[cfg(unix)]
#[test]
fn repl_help_alias_keeps_the_colored_tty_path_end_to_end() {
    let (alias_raw, alias_plain) = run_repl_command("help history", true);
    let (canonical_raw, canonical_plain) = run_repl_command("history --help", true);
    assert!(alias_raw.contains("\u{1b}["), "output={alias_raw:?}");
    assert!(
        canonical_raw.contains("\u{1b}["),
        "output={canonical_raw:?}"
    );
    assert_eq!(help_body(&alias_plain), help_body(&canonical_plain));
}
