#[cfg(unix)]
pub(crate) use crate::output_support::{first_json_row, parse_json_stdout};
#[cfg(unix)]
use crate::temp_support::{TestTempDir, make_temp_dir};
#[cfg(unix)]
use assert_cmd::Command;
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
pub(crate) fn osp_command(home: &Path) -> Command {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("osp"));
    cmd.env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("TERM", "xterm-256color")
        .env("LANG", "C.UTF-8")
        .env("NO_COLOR", "1");
    for (key, value) in crate::test_env::isolated_env(home) {
        cmd.env(key, value);
    }
    cmd
}

#[cfg(unix)]
pub(crate) fn write_config(home: &Path, config: &str) {
    let config_dir = home.join(".config").join("osp");
    std::fs::create_dir_all(&config_dir).expect("config dir should be created");
    std::fs::write(config_dir.join("config.toml"), config).expect("config should be written");
}

#[cfg(unix)]
pub(crate) fn stderr_utf8(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).expect("stderr should be utf-8")
}

#[cfg(unix)]
pub(crate) fn write_executable_script(path: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::write(path, script).expect("script should be written");
    let mut perms = std::fs::metadata(path)
        .expect("script metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).expect("script should be executable");
}

#[cfg(unix)]
pub(crate) fn write_table_plugin(
    dir: &Path,
    plugin_id: &str,
    command_name: &str,
    message: &str,
) -> PathBuf {
    let plugin_path = dir.join(format!("osp-{plugin_id}"));
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{plugin_id}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{command_name}","about":"{plugin_id} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"{message}"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
        plugin_id = plugin_id,
        command_name = command_name,
        message = message,
    );
    write_executable_script(&plugin_path, &script);
    plugin_path
}

#[cfg(unix)]
pub(crate) fn write_nonzero_plugin(
    dir: &Path,
    plugin_id: &str,
    command_name: &str,
    status_code: i32,
    stderr: &str,
) -> PathBuf {
    let plugin_path = dir.join(format!("osp-{plugin_id}"));
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{plugin_id}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{command_name}","about":"{plugin_id} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

echo "{stderr}" >&2
exit {status_code}
"#,
        plugin_id = plugin_id,
        command_name = command_name,
        stderr = stderr,
        status_code = status_code,
    );
    write_executable_script(&plugin_path, &script);
    plugin_path
}

#[cfg(unix)]
pub(crate) fn write_timeout_plugin(dir: &Path, plugin_id: &str, command_name: &str) -> PathBuf {
    let plugin_path = dir.join(format!("osp-{plugin_id}"));
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{plugin_id}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{command_name}","about":"{plugin_id} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

sleep 1
cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"late"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
        plugin_id = plugin_id,
        command_name = command_name,
    );
    write_executable_script(&plugin_path, &script);
    plugin_path
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ReplPtyColorMode {
    #[default]
    Plain,
    Always,
}

#[cfg(unix)]
#[derive(Clone, Debug)]
pub(crate) struct ReplPtyConfig<'a> {
    color_mode: ReplPtyColorMode,
    simple_prompt: bool,
    config: Option<&'a str>,
    plugins_dir: Option<&'a Path>,
    intro_override: Option<&'a str>,
}

#[cfg(unix)]
impl Default for ReplPtyConfig<'_> {
    fn default() -> Self {
        Self {
            color_mode: ReplPtyColorMode::Plain,
            simple_prompt: true,
            config: None,
            plugins_dir: None,
            intro_override: Some("none"),
        }
    }
}

#[cfg(unix)]
impl<'a> ReplPtyConfig<'a> {
    pub(crate) fn with_color_mode(mut self, color_mode: ReplPtyColorMode) -> Self {
        self.color_mode = color_mode;
        self
    }

    pub(crate) fn with_simple_prompt(mut self, simple_prompt: bool) -> Self {
        self.simple_prompt = simple_prompt;
        self
    }

    pub(crate) fn with_config(mut self, config: &'a str) -> Self {
        self.config = Some(config);
        self
    }

    pub(crate) fn with_plugins_dir(mut self, plugins_dir: &'a Path) -> Self {
        self.plugins_dir = Some(plugins_dir);
        self
    }

    pub(crate) fn with_intro_override(mut self, intro_override: Option<&'a str>) -> Self {
        self.intro_override = intro_override;
        self
    }
}

#[cfg(unix)]
pub(crate) struct ReplPtySession {
    child: Box<dyn portable_pty::Child + Send>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    output: Arc<Mutex<String>>,
    _home: TestTempDir,
    _plugins: Option<TestTempDir>,
}

#[cfg(unix)]
pub(crate) struct PtyCommandOutput {
    pub(crate) stdout: String,
    pub(crate) status: portable_pty::ExitStatus,
    _home: TestTempDir,
    _plugins: TestTempDir,
}

#[cfg(unix)]
fn osp_pty_command_builder(
    home: &Path,
    plugins_path: &Path,
    color_mode: ReplPtyColorMode,
) -> CommandBuilder {
    let mut cmd = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_osp")));
    cmd.env_clear();
    cmd.env("PATH", "/usr/bin:/bin");
    cmd.env("LANG", "C.UTF-8");
    cmd.env("HOME", home);
    cmd.env("XDG_CONFIG_HOME", home.join(".config"));
    cmd.env("XDG_CACHE_HOME", home.join(".cache"));
    cmd.env("XDG_STATE_HOME", home.join(".local/state"));
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLUMNS", "80");
    cmd.env("LINES", "24");
    cmd.env("OSP_PLUGIN_PATH", plugins_path);
    cmd.env("OSP_BUNDLED_PLUGIN_DIR", plugins_path);
    match color_mode {
        ReplPtyColorMode::Always => {
            cmd.env_remove("NO_COLOR");
            cmd.env("OSP__UI__COLOR__MODE", "always");
        }
        ReplPtyColorMode::Plain => {
            cmd.env("NO_COLOR", "1");
        }
    }
    cmd
}

#[cfg(unix)]
pub(crate) fn run_osp_command_in_pty(
    args: &[&str],
    color_mode: ReplPtyColorMode,
) -> PtyCommandOutput {
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
    let mut cmd = osp_pty_command_builder(&home, &plugins, color_mode);
    cmd.args(args);

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
impl ReplPtySession {
    pub(crate) fn spawn(config: ReplPtyConfig<'_>) -> Self {
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
        if let Some(contents) = config.config {
            write_config(&home, contents);
        }

        let (plugins_path, plugins_temp) = match config.plugins_dir {
            Some(path) => (path.to_path_buf(), None),
            None => {
                let temp = make_temp_dir("osp-cli-pty-plugins");
                (temp.to_path_buf(), Some(temp))
            }
        };

        let mut cmd = osp_pty_command_builder(&home, &plugins_path, config.color_mode);
        if let Some(intro) = config.intro_override {
            cmd.env("OSP__REPL__INTRO", intro);
        }
        cmd.env(
            "OSP__REPL__SIMPLE_PROMPT",
            if config.simple_prompt {
                "true"
            } else {
                "false"
            },
        );
        cmd.env("OSP__REPL__HISTORY__ENABLED", "false");
        // PTY tests exercise interactive rendering paths directly and should
        // not depend on cursor-probe timing.
        cmd.env("OSP__REPL__INPUT_MODE", "interactive");
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
                        let chunk = String::from_utf8_lossy(&buf[..n]);
                        output_clone.lock().expect("output lock").push_str(&chunk);
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            child,
            writer,
            output,
            _home: home,
            _plugins: plugins_temp,
        }
    }

    pub(crate) fn output_len(&self) -> usize {
        self.output.lock().expect("output lock").len()
    }

    pub(crate) fn output_since(&self, start: usize) -> String {
        let buf = self.output.lock().expect("output lock");
        if start >= buf.len() {
            String::new()
        } else {
            buf[start..].to_string()
        }
    }

    pub(crate) fn output_snapshot(&self, max_len: usize) -> String {
        let buf = self.output.lock().expect("output lock");
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

    pub(crate) fn plain_output_snapshot(&self, max_len: usize) -> String {
        strip_terminal_noise(&self.output_snapshot(max_len))
    }

    pub(crate) fn write_bytes(&mut self, bytes: &[u8]) {
        let mut writer = self.writer.lock().expect("writer lock");
        writer.write_all(bytes).expect("write to pty");
        writer.flush().expect("flush pty");
    }

    pub(crate) fn wait_for_output_since(
        &self,
        start: usize,
        needle: &str,
        timeout: Duration,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            {
                let buf = self.output.lock().expect("output lock");
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

    pub(crate) fn wait_for_plain_output(&self, needle: &str, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            {
                let buf = self.output.lock().expect("output lock");
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

    pub(crate) fn wait_for_plain_output_since(
        &self,
        start: usize,
        needle: &str,
        timeout: Duration,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            {
                let buf = self.output.lock().expect("output lock");
                if start < buf.len() && strip_terminal_noise(&buf[start..]).contains(needle) {
                    return true;
                }
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    pub(crate) fn wait_for_exit(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            match self.child.try_wait() {
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

    pub(crate) fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(unix)]
pub(crate) fn strip_terminal_noise(text: &str) -> String {
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
pub(crate) fn strip_ansi_preserve_newlines(text: &str) -> String {
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
            '\r' => {}
            ch if ch.is_control() && ch != '\n' => {}
            other => out.push(other),
        }
    }

    out
}
