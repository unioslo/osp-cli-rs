#![allow(missing_docs)]

#[cfg(unix)]
use crate::output_support::parse_json_stdout;
#[cfg(unix)]
use crate::support::{PtyCommandOutput, ReplPtyColorMode, run_osp_command_in_pty};

#[cfg(unix)]
fn run_debug_complete_json_in_pty(color_mode: ReplPtyColorMode) -> PtyCommandOutput {
    run_osp_command_in_pty(
        &[
            "--json",
            "--defaults-only",
            "repl",
            "debug-complete",
            "--line",
            "config",
        ],
        color_mode,
    )
}

#[cfg(unix)]
#[test]
fn explicit_json_debug_complete_stays_plain_when_no_color_is_set() {
    let output = run_debug_complete_json_in_pty(ReplPtyColorMode::Plain);
    assert_eq!(output.status.exit_code(), 0, "command should succeed");
    assert!(!output.stdout.contains('\u{1b}'));

    let payload = parse_json_stdout(output.stdout.as_bytes());
    let matches = payload["matches"]
        .as_array()
        .expect("matches should render as an array");
    assert!(matches.iter().any(|item| item["label"] == "config"));
}

#[cfg(unix)]
#[test]
fn explicit_json_debug_complete_stays_plain_even_in_colored_pty_env() {
    let output = run_debug_complete_json_in_pty(ReplPtyColorMode::Always);
    assert_eq!(output.status.exit_code(), 0, "command should succeed");
    assert!(!output.stdout.contains('\u{1b}'));

    let payload = parse_json_stdout(output.stdout.as_bytes());
    let matches = payload["matches"]
        .as_array()
        .expect("matches should render as an array");
    assert!(matches.iter().any(|item| item["label"] == "config"));
}
