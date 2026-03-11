#![allow(missing_docs)]

#[cfg(unix)]
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::path::PathBuf;
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
fn spawn_repl() -> PtySession {
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
        buf[buf.len().saturating_sub(max_len)..].to_string()
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
fn repl_starts_runs_help_and_exits_end_to_end() {
    let mut session = spawn_repl();

    let start = output_len(&session.output);
    assert!(
        wait_for_output_since(&session.output, start, "default>", Duration::from_secs(3)),
        "expected prompt output after REPL startup; output:\n{}",
        output_snapshot(&session.output, 2000),
    );

    let start = output_len(&session.output);
    write_bytes(&mut session, b"help\r");
    assert!(
        wait_for_output_since(&session.output, start, "Commands", Duration::from_secs(3)),
        "expected help overview after `help`; output:\n{}",
        output_snapshot(&session.output, 2000),
    );
    assert!(
        wait_for_output_since(&session.output, start, "config", Duration::from_secs(3)),
        "expected command overview after `help`; output:\n{}",
        output_snapshot(&session.output, 2000),
    );

    write_bytes(&mut session, b"exit\r");
    assert!(
        wait_for_exit(&mut session.child, Duration::from_secs(3)),
        "expected REPL to exit after `exit`; output:\n{}",
        output_snapshot(&session.output, 2000),
    );
}
