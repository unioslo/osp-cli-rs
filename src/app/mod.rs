//! CLI and REPL startup through the public [`App`] entrypoints.
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
//! - most callers should start with [`App`]
//! - configure the host through [`crate::App::new`] and its `with_*` methods
//! - access-policy hooks inspect configuration and command scope through
//!   [`crate::app::AppRuntime`]
//! - lower-level semantic payloads live in modules like [`crate::core`] and
//!   [`crate::completion`]; this module assembles the host
//!
//! Use this module when you want:
//!
//! - the full CLI / REPL host in-process
//! - the same dispatch/help/completion/config behavior as `osp`
//! - a wrapper crate that injects native commands or product defaults
//!
//! Skip this module and start lower if you only need:
//!
//! - rendering rows/documents: [`crate::ui`]
//! - pure completion trees or guide payloads: [`crate::completion`] or
//!   [`crate::core`]
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
//! - [`App::run_with_sink`] when a test or outer host wants captured
//!   host-rendered one-shot stdout/stderr; REPL input and prompts still use the
//!   process terminal
//! Downstream product-wrapper pattern:
//!
//! - keep site-specific auth, policy, and integration state in the wrapper
//!   crate rather than in [`crate::app`]
//! - build a [`crate::NativeCommandRegistry`] for product-specific commands
//! - build one product-owned defaults layer under `extensions.<site>.*`
//! - inject both through [`App::with_native_commands`] and
//!   [`App::with_product_defaults`]
//! - expose a thin product-level `run_process` or `builder()` API on top
//!   instead of forking generic host behavior

use crate::config::ConfigLayer;
use crate::core::output::OutputFormat;
use crate::native::NativeCommandRegistry;
use crate::ui::messages::{MessageLevel, adjust_verbosity};
use std::ffi::OsString;
use std::sync::Arc;

mod access;
pub(crate) mod assembly;
pub(crate) mod bootstrap;
pub(crate) mod builtin;
pub(crate) mod command_output;
pub(crate) mod command_plan;
pub(crate) mod config_explain;
pub(crate) mod dispatch;
pub(crate) mod external;
pub(crate) mod facts;
pub(crate) mod help;
pub(crate) mod host;
pub(crate) mod logging;
pub(crate) mod runtime;
pub(crate) mod session;
/// UI sink abstractions used by the host entrypoints.
pub(crate) mod sink;
#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests;
pub(crate) mod timing;

pub(crate) use crate::config::is_sensitive_key;
pub use access::{AccessPolicy, Visibility};
pub(crate) use assembly::rebuild_repl_in_place;
pub(crate) use bootstrap::*;
pub(crate) use builtin::*;
pub(crate) use command_output::*;
pub(crate) use config_explain::{
    config_explain_json, config_explain_result, config_value_to_json_for_key,
    explain_runtime_config, format_scope, push_missing_config_key_messages,
    render_config_explain_text,
};
pub(crate) use facts::*;
pub use host::run_from;
pub(crate) use host::*;
pub use runtime::AppRuntime;
pub(crate) use runtime::{
    AppClients, AuthState, ConfigState, LaunchContext, RuntimeContext, TerminalKind, UiState,
};
#[cfg(test)]
pub(crate) use session::DebugTimingBadge;
pub(crate) use session::{
    AppSession, AppState, AppStateBuilder, DebugTimingState, LastFailure, ReplScopeFrame,
    ReplScopeStack,
};
pub use sink::{BufferedUiSink, StdIoUiSink, StderrSpinner, UiSink};

type ReplLineHandler = dyn Fn(&str) -> Option<std::ops::ControlFlow<i32, String>> + Send + Sync;

type ProductAction = dyn Fn(&mut AppRuntime) -> miette::Result<String> + Send + Sync;

/// Optional action-flag argument placed in the invocation configuration layer.
#[derive(Clone, Copy)]
pub struct ActionFlagValue {
    /// Argument name displayed in help.
    pub value_name: &'static str,
    /// Configuration key receiving a supplied argument.
    pub config_key: &'static str,
}

#[derive(Clone)]
struct ProductActionFlag {
    name: &'static str,
    help: &'static str,
    optional_value: Option<ActionFlagValue>,
    run: Arc<ProductAction>,
}

pub(crate) type IntroDecoration =
    dyn Fn(&crate::ui::ResolvedRenderSettings) -> Vec<String> + Send + Sync;

#[derive(Clone, Default)]
pub(crate) struct AppDefinition {
    native_commands: NativeCommandRegistry,
    product_defaults: ConfigLayer,
    product_help_options: Vec<(String, String)>,
    action_flags: Vec<ProductActionFlag>,
    config_flags: Vec<(clap::Arg, &'static str, Option<&'static str>)>,
    access_policy: Option<Arc<dyn AccessPolicy>>,
    pub(crate) repl_line_handler: Option<Arc<ReplLineHandler>>,
    pub(crate) intro_decoration: Option<Arc<IntroDecoration>>,
}

#[derive(Clone, Default)]
/// Top-level application entrypoint for CLI and REPL execution.
///
/// Most embedders should start here instead of trying
/// to assemble runtime/session machinery directly.
///
/// ```no_run
/// use osp_cli::App;
///
/// fn main() {
///     std::process::exit(
///         App::new().run_process(std::env::args_os()),
///     );
/// }
/// ```
#[must_use]
pub struct App {
    definition: Arc<AppDefinition>,
}

impl App {
    /// Creates an application with the default native command registry and no
    /// wrapper-owned defaults.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::App;
    ///
    /// let app = App::new();
    /// let exit = app.run_process(["osp", "--help"]);
    ///
    /// assert_eq!(exit, 0);
    /// ```
    pub fn new() -> Self {
        Self::default()
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
        Arc::make_mut(&mut self.definition).native_commands = native_commands;
        self
    }

    /// Hosts a guided REPL with product-owned introduction and input.
    ///
    /// Return `None` for normal dispatch, `ControlFlow::Continue(text)` to
    /// print a message, or `ControlFlow::Break(code)` to leave the REPL.
    /// Consumed lines do not enter history or replace the previous command
    /// result. The normal intro is suppressed; Ctrl-C and Esc end this REPL.
    /// The line handler also runs for basic terminal input.
    pub fn with_guided_repl(
        mut self,
        handler: impl Fn(&str) -> Option<std::ops::ControlFlow<i32, String>> + Send + Sync + 'static,
    ) -> Self {
        Arc::make_mut(&mut self.definition).repl_line_handler = Some(Arc::new(handler));
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
        Arc::make_mut(&mut self.definition).product_defaults = product_defaults;
        self
    }

    /// Supplies local, trusted terminal artwork beside the intro's opening paragraph.
    ///
    /// Return no rows when decoration is unsuitable. The callback receives resolved
    /// screen capabilities and must not block, perform service calls or write output.
    /// Artwork is omitted from machine output and transformed intro data.
    pub fn with_intro_decoration<F>(mut self, render: F) -> Self
    where
        F: Fn(&crate::ui::ResolvedRenderSettings) -> Vec<String> + Send + Sync + 'static,
    {
        Arc::make_mut(&mut self.definition).intro_decoration = Some(Arc::new(render));
        self
    }

    /// Adds one wrapper-owned option to the root help surface.
    ///
    /// Product wrappers may consume bootstrap options before the generic host
    /// parser runs. Registering them here keeps those real options visible
    /// without teaching the generic grammar product-specific behavior.
    pub fn with_product_help_option(
        mut self,
        syntax: impl Into<String>,
        help: impl Into<String>,
    ) -> Self {
        Arc::make_mut(&mut self.definition)
            .product_help_options
            .push((syntax.into(), help.into()));
        self
    }

    /// Registers an exclusive product action that exits after returning its message.
    ///
    /// An optional [`ActionFlagValue`] accepts zero or one value and places
    /// a supplied value in the invocation's config layer before policy startup.
    /// The action receives the assembled CLI runtime; it never enters the REPL.
    pub fn with_action_flag<F>(
        mut self,
        name: &'static str,
        help: &'static str,
        optional_value: Option<ActionFlagValue>,
        run: F,
    ) -> Self
    where
        F: Fn(&mut AppRuntime) -> miette::Result<String> + Send + Sync + 'static,
    {
        Arc::make_mut(&mut self.definition)
            .action_flags
            .push(ProductActionFlag {
                name,
                help,
                optional_value,
                run: Arc::new(run),
            });
        self
    }

    /// Registers a root flag that sets one invocation-only config value.
    /// Flags targeting the same key are mutually exclusive.
    pub fn with_config_flag(
        mut self,
        name: &'static str,
        help: &'static str,
        key: &'static str,
        value: &'static str,
    ) -> Self {
        Arc::make_mut(&mut self.definition).config_flags.push((
            clap::Arg::new(name)
                .long(name)
                .help(help)
                .action(clap::ArgAction::SetTrue),
            key,
            Some(value),
        ));
        self
    }

    /// Registers a root option whose value sets invocation-only configuration.
    /// The argument owns validation and possible values for shell completion.
    pub fn with_config_value_flag(mut self, argument: clap::Arg, key: &'static str) -> Self {
        Arc::make_mut(&mut self.definition)
            .config_flags
            .push((argument, key, None));
        self
    }

    /// Installs product visibility, startup, acquisition and live session facts.
    ///
    /// ```
    /// use osp_cli::App;
    /// use osp_cli::app::{AccessPolicy, Visibility};
    /// struct ProductAccess;
    /// impl AccessPolicy for ProductAccess {
    ///     fn visibility(&self, path: &[String]) -> Visibility {
    ///         if path.first().is_some_and(|name| name == "internal") {
    ///             Visibility::Hidden(Some("Use the product's operator profile".into()))
    ///         } else {
    ///             Visibility::Shown
    ///         }
    ///     }
    /// }
    /// let app = App::new().with_access_policy(ProductAccess);
    /// ```
    pub fn with_access_policy<P: AccessPolicy + 'static>(mut self, policy: P) -> Self {
        Arc::make_mut(&mut self.definition).access_policy = Some(Arc::new(policy));
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
        crate::startup_trace::initialize();
        let result = host::run_from_with_sink_and_app(args, &mut StdIoUiSink, &self.definition);
        crate::startup_trace::finish();
        result
    }

    /// Runs the application with the provided UI sink.
    ///
    /// Pass the same sink to subsequent calls to append their output.
    /// REPL input and prompts still use the process terminal.
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
    ///
    /// The same application and sink can serve subsequent invocations:
    ///
    /// ```
    /// use osp_cli::App;
    /// use osp_cli::app::BufferedUiSink;
    ///
    /// let mut sink = BufferedUiSink::default();
    /// let app = App::new();
    ///
    /// let first = app.run_with_sink(["osp", "--help"], &mut sink)?;
    /// let second = app.run_process_with_sink(["osp", "--help"], &mut sink);
    ///
    /// assert_eq!(first, 0);
    /// assert_eq!(second, 0);
    /// assert!(!sink.stdout.is_empty());
    /// assert!(sink.stderr.is_empty());
    /// # Ok::<(), miette::Report>(())
    /// ```
    pub fn run_with_sink<I, T>(&self, args: I, sink: &mut dyn UiSink) -> miette::Result<i32>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        crate::startup_trace::initialize();
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
        crate::startup_trace::initialize();
        let mut sink = StdIoUiSink;
        sink::take_stdout_write_failure();
        let code = self.run_process_with_sink(args, &mut sink);
        crate::startup_trace::finish();
        if sink::take_stdout_write_failure() && code == 0 {
            1
        } else {
            code
        }
    }

    /// Runs the application with the provided sink and returns a process exit
    /// code.
    ///
    /// This mirrors [`App::run_process`] and sends host-rendered one-shot output
    /// and user-facing errors through the supplied sink. Interactive REPL input
    /// and prompts still use the process terminal; native commands may also
    /// manage their own I/O.
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
    ///
    /// ```
    /// use osp_cli::app::BufferedUiSink;
    /// use osp_cli::App;
    ///
    /// let mut sink = BufferedUiSink::default();
    /// let exit = App::new()
    ///     .run_process_with_sink(["osp", "--help"], &mut sink);
    ///
    /// assert_eq!(exit, 0);
    /// assert!(!sink.stdout.is_empty());
    /// assert!(sink.stderr.is_empty());
    /// ```
    ///
    /// ```
    /// use osp_cli::App;
    /// use osp_cli::app::BufferedUiSink;
    ///
    /// let mut sink = BufferedUiSink::default();
    /// let exit = App::new()
    ///     .run_process_with_sink(["osp", "--help"], &mut sink);
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
        crate::startup_trace::initialize();
        let args = {
            let _phase =
                tracing::info_span!(target: "osp_startup", "process argv collection").entered();
            args.into_iter().map(Into::into).collect::<Vec<OsString>>()
        };
        let started = std::time::Instant::now();
        let mut timing = host::TimingSummary::default();
        let pipe = args.iter().position(|arg| arg == "|").unwrap_or(args.len());
        let mut scan = crate::cli::invocation::scan_cli_argv(&args[..pipe]);
        let result = host::run_scanned_with_sink_and_app(
            &args[pipe..],
            &mut scan,
            sink,
            &self.definition,
            &mut timing,
        );
        let code = match result {
            Ok(code) => code,
            Err(err) => {
                let (verbosity, debug_level, error_render_settings) = {
                    let _phase = tracing::info_span!(target: "osp_startup", "process error display preparation").entered();
                    if let Some(ui) = host::error_display_settings(&err) {
                        (ui.verbosity, ui.debug_level, ui.render_settings.clone())
                    } else {
                        let (verbose, quiet) = (scan.invocation.verbose, scan.invocation.quiet);
                        let context = help::render_settings_for_help_with_scan(
                            &scan,
                            &self.definition.product_defaults,
                            &crate::config::ConfigLayer::default(),
                        );
                        let verbosity = adjust_verbosity(
                            context
                                .config
                                .as_ref()
                                .map_or(MessageLevel::Success, bootstrap::verbosity_from_config),
                            verbose,
                            quiet,
                        );
                        let debug_level = context
                            .config
                            .as_ref()
                            .map_or(0, bootstrap::debug_level_from_config)
                            .saturating_add(scan.invocation.debug)
                            .min(3);
                        if let Some(config) = &context.config {
                            logging::init_developer_logging(bootstrap::build_logging_config(
                                config,
                                debug_level,
                            ));
                        }
                        (verbosity, debug_level, context.settings)
                    }
                };
                let err = host::report_with_invocation(
                    err,
                    host::ErrorDisplaySettings {
                        verbosity,
                        debug_level,
                        render_settings: error_render_settings.clone(),
                    },
                );
                let evidence = host::report_evidence(&err, verbosity);
                sink::write_diagnostics(sink, &evidence.0, &error_render_settings);
                let rendered = render_process_error(&err, &error_render_settings, verbosity, None);
                if error_render_settings.requested_format() == OutputFormat::Json {
                    sink.write_stdout(&rendered);
                    if debug_level >= 2 {
                        sink.write_stderr(&format!(
                            "{}\n",
                            render_report_message(&err, verbosity, &error_render_settings)
                                .trim_end()
                        ));
                    }
                } else {
                    sink::write_diagnostics(sink, &rendered, &error_render_settings);
                }
                if debug_level >= 2
                    && let Some(backtrace) = host::render_backtrace(&err, &error_render_settings)
                {
                    sink.write_stderr(&format!("{backtrace}\n"));
                }
                sink::write_diagnostics(sink, &evidence.1, &error_render_settings);
                classify_exit_code(&err)
            }
        };
        tracing::info!(target: "osp", exit_code = code,
            elapsed_ms = started.elapsed().as_millis(),
            startup_ms = timing.parse.unwrap_or_default().as_millis(),
            command_ms = timing.execute.unwrap_or_default().as_millis(), "finished");
        code
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
    crate::startup_trace::initialize();
    App::new().run_process(args)
}

/// Runs the default application instance with the provided sink.
///
/// This is shorthand for building [`App::new`] and calling
/// [`App::run_process_with_sink`].
///
/// # Examples
///
/// ```text
/// use osp_cli::app::{BufferedUiSink, run_process_with_sink};
///
/// let mut sink = BufferedUiSink::default();
/// let exit = run_process_with_sink(["osp", "--help"], &mut sink);
///
/// assert_eq!(exit, 0);
/// assert!(!sink.stdout.is_empty());
/// assert!(sink.stderr.is_empty());
/// ```
#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) fn run_process_with_sink<I, T>(args: I, sink: &mut dyn UiSink) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    App::new().run_process_with_sink(args, sink)
}

pub(crate) fn render_process_error(
    err: &miette::Report,
    settings: &crate::ui::RenderSettings,
    verbosity: MessageLevel,
    hint_action: Option<&str>,
) -> String {
    let _ = verbosity;
    let message = render_report_message(err, MessageLevel::Success, settings);
    if settings.requested_format() == OutputFormat::Json {
        let exit_code = classify_exit_code(err);
        let code = match exit_code {
            host::EXIT_CODE_USAGE => "usage_error",
            _ => "command_error",
        };
        return command_output::render_json_failure(
            command_output::json_failure_error(Some(err), code, message, settings),
            exit_code,
            Vec::new(),
            serde_json::Value::Null,
        );
    }

    let error = host::report_error_details(err, MessageLevel::Success, settings);
    let first = error
        .message
        .lines()
        .next()
        .unwrap_or("Command failed.")
        .trim_start_matches("error: ");
    let mut rendered = format!("error: {first}\n");
    if let Some(id) = &error.request_id {
        rendered.push_str(&format!("request-id: {id}\n"));
    }
    if let Some(action) = &error.action {
        rendered.push_str(&format!(
            "hint: {action}{}\n",
            hint_action.unwrap_or_default()
        ));
    }
    let resolved = crate::ui::resolve_settings(settings, crate::ui::RenderTarget::screen(settings));
    resolved.width.map_or_else(
        || rendered.clone(),
        |width| crate::ui::fit_prose(&rendered, width.saturating_sub(resolved.margin).max(1)),
    )
}

/// Emits a levelled message before a command host or credential flow resumes.
/// Uses the same config-derived presentation and filtering as command messages.
pub fn emit_message(
    config: &crate::config::ResolvedConfig,
    level: crate::core::MessageLevel,
    text: impl Into<String>,
    sink: &mut dyn UiSink,
) -> miette::Result<()> {
    let context = RuntimeContext::new(
        None,
        if config.terminal() == Some("repl") {
            TerminalKind::Repl
        } else {
            TerminalKind::Cli
        },
        std::env::var("TERM").ok(),
    );
    let ui = UiState::from_resolved_config(&context, config)?;
    let mut messages = crate::ui::messages::MessageBuffer::default();
    messages.push(level, text);
    command_output::emit_messages_for_ui(config, &ui, &messages, ui.verbosity, sink);
    Ok(())
}
