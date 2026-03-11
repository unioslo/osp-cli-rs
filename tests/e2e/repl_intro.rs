#![allow(missing_docs)]

#[cfg(unix)]
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
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
fn spawn_repl_with_intro_config(config: &str) -> PtySession {
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

    write_repl_config(&home, config);

    let mut cmd = CommandBuilder::new(bin);
    cmd.env("HOME", &home);
    cmd.env("XDG_CONFIG_HOME", home.join(".config"));
    cmd.env("XDG_CACHE_HOME", home.join(".cache"));
    cmd.env("XDG_STATE_HOME", home.join(".local/state"));
    cmd.env("TERM", "xterm-256color");
    cmd.env("LANG", "en_US.UTF-8");
    cmd.env_remove("NO_COLOR");
    cmd.env("OSP__REPL__HISTORY__ENABLED", "false");
    cmd.env("OSP__REPL__INPUT_MODE", "interactive");
    cmd.env("OSP_PLUGIN_PATH", &plugins);
    cmd.env("OSP_BUNDLED_PLUGIN_DIR", &plugins);
    cmd.env("COLUMNS", "80");
    cmd.env("LINES", "24");

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
                        .any(|window| window == cpr_request)
                        && let Ok(mut writer) = writer_clone.lock()
                    {
                        let _ = writer.write_all(b"\x1b[1;1R");
                        let _ = writer.flush();
                    }
                    output_clone
                        .lock()
                        .expect("output lock")
                        .push_str(&String::from_utf8_lossy(&buf[..n]));
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

fn output_snapshot(output: &Arc<Mutex<String>>, max_len: usize) -> String {
    let buf = output.lock().expect("output lock");
    if buf.len() <= max_len {
        buf.clone()
    } else {
        let start = buf
            .char_indices()
            .rev()
            .nth(max_len)
            .map(|(idx, _)| idx)
            .unwrap_or(0);
        buf[start..].to_string()
    }
}

#[cfg(unix)]
fn strip_terminal_noise(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\u{1b}' => {
                if matches!(chars.peek(), Some('[')) {
                    let _ = chars.next();
                    for next in chars.by_ref() {
                        if ('@'..='~').contains(&next) {
                            break;
                        }
                    }
                }
            }
            '\r' | '\n' => out.push('\n'),
            ch if ch.is_control() => {}
            other => out.push(other),
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
fn repl_startup_intro_uses_rich_chrome_and_help_under_auto_runtime() {
    let config = r#"
[default]
profile.default = "default"
ui.presentation = "expressive"
repl.intro = "full"
repl.simple_prompt = true
user.display_name = "demo"
"#;
    let mut session = spawn_repl_with_intro_config(config);

    assert!(
        wait_for_plain_output(&session.output, "default>", Duration::from_secs(3)),
        "expected REPL prompt after startup; output:\n{}",
        output_snapshot(&session.output, 4000),
    );

    let output = output_snapshot(&session.output, 4000);
    let plain = plain_output_snapshot(&session.output, 4000);
    assert!(
        output.contains("\x1b["),
        "expected ANSI-colored intro output on startup; output:\n{output}",
    );
    assert!(
        plain.contains("─ OSP "),
        "expected unicode intro divider on startup; output:\n{output}",
    );
    assert!(
        plain.contains("Keybindings"),
        "expected keybinding section in startup intro; output:\n{output}",
    );
    assert!(
        plain.contains("Pipes"),
        "expected pipe section in startup intro; output:\n{output}",
    );
    assert!(
        plain.contains("Usage"),
        "expected usage section in startup intro; output:\n{output}",
    );
    assert!(
        plain.contains("Commands"),
        "expected commands section in startup intro; output:\n{output}",
    );
    assert!(
        plain.contains("Show this command overview."),
        "expected help overview inside startup intro; output:\n{output}",
    );

    write_bytes(&mut session, b"exit\r");
    assert!(
        wait_for_exit(&mut session.child, Duration::from_secs(3)),
        "expected REPL to exit after `exit`; output:\n{}",
        output_snapshot(&session.output, 4000),
    );
}
