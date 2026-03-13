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
//! - most callers should start with [`App`] or [`App::builder`]
//! - embedders may inspect runtime/session state, but the preferred
//!   construction path still flows through a small number of builders and
//!   constructors here such as [`crate::app::AppStateBuilder`]
//! - lower-level semantic payloads live in modules like [`crate::guide`] and
//!   [`crate::completion`]; this module owns the heavier host machinery
//!
//! Use this module when you want:
//!
//! - the full CLI / REPL host in-process
//! - the same dispatch/help/completion/config behavior as `osp`
//! - a wrapper crate that injects native commands or product defaults
//!
//! Skip this module and start lower if you only need:
//!
//! - LDAP service execution plus DSL stages: [`crate::services`]
//! - rendering rows/documents: [`crate::ui`]
//! - pure completion trees or guide payloads: [`crate::completion`] or
//!   [`crate::guide`]
//!
//! Broad-strokes host flow:
//!
//! ```text
//! argv / REPL request
//!      │
//!      ▼ [ app ]    build host state, load config, assemble command catalog
//!      ▼ [ dispatch ] choose native or plugin command, run it
//!      ▼ [ dsl ]    apply trailing pipeline stages to command output
//!      ▼ [ ui ]     render text to a UiSink or process stdio
//! ```
//!
//! Most callers only need one of these shapes:
//!
//! - [`App::run_from`] when they want a structured `Result<i32>`
//! - [`App::run_process`] when they want process-style exit code conversion
//! - [`App::with_sink`] or [`App::builder`] plus
//!   [`AppBuilder::build_with_sink`] when a test or outer host wants captured
//!   stdout/stderr instead of touching process stdio
//! - [`crate::services`] when this full host layer is more machinery than the
//!   integration needs
//!
//! Downstream product-wrapper pattern:
//!
//! - keep site-specific auth, policy, and integration state in the wrapper
//!   crate rather than in [`crate::app`]
//! - build a [`crate::NativeCommandRegistry`] for product-specific commands
//! - build one product-owned defaults layer under `extensions.<site>.*`
//! - inject both through [`App::builder`], then
//!   [`AppBuilder::with_native_commands`] and
//!   [`AppBuilder::with_product_defaults`]
//! - expose a thin product-level `run_process` or `builder()` API on top
//!   instead of forking generic host behavior

use crate::config::ConfigLayer;
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
    AppClients, AppRuntime, AuthState, ConfigState, LaunchContext, RuntimeContext, TerminalKind,
    UiState,
};
#[cfg(test)]
pub(crate) use session::AppStateInit;
pub use session::{
    AppSession, AppSessionBuilder, AppState, AppStateBuilder, DebugTimingBadge, DebugTimingState,
    LastFailure, ReplScopeFrame, ReplScopeStack,
};
pub use sink::{BufferedUiSink, StdIoUiSink, UiSink};

#[derive(Clone, Default)]
pub(crate) struct AppDefinition {
    native_commands: NativeCommandRegistry,
    product_defaults: ConfigLayer,
}

impl AppDefinition {
    fn with_native_commands(mut self, native_commands: NativeCommandRegistry) -> Self {
        self.native_commands = native_commands;
        self
    }

    fn with_product_defaults(mut self, product_defaults: ConfigLayer) -> Self {
        self.product_defaults = product_defaults;
        self
    }
}

#[derive(Clone, Default)]
/// Top-level application entrypoint for CLI and REPL execution.
///
/// Most embedders should start here or with [`AppBuilder`] instead of trying
/// to assemble runtime/session machinery directly.
#[must_use]
pub struct App {
    definition: AppDefinition,
}

impl App {
    /// Starts the canonical public builder for host construction.
    pub fn builder() -> AppBuilder {
        AppBuilder::default()
    }

    /// Creates an application with the default native command registry and no
    /// wrapper-owned defaults.
    pub fn new() -> Self {
        Self::builder().build()
    }

    /// Replaces the native command registry used for command dispatch.
    ///
    /// Use this when an embedder wants extra in-process commands to participate
    /// in the same command surface as the built-in host commands. When omitted,
    /// the application uses the crate's default native-command registry.
    ///
    /// This is the main extension seam for downstream product crates that wrap
    /// `osp-cli` and add site-specific native commands while keeping the rest
    /// of the host/runtime behavior unchanged.
    ///
    /// # Examples
    ///
    /// ```
    /// use anyhow::Result;
    /// use clap::Command;
    /// use osp_cli::app::BufferedUiSink;
    /// use osp_cli::{
    ///     App, NativeCommand, NativeCommandContext, NativeCommandOutcome, NativeCommandRegistry,
    /// };
    ///
    /// struct VersionCommand;
    ///
    /// impl NativeCommand for VersionCommand {
    ///     fn command(&self) -> Command {
    ///         Command::new("version").about("Show custom version")
    ///     }
    ///
    ///     fn execute(
    ///         &self,
    ///         _args: &[String],
    ///         _context: &NativeCommandContext<'_>,
    ///     ) -> Result<NativeCommandOutcome> {
    ///         Ok(NativeCommandOutcome::Exit(0))
    ///     }
    /// }
    ///
    /// let app = App::new().with_native_commands(
    ///     NativeCommandRegistry::new().with_command(VersionCommand),
    /// );
    /// let mut sink = BufferedUiSink::default();
    /// let exit = app.run_process_with_sink(["osp", "--help"], &mut sink);
    ///
    /// assert_eq!(exit, 0);
    /// assert!(sink.stdout.contains("version"));
    /// ```
    pub fn with_native_commands(mut self, native_commands: NativeCommandRegistry) -> Self {
        self.definition = self.definition.with_native_commands(native_commands);
        self
    }

    /// Replaces the product-owned defaults layered into runtime bootstrap.
    ///
    /// Use this when a wrapper crate owns extension keys such as
    /// `extensions.<site>.*` and wants them resolved through the normal host
    /// bootstrap path instead of maintaining a side-channel config helper.
    ///
    /// When omitted, the application uses only the built-in runtime defaults.
    pub fn with_product_defaults(mut self, product_defaults: ConfigLayer) -> Self {
        self.definition = self.definition.with_product_defaults(product_defaults);
        self
    }

    /// Runs the application and returns a structured exit status.
    ///
    /// Use this when your caller wants ordinary Rust error handling instead of
    /// the process-style exit code conversion performed by
    /// [`App::run_process`].
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::App;
    ///
    /// let exit = App::new().run_from(["osp", "--help"])?;
    ///
    /// assert_eq!(exit, 0);
    /// # Ok::<(), miette::Report>(())
    /// ```
    pub fn run_from<I, T>(&self, args: I) -> miette::Result<i32>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        host::run_from_with_sink_and_app(args, &mut StdIoUiSink, &self.definition)
    }

    /// Binds the application to a specific UI sink for repeated invocations.
    ///
    /// Prefer this in tests, editor integrations, or foreign hosts that need
    /// the same host behavior as `osp` but want the rendered text captured in a
    /// buffer instead of written to process stdio.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::app::BufferedUiSink;
    /// use osp_cli::App;
    ///
    /// let mut sink = BufferedUiSink::default();
    /// let exit = App::new()
    ///     .with_sink(&mut sink)
    ///     .run_process(["osp", "--help"]);
    ///
    /// assert_eq!(exit, 0);
    /// assert!(!sink.stdout.is_empty());
    /// assert!(sink.stderr.is_empty());
    /// ```
    pub fn with_sink<'a>(self, sink: &'a mut dyn UiSink) -> AppRunner<'a> {
        AppRunner { app: self, sink }
    }

    /// Runs the application with the provided UI sink.
    ///
    /// Prefer this over [`App::with_sink`] when the caller only needs one
    /// invocation. Use [`App::with_sink`] and [`AppRunner`] when the same sink
    /// should be reused across multiple calls.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::App;
    /// use osp_cli::app::BufferedUiSink;
    ///
    /// let mut sink = BufferedUiSink::default();
    /// let exit = App::new().run_with_sink(["osp", "--help"], &mut sink)?;
    ///
    /// assert_eq!(exit, 0);
    /// assert!(!sink.stdout.is_empty());
    /// assert!(sink.stderr.is_empty());
    /// # Ok::<(), miette::Report>(())
    /// ```
    pub fn run_with_sink<I, T>(&self, args: I, sink: &mut dyn UiSink) -> miette::Result<i32>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        host::run_from_with_sink_and_app(args, sink, &self.definition)
    }

    /// Runs the application and converts execution failures into process exit
    /// codes.
    ///
    /// Use this when the caller wants `osp`-style process behavior rather than
    /// structured error propagation. User-facing failures are rendered to the
    /// process stdio streams before the exit code is returned.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::App;
    ///
    /// let exit = App::new().run_process(["osp", "--help"]);
    ///
    /// assert_eq!(exit, 0);
    /// ```
    pub fn run_process<I, T>(&self, args: I) -> i32
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let mut sink = StdIoUiSink;
        self.run_process_with_sink(args, &mut sink)
    }

    /// Runs the application with the provided sink and returns a process exit
    /// code.
    ///
    /// This mirrors [`App::run_process`] but writes all rendered output and
    /// user-facing errors through the supplied sink instead of touching process
    /// stdio.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::App;
    /// use osp_cli::app::BufferedUiSink;
    ///
    /// let mut sink = BufferedUiSink::default();
    /// let exit = App::new().run_process_with_sink(["osp", "--help"], &mut sink);
    ///
    /// assert_eq!(exit, 0);
    /// assert!(!sink.stdout.is_empty());
    /// assert!(sink.stderr.is_empty());
    /// ```
    pub fn run_process_with_sink<I, T>(&self, args: I, sink: &mut dyn UiSink) -> i32
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let args = args.into_iter().map(Into::into).collect::<Vec<OsString>>();
        let message_verbosity = bootstrap_message_verbosity(&args);

        match host::run_from_with_sink_and_app(args, sink, &self.definition) {
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

/// Reusable runner that keeps an [`App`] paired with a borrowed UI sink.
///
/// Prefer [`App::run_with_sink`] for one-shot calls. This type exists for
/// scoped reuse when the same sink should back multiple invocations.
///
/// # Lifetime
///
/// `'a` is the lifetime of the mutable borrow of the sink passed to
/// [`App::with_sink`]. That means the runner cannot outlive the borrowed sink
/// and is mainly useful as a stack-scoped helper. It is not a good cross-thread
/// or async handoff type in its current form because it stores `&'a mut dyn
/// UiSink`.
///
/// # Examples
///
/// ```
/// use osp_cli::App;
/// use osp_cli::app::BufferedUiSink;
///
/// let mut sink = BufferedUiSink::default();
/// let mut runner = App::new().with_sink(&mut sink);
///
/// let first = runner.run_from(["osp", "--help"])?;
/// let second = runner.run_process(["osp", "--help"]);
///
/// assert_eq!(first, 0);
/// assert_eq!(second, 0);
/// assert!(!sink.stdout.is_empty());
/// assert!(sink.stderr.is_empty());
/// # Ok::<(), miette::Report>(())
/// ```
#[must_use = "AppRunner only has an effect when you call run_from or run_process on it"]
pub struct AppRunner<'a> {
    app: App,
    sink: &'a mut dyn UiSink,
}

impl<'a> AppRunner<'a> {
    /// Runs the application and returns a structured exit status.
    ///
    /// This is the bound-sink counterpart to [`App::run_with_sink`]. The
    /// borrowed sink stays attached so later calls on the same runner append to
    /// the same buffered or redirected output destination.
    pub fn run_from<I, T>(&mut self, args: I) -> miette::Result<i32>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        self.app.run_with_sink(args, self.sink)
    }

    /// Runs the application and converts execution failures into process exit
    /// codes.
    ///
    /// This is the bound-sink counterpart to [`App::run_process_with_sink`].
    /// User-facing failures are rendered into the already-bound sink before the
    /// numeric exit code is returned.
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
///
/// # Examples
///
/// Minimal embedder `main.rs`:
///
/// ```no_run
/// use osp_cli::App;
///
/// fn main() {
///     std::process::exit(
///         App::builder().build().run_process(std::env::args_os()),
///     );
/// }
/// ```
#[must_use]
pub struct AppBuilder {
    definition: AppDefinition,
}

impl AppBuilder {
    /// Replaces the native command registry used by the built application.
    ///
    /// This is the builder-friendly way to extend the host with extra native
    /// commands before calling [`AppBuilder::build`]. When omitted, the built
    /// app uses the crate's default native-command registry.
    ///
    /// This is the builder-side equivalent of [`App::with_native_commands`].
    /// Prefer it when a wrapper crate wants to finish command registration
    /// before deciding whether to build an owned [`App`] or a sink-bound
    /// [`AppRunner`].
    pub fn with_native_commands(mut self, native_commands: NativeCommandRegistry) -> Self {
        self.definition = self.definition.with_native_commands(native_commands);
        self
    }

    /// Replaces the product-owned defaults layered into runtime bootstrap.
    ///
    /// Wrapper crates should put site-owned keys under `extensions.<site>.*`
    /// and inject that layer here so native commands, `config get`, `config
    /// explain`, help rendering, and REPL rebuilds all see the same resolved
    /// config.
    ///
    /// If omitted, the built app uses only the crate's built-in runtime
    /// defaults.
    pub fn with_product_defaults(mut self, product_defaults: ConfigLayer) -> Self {
        self.definition = self.definition.with_product_defaults(product_defaults);
        self
    }

    /// Builds an [`App`] from the current builder state.
    ///
    /// Choose this when you want an owned application value that can be reused
    /// across many calls. Use [`AppBuilder::build_with_sink`] when binding the
    /// output sink is part of the setup.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::App;
    ///
    /// let app = App::builder().build();
    /// let exit = app.run_process(["osp", "--help"]);
    ///
    /// assert_eq!(exit, 0);
    /// ```
    pub fn build(self) -> App {
        App {
            definition: self.definition,
        }
    }

    /// Builds an [`AppRunner`] bound to the provided UI sink.
    ///
    /// This is the shortest path for tests and embedders that want one sink
    /// binding plus the full host behavior.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::App;
    /// use osp_cli::app::BufferedUiSink;
    ///
    /// let mut sink = BufferedUiSink::default();
    /// let exit = App::builder()
    ///     .build_with_sink(&mut sink)
    ///     .run_process(["osp", "--help"]);
    ///
    /// assert_eq!(exit, 0);
    /// assert!(!sink.stdout.is_empty());
    /// assert!(sink.stderr.is_empty());
    /// ```
    pub fn build_with_sink<'a>(self, sink: &'a mut dyn UiSink) -> AppRunner<'a> {
        self.build().with_sink(sink)
    }
}

/// Runs the default application instance and returns a process exit code.
///
/// This is shorthand for building [`App::new`] and calling
/// [`App::run_process`], using process stdio for rendered output and
/// user-facing errors.
///
/// # Examples
///
/// ```
/// let exit = osp_cli::app::run_process(["osp", "--help"]);
///
/// assert_eq!(exit, 0);
/// ```
pub fn run_process<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let mut sink = StdIoUiSink;
    run_process_with_sink(args, &mut sink)
}

/// Runs the default application instance with the provided sink.
///
/// This is shorthand for building [`App::new`] and calling
/// [`App::run_process_with_sink`].
///
/// # Examples
///
/// ```
/// use osp_cli::app::{BufferedUiSink, run_process_with_sink};
///
/// let mut sink = BufferedUiSink::default();
/// let exit = run_process_with_sink(["osp", "--help"], &mut sink);
///
/// assert_eq!(exit, 0);
/// assert!(!sink.stdout.is_empty());
/// assert!(sink.stderr.is_empty());
/// ```
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
