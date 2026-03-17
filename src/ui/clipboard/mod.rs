//! Clipboard transport for the canonical UI pipeline.
//!
//! This module owns the side effect of copying plain text. It does not know how
//! output gets rendered for copy/paste; that stays in the UI facade.

use std::fmt::{Display, Formatter};
use std::io::{IsTerminal, Write};

mod backend;

#[cfg(test)]
mod tests;

/// Clipboard service that tries OSC 52 and platform-specific clipboard helpers.
#[derive(Debug, Clone)]
#[must_use]
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

        if self.prefer_osc52 && std::io::stdout().is_terminal() && backend::osc52_enabled() {
            let max_bytes = backend::osc52_max_bytes();
            let encoded_len = backend::base64_encoded_len(text.len());
            if encoded_len <= max_bytes {
                attempts.push("osc52".to_string());
                self.copy_via_osc52(text)?;
                return Ok(());
            }
            attempts.push(format!("osc52 (payload {encoded_len} > {max_bytes})"));
        }

        for command in backend::platform_backends() {
            attempts.push(command.command.to_string());
            match backend::copy_via_command(command, text) {
                Ok(()) => return Ok(()),
                Err(ClipboardError::SpawnFailed { .. }) => continue,
                Err(error) => return Err(error),
            }
        }

        Err(ClipboardError::NoBackendAvailable { attempts })
    }

    fn copy_via_osc52(&self, text: &str) -> Result<(), ClipboardError> {
        let payload = backend::osc52_payload(text);
        std::io::stdout()
            .write_all(payload.as_bytes())
            .map_err(|err| ClipboardError::Io(err.to_string()))?;
        std::io::stdout()
            .flush()
            .map_err(|err| ClipboardError::Io(err.to_string()))?;
        Ok(())
    }
}
