use std::fmt::{Display, Formatter};
use std::io::{IsTerminal, Write};
use std::process::{Command, Stdio};

use crate::ui::{Document, RenderSettings, render_document_for_copy};

/// Clipboard service that tries OSC 52 and platform-specific clipboard helpers.
#[derive(Debug, Clone)]
pub struct ClipboardService {
    prefer_osc52: bool,
}

impl Default for ClipboardService {
    fn default() -> Self {
        Self { prefer_osc52: true }
    }
}

/// Errors returned while copying rendered output to the clipboard.
#[derive(Debug)]
pub enum ClipboardError {
    /// No supported clipboard backend was available.
    NoBackendAvailable {
        /// Backend attempts that were tried or skipped.
        attempts: Vec<String>,
    },
    /// A clipboard helper process could not be spawned.
    SpawnFailed {
        /// Command that failed to start.
        command: String,
        /// Human-readable spawn failure reason.
        reason: String,
    },
    /// A clipboard helper process exited with failure status.
    CommandFailed {
        /// Command that was run.
        command: String,
        /// Exit status code, or `1` when unavailable.
        status: i32,
        /// Standard error output captured from the helper.
        stderr: String,
    },
    /// Local I/O failure while preparing or sending clipboard data.
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
    /// Creates a clipboard service with the default backend order.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enables or disables OSC 52 before falling back to external commands.
    pub fn with_osc52(mut self, enabled: bool) -> Self {
        self.prefer_osc52 = enabled;
        self
    }

    /// Copies raw text to the clipboard.
    ///
    /// Returns an error if no backend succeeds or if a backend fails after starting.
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

    /// Renders a document for copy/paste and writes the resulting text to the clipboard.
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
    if input_len == 0 {
        return 0;
    }

    input_len.div_ceil(3).saturating_mul(4)
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
    use std::sync::{Mutex, OnceLock};

    use crate::core::output::OutputFormat;
    use crate::ui::{
        Document, RenderSettings,
        document::{Block, LineBlock, LinePart},
    };

    use super::{
        ClipboardError, ClipboardService, OSC52_MAX_BYTES_DEFAULT, base64_encode,
        base64_encoded_len, copy_via_command, osc52_enabled, osc52_max_bytes, platform_backends,
    };

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn acquire_env_lock() -> std::sync::MutexGuard<'static, ()> {
        env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn set_path_for_test(value: Option<&str>) {
        let key = "PATH";
        // Safety: these tests mutate process-global environment state in a
        // scoped setup/teardown pattern and do not spawn concurrent threads.
        match value {
            Some(value) => unsafe { std::env::set_var(key, value) },
            None => unsafe { std::env::remove_var(key) },
        }
    }

    fn set_env_for_test(key: &str, value: Option<&str>) {
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
    fn base64_length_and_env_helpers_behave_predictably() {
        let _guard = acquire_env_lock();
        assert_eq!(base64_encoded_len(0), 0);
        assert_eq!(base64_encoded_len(1), 4);
        assert_eq!(base64_encoded_len(4), 8);

        let osc52_original = std::env::var("OSC52").ok();
        let max_original = std::env::var("OSC52_MAX_BYTES").ok();

        set_env_for_test("OSC52", Some("off"));
        assert!(!osc52_enabled());
        set_env_for_test("OSC52", Some("yes"));
        assert!(osc52_enabled());

        set_env_for_test("OSC52_MAX_BYTES", Some("4096"));
        assert_eq!(osc52_max_bytes(), 4096);
        set_env_for_test("OSC52_MAX_BYTES", Some("0"));
        assert_eq!(osc52_max_bytes(), 100_000);

        set_env_for_test("OSC52", osc52_original.as_deref());
        set_env_for_test("OSC52_MAX_BYTES", max_original.as_deref());
    }

    #[test]
    fn clipboard_error_display_covers_backend_spawn_and_status_cases() {
        assert_eq!(
            ClipboardError::NoBackendAvailable {
                attempts: vec!["osc52".to_string(), "xclip".to_string()],
            }
            .to_string(),
            "no clipboard backend available (tried: osc52, xclip)"
        );
        assert_eq!(
            ClipboardError::SpawnFailed {
                command: "xclip".to_string(),
                reason: "missing".to_string(),
            }
            .to_string(),
            "failed to start clipboard command `xclip`: missing"
        );
        assert_eq!(
            ClipboardError::CommandFailed {
                command: "xclip".to_string(),
                status: 7,
                stderr: "no display".to_string(),
            }
            .to_string(),
            "clipboard command `xclip` failed with status 7: no display"
        );
        assert_eq!(
            ClipboardError::Io("broken pipe".to_string()).to_string(),
            "clipboard I/O error: broken pipe"
        );
    }

    #[test]
    fn command_backend_reports_success_and_failure() {
        let _guard = acquire_env_lock();
        copy_via_command("/bin/sh", &["-c", "cat >/dev/null"], "hello")
            .expect("shell sink should succeed");

        let err = copy_via_command("/bin/sh", &["-c", "echo nope >&2; exit 7"], "hello")
            .expect_err("non-zero clipboard command should fail");
        assert!(matches!(
            err,
            ClipboardError::CommandFailed {
                status: 7,
                ref stderr,
                ..
            } if stderr.contains("nope")
        ));
    }

    #[test]
    fn platform_backends_prefers_wayland_when_present() {
        let _guard = acquire_env_lock();
        let original = std::env::var("WAYLAND_DISPLAY").ok();
        set_env_for_test("WAYLAND_DISPLAY", Some("wayland-0"));
        let backends = platform_backends();
        set_env_for_test("WAYLAND_DISPLAY", original.as_deref());

        if cfg!(target_os = "windows") || cfg!(target_os = "macos") {
            assert!(!backends.is_empty());
        } else {
            assert_eq!(backends[0].command, "wl-copy");
        }
    }

    #[test]
    fn copy_without_osc52_reports_no_backend_when_path_is_empty() {
        let _guard = acquire_env_lock();
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

    #[test]
    fn copy_document_uses_same_backend_path() {
        let _guard = acquire_env_lock();
        let key = "PATH";
        let original = std::env::var(key).ok();
        set_path_for_test(Some(""));

        let service = ClipboardService::new().with_osc52(false);
        let document = Document {
            blocks: vec![Block::Line(LineBlock {
                parts: vec![LinePart {
                    text: "hello".to_string(),
                    token: None,
                }],
            })],
        };
        let result =
            service.copy_document(&document, &RenderSettings::test_plain(OutputFormat::Table));

        if let Some(value) = original {
            set_path_for_test(Some(&value));
        } else {
            set_path_for_test(None);
        }

        assert!(matches!(
            result,
            Err(ClipboardError::NoBackendAvailable { .. })
                | Err(ClipboardError::SpawnFailed { .. })
        ));
    }

    #[test]
    fn command_backend_reports_spawn_failure_for_missing_binary() {
        let err = copy_via_command("/definitely/missing/clipboard-bin", &[], "hello")
            .expect_err("missing binary should fail to spawn");
        assert!(matches!(err, ClipboardError::SpawnFailed { .. }));
    }

    #[test]
    fn platform_backends_include_x11_fallbacks_without_wayland() {
        let _guard = acquire_env_lock();
        let original = std::env::var("WAYLAND_DISPLAY").ok();
        set_env_for_test("WAYLAND_DISPLAY", None);
        let backends = platform_backends();
        set_env_for_test("WAYLAND_DISPLAY", original.as_deref());

        if !(cfg!(target_os = "windows") || cfg!(target_os = "macos")) {
            let names = backends
                .iter()
                .map(|backend| backend.command)
                .collect::<Vec<_>>();
            assert!(names.contains(&"xclip"));
            assert!(names.contains(&"xsel"));
        }
    }

    #[test]
    fn command_failure_without_stderr_uses_short_display() {
        let err = ClipboardError::CommandFailed {
            command: "xclip".to_string(),
            status: 9,
            stderr: String::new(),
        };
        assert_eq!(
            err.to_string(),
            "clipboard command `xclip` failed with status 9"
        );
    }

    #[test]
    fn osc52_helpers_respect_env_toggles_and_defaults() {
        let _guard = acquire_env_lock();
        let original_enabled = std::env::var("OSC52").ok();
        let original_max = std::env::var("OSC52_MAX_BYTES").ok();

        set_env_for_test("OSC52", Some("off"));
        assert!(!osc52_enabled());
        set_env_for_test("OSC52", Some("FALSE"));
        assert!(!osc52_enabled());
        set_env_for_test("OSC52", None);
        assert!(osc52_enabled());

        set_env_for_test("OSC52_MAX_BYTES", Some("2048"));
        assert_eq!(osc52_max_bytes(), 2048);
        set_env_for_test("OSC52_MAX_BYTES", Some("0"));
        assert_eq!(osc52_max_bytes(), OSC52_MAX_BYTES_DEFAULT);
        set_env_for_test("OSC52_MAX_BYTES", Some("wat"));
        assert_eq!(osc52_max_bytes(), OSC52_MAX_BYTES_DEFAULT);

        set_env_for_test("OSC52", original_enabled.as_deref());
        set_env_for_test("OSC52_MAX_BYTES", original_max.as_deref());
    }

    #[test]
    fn base64_helpers_cover_empty_and_padded_inputs() {
        assert_eq!(base64_encoded_len(0), 0);
        assert_eq!(base64_encoded_len(1), 4);
        assert_eq!(base64_encoded_len(2), 4);
        assert_eq!(base64_encoded_len(3), 4);
        assert_eq!(base64_encoded_len(4), 8);

        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
    }

    #[test]
    fn clipboard_service_builders_toggle_osc52_preference() {
        let default = ClipboardService::new();
        assert!(default.prefer_osc52);

        let disabled = ClipboardService::new().with_osc52(false);
        assert!(!disabled.prefer_osc52);
    }

    #[test]
    fn copy_via_osc52_writer_is_callable_unit() {
        let _guard = acquire_env_lock();
        ClipboardService::new()
            .copy_via_osc52("ping")
            .expect("osc52 writer should succeed on stdout");
    }
}
