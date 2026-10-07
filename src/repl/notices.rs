//! Lines that background work prints above the live REPL prompt.
//!
//! Startup must never wait on the network, so work such as "what is waiting
//! on you" runs on its own thread and posts its line here. The interactive
//! editor prints posted lines above the prompt as they arrive; lines posted
//! before the editor starts are held and printed when it does. One-shot
//! commands do not post notices. The basic reader prints them on stderr.

use std::sync::Mutex;

use reedline::ExternalPrinter;

struct Notices {
    printer: Option<ExternalPrinter<String>>,
    pending: Vec<String>,
    basic: bool,
}

impl Notices {
    fn post(&mut self, line: String) {
        if self.basic {
            eprintln!("\n{line}");
            return;
        }
        match &self.printer {
            // A full channel only drops a notice; it never stalls background work.
            Some(printer) => {
                let _ = printer.sender().try_send(line);
            }
            None => self.pending.push(line),
        }
    }

    fn attach(&mut self, printer: &ExternalPrinter<String>) {
        self.basic = false;
        for line in self.pending.drain(..) {
            let _ = printer.sender().try_send(line);
        }
        self.printer = Some(printer.clone());
    }
}

static NOTICES: Mutex<Notices> = Mutex::new(Notices {
    printer: None,
    pending: Vec::new(),
    basic: false,
});

/// Prints `line` above the REPL prompt without blocking the caller.
pub fn post_notice(line: impl Into<String>) {
    NOTICES
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .post(line.into());
}

/// Routes notices to the running editor and flushes those posted earlier.
pub(crate) fn attach(printer: &ExternalPrinter<String>) {
    NOTICES
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .attach(printer);
}

#[cfg(test)]
mod tests {
    use super::{ExternalPrinter, Notices};

    #[test]
    fn notices_posted_before_the_editor_print_once_it_starts_unit() {
        let mut notices = Notices {
            printer: None,
            pending: Vec::new(),
            basic: false,
        };
        notices.post("early".into());
        let printer = ExternalPrinter::<String>::default();
        notices.attach(&printer);
        notices.post("late".into());
        assert_eq!(printer.get_line().as_deref(), Some("early"));
        assert_eq!(printer.get_line().as_deref(), Some("late"));
        assert_eq!(printer.get_line(), None);
    }
}

/// Flush queued notices when terminal capabilities require the basic reader.
pub(crate) fn attach_basic() {
    let mut notices = NOTICES.lock().unwrap_or_else(|err| err.into_inner());
    if let Some(printer) = notices.printer.take() {
        while let Some(line) = printer.get_line() {
            eprintln!("{line}");
        }
    }
    for line in notices.pending.drain(..) {
        eprintln!("{line}");
    }
    notices.basic = true;
}
