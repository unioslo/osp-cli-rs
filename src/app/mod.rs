//! Main host-facing entrypoints plus bootstrap/runtime state.

use crate::ui::messages::{MessageBuffer, MessageLevel, adjust_verbosity};
use std::ffi::OsString;

pub(crate) mod bootstrap;
pub(crate) mod command_output;
pub(crate) mod config_explain;
pub(crate) mod dispatch;
pub(crate) mod external;
pub(crate) mod help;
pub(crate) mod host;
pub(crate) mod logging;
pub(crate) mod repl_lifecycle;
pub mod runtime;
pub mod session;
pub mod sink;
#[cfg(test)]
mod tests;
pub(crate) mod timing;

pub(crate) use bootstrap::*;
pub(crate) use command_output::*;
pub use host::run_from;
pub(crate) use host::*;
pub use runtime::{
    AppClients, AppRuntime, AuthState, ConfigState, LaunchContext, RuntimeContext, TerminalKind,
    UiState,
};
pub(crate) use session::AppStateInit;
pub use session::{
    AppSession, AppState, DebugTimingBadge, DebugTimingState, LastFailure, ReplScopeFrame,
    ReplScopeStack,
};
pub use sink::{BufferedUiSink, StdIoUiSink, UiSink};

#[derive(Debug, Default, Clone, Copy)]
pub struct App;

impl App {
    pub const fn new() -> Self {
        Self
    }

    pub fn run_from<I, T>(&self, args: I) -> miette::Result<i32>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        run_from(args)
    }

    pub fn with_sink<'a>(self, sink: &'a mut dyn UiSink) -> AppRunner<'a> {
        AppRunner { app: self, sink }
    }

    pub fn run_with_sink<I, T>(&self, args: I, sink: &mut dyn UiSink) -> miette::Result<i32>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        host::run_from_with_sink(args, sink)
    }

    pub fn run_process<I, T>(&self, args: I) -> i32
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        run_process(args)
    }

    pub fn run_process_with_sink<I, T>(&self, args: I, sink: &mut dyn UiSink) -> i32
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        run_process_with_sink(args, sink)
    }
}

pub struct AppRunner<'a> {
    app: App,
    sink: &'a mut dyn UiSink,
}

impl<'a> AppRunner<'a> {
    pub fn run_from<I, T>(&mut self, args: I) -> miette::Result<i32>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        self.app.run_with_sink(args, self.sink)
    }

    pub fn run_process<I, T>(&mut self, args: I) -> i32
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        self.app.run_process_with_sink(args, self.sink)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AppBuilder;

impl AppBuilder {
    pub const fn new() -> Self {
        Self
    }

    pub fn build(self) -> App {
        App::new()
    }

    pub fn build_with_sink<'a>(self, sink: &'a mut dyn UiSink) -> AppRunner<'a> {
        self.build().with_sink(sink)
    }
}

pub fn run_process<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let mut sink = StdIoUiSink;
    run_process_with_sink(args, &mut sink)
}

pub fn run_process_with_sink<I, T>(args: I, sink: &mut dyn UiSink) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<OsString>>();
    let message_verbosity = bootstrap_message_verbosity(&args);

    match host::run_from_with_sink(args, sink) {
        Ok(code) => code,
        Err(err) => {
            let mut messages = MessageBuffer::default();
            messages.error(render_report_message(&err, message_verbosity));
            sink.write_stderr(&messages.render_grouped(message_verbosity));
            classify_exit_code(&err)
        }
    }
}

fn bootstrap_message_verbosity(args: &[OsString]) -> MessageLevel {
    let mut verbose = 0u8;
    let mut quiet = 0u8;

    for token in args.iter().skip(1) {
        let Some(value) = token.to_str() else {
            continue;
        };

        if value == "--" {
            break;
        }

        match value {
            "--verbose" => {
                verbose = verbose.saturating_add(1);
                continue;
            }
            "--quiet" => {
                quiet = quiet.saturating_add(1);
                continue;
            }
            _ => {}
        }

        if value.starts_with('-') && !value.starts_with("--") {
            for ch in value.chars().skip(1) {
                match ch {
                    'v' => verbose = verbose.saturating_add(1),
                    'q' => quiet = quiet.saturating_add(1),
                    _ => {}
                }
            }
        }
    }

    adjust_verbosity(MessageLevel::Success, verbose, quiet)
}
