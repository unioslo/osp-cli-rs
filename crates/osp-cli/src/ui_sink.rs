/// Terminal-facing output sink for stdout/stderr emission.
pub trait UiSink {
    fn write_stdout(&mut self, text: &str);
    fn write_stderr(&mut self, text: &str);
}

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

#[derive(Default, Debug)]
pub struct BufferedUiSink {
    pub stdout: String,
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
