use crate::temp_support::make_temp_dir;
use std::io::Write;
use std::process::{Command, Output, Stdio};

fn run_basic_repl(input: &[u8]) -> Output {
    let home = make_temp_dir("osp-cli-repl-basic-home");

    Command::new(env!("CARGO_BIN_EXE_osp"))
        .env("HOME", home.path())
        .env("XDG_CONFIG_HOME", home.path().join(".config"))
        .env("XDG_CACHE_HOME", home.path().join(".cache"))
        .env("XDG_STATE_HOME", home.path().join(".local/state"))
        .env("TERM", "dumb")
        .env("NO_COLOR", "1")
        .arg("--defaults-only")
        .arg("--quiet")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            child
                .stdin
                .as_mut()
                .expect("stdin should be piped")
                .write_all(input)?;
            child.wait_with_output()
        })
        .expect("basic repl should run")
}

#[test]
fn repl_basic_mode_runs_help_and_exit_without_tty() {
    let output = run_basic_repl(b"help\nexit\n");
    assert!(
        output.status.success(),
        "basic repl should exit successfully; stderr:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("default> "));
    assert!(stdout.contains("Commands"));
    assert!(stdout.contains("help"));
    assert!(stdout.contains("exit"));
    assert!(stderr.contains("Warning: Input is not a terminal"));
}

#[test]
fn repl_basic_mode_exits_cleanly_on_immediate_eof() {
    let output = run_basic_repl(b"");
    assert!(
        output.status.success(),
        "basic repl should exit cleanly on EOF; stderr:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("default> "));
    assert!(stderr.contains("Warning: Input is not a terminal"));
}

#[test]
fn repl_basic_mode_restarts_after_refresh_without_tty() {
    let output = run_basic_repl(b"plugins refresh\nexit\n");
    assert!(
        output.status.success(),
        "basic repl restart path should succeed; stderr:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let prompt_count = stdout.matches("default> ").count();
    let warning_count = stderr.matches("Warning: Input is not a terminal").count();

    assert!(
        prompt_count >= 2,
        "expected refresh to restart and render the prompt again; stdout:\n{stdout}"
    );
    assert!(
        warning_count >= 2,
        "expected refresh restart to re-enter basic-mode fallback; stderr:\n{stderr}"
    );
}
