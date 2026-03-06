use std::fmt::{Display, Formatter};
use std::io::{IsTerminal, Write};
use std::process::{Command, Stdio};

use crate::{Document, RenderSettings, render_document_for_copy};

#[derive(Debug, Clone)]
pub struct ClipboardService {
    prefer_osc52: bool,
}

impl Default for ClipboardService {
    fn default() -> Self {
        Self { prefer_osc52: true }
    }
}

#[derive(Debug)]
pub enum ClipboardError {
    NoBackendAvailable {
        attempts: Vec<String>,
    },
    SpawnFailed {
        command: String,
        reason: String,
    },
    CommandFailed {
        command: String,
        status: i32,
        stderr: String,
    },
    Io(String),
}

impl Display for ClipboardError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ClipboardError::NoBackendAvailable { attempts } => {
                write!(
                    f,
                    "no clipboard backend available (tried: {})",
                    attempts.join(", ")
                )
            }
            ClipboardError::SpawnFailed { command, reason } => {
                write!(f, "failed to start clipboard command `{command}`: {reason}")
            }
            ClipboardError::CommandFailed {
                command,
                status,
                stderr,
            } => {
                if stderr.trim().is_empty() {
                    write!(
                        f,
                        "clipboard command `{command}` failed with status {status}"
                    )
                } else {
                    write!(
                        f,
                        "clipboard command `{command}` failed with status {status}: {}",
                        stderr.trim()
                    )
                }
            }
            ClipboardError::Io(reason) => write!(f, "clipboard I/O error: {reason}"),
        }
    }
}

impl std::error::Error for ClipboardError {}

impl ClipboardService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_osc52(mut self, enabled: bool) -> Self {
        self.prefer_osc52 = enabled;
        self
    }

    pub fn copy_text(&self, text: &str) -> Result<(), ClipboardError> {
        let mut attempts = Vec::new();

        if self.prefer_osc52 && std::io::stdout().is_terminal() && osc52_enabled() {
            let max_bytes = osc52_max_bytes();
            let encoded_len = base64_encoded_len(text.len());
            if encoded_len <= max_bytes {
                attempts.push("osc52".to_string());
                self.copy_via_osc52(text)?;
                return Ok(());
            }
            attempts.push(format!("osc52 (payload {encoded_len} > {max_bytes})"));
        }

        for backend in platform_backends() {
            attempts.push(backend.command.to_string());
            match copy_via_command(backend.command, backend.args, text) {
                Ok(()) => return Ok(()),
                Err(ClipboardError::SpawnFailed { .. }) => continue,
                Err(error) => return Err(error),
            }
        }

        Err(ClipboardError::NoBackendAvailable { attempts })
    }

    pub fn copy_document(
        &self,
        document: &Document,
        settings: &RenderSettings,
    ) -> Result<(), ClipboardError> {
        let text = render_document_for_copy(document, settings);
        self.copy_text(&text)
    }

    fn copy_via_osc52(&self, text: &str) -> Result<(), ClipboardError> {
        let encoded = base64_encode(text.as_bytes());
        let payload = format!("\x1b]52;c;{encoded}\x07");
        std::io::stdout()
            .write_all(payload.as_bytes())
            .map_err(|err| ClipboardError::Io(err.to_string()))?;
        std::io::stdout()
            .flush()
            .map_err(|err| ClipboardError::Io(err.to_string()))?;
        Ok(())
    }
}

struct ClipboardBackend {
    command: &'static str,
    args: &'static [&'static str],
}

fn platform_backends() -> Vec<ClipboardBackend> {
    let mut backends = Vec::new();

    if cfg!(target_os = "macos") {
        backends.push(ClipboardBackend {
            command: "pbcopy",
            args: &[],
        });
        return backends;
    }

    if cfg!(target_os = "windows") {
        backends.push(ClipboardBackend {
            command: "clip",
            args: &[],
        });
        return backends;
    }

    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        backends.push(ClipboardBackend {
            command: "wl-copy",
            args: &[],
        });
    }

    backends.push(ClipboardBackend {
        command: "xclip",
        args: &["-selection", "clipboard"],
    });
    backends.push(ClipboardBackend {
        command: "xsel",
        args: &["--clipboard", "--input"],
    });

    backends
}

fn copy_via_command(command: &str, args: &[&str], text: &str) -> Result<(), ClipboardError> {
    let mut child = Command::new(command)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| ClipboardError::SpawnFailed {
            command: command.to_string(),
            reason: err.to_string(),
        })?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|err| ClipboardError::Io(err.to_string()))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|err| ClipboardError::Io(err.to_string()))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(ClipboardError::CommandFailed {
            command: command.to_string(),
            status: output.status.code().unwrap_or(1),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }
}

const BASE64_TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const OSC52_MAX_BYTES_DEFAULT: usize = 100_000;

fn osc52_enabled() -> bool {
    match std::env::var("OSC52") {
        Ok(value) => {
            let value = value.trim().to_ascii_lowercase();
            !(value == "0" || value == "false" || value == "off")
        }
        Err(_) => true,
    }
}

fn osc52_max_bytes() -> usize {
    std::env::var("OSC52_MAX_BYTES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(OSC52_MAX_BYTES_DEFAULT)
}

fn base64_encoded_len(input_len: usize) -> usize {
    input_len.saturating_add(2).div_ceil(3).saturating_mul(4)
}

fn base64_encode(input: &[u8]) -> String {
    if input.is_empty() {
        return String::new();
    }

    let mut output = String::with_capacity(input.len().div_ceil(3) * 4);
    let mut index = 0usize;

    while index < input.len() {
        let b0 = input[index];
        let b1 = input.get(index + 1).copied().unwrap_or(0);
        let b2 = input.get(index + 2).copied().unwrap_or(0);

        let chunk = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);

        let i0 = ((chunk >> 18) & 0x3f) as usize;
        let i1 = ((chunk >> 12) & 0x3f) as usize;
        let i2 = ((chunk >> 6) & 0x3f) as usize;
        let i3 = (chunk & 0x3f) as usize;

        output.push(BASE64_TABLE[i0] as char);
        output.push(BASE64_TABLE[i1] as char);

        if index + 1 < input.len() {
            output.push(BASE64_TABLE[i2] as char);
        } else {
            output.push('=');
        }

        if index + 2 < input.len() {
            output.push(BASE64_TABLE[i3] as char);
        } else {
            output.push('=');
        }

        index += 3;
    }

    output
}

#[cfg(test)]
mod tests {
    use super::{ClipboardError, ClipboardService, base64_encode};

    fn set_path_for_test(value: Option<&str>) {
        let key = "PATH";
        // Safety: these tests mutate process-global environment state in a
        // scoped setup/teardown pattern and do not spawn concurrent threads.
        match value {
            Some(value) => unsafe { std::env::set_var(key, value) },
            None => unsafe { std::env::remove_var(key) },
        }
    }

    #[test]
    fn base64_encoder_matches_known_values() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
    }

    #[test]
    fn copy_without_osc52_reports_no_backend_when_path_is_empty() {
        let key = "PATH";
        let original = std::env::var(key).ok();
        set_path_for_test(Some(""));

        let service = ClipboardService::new().with_osc52(false);
        let result = service.copy_text("hello");

        if let Some(value) = original {
            set_path_for_test(Some(&value));
        } else {
            set_path_for_test(None);
        }

        match result {
            Err(ClipboardError::NoBackendAvailable { attempts }) => {
                assert!(!attempts.is_empty());
            }
            Err(ClipboardError::SpawnFailed { .. }) => {
                // Acceptable when command lookup fails immediately.
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }
}
