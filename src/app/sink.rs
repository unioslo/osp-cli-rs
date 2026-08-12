//! Terminal-facing output sinks used by the host layer.
//!
//! This module exists so host entrypoints can emit stdout/stderr through a
//! small abstraction that works both for the real terminal and for tests.
//!
//! Contract:
//!
//! - sinks are intentionally tiny and text-oriented
//! - buffering, snapshotting, or process stdio forwarding belong here
//! - higher-level rendering and message formatting belong elsewhere

use std::io::{self, Write};
use std::process::{Command, Stdio};

/// Terminal-facing output sink for stdout/stderr emission.
///
/// Implementors should forward or buffer the supplied text exactly as received;
/// higher layers already handle rendering, grouping, and newline decisions.
/// Callers may write to stdout and stderr independently and can assume that
/// empty writes are harmless.
///
/// # Examples
///
/// ```
/// use osp_cli::app::UiSink;
///
/// #[derive(Default)]
/// struct CaptureSink {
///     stdout: String,
///     stderr: String,
/// }
///
/// impl UiSink for CaptureSink {
///     fn write_stdout(&mut self, text: &str) {
///         self.stdout.push_str(text);
///     }
///
///     fn write_stderr(&mut self, text: &str) {
///         self.stderr.push_str(text);
///     }
/// }
///
/// let mut sink = CaptureSink::default();
/// sink.write_stdout("ok");
/// sink.write_stderr("warn");
///
/// assert_eq!(sink.stdout, "ok");
/// assert_eq!(sink.stderr, "warn");
/// ```
pub trait UiSink {
    /// Writes text to the sink's stdout channel.
    fn write_stdout(&mut self, text: &str);

    /// Writes a complete human-facing document through a pager when supported.
    ///
    /// Non-terminal and buffered sinks deliberately fall back to an ordinary
    /// stdout write so embedding and capture behavior stays deterministic.
    fn write_stdout_paged(&mut self, text: &str, _pager: &str) {
        self.write_stdout(text);
    }

    /// Writes text to the sink's stderr channel.
    fn write_stderr(&mut self, text: &str);
}

/// Sink that forwards output directly to the process stdio streams.
///
/// Empty writes are ignored.
#[derive(Default)]
pub struct StdIoUiSink;

impl UiSink for StdIoUiSink {
    fn write_stdout(&mut self, text: &str) {
        if !text.is_empty() {
            let mut stdout = io::stdout().lock();
            if let Err(err) = stdout
                .write_all(text.as_bytes())
                .and_then(|()| stdout.flush())
                && err.kind() != io::ErrorKind::BrokenPipe
            {
                let _ = writeln!(io::stderr(), "failed to write command output: {err}");
            }
        }
    }

    fn write_stderr(&mut self, text: &str) {
        if !text.is_empty() {
            let mut stderr = io::stderr().lock();
            let _ = stderr
                .write_all(text.as_bytes())
                .and_then(|()| stderr.flush());
        }
    }

    fn write_stdout_paged(&mut self, text: &str, pager: &str) {
        let Ok(mut child) = Command::new("sh")
            .args(["-c", pager])
            .stdin(Stdio::piped())
            .spawn()
        else {
            self.write_stdout(text);
            return;
        };

        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        if !matches!(child.wait(), Ok(status) if status.success()) {
            self.write_stdout(text);
        }
    }
}

/// Sink that buffers stdout and stderr for assertions and snapshot tests.
///
/// # Examples
///
/// ```
/// use osp_cli::app::{BufferedUiSink, UiSink};
///
/// let mut sink = BufferedUiSink::default();
/// sink.write_stdout("ok");
/// sink.write_stderr("warn");
///
/// assert_eq!(sink.stdout, "ok");
/// assert_eq!(sink.stderr, "warn");
/// ```
#[derive(Default, Debug)]
pub struct BufferedUiSink {
    /// Buffered stdout content in write order.
    pub stdout: String,

    /// Buffered stderr content in write order.
    pub stderr: String,
}

impl UiSink for BufferedUiSink {
    fn write_stdout(&mut self, text: &str) {
        self.stdout.push_str(text);
    }

    fn write_stderr(&mut self, text: &str) {
        self.stderr.push_str(text);
    }
}
