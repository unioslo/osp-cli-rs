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

/// Terminal-facing output sink for stdout/stderr emission.
pub trait UiSink {
    /// Writes text to the sink's stdout channel.
    fn write_stdout(&mut self, text: &str);

    /// Writes text to the sink's stderr channel.
    fn write_stderr(&mut self, text: &str);
}

/// Sink that forwards output directly to the process stdio streams.
#[derive(Default)]
pub struct StdIoUiSink;

impl UiSink for StdIoUiSink {
    fn write_stdout(&mut self, text: &str) {
        if !text.is_empty() {
            print!("{text}");
        }
    }

    fn write_stderr(&mut self, text: &str) {
        if !text.is_empty() {
            eprint!("{text}");
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
