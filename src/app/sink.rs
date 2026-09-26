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

use std::fmt::Write as _;
use std::io::{self, IsTerminal, Write};
use std::process::{Command, Stdio};

use terminal_size::{Height, Width, terminal_size_of};

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

    /// Reports whether stderr is an interactive terminal.
    fn stderr_is_terminal(&self) -> bool {
        false
    }

    /// Returns the physical stderr width when it is available.
    fn stderr_width(&self) -> Option<usize> {
        None
    }

    /// Returns the physical stderr height when it is available.
    fn stderr_height(&self) -> Option<usize> {
        None
    }

    /// Writes one transient progress document.
    ///
    /// Sinks that do not own terminal replacement state append the document to
    /// stderr. The host's progress wrapper supplies replacement state where it
    /// is available.
    fn write_progress(&mut self, text: &str, _replace: bool) {
        self.write_stderr(text);
    }

    /// Clears a previously replaceable progress document, if any.
    fn clear_progress(&mut self) {}
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

    fn stderr_is_terminal(&self) -> bool {
        io::stderr().is_terminal()
    }

    fn stderr_width(&self) -> Option<usize> {
        terminal_size_of(io::stderr())
            .map(|(Width(width), _)| width as usize)
            .or_else(|| {
                std::env::var("COLUMNS")
                    .ok()
                    .and_then(|value| value.parse::<usize>().ok())
                    .filter(|width| *width > 0)
            })
    }

    fn stderr_height(&self) -> Option<usize> {
        terminal_size_of(io::stderr())
            .map(|(_, Height(height))| height as usize)
            .or_else(|| {
                std::env::var("LINES")
                    .ok()
                    .and_then(|value| value.parse::<usize>().ok())
                    .filter(|height| *height > 0)
            })
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

/// Adds terminal-aware replacement to an existing stderr sink.
pub(crate) struct ProgressUiSink<'a> {
    inner: &'a mut dyn UiSink,
    previous_lines: usize,
}

impl<'a> ProgressUiSink<'a> {
    pub(crate) fn new(inner: &'a mut dyn UiSink) -> Self {
        Self {
            inner,
            previous_lines: 0,
        }
    }

    fn append(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.inner.write_stderr(text);
        if !text.ends_with('\n') {
            self.inner.write_stderr("\n");
        }
    }

    fn display_width(text: &str) -> usize {
        crate::ui::display_width(text)
    }

    fn physical_line_count(&self, text: &str) -> usize {
        let width = self.inner.stderr_width().filter(|width| *width > 0);
        let lines = text.split_terminator('\n').collect::<Vec<_>>();
        if lines.is_empty() {
            return 0;
        }
        lines
            .into_iter()
            .map(|line| {
                let line_width = Self::display_width(line);
                match width {
                    Some(width) if line_width > 0 => line_width.div_ceil(width),
                    _ => 1,
                }
            })
            .sum()
    }

    fn erase_previous(&mut self) {
        if self.previous_lines == 0 || !self.inner.stderr_is_terminal() {
            self.previous_lines = 0;
            return;
        }

        let lines = self.previous_lines;
        if self
            .inner
            .stderr_height()
            .is_some_and(|height| lines >= height)
        {
            // The old block is already taller than the visible terminal. Do
            // not emit an unsafe cursor jump; append the next document.
            self.previous_lines = 0;
            return;
        }
        let mut escape = format!("\x1b[{lines}A");
        for index in 0..lines {
            escape.push_str("\r\x1b[2K");
            if index + 1 < lines {
                escape.push_str("\x1b[1B");
            }
        }
        if lines > 1 {
            let _ = write!(escape, "\x1b[{}A", lines - 1);
        }
        self.inner.write_stderr(&escape);
        self.previous_lines = 0;
    }
}

impl UiSink for ProgressUiSink<'_> {
    fn write_stdout(&mut self, text: &str) {
        self.write_progress(text, false);
    }

    fn write_stderr(&mut self, text: &str) {
        self.clear_progress();
        self.inner.write_stderr(text);
    }

    fn stderr_is_terminal(&self) -> bool {
        self.inner.stderr_is_terminal()
    }

    fn stderr_width(&self) -> Option<usize> {
        self.inner.stderr_width()
    }

    fn stderr_height(&self) -> Option<usize> {
        self.inner.stderr_height()
    }

    fn write_progress(&mut self, text: &str, replace: bool) {
        if text.is_empty() {
            return;
        }
        let physical_lines = self.physical_line_count(text);
        let can_replace = replace
            && self.stderr_is_terminal()
            && self
                .stderr_height()
                .is_none_or(|height| physical_lines < height);
        if can_replace {
            self.erase_previous();
        } else {
            self.clear_progress();
        }
        self.append(text);
        self.previous_lines = if can_replace { physical_lines } else { 0 };
    }

    fn clear_progress(&mut self) {
        self.erase_previous();
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
