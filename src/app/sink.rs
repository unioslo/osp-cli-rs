/// Terminal-facing output sink for stdout/stderr emission.
pub trait UiSink {
    /// Writes text to the sink's stdout channel.
    fn write_stdout(&mut self, text: &str);

    /// Writes text to the sink's stderr channel.
    fn write_stderr(&mut self, text: &str);
}

#[derive(Default)]
/// Sink that forwards output directly to the process stdio streams.
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

#[derive(Default, Debug)]
/// Sink that buffers stdout and stderr for assertions and snapshot tests.
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
