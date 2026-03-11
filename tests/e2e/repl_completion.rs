#![allow(missing_docs)]

#[cfg(unix)]
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
#[cfg(unix)]
use serde_json::Value;
#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::sync::{Arc, Mutex};
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
struct PtySession {
    child: Box<dyn portable_pty::Child + Send>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    output: Arc<Mutex<String>>,
    _home: PathBuf,
    _plugins: PathBuf,
}

#[cfg(unix)]
fn make_temp_dir(prefix: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be valid")
        .as_nanos();
    dir.push(format!("{prefix}-{nonce}"));
    std::fs::create_dir_all(&dir).expect("temp dir should be created");
    dir
}

#[cfg(unix)]
fn write_repl_config(home: &Path, config: &str) {
    let config_dir = home.join(".config").join("osp");
    std::fs::create_dir_all(&config_dir).expect("config dir should be created");
    std::fs::write(config_dir.join("config.toml"), config).expect("config should be written");
}

#[cfg(unix)]
fn spawn_repl(trace: bool) -> PtySession {
    spawn_repl_with_config(trace, None)
}

#[cfg(unix)]
fn spawn_repl_with_config(trace: bool, config: Option<&str>) -> PtySession {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("open pty");

    let home = make_temp_dir("osp-cli-pty-home");
    let plugins = make_temp_dir("osp-cli-pty-plugins");
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_osp"));

    if let Some(config) = config {
        write_repl_config(&home, config);
    }

    let mut cmd = CommandBuilder::new(bin);
    cmd.env("HOME", &home);
    cmd.env("XDG_CONFIG_HOME", home.join(".config"));
    cmd.env("XDG_CACHE_HOME", home.join(".cache"));
    cmd.env("XDG_STATE_HOME", home.join(".local/state"));
    cmd.env("TERM", "xterm-256color");
    cmd.env("NO_COLOR", "1");
    cmd.env("OSP__REPL__INTRO", "none");
    cmd.env("OSP__REPL__SIMPLE_PROMPT", "true");
    cmd.env("OSP__REPL__HISTORY__ENABLED", "false");
    // These PTY tests exercise completion behavior, not auto-detection of
    // cursor-position support. Force interactive mode so completion stays
    // deterministic even when the PTY harness races the CPR probe.
    cmd.env("OSP__REPL__INPUT_MODE", "interactive");
    cmd.env("OSP_PLUGIN_PATH", &plugins);
    cmd.env("OSP_BUNDLED_PLUGIN_DIR", &plugins);
    cmd.env("COLUMNS", "80");
    cmd.env("LINES", "24");
    if trace {
        cmd.env("OSP_REPL_TRACE_COMPLETION", "1");
    }

    let child = pair.slave.spawn_command(cmd).expect("spawn osp repl");
    let mut reader = pair.master.try_clone_reader().expect("clone reader");
    let writer = Arc::new(Mutex::new(pair.master.take_writer().expect("take writer")));

    let output = Arc::new(Mutex::new(String::new()));
    let output_clone = Arc::clone(&output);
    let writer_clone = Arc::clone(&writer);
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let cpr_request = [0x1b, 0x5b, 0x36, 0x6e];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if buf[..n]
                        .windows(cpr_request.len())
                        .any(|w| w == cpr_request)
                        && let Ok(mut writer) = writer_clone.lock()
                    {
                        let _ = writer.write_all(b"\x1b[1;1R");
                        let _ = writer.flush();
                    }
                    let chunk = String::from_utf8_lossy(&buf[..n]);
                    output_clone.lock().expect("output lock").push_str(&chunk);
                }
                Err(_) => break,
            }
        }
    });

    PtySession {
        child,
        writer,
        output,
        _home: home,
        _plugins: plugins,
    }
}

#[cfg(unix)]
fn output_len(output: &Arc<Mutex<String>>) -> usize {
    output.lock().expect("output lock").len()
}

#[cfg(unix)]
fn wait_for_output_since(
    output: &Arc<Mutex<String>>,
    start: usize,
    needle: &str,
    timeout: Duration,
) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        {
            let buf = output.lock().expect("output lock");
            if start < buf.len() && buf[start..].contains(needle) {
                return true;
            }
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(unix)]
fn output_snapshot(output: &Arc<Mutex<String>>, max_len: usize) -> String {
    let buf = output.lock().expect("output lock");
    if buf.len() <= max_len {
        buf.clone()
    } else {
        let mut start = buf.len().saturating_sub(max_len);
        while start < buf.len() && !buf.is_char_boundary(start) {
            start += 1;
        }
        buf[start..].to_string()
    }
}

#[cfg(unix)]
fn strip_terminal_noise(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut idx = 0usize;

    while idx < bytes.len() {
        match bytes[idx] {
            b'\x1b' => {
                idx += 1;
                if idx >= bytes.len() {
                    break;
                }
                match bytes[idx] {
                    b'[' => {
                        idx += 1;
                        while idx < bytes.len() {
                            let byte = bytes[idx];
                            idx += 1;
                            if (b'@'..=b'~').contains(&byte) {
                                break;
                            }
                        }
                    }
                    b']' => {
                        idx += 1;
                        while idx < bytes.len() {
                            if bytes[idx] == b'\x07' {
                                idx += 1;
                                break;
                            }
                            if bytes[idx] == b'\x1b'
                                && idx + 1 < bytes.len()
                                && bytes[idx + 1] == b'\\'
                            {
                                idx += 2;
                                break;
                            }
                            idx += 1;
                        }
                    }
                    _ => idx += 1,
                }
            }
            b'\r' | b'\n' => {
                out.push('\n');
                idx += 1;
            }
            byte if byte.is_ascii_control() => {
                idx += 1;
            }
            _ => {
                let ch = text[idx..]
                    .chars()
                    .next()
                    .expect("utf-8 cursor should stay on a character boundary");
                out.push(ch);
                idx += ch.len_utf8();
            }
        }
    }

    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(unix)]
fn plain_output_snapshot(output: &Arc<Mutex<String>>, max_len: usize) -> String {
    strip_terminal_noise(&output_snapshot(output, max_len))
}

#[cfg(unix)]
fn wait_for_plain_output(output: &Arc<Mutex<String>>, needle: &str, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        {
            let buf = output.lock().expect("output lock");
            if strip_terminal_noise(&buf).contains(needle) {
                return true;
            }
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(unix)]
fn trace_events(output: &str) -> Vec<Value> {
    output
        .lines()
        .filter_map(|line| line.find('{').map(|idx| &line[idx..]))
        .filter_map(|json| serde_json::from_str::<Value>(json).ok())
        .collect()
}

#[cfg(unix)]
fn wait_for_trace_event<F>(output: &Arc<Mutex<String>>, predicate: F, timeout: Duration) -> bool
where
    F: Fn(&Value) -> bool,
{
    let deadline = Instant::now() + timeout;
    loop {
        {
            let buf = output.lock().expect("output lock");
            if trace_events(&buf).iter().any(&predicate) {
                return true;
            }
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(unix)]
fn write_bytes(session: &mut PtySession, bytes: &[u8]) {
    let mut writer = session.writer.lock().expect("writer lock");
    writer.write_all(bytes).expect("write to pty");
    writer.flush().expect("flush pty");
}

#[cfg(unix)]
fn wait_for_exit(child: &mut Box<dyn portable_pty::Child + Send>, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) => {}
            Err(_) => return false,
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(unix)]
#[test]
fn repl_tab_opens_menu_and_moves_selection() {
    let mut session = spawn_repl(true);

    let start = output_len(&session.output);
    write_bytes(&mut session, b"\t");
    assert!(
        wait_for_output_since(
            &session.output,
            start,
            "\"selected_index\":0",
            Duration::from_secs(3)
        ),
        "expected menu activation trace; output:\n{}",
        output_snapshot(&session.output, 2000),
    );

    let start = output_len(&session.output);
    write_bytes(&mut session, b"\t");
    assert!(
        wait_for_output_since(
            &session.output,
            start,
            "\"event\":\"cycle\"",
            Duration::from_secs(3)
        ),
        "expected cycle trace on Tab; output:\n{}",
        output_snapshot(&session.output, 2000),
    );
    assert!(
        wait_for_output_since(
            &session.output,
            start,
            "\"buffer_after\":\"help\"",
            Duration::from_secs(3)
        ),
        "expected buffer to update on cycle; output:\n{}",
        output_snapshot(&session.output, 2000),
    );

    write_bytes(&mut session, b"\t");
    assert!(
        wait_for_output_since(
            &session.output,
            start,
            "\"selected_index\":1",
            Duration::from_secs(3)
        ),
        "expected selection to move on third Tab; output:\n{}",
        output_snapshot(&session.output, 2000),
    );

    write_bytes(&mut session, b"\x1b"); // close menu
    write_bytes(&mut session, b"\x03"); // cancel line
    write_bytes(&mut session, b"exit\r\r");
    if !wait_for_exit(&mut session.child, Duration::from_secs(2)) {
        let _ = session.child.kill();
        let _ = session.child.wait();
    }
}

#[cfg(unix)]
#[test]
fn repl_tab_completes_single_match_and_exits() {
    let mut session = spawn_repl(false);

    write_bytes(&mut session, b"ex");
    write_bytes(&mut session, b"\t");
    write_bytes(&mut session, b"\t");
    assert!(
        wait_for_plain_output(&session.output, "exit default>", Duration::from_secs(5)),
        "expected tab completion to render `exit` in the prompt; output:\n{}",
        plain_output_snapshot(&session.output, 4000),
    );
    write_bytes(&mut session, b"\r");
    write_bytes(&mut session, b"\r");

    if !wait_for_exit(&mut session.child, Duration::from_secs(3)) {
        let _ = session.child.kill();
        let _ = session.child.wait();
        panic!("expected repl to exit after completion");
    }
    assert!(
        !plain_output_snapshot(&session.output, 4000).contains("unrecognized subcommand"),
        "expected tab completion to turn `ex` into `exit`; output:\n{}",
        plain_output_snapshot(&session.output, 4000),
    );
}

#[cfg(unix)]
#[test]
fn repl_tab_accepts_single_visible_completion() {
    let mut session = spawn_repl(false);

    write_bytes(&mut session, b"the");
    write_bytes(&mut session, b"\t");
    write_bytes(&mut session, b" show dracula --value\r");
    assert!(
        wait_for_plain_output(
            &session.output,
            "theme show dracula --value",
            Duration::from_secs(5)
        ),
        "expected tab to accept `theme` before the follow-up command text; output:\n{}",
        plain_output_snapshot(&session.output, 4000),
    );

    write_bytes(&mut session, b"\x03");
    write_bytes(&mut session, b"exit\r\r");
    if !wait_for_exit(&mut session.child, Duration::from_secs(3)) {
        let _ = session.child.kill();
        let _ = session.child.wait();
    }
}

#[cfg(unix)]
#[test]
fn repl_completion_respects_leading_invocation_flags() {
    let mut session = spawn_repl(false);

    write_bytes(&mut session, b"--json he");

    write_bytes(&mut session, b"\t");
    assert!(
        wait_for_plain_output(&session.output, "--json help", Duration::from_secs(5)),
        "expected completion to preserve the invocation flag in the prompt; output:\n{}",
        plain_output_snapshot(&session.output, 4000),
    );

    let start = output_len(&session.output);
    write_bytes(&mut session, b"\r");
    assert!(
        wait_for_output_since(
            &session.output,
            start,
            "\"name\": \"config\"",
            Duration::from_secs(5)
        ),
        "expected completion to preserve invocation flag and run `--json help`; output:\n{}",
        output_snapshot(&session.output, 4000),
    );
    assert!(
        wait_for_output_since(
            &session.output,
            start,
            "\"short_help\": \"Inspect and edit runtime config\"",
            Duration::from_secs(5)
        ),
        "expected JSON help payload after completing `he` to `help`; output:\n{}",
        output_snapshot(&session.output, 4000),
    );

    write_bytes(&mut session, b"\x03");
    write_bytes(&mut session, b"exit\r\r");
    if !wait_for_exit(&mut session.child, Duration::from_secs(3)) {
        let _ = session.child.kill();
        let _ = session.child.wait();
    }
}

#[cfg(unix)]
#[test]
fn repl_completion_resolves_fixed_key_aliases_end_to_end() {
    let mut session = spawn_repl_with_config(
        false,
        Some(
            r#"
[default]
alias.pres = "config set ui.presentation"
"#,
        ),
    );

    write_bytes(&mut session, b"pres au");

    write_bytes(&mut session, b"\t");
    assert!(
        wait_for_plain_output(&session.output, "pres austere", Duration::from_secs(6)),
        "expected alias completion to resolve through the alias command context; output:\n{}",
        plain_output_snapshot(&session.output, 4000),
    );

    write_bytes(&mut session, b"\x03");
    write_bytes(&mut session, b"exit\r\r");
    if !wait_for_exit(&mut session.child, Duration::from_secs(3)) {
        let _ = session.child.kill();
        let _ = session.child.wait();
    }
}

#[cfg(unix)]
#[test]
fn repl_enter_submits_current_line_without_accepting_menu_completion() {
    let mut session = spawn_repl(true);

    write_bytes(&mut session, b"history ");

    let start = output_len(&session.output);
    write_bytes(&mut session, b"\t");
    assert!(
        wait_for_output_since(
            &session.output,
            start,
            "\"selected_index\":0",
            Duration::from_secs(3)
        ),
        "expected menu activation trace; output:\n{}",
        output_snapshot(&session.output, 2000),
    );

    let start = output_len(&session.output);
    write_bytes(&mut session, b"\r");
    assert!(
        wait_for_output_since(
            &session.output,
            start,
            "history <COMMAND>",
            Duration::from_secs(3)
        ),
        "expected Enter to submit the current line instead of accepting `list`; output:\n{}",
        output_snapshot(&session.output, 4000),
    );
    assert!(
        !output_snapshot(&session.output, 4000).contains("History is disabled."),
        "expected Enter not to accept the first completion; output:\n{}",
        output_snapshot(&session.output, 4000),
    );

    write_bytes(&mut session, b"\x03");
    write_bytes(&mut session, b"exit\r\r");
    if !wait_for_exit(&mut session.child, Duration::from_secs(3)) {
        let _ = session.child.kill();
        let _ = session.child.wait();
    }
}

#[cfg(unix)]
#[test]
fn repl_theme_show_menu_omits_global_flags_end_to_end() {
    let mut session = spawn_repl(true);

    let start = output_len(&session.output);
    write_bytes(&mut session, b"theme show ");
    assert!(
        wait_for_output_since(
            &session.output,
            start,
            "theme show ",
            Duration::from_secs(3)
        ),
        "expected typed input to be echoed before completion; output:\n{}",
        output_snapshot(&session.output, 4000),
    );

    write_bytes(&mut session, b"\t");
    assert!(
        wait_for_trace_event(
            &session.output,
            |event| {
                event.get("event").and_then(Value::as_str) == Some("complete")
                    && event.get("line").and_then(Value::as_str) == Some("theme show ")
                    && event
                        .get("matches")
                        .and_then(Value::as_array)
                        .map(|matches| {
                            matches
                                .iter()
                                .any(|item| item.as_str() == Some("catppuccin"))
                        })
                        .unwrap_or(false)
            },
            Duration::from_secs(5),
        ),
        "expected theme-name completion trace for `theme show `; output:\n{}",
        output_snapshot(&session.output, 8000),
    );

    let output = output_snapshot(&session.output, 4000);
    assert!(
        output.contains("\"matches\":[\"catppuccin\",\"dracula\",\"gruvbox\",\"molokai\",\"nord\",\"plain\",\"rose-pine-moon\",\"tokyonight\"]"),
        "expected only theme names after `theme show `; output:\n{}",
        output,
    );

    write_bytes(&mut session, b"\x03");
    write_bytes(&mut session, b"exit\r\r");
    if !wait_for_exit(&mut session.child, Duration::from_secs(3)) {
        let _ = session.child.kill();
        let _ = session.child.wait();
    }
}

#[cfg(unix)]
#[test]
fn repl_theme_show_tab_cycle_keeps_menu_anchor_stable_end_to_end() {
    let mut session = spawn_repl(true);

    let start = output_len(&session.output);
    write_bytes(&mut session, b"theme show ");
    assert!(
        wait_for_output_since(
            &session.output,
            start,
            "theme show ",
            Duration::from_secs(3)
        ),
        "expected typed input to be echoed before completion; output:\n{}",
        output_snapshot(&session.output, 4000),
    );

    write_bytes(&mut session, b"\t");
    assert!(
        wait_for_trace_event(
            &session.output,
            |event| {
                event.get("event").and_then(Value::as_str) == Some("complete")
                    && event.get("line").and_then(Value::as_str) == Some("theme show ")
                    && event
                        .get("matches")
                        .and_then(Value::as_array)
                        .map(|matches| {
                            matches
                                .iter()
                                .any(|item| item.as_str() == Some("catppuccin"))
                        })
                        .unwrap_or(false)
            },
            Duration::from_secs(5),
        ),
        "expected theme-name completion trace for `theme show `; output:\n{}",
        output_snapshot(&session.output, 8000),
    );

    let start = output_len(&session.output);
    write_bytes(&mut session, b"\t\t");
    assert!(
        wait_for_output_since(
            &session.output,
            start,
            "\"buffer_after\":\"theme show dracula\"",
            Duration::from_secs(3)
        ),
        "expected third Tab to cycle to the next theme value; output:\n{}",
        output_snapshot(&session.output, 4000),
    );

    let output = output_snapshot(&session.output, 8000);
    let traces = trace_events(&output);
    let activation_indent = traces
        .iter()
        .find(|event| {
            event.get("event").and_then(Value::as_str) == Some("complete")
                && event.get("line").and_then(Value::as_str) == Some("theme show ")
                && event
                    .get("matches")
                    .and_then(Value::as_array)
                    .map(|matches| {
                        matches
                            .iter()
                            .any(|item| item.as_str() == Some("catppuccin"))
                    })
                    .unwrap_or(false)
        })
        .and_then(|event| event.get("menu_indent"))
        .and_then(Value::as_u64)
        .expect("activation trace should include menu_indent");
    let dracula_cycle_indent = traces
        .iter()
        .find(|event| {
            event.get("buffer_after").and_then(Value::as_str) == Some("theme show dracula")
        })
        .and_then(|event| event.get("menu_indent"))
        .and_then(Value::as_u64)
        .expect("dracula cycle trace should include menu_indent");
    let dracula_complete_indent = traces
        .iter()
        .rev()
        .find(|event| {
            event.get("event").and_then(Value::as_str) == Some("complete")
                && event.get("line").and_then(Value::as_str) == Some("theme show dracula")
        })
        .and_then(|event| event.get("menu_indent"))
        .and_then(Value::as_u64)
        .expect("dracula complete trace should include menu_indent");

    assert_eq!(
        dracula_cycle_indent, activation_indent,
        "expected tab-cycling to keep the menu anchor fixed; output:\n{}",
        output,
    );
    assert_eq!(
        dracula_complete_indent, activation_indent,
        "expected follow-up render after cycling to keep the same menu anchor; output:\n{}",
        output,
    );

    write_bytes(&mut session, b"\x03");
    write_bytes(&mut session, b"exit\r\r");
    if !wait_for_exit(&mut session.child, Duration::from_secs(3)) {
        let _ = session.child.kill();
        let _ = session.child.wait();
    }
}

#[cfg(unix)]
#[test]
fn repl_close_menu_keeps_typed_input() {
    let mut session = spawn_repl(true);

    let start = output_len(&session.output);
    write_bytes(&mut session, b"\t");
    assert!(
        wait_for_output_since(
            &session.output,
            start,
            "\"selected_index\":0",
            Duration::from_secs(3)
        ),
        "expected menu activation trace; output:\n{}",
        output_snapshot(&session.output, 2000),
    );

    let start = output_len(&session.output);
    write_bytes(&mut session, b"\x1b");
    write_bytes(&mut session, b"co");
    assert!(
        wait_for_output_since(&session.output, start, "co", Duration::from_secs(3)),
        "expected typed input after closing menu; output:\n{}",
        output_snapshot(&session.output, 2000),
    );

    write_bytes(&mut session, b"\x03");
    write_bytes(&mut session, b"exit\r\r");
    if !wait_for_exit(&mut session.child, Duration::from_secs(3)) {
        let _ = session.child.kill();
        let _ = session.child.wait();
    }
}
