//! Top-level host entrypoint glue.
//!
//! This module ties together argv scanning, runtime/config bootstrap, dispatch
//! planning, sink rendering, and final exit-code classification. It is the
//! main "run the app like a program" surface above the lower-level runtime and
//! dispatch helpers.

use crate::config::ResolvedConfig;
use crate::native::NativeCommandRegistry;
use crate::repl;
use clap::{CommandFactory, FromArgMatches};
use miette::{Result, miette};
use nu_ansi_term::Style;

use crate::core::document::{DocBlock, Document, DocumentEntry, HelpLevel};
use crate::ui::RenderSettings;
use std::backtrace::Backtrace;
use std::borrow::Cow;
use std::ffi::OsString;
use std::fmt::{Debug, Formatter};
use std::time::Instant;

use super::help;
use super::help::help_level;
use crate::app::logging::{DeveloperLoggingConfig, init_developer_logging};
use crate::app::sink::{StdIoUiSink, UiSink};
use crate::app::{AppClients, AppState, AuthState, UiState};
use crate::cli::invocation::{
    InvocationOptions, ScannedCliArgs, extend_with_invocation_help, scan_cli_argv,
};
use crate::cli::{Cli, Commands};
use crate::plugin::CommandCatalogEntry;

#[cfg(test)]
pub(crate) use super::assembly::rebuild_repl_state;
pub(crate) use super::dispatch::{
    DispatchPlan, RunAction, build_dispatch_plan, normalize_profile_override,
};
use super::external::run_external_command;
pub(crate) use super::external::run_external_command_with_help_renderer;
pub(crate) use super::timing::{TimingSummary, format_timing_badge};
pub(crate) const CMD_PLUGINS: &str = "plugins";
pub(crate) const CMD_DOCTOR: &str = "doctor";
pub(crate) const CMD_CONFIG: &str = "config";
pub(crate) const CMD_THEME: &str = "theme";
pub(crate) const CMD_HISTORY: &str = "history";
pub(crate) const CMD_CONTEXT: &str = "context";
pub(crate) const CMD_INTRO: &str = "intro";
pub(crate) const CMD_HELP: &str = "help";
pub(crate) const CMD_LIST: &str = "list";
pub const EXIT_CODE_ERROR: i32 = 1;
pub const EXIT_CODE_USAGE: i32 = 2;
pub const EXIT_CODE_WAITING_APPROVAL: i32 = 3;
pub(crate) const DEFAULT_REPL_PROMPT: &str = "╭─{user}@{domain} {indicator}\n╰─{profile}> ";
pub(crate) const CURRENT_TERMINAL_SENTINEL: &str = "__current__";

#[derive(Debug, Clone)]
pub(crate) struct ReplCommandSpec {
    pub(crate) name: Cow<'static, str>,
    pub(crate) supports_dsl: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedInvocation {
    pub(crate) source: bool,
    pub(crate) ui: UiState,
    pub(crate) help_level: HelpLevel,
}

struct PreparedHostRun {
    state: AppState,
    dispatch: DispatchPlan,
    invocation_ui: ResolvedInvocation,
}

#[derive(Debug, Clone)]
pub(crate) struct ErrorDisplaySettings {
    pub(crate) verbosity: crate::ui::messages::MessageLevel,
    pub(crate) debug_level: u8,
    pub(crate) render_settings: RenderSettings,
}

impl From<&UiState> for ErrorDisplaySettings {
    fn from(ui: &UiState) -> Self {
        Self {
            verbosity: ui.verbosity,
            debug_level: ui.debug_level,
            render_settings: ui.render_settings.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ErrorContext {
    NativeExecution,
    Config,
    Operation(&'static str),
}

impl std::fmt::Display for ErrorContext {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NativeExecution => "native command execution failed",
            Self::Config => "config resolution failed",
            Self::Operation(text) => text,
        })
    }
}

#[derive(Debug)]
struct BoxedContextError {
    context: ErrorContext,
    source: Box<dyn std::error::Error + Send + Sync + 'static>,
    backtrace: Backtrace,
    command_path: Option<String>,
    command_corrections: Vec<String>,
    structured: Option<crate::core::CommandError>,
}

#[derive(Debug)]
struct ReportContextError {
    context: Option<ErrorContext>,
    source: miette::Report,
    error_display: Option<ErrorDisplaySettings>,
    diagnostics: Option<crate::diagnostics::DiagnosticSink>,
    backtrace: Option<Backtrace>,
}

#[derive(Clone, Copy)]
struct KnownErrorChain<'a> {
    boxed_context: Option<&'a BoxedContextError>,
    clap: Option<&'a clap::Error>,
    config: Option<&'a crate::config::ConfigError>,
}

impl<'a> KnownErrorChain<'a> {
    fn inspect(err: &'a miette::Report) -> Self {
        Self {
            boxed_context: find_error_in_chain::<BoxedContextError>(err),
            clap: find_error_in_chain::<clap::Error>(err),
            config: find_error_in_chain::<crate::config::ConfigError>(err),
        }
    }
}

impl std::fmt::Display for BoxedContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.context, f)
    }
}

impl std::error::Error for BoxedContextError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}

impl miette::Diagnostic for BoxedContextError {}

impl std::fmt::Display for ReportContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.context {
            Some(context) => std::fmt::Display::fmt(&context, f),
            None => std::fmt::Display::fmt(&self.source, f),
        }
    }
}

impl std::error::Error for ReportContextError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}
impl miette::Diagnostic for ReportContextError {
    fn code<'a>(&'a self) -> Option<Box<dyn std::fmt::Display + 'a>> {
        self.source.code()
    }

    fn severity(&self) -> Option<miette::Severity> {
        self.source.severity()
    }

    fn help<'a>(&'a self) -> Option<Box<dyn std::fmt::Display + 'a>> {
        self.source.help()
    }

    fn url<'a>(&'a self) -> Option<Box<dyn std::fmt::Display + 'a>> {
        self.source.url()
    }

    fn source_code(&self) -> Option<&dyn miette::SourceCode> {
        self.source.source_code()
    }

    fn labels(&self) -> Option<Box<dyn Iterator<Item = miette::LabeledSpan> + '_>> {
        self.source.labels()
    }

    fn related<'a>(&'a self) -> Option<Box<dyn Iterator<Item = &'a dyn miette::Diagnostic> + 'a>> {
        self.source.related()
    }

    fn diagnostic_source(&self) -> Option<&dyn miette::Diagnostic> {
        Some(self.source.as_ref())
    }
}

/// Runs the top-level CLI entrypoint from an argv-like iterator.
///
/// This is the library-friendly wrapper around the binary entrypoint and
/// returns the process exit code that should be reported to the caller.
pub fn run_from<I, T>(args: I) -> Result<i32>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    crate::startup_trace::initialize();
    let mut sink = StdIoUiSink;
    let result = run_from_with_sink(args, &mut sink);
    crate::startup_trace::finish();
    result
}

pub(crate) fn run_from_with_sink<I, T>(args: I, sink: &mut dyn UiSink) -> Result<i32>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    run_from_with_sink_and_app(args, sink, &std::sync::Arc::default())
}

pub(crate) fn run_from_with_sink_and_app<I, T>(
    args: I,
    sink: &mut dyn UiSink,
    app: &std::sync::Arc<super::AppDefinition>,
) -> Result<i32>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    crate::startup_trace::initialize();
    let argv = {
        let _phase = tracing::info_span!(target: "osp_startup", "argv collection").entered();
        args.into_iter().map(Into::into).collect::<Vec<OsString>>()
    };
    let pipe = argv.iter().position(|arg| arg == "|").unwrap_or(argv.len());
    let mut scan = scan_cli_argv(&argv[..pipe]);
    run_scanned_with_sink_and_app(
        &argv[pipe..],
        &mut scan,
        sink,
        app,
        &mut TimingSummary::default(),
    )
}

pub(crate) fn run_scanned_with_sink_and_app(
    suffix: &[OsString],
    scanned: &mut ScannedCliArgs,
    sink: &mut dyn UiSink,
    app: &std::sync::Arc<super::AppDefinition>,
    timing: &mut TimingSummary,
) -> Result<i32> {
    {
        let _phase =
            tracing::info_span!(target: "osp_startup", "developer logging bootstrap").entered();
        init_developer_logging(DeveloperLoggingConfig::new(scanned.invocation.debug));
    }
    let (suffix, stages) = {
        let _phase = tracing::info_span!(target: "osp_startup", "argv/invocation scan").entered();
        if let Some(error) = scanned.error.take() {
            return Err(error);
        }
        let suffix = suffix
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let mut stage_tokens = vec!["command".to_string()];
        stage_tokens.extend(suffix.iter().cloned());
        let stages = crate::cli::pipeline::split_command_tokens(&stage_tokens).stages;
        crate::cli::validate_cli_dsl_stages(&stages)?;
        (suffix, stages)
    };
    let (parsed, help_command) = {
        let _phase = tracing::info_span!(target: "osp_startup", "clap parse").entered();
        let mut command = Cli::command();
        for (argument, key, _) in &app.config_flags {
            let conflicts = app
                .config_flags
                .iter()
                .filter(|(other, other_key, _)| {
                    other.get_id() != argument.get_id() && other_key == key
                })
                .map(|(other, _, _)| other.get_id().clone());
            command = command.arg(argument.clone().conflicts_with_all(conflicts));
        }
        for flag in &app.action_flags {
            let mut arg = clap::Arg::new(flag.name).long(flag.name).help(flag.help);
            arg = if let Some(value) = flag.optional_value {
                arg.value_name(value.value_name)
                    .num_args(0..=1)
                    .default_missing_value("")
            } else {
                arg.action(clap::ArgAction::SetTrue)
            };
            command = command.arg(arg);
        }
        // External words use framework dispatch rather than Clap's generated
        // help command. Builtins keep their own Clap help subcommands.
        let external = command
            .clone()
            .disable_help_subcommand(true)
            .try_get_matches_from(scanned.argv.iter().cloned())
            .ok()
            .and_then(|matches| Cli::from_arg_matches(&matches).ok())
            .is_some_and(|cli| matches!(cli.command, Some(Commands::External(_))));
        let parsed = command
            .clone()
            .disable_help_subcommand(external)
            .try_get_matches_from(scanned.argv.iter().cloned())
            .and_then(|matches| {
                let mut cli = Cli::from_arg_matches(&matches)?;
                // Resolve built-in extensions before attaching one-shot stages.
                cli.command = cli.command.map(Commands::with_app_doctor);
                cli.product_overrides = product_config_overrides(app, &matches);
                for flag in &app.action_flags {
                    if matches.value_source(flag.name) == Some(clap::parser::ValueSource::CommandLine) {
                        if cli.product_action.is_some() || cli.command.is_some() {
                            return Err(clap::Error::raw(clap::error::ErrorKind::ArgumentConflict,
                                "product action flags cannot be combined with commands or other actions"));
                        }
                        cli.product_action = Some(flag.name.to_string());
                    }
                }
                Ok(cli)
            });
        (parsed, command)
    };
    match parsed {
        Ok(Cli {
            command: Some(Commands::Completions(args)),
            ..
        }) => {
            if !stages.is_empty() {
                return Err(miette!(
                    "shell completion scripts do not support DSL pipeline stages"
                ));
            }
            let _phase = tracing::info_span!(target: "osp_startup", "command execution").entered();
            sink.write_stdout(&crate::cli::render_shell_completions(args, &help_command));
            Ok(0)
        }
        Ok(mut cli) => {
            if let Some(Commands::External(tokens)) = cli.command.as_mut() {
                tokens.extend(suffix);
                run(
                    cli,
                    scanned.invocation.clone(),
                    &[],
                    &resolved_command_path(&help_command, &scanned.argv),
                    sink,
                    app,
                    timing,
                )
            } else {
                run(
                    cli,
                    scanned.invocation.clone(),
                    &stages,
                    &resolved_command_path(&help_command, &scanned.argv),
                    sink,
                    app,
                    timing,
                )
            }
        }
        Err(err) => handle_clap_parse_error(scanned, &help_command, err, &stages, sink, app),
    }
}

fn product_config_overrides(
    app: &std::sync::Arc<super::AppDefinition>,
    matches: &clap::ArgMatches,
) -> crate::config::ConfigLayer {
    let mut overrides = crate::config::ConfigLayer::default();
    for (argument, key, fixed) in &app.config_flags {
        let name = argument.get_id().as_str();
        if let Some(value) = fixed {
            if matches.get_flag(name) {
                overrides.set(*key, *value);
            }
        } else if let Some(value) = matches.get_many::<String>(name) {
            overrides.set(*key, value.cloned().collect::<Vec<_>>().join(","));
        }
    }
    for flag in &app.action_flags {
        if let Some(argument) = flag.optional_value
            && let Some(value) = matches
                .get_one::<String>(flag.name)
                .filter(|v| !v.is_empty())
        {
            overrides.set(argument.config_key, value.clone());
        }
    }
    overrides
}

fn handle_clap_parse_error(
    scanned: &ScannedCliArgs,
    command: &clap::Command,
    err: clap::Error,
    stages: &[String],
    sink: &mut dyn UiSink,
    app: &std::sync::Arc<super::AppDefinition>,
) -> Result<i32> {
    match err.kind() {
        clap::error::ErrorKind::DisplayHelp
        | clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
            let _phase = tracing::info_span!(target: "osp_startup", "command execution").entered();
            let tokens = scanned
                .argv
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            let product_overrides = crate::cli::passive_command(command.clone())
                .try_get_matches_from(crate::cli::help_parse_tokens(&tokens))
                .map(|matches| {
                    let mut overrides = product_config_overrides(app, &matches);
                    if let Ok(mut cli) = Cli::from_arg_matches(&matches) {
                        cli.product_overrides = overrides;
                        overrides = super::build_cli_session_layer(&cli).unwrap_or_default();
                    }
                    overrides
                })
                .unwrap_or_default();
            let help_context = help::render_settings_for_help_with_scan(
                scanned,
                &app.product_defaults,
                &product_overrides,
            );
            if let Some(config) = &help_context.config {
                sink.write_stderr(&config.ignored_keys_warning());
                let _phase = tracing::info_span!(target: "osp_startup", "native command configure")
                    .entered();
                app.native_commands.configure(config, false);
            }
            let command = crate::cli::commands::doctor::with_app_commands(
                command.clone(),
                &app.native_commands,
            );
            let (mut body, root_help, long_help) = crate::cli::command_help_document(
                command.clone(),
                &tokens,
                &crate::core::CommandScope::OneShot,
            );
            extend_with_invocation_help(&mut body, help_context.help_level);
            if root_help {
                add_product_option_help(&mut body, &app.product_help_options);
                let auth =
                    AuthState::for_policy(app.access_policy.clone(), help_context.config.as_ref());
                add_native_command_help(&mut body, &app.native_commands, &auth);
                if let Some(config) = &help_context.config {
                    let matches = crate::cli::passive_command(command.clone())
                        .try_get_matches_from(crate::cli::help_parse_tokens(&tokens))
                        .ok();
                    let mut launch = super::LaunchContext::default();
                    if let Some(matches) = &matches {
                        launch.plugin_dirs = matches
                            .try_get_many::<std::path::PathBuf>("plugin_dirs")
                            .ok()
                            .flatten()
                            .into_iter()
                            .flatten()
                            .cloned()
                            .collect();
                        let flag = |name| {
                            matches
                                .try_get_one::<bool>(name)
                                .ok()
                                .flatten()
                                .copied()
                                .unwrap_or(false)
                        };
                        launch.runtime_load = crate::cli::runtime_load_options_from_flags(
                            flag("no_env"),
                            flag("no_config_file"),
                            flag("defaults_only"),
                        );
                    }
                    let plugins = super::assembly::build_plugin_manager(config, &launch);
                    let natives = app
                        .native_commands
                        .catalog()
                        .into_iter()
                        .map(|e| e.name)
                        .collect::<std::collections::HashSet<_>>();
                    let entries = plugins
                        .command_catalog()
                        .into_iter()
                        .filter(|entry| {
                            !natives.contains(&entry.name)
                                && !crate::cli::builtin_command_names().contains(&entry.name)
                                && auth.is_external_command_visible(&entry.name)
                        })
                        .map(|entry| DocumentEntry {
                            role: crate::core::document::EntryRole::Command,
                            name: entry.name,
                            description: entry.about,
                        })
                        .collect::<Vec<_>>();
                    if !entries.is_empty() {
                        body.blocks.push(DocBlock::section(
                            "Plugins",
                            vec![DocBlock::Entries { entries }],
                        ));
                    }
                }
            }
            let mut filtered = body.filtered_for_help_level(help_context.help_level);
            let dsl_help = stages.iter().find_map(|raw| {
                let parsed = crate::dsl::parse::pipeline::parse_stage(raw).ok()?;
                crate::cli::is_cli_help_stage(&parsed).then_some(parsed.spec)
            });
            if let Some(spec) = &dsl_help {
                filtered = crate::dsl::verb_info::help_document(spec)?;
            }
            let stages = if dsl_help.is_some() { &[][..] } else { stages };
            let mut output = crate::dsl::apply_output_pipeline(filtered.to_output_result(), stages)
                .map_err(|error| miette!("failed to transform help output: {error}"))?;
            output.meta.document_long_help = long_help;
            let rendered = crate::ui::render(
                &output,
                &crate::ui::RenderRequest {
                    layout: Some(help_context.layout),
                    ..crate::ui::RenderRequest::new(&help_context.settings)
                },
            );
            sink.write_stdout(&rendered);
            Ok(0)
        }
        clap::error::ErrorKind::DisplayVersion => {
            if !stages.is_empty() {
                return Err(miette!("version text does not support DSL pipeline stages"));
            }
            let _phase = tracing::info_span!(target: "osp_startup", "command execution").entered();
            sink.write_stdout(&err.to_string());
            Ok(0)
        }
        _ => Err(report_clap_error_with_context(
            err,
            crate::app::host::ErrorContext::Operation("failed to parse CLI arguments"),
            command.clone(),
            scanned.argv.clone(),
            None,
        )),
    }
}

// Keep the top-level CLI entrypoint readable as a table of contents:
// normalize input -> bootstrap runtime state -> hand off to the selected mode.
fn run(
    mut cli: Cli,
    invocation: InvocationOptions,
    stages: &[String],
    command_path: &[String],
    sink: &mut dyn UiSink,
    app: &std::sync::Arc<super::AppDefinition>,
    timing: &mut TimingSummary,
) -> Result<i32> {
    let run_started = Instant::now();
    let PreparedHostRun {
        mut state,
        dispatch,
        mut invocation_ui,
    } = {
        let _phase = tracing::info_span!(target: "osp_startup", "host startup").entered();
        prepare_host_run(&mut cli, &invocation, app, run_started)?
    };

    sink.write_stderr(&state.runtime.config.resolved().ignored_keys_warning());

    if let Some(help) = crate::repl::completion::maybe_render_dsl_help(
        crate::repl::ReplViewContext::from_parts(&state.runtime, &state.session),
        stages,
    ) {
        sink.write_stdout(&help);
        return Ok(0);
    }
    if matches!(dispatch.action, RunAction::Builtin(_)) {
        apply_command_format(
            state.runtime.config.resolved(),
            &mut invocation_ui.ui,
            command_path,
        );
    }
    if !stages.is_empty()
        && matches!(
            dispatch.action,
            RunAction::Repl | RunAction::ProductAction(_)
        )
    {
        return Err(miette!("a pipeline requires a command"));
    }

    let action_started = Instant::now();
    let is_repl = matches!(dispatch.action, RunAction::Repl);
    let action = dispatch.action;
    let result = {
        let _phase = (!is_repl)
            .then(|| tracing::info_span!(target: "osp_startup", "command execution").entered());
        match action {
            RunAction::ProductAction(name) => {
                let flag = app
                    .action_flags
                    .iter()
                    .find(|flag| flag.name == name)
                    .ok_or_else(|| miette!("unknown product action: {name}"))?;
                let diagnostics = crate::diagnostics::DiagnosticSink::default();
                let _scope = diagnostics.enter();
                state.runtime.ui = invocation_ui.ui.clone();
                match (flag.run)(&mut state.runtime) {
                    Ok(message) => {
                        let evidence = diagnostics.render(invocation_ui.ui.verbosity, false);
                        sink.write_stderr(&evidence.0);
                        if !message.is_empty() {
                            sink.write_stdout(&format!("{message}\n"));
                        }
                        sink.write_stderr(&evidence.1);
                        Ok(0)
                    }
                    Err(error) => Err(report_with_evidence(error, diagnostics)),
                }
            }
            RunAction::Repl => {
                state.runtime.ui = invocation_ui.ui.clone();
                repl::run_plugin_repl(&mut state)
            }
            RunAction::External(tokens) => {
                run_external_command(&mut state, &tokens, &invocation, &mut invocation_ui, sink)
            }
            RunAction::Builtin(command) => super::run_cli_builtin_command_parts(
                &mut state.runtime,
                &mut state.session,
                &state.clients,
                &invocation_ui,
                command,
                stages,
                sink,
            ),
        }
    };

    if !is_repl {
        let total = run_started.elapsed();
        let startup = action_started.saturating_duration_since(run_started);
        *timing = TimingSummary {
            total,
            parse: Some(startup),
            execute: Some(total.saturating_sub(startup)),
            render: None,
        };
    }

    tracing::info!(target: "osp::diagnostic", "debug: Command complete; exit={}; startup={} ms; dispatch={} ms.", result.as_ref().copied().unwrap_or(1), action_started.saturating_duration_since(run_started).as_millis(), action_started.elapsed().as_millis());
    tracing::trace!(target: "osp::diagnostic", "trace: +{} ms dispatch.end; exit={}.", run_started.elapsed().as_millis(), result.as_ref().copied().unwrap_or(1));
    result.map_err(|err| report_with_invocation(err, ErrorDisplaySettings::from(&invocation_ui.ui)))
}

// Startup is phase-based:
// 1. choose the action from argv alone
// 2. assemble the runtime/session layer for the chosen action
// 3. apply startup-time side effects before handing off to execution
fn prepare_host_run(
    cli: &mut Cli,
    invocation: &InvocationOptions,
    app: &std::sync::Arc<super::AppDefinition>,
    run_started: Instant,
) -> Result<PreparedHostRun> {
    let dispatch = {
        let _phase = tracing::info_span!(target: "osp_startup", "dispatch planning").entered();
        build_dispatch_plan(cli)?
    };
    tracing::debug!(
        action = %dispatch.action.name(),
        profile_override = ?dispatch.profile_override,
        "built dispatch plan"
    );

    let terminal_kind = dispatch.action.terminal_kind();
    let mut state = {
        super::assembly::build_host(super::assembly::HostBuildRequest::Startup {
            cli,
            action: &dispatch.action,
            profile_override: dispatch.profile_override.clone(),
            terminal_kind,
            started: run_started,
            definition: app.clone(),
        })?
        .state()
    };
    {
        let _phase = tracing::info_span!(target: "osp_startup", "startup hook").entered();
        if let Some(policy) = &app.access_policy {
            let base_verbosity = state.runtime.ui.verbosity;
            state.runtime.ui.verbosity = resolve_invocation_ui(
                state.runtime.config.resolved(),
                &state.runtime.ui,
                invocation,
            )
            .ui
            .verbosity;
            policy.startup(&mut state.runtime)?;
            state.runtime.ui.verbosity = base_verbosity;
        }
    }
    let invocation_ui = resolve_invocation_ui(
        state.runtime.config.resolved(),
        &state.runtime.ui,
        invocation,
    );
    super::assembly::apply_runtime_side_effects(
        state.runtime.config.resolved(),
        invocation_ui.ui.debug_level,
        &state.runtime.themes,
    );
    tracing::debug!(
        debug_level = invocation_ui.ui.debug_level,
        "developer logging initialized"
    );
    tracing::info!(
        profile = %state.runtime.config.resolved().active_profile(),
        terminal = %state.runtime.context.terminal_kind().as_config_terminal(),
        action = %dispatch.action.name(),
        plugin_timeout_ms = super::plugin_process_timeout(state.runtime.config.resolved()).as_millis(),
        "osp session initialized"
    );

    tracing::info!(target: "osp::diagnostic", "debug: Startup complete; profile={}.", crate::diagnostics::safe_text(state.runtime.config.resolved().active_profile()));
    if invocation_ui.ui.debug_level >= 2 {
        let config = state.runtime.config.resolved();
        if let Some(path) =
            crate::config::RuntimeConfigPaths::discover_with(state.runtime.launch.runtime_load)
                .config_file
        {
            tracing::debug!(target: "osp::diagnostic", "debug: Config loaded; source={}; profile={}.", crate::diagnostics::safe_text(&path.display().to_string()), crate::diagnostics::safe_text(config.active_profile()));
        }
        for key in ["ui.verbosity", "debug.level"] {
            if let Some(entry) = config.values().get(key) {
                tracing::debug!(target: "osp::diagnostic", "debug: Config selected; key={key}; source={}.", entry.source);
            }
        }
    }
    tracing::debug!(target: "osp::diagnostic", "debug: Command scope resolved; scope=root.");
    tracing::trace!(target: "osp::diagnostic", "trace: +{} ms startup.end; terminal={}.", run_started.elapsed().as_millis(), state.runtime.context.terminal_kind().as_config_terminal());
    Ok(PreparedHostRun {
        state,
        dispatch,
        invocation_ui,
    })
}

pub(crate) fn authorized_command_catalog_for(
    auth: &AuthState,
    clients: &AppClients,
    native_catalog: Vec<CommandCatalogEntry>,
) -> Vec<CommandCatalogEntry> {
    let mut all = native_catalog;
    let mut names = all
        .iter()
        .map(|entry| entry.name.clone())
        .collect::<std::collections::HashSet<_>>();
    all.extend(
        clients
            .plugins()
            .command_catalog()
            .into_iter()
            .filter(|entry| {
                !crate::cli::builtin_command_names().contains(&entry.name)
                    && names.insert(entry.name.clone())
            }),
    );
    all.sort_by(|left, right| left.name.cmp(&right.name));
    all.into_iter()
        .filter(|entry| auth.is_external_command_visible(&entry.name))
        .map(|mut entry| {
            auth.filter_command_spec(&mut entry.completion, &[]);
            entry.subcommands.retain(|name| {
                entry
                    .completion
                    .children
                    .values()
                    .any(|child| child.name == *name)
            });
            entry
        })
        .collect()
}

pub(crate) fn resolve_invocation_ui(
    config: &ResolvedConfig,
    ui: &UiState,
    invocation: &InvocationOptions,
) -> ResolvedInvocation {
    let mut render_settings = ui.render_settings.clone();
    // A process-level format flag remains explicit until this command overrides it.
    render_settings.format_explicit =
        ui.render_settings.format_explicit || invocation.format.is_some();
    if let Some(format) = invocation.format {
        render_settings.format = format;
    }
    if let Some(mode) = invocation.mode {
        render_settings.mode = mode;
    }
    if let Some(color) = invocation.color {
        render_settings.color = color;
    }
    if let Some(unicode) = invocation.unicode {
        render_settings.unicode = unicode;
    }

    let help_level = help_level(config, invocation.verbose, invocation.quiet);
    render_settings.table_hints = invocation.quiet == 0 && help_level != HelpLevel::None;
    ResolvedInvocation {
        source: invocation.source,
        ui: UiState::new(
            render_settings,
            crate::ui::messages::adjust_verbosity(
                ui.verbosity,
                invocation.verbose,
                invocation.quiet,
            ),
            ui.debug_level.saturating_add(invocation.debug).min(3),
        ),
        help_level,
    }
}
/// Applies an exact configured default only to this invocation's settings clone.
pub(crate) fn apply_command_format(config: &ResolvedConfig, ui: &mut UiState, path: &[String]) {
    if ui.render_settings.format_explicit || path.is_empty() {
        return;
    }
    let key = format!("ui.command_formats.{}", path.join("."));
    if let Some(format) = config
        .get_string(&key)
        .and_then(crate::core::OutputFormat::parse)
    {
        ui.render_settings.format = format;
        ui.render_settings.guide_default_format = false;
    }
}

/// Maps a reported error to the CLI exit code family used by OSP.
pub fn classify_exit_code(err: &miette::Report) -> i32 {
    let known = KnownErrorChain::inspect(err);
    if find_error_in_chain::<crate::ui::interrupt::PromptCancelled>(err).is_some() {
        130
    } else if known.clap.is_some() {
        EXIT_CODE_USAGE
    } else {
        EXIT_CODE_ERROR
    }
}

/// Resolves recovery advice from error data before rendering either format.
pub(crate) fn report_error_details(
    err: &miette::Report,
    level: crate::core::MessageLevel,
    settings: &RenderSettings,
) -> crate::core::CommandError {
    use crate::core::MessageLevel;
    let known = KnownErrorChain::inspect(err);
    let detailed = level >= MessageLevel::Info;
    let structured = find_error_in_chain::<crate::core::CommandError>(err);
    let mut error = if let Some(error) = structured {
        error.clone()
    } else if let Some(clap) = known.clap {
        crate::core::CommandError::new(
            if let Some(clap::error::ContextValue::String(name)) =
                clap.get(clap::error::ContextKind::InvalidSubcommand)
            {
                let path = known
                    .boxed_context
                    .and_then(|c| c.command_path.as_deref())
                    .unwrap_or_default();
                let command = settings
                    .runtime
                    .command_scope
                    .command(&format!("{path} {name}"));
                format!(
                    "Unknown command '{}'.",
                    command.strip_prefix("osp ").unwrap_or(&command)
                )
            } else if detailed {
                clap.to_string()
            } else {
                terse_clap_error(clap)
            },
        )
    } else {
        base_error_message(err, &known)
    };
    if !detailed && error.action.is_none() {
        error.action = known_error_hint(&known, &settings.runtime.command_scope)
            .filter(|hint| !error.message.contains(hint));
    }
    error
}

/// Renders a concise failure below `Info`, or one detailed report at `Info` and above.
/// Backtraces are diagnostics emitted separately by the debug axis.
pub fn render_report_message(
    err: &miette::Report,
    level: crate::core::MessageLevel,
    settings: &RenderSettings,
) -> String {
    use crate::core::MessageLevel;
    let detailed = level >= MessageLevel::Info;
    let mut rendered = report_error_details(err, level, settings).render();
    if !detailed {
        return rendered;
    }

    let mut seen = vec![rendered.clone()];
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err.as_ref());
    while let Some(source) = current {
        // Presentation metadata and backend data are not additional causes.
        let metadata = source
            .downcast_ref::<ReportContextError>()
            .is_some_and(|context| context.context.is_none());
        let backend = source.is::<super::command_output::CommandBackendDetails>();
        let native = source
            .downcast_ref::<BoxedContextError>()
            .is_some_and(|context| context.context == ErrorContext::NativeExecution);
        let text = source.to_string();
        let clap_summary = source.is::<clap::Error>()
            && find_error_in_chain::<crate::core::CommandError>(err)
                .is_some_and(|error| text.trim().trim_start_matches("error: ") == error.message);
        if !clap_summary
            && !metadata
            && !backend
            && !native
            && !text.is_empty()
            && !seen.iter().any(|previous| previous.contains(&text))
        {
            rendered.push_str("\nCaused by: ");
            rendered.push_str(&text);
            seen.push(text);
        }
        current = source.source();
    }
    rendered
}

pub(crate) fn capture_missing_report_backtrace(err: &miette::Report) -> Option<Backtrace> {
    report_backtrace(err)
        .is_none()
        .then(Backtrace::force_capture)
}

fn report_backtrace(err: &miette::Report) -> Option<&Backtrace> {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err.as_ref());
    let mut captured = None;
    while let Some(source) = current {
        if let Some(error) = source.downcast_ref::<BoxedContextError>() {
            return Some(&error.backtrace);
        }
        if let Some(error) = source.downcast_ref::<super::command_output::CommandBackendDetails>() {
            return Some(&error.backtrace);
        }
        if let Some(error) = source.downcast_ref::<ReportContextError>()
            && error.backtrace.is_some()
        {
            captured = error.backtrace.as_ref();
        }
        current = source.source();
    }
    captured
}

pub(crate) fn render_backtrace(err: &miette::Report, settings: &RenderSettings) -> Option<String> {
    report_backtrace(err).map(|trace| {
        let mut causes = Vec::new();
        let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err.as_ref());
        while let Some(source) = current {
            let context = source
                .downcast_ref::<BoxedContextError>()
                .map(|error| error.context)
                .or_else(|| {
                    source
                        .downcast_ref::<ReportContextError>()
                        .and_then(|error| error.context)
                });
            if let Some(context) = context {
                let text = context.to_string();
                if !causes.contains(&text) {
                    causes.push(text);
                }
            }
            current = source.source();
        }
        let mut rendered = String::new();
        if !causes.is_empty() {
            rendered.push_str(&format!("debug: Error context: {}.\n", causes.join(" -> ")));
        }
        rendered.push_str(&format_backtrace(&trace.to_string(), settings));
        rendered
    })
}

fn format_backtrace(backtrace: &str, settings: &RenderSettings) -> String {
    let resolved = settings.resolve_render_settings();
    let heading = if resolved.color && {
        use std::io::IsTerminal;
        std::io::stderr().is_terminal()
    } {
        Style::new().bold().paint("Stack backtrace").to_string()
    } else {
        "Stack backtrace".to_string()
    };
    format!("{heading}:\n{backtrace}")
}

fn known_error_hint(
    known: &KnownErrorChain<'_>,
    scope: &crate::core::CommandScope,
) -> Option<String> {
    if let Some(config_err) = known.config {
        if matches!(
            config_err,
            crate::config::ConfigError::UnknownProfile { .. }
                | crate::config::ConfigError::FileRead { .. }
        ) {
            return None;
        }
        return Some(match config_err {
            crate::config::ConfigError::UnknownProfile { .. } => {
                format!(
                    "run `{}` or choose a known profile",
                    scope.command("config explain profile.default")
                )
            }
            crate::config::ConfigError::InsecureSecretsPermissions { .. } => {
                "restrict the secrets file permissions to 0600".to_string()
            }
            _ => format!(
                "run `{}` to inspect config provenance",
                scope.command("config explain <key>")
            ),
        });
    }

    if let Some(context) = known.boxed_context
        && !context.command_corrections.is_empty()
    {
        let more = if context.command_corrections.len() > 3 {
            format!(
                " Run {}.",
                scope.command(
                    format!(
                        "{} --help",
                        context.command_path.as_deref().unwrap_or_default()
                    )
                    .trim_start()
                )
            )
        } else {
            String::new()
        };
        return Some(format!(
            "Did you mean {}?{more}",
            context
                .command_corrections
                .iter()
                .take(3)
                .map(|command| scope.command(command))
                .collect::<Vec<_>>()
                .join(" or ")
        ));
    }
    if let Some(clap) = known.clap {
        let path = known
            .boxed_context
            .and_then(|context| context.command_path.as_deref());
        return Some(clap_error_action(clap, path, scope));
    }

    None
}

/// None preserves prior hints; Some(empty) suppresses insertion; Some(list) replaces choices.
pub(crate) fn report_hint_commands(
    err: &miette::Report,
    verbosity: crate::core::MessageLevel,
    input: Option<(&str, &crate::core::CommandScope)>,
) -> Option<Vec<String>> {
    if let Some(hint) = find_error_in_chain::<crate::core::CommandHint>(err) {
        return Some(vec![hint.command.clone()]);
    }
    if let Some(backend) = find_error_in_chain::<super::command_output::CommandBackendDetails>(err)
    {
        return backend
            .error
            .details
            .get("command")
            .and_then(serde_json::Value::as_str)
            .filter(|command| !command.chars().any(char::is_control))
            .map(|command| vec![command.to_string()]);
    }
    let known = KnownErrorChain::inspect(err);
    if let Some(context) = known.boxed_context
        && !context.command_corrections.is_empty()
    {
        return Some(if context.command_corrections.len() <= 3 {
            context
                .command_corrections
                .iter()
                .filter_map(|command| match input {
                    Some((line, scope)) => corrected_input(line, command, scope),
                    None => Some(command.clone()),
                })
                .collect()
        } else {
            Vec::new()
        });
    }
    if verbosity >= crate::core::MessageLevel::Info {
        return None;
    }
    if known.clap.is_none()
        && known
            .boxed_context
            .is_some_and(|context| context.command_path.is_some())
    {
        return Some(Vec::new());
    }
    let error = known.clap?;
    if error
        .get(clap::error::ContextKind::SuggestedSubcommand)
        .is_some()
    {
        return Some(Vec::new());
    }
    if error.kind() == clap::error::ErrorKind::InvalidSubcommand {
        return Some(Vec::new());
    }
    let path = known
        .boxed_context
        .and_then(|context| context.command_path.as_deref())
        .unwrap_or("");
    Some(vec![format!("{path} --help").trim_start().to_string()])
}

/// Splice the correction into the submitted text, retaining its quoting and pipeline.
fn corrected_input(line: &str, command: &str, scope: &crate::core::CommandScope) -> Option<String> {
    let scoped = scope.command(command);
    let corrected = shell_words::split(&scoped).ok()?;
    use crate::core::shell_words::{LexEventKind, scan_shell};
    let mut spans = Vec::new();
    let mut start = None;
    let mut command_end = line.len();
    scan_shell(line, |event| {
        if event.index >= command_end {
            return;
        }
        match event.kind {
            LexEventKind::Separator | LexEventKind::Pipe => {
                if let Some(start) = start.take() {
                    spans.push(start..event.index);
                }
                if event.kind == LexEventKind::Pipe {
                    command_end = event.index;
                }
            }
            _ => {
                start.get_or_insert(event.index);
            }
        }
    });
    if let Some(start) = start {
        spans.push(start..command_end);
    }
    let words = shell_words::split(&line[..command_end]).ok()?;
    if words.len() != corrected.len() {
        return None;
    }
    let mut replacements = words
        .iter()
        .zip(&corrected)
        .enumerate()
        .filter(|(_, (original, word))| original != word);
    let (index, (_, word)) = replacements.next()?;
    // Clap corrects one subcommand. A different token layout (for example an
    // expanded alias) cannot safely be mapped back to the submitted text.
    if replacements.next().is_some() {
        return None;
    }
    let mut input = line.to_string();
    input.replace_range(
        spans.get(index)?.clone(),
        &crate::core::shell_words::escape_for_shell(word),
    );
    let prefix = command.strip_suffix(&scoped)?;
    Some(format!("{prefix}{input}"))
}

fn terse_clap_error(error: &clap::Error) -> String {
    let rendered = error.to_string();
    let mut lines = rendered
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let first = lines.next().unwrap_or("command usage is invalid");
    let mut terse = first
        .strip_prefix("error:")
        .unwrap_or(first)
        .trim()
        .to_string();
    if error.kind() == clap::error::ErrorKind::MissingRequiredArgument {
        for argument in lines
            .take_while(|line| !line.starts_with("Usage:"))
            .filter(|line| line.starts_with(['<', '-']))
        {
            terse.push(' ');
            terse.push_str(argument);
        }
    }
    terse
}

fn clap_error_action(
    error: &clap::Error,
    command_path: Option<&str>,
    scope: &crate::core::CommandScope,
) -> String {
    if let Some(values) = error
        .get(clap::error::ContextKind::SuggestedValue)
        .or_else(|| error.get(clap::error::ContextKind::ValidValue))
    {
        let values = match values {
            clap::error::ContextValue::String(value) => vec![format!("`{value}`")],
            clap::error::ContextValue::Strings(values) => {
                values.iter().map(|value| format!("`{value}`")).collect()
            }
            _ => Vec::new(),
        };
        if !values.is_empty() {
            return format!("Use {}.", values.join(" or "));
        }
    }
    match command_path.filter(|path| !path.is_empty()) {
        Some(prefix) => format!("Run {}.", scope.command(&format!("{prefix} --help"))),
        None => format!("Run {}.", scope.command("--help")),
    }
}

fn base_error_message(
    err: &miette::Report,
    known: &KnownErrorChain<'_>,
) -> crate::core::CommandError {
    if let Some(config_err) = known.config {
        if let crate::config::ConfigError::InvalidConfigValue { key, reason } = config_err
            && key == "debug.level"
        {
            let path = crate::config::RuntimeConfigPaths::discover()
                .config_file
                .map(|path| format!(" ({})", path.display()))
                .unwrap_or_default();
            return crate::core::CommandError::new(format!("debug.level {reason}{path}."))
                .with_action("Fix it, or run osp config set debug.level 0.");
        }
        if let crate::config::ConfigError::FileRead { path, .. } = config_err
            && !std::path::Path::new(path).exists()
        {
            return crate::core::CommandError::new(format!("Config file {path} does not exist."));
        }
        if let crate::config::ConfigError::LayerLoad { path, inner } = config_err {
            let message = inner.to_string();
            let mut location = crate::diagnostics::safe_text(path);
            if let crate::config::ConfigError::TomlParse(diagnostic) = inner.as_ref()
                && let Some((line, column)) = diagnostic.location()
            {
                location.push_str(&format!(":{line}:{column}"));
            }
            return crate::core::CommandError::new(format!(
                "{} ({}).",
                message.lines().next().unwrap_or("Invalid config"),
                location
            ))
            .with_action("Fix the config file.");
        }
        return crate::core::CommandError::new(config_err.to_string());
    }

    if let Some(clap_err) = known.clap {
        return crate::core::CommandError::new(clap_err.to_string());
    }

    if let Some(context) = known.boxed_context
        && context.context == ErrorContext::NativeExecution
    {
        return crate::core::CommandError::new(context.source.to_string());
    }

    if let Some(details) =
        find_error_in_chain::<crate::app::command_output::CommandBackendDetails>(err)
        && let Some(source) = std::error::Error::source(details)
    {
        return crate::core::CommandError::new(source.to_string());
    }

    let outer = err.to_string();
    let mut deepest_source = None;
    let mut current = err.source();
    while let Some(source) = current {
        let text = source.to_string();
        if !text.is_empty() {
            deepest_source = Some(text);
        }
        current = source.source();
    }

    crate::core::CommandError::new(match deepest_source {
        // Context wrappers are still useful for verbose output, but at default
        // message levels users usually need the concrete actionable cause.
        Some(source) if find_error_in_chain::<ReportContextError>(err).is_some() => source,
        _ => outer,
    })
}

/// Use Clap's ranked candidates without changing unknown-command error semantics.
pub(crate) fn report_unknown_repl_command(
    command: &str,
    args: &[String],
    names: std::collections::BTreeSet<String>,
    scope: &crate::core::CommandScope,
) -> miette::Report {
    // The registry owns help too; do not synthesize a second help subcommand.
    let grammar = clap::Command::new("osp")
        .disable_help_subcommand(true)
        .subcommands(names.into_iter().map(clap::Command::new));
    let words = std::iter::once(OsString::from("osp"))
        .chain(std::iter::once(OsString::from(command)))
        .chain(args.iter().map(OsString::from))
        .collect::<Vec<_>>();
    let corrections = grammar
        .clone()
        .try_get_matches_from(&words)
        .err()
        .map(|mut error| {
            use clap::error::{ContextKind, ContextValue};
            // Root recovery has always offered the closest command. Clap ranks
            // its candidates from least to most similar.
            if let Some(ContextValue::Strings(choices)) =
                error.get(ContextKind::SuggestedSubcommand)
                && let Some(best) = choices.last().cloned()
            {
                error.insert(
                    ContextKind::SuggestedSubcommand,
                    ContextValue::Strings(vec![best]),
                );
            }
            corrected_subcommands(&error, &grammar, &words, &[])
        })
        .unwrap_or_default();
    let mut error = crate::core::CommandError::new(format!("Unknown command '{command}'."));
    if corrections.is_empty() {
        error.action = Some(format!("Run {} to list commands.", scope.command("help")));
    }
    miette::Report::new(BoxedContextError {
        context: ErrorContext::Operation("unknown REPL command"),
        source: Box::new(error),
        backtrace: Backtrace::force_capture(),
        command_path: Some(String::new()),
        command_corrections: corrections,
        structured: None,
    })
}

pub(crate) fn report_clap_error_with_context<I, T>(
    err: clap::Error,
    context: ErrorContext,
    command: clap::Command,
    args: I,
    structured: Option<crate::core::CommandError>,
) -> miette::Report
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = args.into_iter().map(Into::into).collect::<Vec<OsString>>();
    let path = resolved_command_path(&command, &args);
    let command_corrections = corrected_subcommands(&err, &command, &args, &path);
    miette::Report::new(BoxedContextError {
        context,
        source: Box::new(err),
        backtrace: Backtrace::force_capture(),
        command_path: Some(path.join(" ")),
        command_corrections,
        structured,
    })
}

// Passive parsing keeps the resolved path even when later arguments are missing.
fn resolved_command_path(command: &clap::Command, args: &[OsString]) -> Vec<String> {
    let mut path = if command.get_name() == "osp" {
        Vec::new()
    } else {
        vec![command.get_name().to_string()]
    };
    if let Ok(matches) = crate::cli::passive_command(command.clone()).try_get_matches_from(args) {
        let mut current = &matches;
        while let Some((name, child)) = current.subcommand() {
            path.push(name.to_string());
            current = child;
        }
    }
    path
}

fn corrected_subcommands(
    error: &clap::Error,
    command: &clap::Command,
    args: &[OsString],
    path: &[String],
) -> Vec<String> {
    use clap::error::{ContextKind, ContextValue, ErrorKind};
    if error.kind() != ErrorKind::InvalidSubcommand {
        return Vec::new();
    }
    let (Some(ContextValue::String(invalid)), Some(ContextValue::Strings(suggestions))) = (
        error.get(ContextKind::InvalidSubcommand),
        error.get(ContextKind::SuggestedSubcommand),
    ) else {
        return Vec::new();
    };
    let Some(first) = suggestions.first() else {
        return Vec::new();
    };
    let mut expected = path.to_vec();
    expected.push(first.clone());
    // A matching word may also be an earlier option value. Only replace the
    // occurrence that the grammar accepts as a child of the resolved command.
    let argument_start = usize::from(!command.is_no_binary_name_set());
    let Some(index) = (argument_start..args.len()).find(|&index| {
        if args[index].to_str() != Some(invalid) {
            return false;
        }
        let mut prefix = args[..=index].to_vec();
        prefix[index] = first.into();
        resolved_command_path(command, &prefix) == expected
    }) else {
        return Vec::new();
    };
    let start = if command.get_name() == "osp" {
        argument_start
    } else {
        0
    };
    suggestions
        .iter()
        .map(|suggestion| {
            args.iter()
                .enumerate()
                .skip(start)
                .map(|(position, argument)| {
                    let word = if position == index {
                        suggestion.as_str()
                    } else {
                        argument.to_str()?
                    };
                    Some(crate::core::shell_words::escape_for_shell(word))
                })
                .collect::<Option<Vec<_>>>()
                .map(|words| words.join(" "))
        })
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default()
}

pub(crate) fn report_std_error_with_context<E>(err: E, context: ErrorContext) -> miette::Report
where
    E: std::error::Error + Send + Sync + 'static,
{
    miette::Report::new(BoxedContextError {
        context,
        source: Box::new(err),
        backtrace: Backtrace::force_capture(),
        command_path: None,
        command_corrections: Vec::new(),
        structured: None,
    })
}

pub(crate) fn report_anyhow_with_context(
    err: anyhow::Error,
    context: ErrorContext,
) -> miette::Report {
    let structured = err.downcast_ref::<crate::core::CommandError>().cloned();
    miette::Report::new(BoxedContextError {
        context,
        source: err.reallocate_into_boxed_dyn_error_without_backtrace(),
        backtrace: Backtrace::force_capture(),
        command_path: None,
        command_corrections: Vec::new(),
        structured,
    })
}

pub(crate) fn report_report_with_context(
    err: miette::Report,
    context: ErrorContext,
) -> miette::Report {
    miette::Report::new(ReportContextError {
        context: Some(context),
        source: err,
        error_display: None,
        diagnostics: None,
        backtrace: Some(Backtrace::force_capture()),
    })
}

pub(crate) fn report_with_invocation(
    err: miette::Report,
    settings: ErrorDisplaySettings,
) -> miette::Report {
    let backtrace = capture_missing_report_backtrace(&err);
    miette::Report::new(ReportContextError {
        context: None,
        source: err,
        error_display: Some(settings),
        diagnostics: None,
        backtrace,
    })
}

pub(crate) fn report_with_evidence(
    err: miette::Report,
    diagnostics: crate::diagnostics::DiagnosticSink,
) -> miette::Report {
    miette::Report::new(ReportContextError {
        context: None,
        source: err,
        error_display: None,
        diagnostics: Some(diagnostics),
        backtrace: None,
    })
}

pub(crate) fn report_evidence(
    err: &miette::Report,
    level: crate::core::MessageLevel,
) -> (String, String) {
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err.as_ref());
    while let Some(source) = current {
        if let Some(context) = source.downcast_ref::<ReportContextError>()
            && let Some(diagnostics) = &context.diagnostics
        {
            return diagnostics.render(level, true);
        }
        current = source.source();
    }
    Default::default()
}

pub(crate) fn error_display_settings(err: &miette::Report) -> Option<&ErrorDisplaySettings> {
    find_error_in_chain::<ReportContextError>(err)
        .and_then(|context| context.error_display.as_ref())
}

pub(super) fn find_error_in_chain<E>(err: &miette::Report) -> Option<&E>
where
    E: std::error::Error + 'static,
{
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err.as_ref());
    while let Some(source) = current {
        if let Some(found) = source.downcast_ref::<E>() {
            return Some(found);
        }
        if let Some(found) = source
            .downcast_ref::<BoxedContextError>()
            .and_then(|context| context.structured.as_ref())
            .and_then(|error| (error as &dyn std::error::Error).downcast_ref::<E>())
        {
            return Some(found);
        }
        current = source.source();
    }
    None
}

pub(super) fn add_native_command_help(
    view: &mut Document,
    native_commands: &NativeCommandRegistry,
    auth: &AuthState,
) {
    let entries = native_commands
        .catalog()
        .into_iter()
        .filter(|entry| entry.name != "doctor")
        .filter(|entry| auth.is_external_command_visible(&entry.name))
        .filter(|entry| {
            native_commands
                .command(&entry.name)
                .is_some_and(|command| !command.command().is_hide_set())
        })
        .map(|entry| DocumentEntry {
            role: crate::core::document::EntryRole::Command,
            name: entry.name,
            description: entry.about.trim().to_string(),
        })
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return;
    }
    if let Some(DocBlock::Section { blocks, .. }) = view
        .blocks
        .iter_mut()
        .find(|block| matches!(block, DocBlock::Section { title, .. } if title == "Commands"))
    {
        if let Some(DocBlock::Entries { entries: commands }) = blocks.first_mut() {
            commands.splice(0..0, entries);
        }
    } else {
        view.blocks.push(DocBlock::section(
            "Commands",
            vec![DocBlock::Entries { entries }],
        ));
    }
}

fn add_product_option_help(view: &mut Document, options: &[(String, String)]) {
    if !options.is_empty() {
        view.blocks.push(
            DocBlock::section(
                "Product options",
                vec![DocBlock::Entries {
                    entries: options
                        .iter()
                        .map(|(name, description)| DocumentEntry {
                            role: crate::core::document::EntryRole::LongOption,
                            name: name.clone(),
                            description: description.clone(),
                        })
                        .collect(),
                }],
            )
            .with_role(crate::core::document::SectionRole::CompactOptions),
        );
    }
}
