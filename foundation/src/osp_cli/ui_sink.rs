pub(crate) trait UiSink {
    fn write_stdout(&mut self, text: &str);
    fn write_stderr(&mut self, text: &str);
}

#[derive(Default)]
pub(crate) struct StdIoUiSink;

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

#[cfg(test)]
#[derive(Default, Debug)]
pub(crate) struct BufferedUiSink {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

#[cfg(test)]
impl UiSink for BufferedUiSink {
    fn write_stdout(&mut self, text: &str) {
        self.stdout.push_str(text);
    }

    fn write_stderr(&mut self, text: &str) {
        self.stderr.push_str(text);
    }
}
