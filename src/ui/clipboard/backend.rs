use std::io::Write;
use std::process::{Command, Stdio};

use super::ClipboardError;

pub(crate) const OSC52_MAX_BYTES_DEFAULT: usize = 100_000;

#[derive(Debug, Clone, Copy)]
pub(crate) struct ClipboardCommand {
    pub(crate) command: &'static str,
    pub(crate) args: &'static [&'static str],
}

pub(crate) fn platform_backends() -> Vec<ClipboardCommand> {
    let mut backends = Vec::new();

    if cfg!(target_os = "macos") {
        backends.push(ClipboardCommand {
            command: "pbcopy",
            args: &[],
        });
        return backends;
    }

    if cfg!(target_os = "windows") {
        backends.push(ClipboardCommand {
            command: "clip",
            args: &[],
        });
        return backends;
    }

    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        backends.push(ClipboardCommand {
            command: "wl-copy",
            args: &[],
        });
    }

    backends.push(ClipboardCommand {
        command: "xclip",
        args: &["-selection", "clipboard"],
    });
    backends.push(ClipboardCommand {
        command: "xsel",
        args: &["--clipboard", "--input"],
    });

    backends
}

pub(crate) fn copy_via_command(
    command: ClipboardCommand,
    text: &str,
) -> Result<(), ClipboardError> {
    let mut child = Command::new(command.command)
        .args(command.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| ClipboardError::SpawnFailed {
            command: command.command.to_string(),
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
            command: command.command.to_string(),
            status: output.status.code().unwrap_or(1),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }
}

pub(crate) fn osc52_enabled() -> bool {
    match std::env::var("OSC52") {
        Ok(value) => {
            let value = value.trim().to_ascii_lowercase();
            !(value == "0" || value == "false" || value == "off")
        }
        Err(_) => true,
    }
}

pub(crate) fn osc52_max_bytes() -> usize {
    std::env::var("OSC52_MAX_BYTES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(OSC52_MAX_BYTES_DEFAULT)
}

pub(crate) fn osc52_payload(text: &str) -> String {
    let encoded = base64_encode(text.as_bytes());
    format!("\x1b]52;c;{encoded}\x07")
}

pub(crate) fn base64_encoded_len(input_len: usize) -> usize {
    if input_len == 0 {
        return 0;
    }

    input_len.div_ceil(3).saturating_mul(4)
}

pub(crate) fn base64_encode(input: &[u8]) -> String {
    if input.is_empty() {
        return String::new();
    }

    const BASE64_TABLE: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

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
