//! The app module exists to turn the library pieces into a running program.
//!
//! This is the process-facing layer of the crate. It wires together CLI
//! parsing, config loading, plugin/catalog setup, rendering, and REPL startup
//! into the public [`App`] entrypoints. Lower-level modules like
//! [`crate::config`], [`crate::ui`], and [`crate::repl`] stay reusable because
//! this module is where the product-level orchestration happens.
//!
//! Contract:
//!
//! - this is allowed to depend on the rest of the crate because it is the host
//!   composition layer
//! - the dependency should not point the other way; lower-level modules should
//!   not import [`crate::app`] to get work done
//!
//! Public API shape:
//!
//! - most callers should start with [`App`] or [`AppBuilder`]
//! - embedders may inspect runtime/session state, but the preferred
//!   construction path still flows through builders and constructors here such
//!   as [`crate::app::AppStateBuilder`], [`crate::app::UiStateBuilder`], and
//!   [`crate::app::LaunchContextBuilder`]
//! - lower-level semantic payloads live in modules like [`crate::guide`] and
//!   [`crate::completion`]; this module owns the heavier host machinery

use crate::native::NativeCommandRegistry;
use crate::ui::messages::{MessageBuffer, MessageLevel, adjust_verbosity};
use std::ffi::OsString;

pub(crate) mod assembly;
pub(crate) mod bootstrap;
pub(crate) mod command_output;
pub(crate) mod config_explain;
pub(crate) mod dispatch;
pub(crate) mod external;
pub(crate) mod help;
pub(crate) mod host;
pub(crate) mod logging;
pub(crate) mod rebuild;
pub(crate) mod repl_lifecycle;
pub(crate) mod runtime;
pub(crate) mod session;
/// UI sink abstractions used by the host entrypoints.
pub(crate) mod sink;
#[cfg(test)]
mod tests;
pub(crate) mod timing;

pub(crate) use bootstrap::*;
pub(crate) use command_output::*;
pub use host::run_from;
pub(crate) use host::*;
pub(crate) use repl_lifecycle::rebuild_repl_in_place;
pub use runtime::{
    AppClients, AppClientsBuilder, AppRuntime, AuthState, ConfigState, LaunchContext,
    LaunchContextBuilder, RuntimeContext, TerminalKind, UiState, UiStateBuilder,
};
#[cfg(test)]
pub(crate) use session::AppStateInit;
pub use session::{
    AppSession, AppSessionBuilder, AppState, AppStateBuilder, DebugTimingBadge, DebugTimingState,
    LastFailure, ReplScopeFrame, ReplScopeStack,
};
pub use sink::{BufferedUiSink, StdIoUiSink, UiSink};

#[derive(Clone, Default)]
/// Top-level application entrypoint for CLI and REPL execution.
///
/// Most embedders should start here or with [`AppBuilder`] instead of trying
/// to assemble runtime/session machinery directly.
pub struct App {
    native_commands: NativeCommandRegistry,
}

impl App {
    /// Creates an application with the default native command registry.
    pub fn new() -> Self {
        Self {
            native_commands: NativeCommandRegistry::default(),
        }
    }

    /// Replaces the native command registry used for command dispatch.
    pub fn with_native_commands(mut self, native_commands: NativeCommandRegistry) -> Self {
        self.native_commands = native_commands;
        self
    }

    /// Runs the application and returns a structured exit status.
    pub fn run_from<I, T>(&self, args: I) -> miette::Result<i32>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        host::run_from_with_sink_and_native(args, &mut StdIoUiSink, &self.native_commands)
    }

    /// Binds the application to a specific UI sink for repeated invocations.
    pub fn with_sink<'a>(self, sink: &'a mut dyn UiSink) -> AppRunner<'a> {
        AppRunner { app: self, sink }
    }

    /// Runs the application with the provided UI sink.
    pub fn run_with_sink<I, T>(&self, args: I, sink: &mut dyn UiSink) -> miette::Result<i32>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        host::run_from_with_sink_and_native(args, sink, &self.native_commands)
    }

    /// Runs the application and converts execution failures into process exit codes.
    pub fn run_process<I, T>(&self, args: I) -> i32
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let mut sink = StdIoUiSink;
        self.run_process_with_sink(args, &mut sink)
    }

    /// Runs the application with the provided sink and returns a process exit code.
    pub fn run_process_with_sink<I, T>(&self, args: I, sink: &mut dyn UiSink) -> i32
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let args = args.into_iter().map(Into::into).collect::<Vec<OsString>>();
        let message_verbosity = bootstrap_message_verbosity(&args);

        match host::run_from_with_sink_and_native(args, sink, &self.native_commands) {
            Ok(code) => code,
            Err(err) => {
                let mut messages = MessageBuffer::default();
                messages.error(render_report_message(&err, message_verbosity));
                sink.write_stderr(&messages.render_grouped(message_verbosity));
                classify_exit_code(&err)
            }
        }
    }
}

/// Reusable runner that keeps an [`App`] paired with a UI sink.
pub struct AppRunner<'a> {
    app: App,
    sink: &'a mut dyn UiSink,
}

impl<'a> AppRunner<'a> {
    /// Runs the application and returns a structured exit status.
    pub fn run_from<I, T>(&mut self, args: I) -> miette::Result<i32>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        self.app.run_with_sink(args, self.sink)
    }

    /// Runs the application and converts execution failures into process exit codes.
    pub fn run_process<I, T>(&mut self, args: I) -> i32
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        self.app.run_process_with_sink(args, self.sink)
    }
}

#[derive(Clone, Default)]
/// Builder for configuring an [`App`] before construction.
///
/// This is the canonical public composition surface for host-level setup.
pub struct AppBuilder {
    native_commands: NativeCommandRegistry,
}

impl AppBuilder {
    /// Creates a builder with the default native command registry.
    pub fn new() -> Self {
        Self {
            native_commands: NativeCommandRegistry::default(),
        }
    }

    /// Replaces the native command registry used by the built application.
    pub fn with_native_commands(mut self, native_commands: NativeCommandRegistry) -> Self {
        self.native_commands = native_commands;
        self
    }

    /// Builds an [`App`] from the current builder state.
    pub fn build(self) -> App {
        App::new().with_native_commands(self.native_commands)
    }

    /// Builds an [`AppRunner`] bound to the provided UI sink.
    pub fn build_with_sink<'a>(self, sink: &'a mut dyn UiSink) -> AppRunner<'a> {
        self.build().with_sink(sink)
    }
}

/// Runs the default application instance and returns a process exit code.
pub fn run_process<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let mut sink = StdIoUiSink;
    run_process_with_sink(args, &mut sink)
}

/// Runs the default application instance with the provided sink.
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
