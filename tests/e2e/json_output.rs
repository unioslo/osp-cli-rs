#![allow(missing_docs)]

#[cfg(unix)]
use crate::temp_support::{TestTempDir, make_temp_dir};
#[cfg(unix)]
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
#[cfg(unix)]
use serde_json::Value;
#[cfg(unix)]
use std::io::Read;
#[cfg(unix)]
use std::path::PathBuf;

#[cfg(unix)]
struct PtyCommandOutput {
    stdout: String,
    status: portable_pty::ExitStatus,
    _home: TestTempDir,
    _plugins: TestTempDir,
}

#[cfg(unix)]
fn run_debug_complete_json_in_pty(colored: bool) -> PtyCommandOutput {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("open pty");

    let home = make_temp_dir("osp-cli-json-pty-home");
    let plugins = make_temp_dir("osp-cli-json-pty-plugins");
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_osp"));

    let mut cmd = CommandBuilder::new(bin);
    cmd.env_clear();
    cmd.env("PATH", "/usr/bin:/bin");
    cmd.env("LANG", "C.UTF-8");
    cmd.env("HOME", &home);
    cmd.env("XDG_CONFIG_HOME", home.join(".config"));
    cmd.env("XDG_CACHE_HOME", home.join(".cache"));
    cmd.env("XDG_STATE_HOME", home.join(".local/state"));
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    cmd.env("COLUMNS", "80");
    cmd.env("LINES", "24");
    cmd.env("OSP_PLUGIN_PATH", &plugins);
    cmd.env("OSP_BUNDLED_PLUGIN_DIR", &plugins);
    if colored {
        cmd.env("OSP__UI__COLOR__MODE", "always");
    } else {
        cmd.env("NO_COLOR", "1");
    }
    cmd.args([
        "--json",
        "--defaults-only",
        "repl",
        "debug-complete",
        "--line",
        "config",
    ]);

    let mut child = pair.slave.spawn_command(cmd).expect("spawn osp");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("clone reader");
    let mut stdout = String::new();
    reader.read_to_string(&mut stdout).expect("read pty stdout");
    let status = child.wait().expect("wait for child");

    PtyCommandOutput {
        stdout,
        status,
        _home: home,
        _plugins: plugins,
    }
}

#[cfg(unix)]
fn assert_parseable_json(output: &str) -> Value {
    serde_json::from_str(output).unwrap_or_else(|err| {
        panic!("stdout should be valid json: {err}\n{output}");
    })
}

#[cfg(unix)]
#[test]
fn explicit_json_debug_complete_stays_plain_when_no_color_is_set() {
    let output = run_debug_complete_json_in_pty(false);
    assert_eq!(output.status.exit_code(), 0, "command should succeed");
    assert!(!output.stdout.contains('\u{1b}'));

    let payload = assert_parseable_json(&output.stdout);
    let matches = payload["matches"]
        .as_array()
        .expect("matches should render as an array");
    assert!(matches.iter().any(|item| item["label"] == "config"));
}

#[cfg(unix)]
#[test]
fn explicit_json_debug_complete_stays_plain_even_in_colored_pty_env() {
    let output = run_debug_complete_json_in_pty(true);
    assert_eq!(output.status.exit_code(), 0, "command should succeed");
    assert!(!output.stdout.contains('\u{1b}'));

    let payload = assert_parseable_json(&output.stdout);
    let matches = payload["matches"]
        .as_array()
        .expect("matches should render as an array");
    assert!(matches.iter().any(|item| item["label"] == "config"));
}
