use crate::config::ResolvedConfig;
use crate::core::output::OutputFormat;
use crate::native::{NativeCommandCatalogEntry, NativeCommandRegistry};
use crate::repl;
use clap::Parser;
use miette::{Result, WrapErr, miette};

use crate::guide::{GuideSection, GuideSectionKind, GuideView, HelpLevel};
use crate::ui::RenderSettings;
use crate::ui::messages::MessageLevel;
use std::borrow::Cow;
use std::ffi::OsString;
use std::time::Instant;

use super::help;
use super::help::help_level;
use crate::app::logging::{bootstrap_logging_config, init_developer_logging};
use crate::app::sink::{StdIoUiSink, UiSink};
use crate::app::{AppClients, AppState, AuthState, UiState};
use crate::cli::Cli;
use crate::cli::invocation::{InvocationOptions, extend_with_invocation_help, scan_cli_argv};
use crate::plugin::{CommandCatalogEntry, PluginDispatchError};

pub(crate) use super::bootstrap::{
    RuntimeConfigRequest, prepare_startup_host, resolve_runtime_config,
};
pub(crate) use super::command_output::run_cli_command_with_ui;
pub(crate) use super::dispatch::{
    DispatchPlan, RunAction, build_dispatch_plan, ensure_builtin_visible_for,
    ensure_dispatch_visibility, ensure_plugin_visible_for, normalize_cli_profile,
    normalize_profile_override,
};
use super::external::run_external_command;
pub(crate) use super::external::run_external_command_with_help_renderer;
#[cfg(test)]
pub(crate) use super::repl_lifecycle::rebuild_repl_state;
pub(crate) use super::timing::{TimingSummary, format_timing_badge, right_align_timing_line};
pub(crate) use crate::plugin::config::{
    PluginConfigEntry, PluginConfigScope, plugin_config_entries,
};
#[cfg(test)]
pub(crate) use crate::plugin::config::{
    collect_plugin_config_env, config_value_to_plugin_env, plugin_config_env_name,
};

pub(crate) const CMD_PLUGINS: &str = "plugins";
pub(crate) const CMD_DOCTOR: &str = "doctor";
pub(crate) const CMD_CONFIG: &str = "config";
pub(crate) const CMD_THEME: &str = "theme";
pub(crate) const CMD_HISTORY: &str = "history";
pub(crate) const CMD_INTRO: &str = "intro";
pub(crate) const CMD_HELP: &str = "help";
pub(crate) const CMD_LIST: &str = "list";
pub(crate) const CMD_SHOW: &str = "show";
pub(crate) const CMD_USE: &str = "use";
pub const EXIT_CODE_ERROR: i32 = 1;
pub const EXIT_CODE_USAGE: i32 = 2;
pub const EXIT_CODE_CONFIG: i32 = 3;
pub const EXIT_CODE_PLUGIN: i32 = 4;
pub(crate) const DEFAULT_REPL_PROMPT: &str = "╭─{user}@{domain} {indicator}\n╰─{profile}> ";
pub(crate) const CURRENT_TERMINAL_SENTINEL: &str = "__current__";
pub(crate) const REPL_SHELLABLE_COMMANDS: [&str; 5] = ["nh", "mreg", "ldap", "vm", "orch"];

#[derive(Debug, Clone)]
pub(crate) struct ReplCommandSpec {
    pub(crate) name: Cow<'static, str>,
    pub(crate) supports_dsl: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedInvocation {
    pub(crate) ui: UiState,
    pub(crate) plugin_provider: Option<String>,
    pub(crate) help_level: HelpLevel,
}

struct PreparedHostRun {
    state: AppState,
    dispatch: DispatchPlan,
    invocation_ui: ResolvedInvocation,
}

#[derive(Debug)]
struct ContextError<E> {
    context: &'static str,
    source: E,
}

#[derive(Clone, Copy)]
struct KnownErrorChain<'a> {
    clap: Option<&'a clap::Error>,
    config: Option<&'a crate::config::ConfigError>,
    plugin: Option<&'a PluginDispatchError>,
}

impl<'a> KnownErrorChain<'a> {
    fn inspect(err: &'a miette::Report) -> Self {
        Self {
            clap: find_error_in_chain::<clap::Error>(err),
            config: find_error_in_chain::<crate::config::ConfigError>(err),
            plugin: find_error_in_chain::<PluginDispatchError>(err),
        }
    }
}

impl<E> std::fmt::Display for ContextError<E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.context)
    }
}

impl<E> std::error::Error for ContextError<E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

impl<E> miette::Diagnostic for ContextError<E> where E: std::error::Error + Send + Sync + 'static {}

/// Runs the top-level CLI entrypoint from an argv-like iterator.
///
/// This is the library-friendly wrapper around the binary entrypoint and
/// returns the process exit code that should be reported to the caller.
pub fn run_from<I, T>(args: I) -> Result<i32>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let mut sink = StdIoUiSink;
    run_from_with_sink(args, &mut sink)
}

pub(crate) fn run_from_with_sink<I, T>(args: I, sink: &mut dyn UiSink) -> Result<i32>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    run_from_with_sink_and_app(args, sink, &super::AppDefinition::default())
}

pub(crate) fn run_from_with_sink_and_app<I, T>(
    args: I,
    sink: &mut dyn UiSink,
    app: &super::AppDefinition,
) -> Result<i32>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let argv = args.into_iter().map(Into::into).collect::<Vec<OsString>>();
    init_developer_logging(bootstrap_logging_config(&argv));
    let scanned = scan_cli_argv(&argv)?;
    match Cli::try_parse_from(scanned.argv.iter().cloned()) {
        Ok(cli) => run(cli, scanned.invocation, sink, app),
        Err(err) => handle_clap_parse_error(&argv, err, sink, app),
    }
}

fn handle_clap_parse_error(
    args: &[OsString],
    err: clap::Error,
    sink: &mut dyn UiSink,
    app: &super::AppDefinition,
) -> Result<i32> {
    match err.kind() {
        clap::error::ErrorKind::DisplayHelp => {
            let help_context = help::render_settings_for_help(args, &app.product_defaults);
            let mut body = GuideView::from_text(&err.to_string());
            extend_with_invocation_help(&mut body, help_context.help_level);
            add_native_command_help(&mut body, &app.native_commands);
            let filtered = body.filtered_for_help_level(help_context.help_level);
            let rendered = crate::ui::render_structured_output_with_source_guide(
                &filtered.to_output_result(),
                Some(&filtered),
                &help_context.settings,
                help_context.layout,
            );
            sink.write_stdout(&rendered);
            Ok(0)
        }
        clap::error::ErrorKind::DisplayVersion => {
            sink.write_stdout(&err.to_string());
            Ok(0)
        }
        _ => Err(report_std_error_with_context(
            err,
            "failed to parse CLI arguments",
        )),
    }
}

// Keep the top-level CLI entrypoint readable as a table of contents:
// normalize input -> bootstrap runtime state -> hand off to the selected mode.
fn run(
    mut cli: Cli,
    invocation: InvocationOptions,
    sink: &mut dyn UiSink,
    app: &super::AppDefinition,
) -> Result<i32> {
    let run_started = Instant::now();
    if invocation.cache {
        return Err(miette!(
            "`--cache` is only available inside the interactive REPL"
        ));
    }

    let PreparedHostRun {
        mut state,
        dispatch,
        invocation_ui,
    } = prepare_host_run(&mut cli, &invocation, app, run_started)?;

    let action_started = Instant::now();
    let is_repl = matches!(dispatch.action, RunAction::Repl);
    let action = dispatch.action;
    let result = match action {
        RunAction::Repl => {
            state.runtime.ui = invocation_ui.ui.clone();
            repl::run_plugin_repl(&mut state)
        }
        RunAction::External(tokens) => run_external_command(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            &tokens,
            &invocation_ui,
        )
        .and_then(|result| {
            run_cli_command_with_ui(
                state.runtime.config.resolved(),
                &invocation_ui.ui,
                result,
                sink,
            )
        }),
        action => {
            let Some(command) = action.into_builtin_command() else {
                return Err(miette!(
                    "internal error: non-builtin run action reached builtin dispatch"
                ));
            };
            super::run_cli_builtin_command_parts(
                &mut state.runtime,
                &mut state.session,
                &state.clients,
                &invocation_ui,
                command,
                sink,
            )
        }
    };

    if !is_repl && invocation_ui.ui.debug_verbosity > 0 {
        let total = run_started.elapsed();
        let startup = action_started.saturating_duration_since(run_started);
        let command = total.saturating_sub(startup);
        let footer = right_align_timing_line(
            TimingSummary {
                total,
                parse: if invocation_ui.ui.debug_verbosity >= 3 {
                    Some(startup)
                } else {
                    None
                },
                execute: if invocation_ui.ui.debug_verbosity >= 3 {
                    Some(command)
                } else {
                    None
                },
                render: None,
            },
            invocation_ui.ui.debug_verbosity,
            &invocation_ui.ui.render_settings.resolve_render_settings(),
        );
        if !footer.is_empty() {
            sink.write_stderr(&footer);
        }
    }

    result
}

// Startup is phase-based:
// 1. bootstrap just enough config to understand profiles and dispatch
// 2. assemble the runtime/session layer for the chosen action
// 3. apply startup-time side effects before handing off to execution
fn prepare_host_run(
    cli: &mut Cli,
    invocation: &InvocationOptions,
    app: &super::AppDefinition,
    run_started: Instant,
) -> Result<PreparedHostRun> {
    let normalized_profile = normalize_cli_profile(cli);
    let runtime_load = cli.runtime_load_options();
    let initial_config = resolve_runtime_config(
        RuntimeConfigRequest::new(normalized_profile.clone(), Some("cli"))
            .with_runtime_load(runtime_load)
            .with_product_defaults(app.product_defaults.clone()),
    )
    .wrap_err("failed to resolve initial config for startup")?;
    let known_profiles = initial_config.known_profiles().clone();
    let dispatch = build_dispatch_plan(cli, &known_profiles)?;
    tracing::debug!(
        action = ?dispatch.action,
        profile_override = ?dispatch.profile_override,
        known_profiles = known_profiles.len(),
        "built dispatch plan"
    );

    let terminal_kind = dispatch.action.terminal_kind();
    let prepared = prepare_startup_host(
        cli,
        dispatch.profile_override.clone(),
        terminal_kind,
        run_started,
        &app.product_defaults,
    )?;
    let mut state = crate::app::AppStateBuilder::from_host_inputs(
        prepared.runtime_context,
        prepared.config,
        prepared.host_inputs,
    )
    .with_launch(prepared.launch_context)
    .with_native_commands(app.native_commands.clone())
    .build();
    state
        .runtime
        .set_product_defaults(app.product_defaults.clone());
    ensure_dispatch_visibility(&state.runtime.auth, &dispatch.action)?;
    let invocation_ui = resolve_invocation_ui(
        state.runtime.config.resolved(),
        &state.runtime.ui,
        invocation,
    );
    super::assembly::apply_runtime_side_effects(
        state.runtime.config.resolved(),
        invocation_ui.ui.debug_verbosity,
        &state.runtime.themes,
    );
    tracing::debug!(
        debug_count = invocation_ui.ui.debug_verbosity,
        "developer logging initialized"
    );
    tracing::info!(
        profile = %state.runtime.config.resolved().active_profile(),
        terminal = %state.runtime.context.terminal_kind().as_config_terminal(),
        action = ?dispatch.action,
        plugin_timeout_ms = super::plugin_process_timeout(state.runtime.config.resolved()).as_millis(),
        "osp session initialized"
    );

    Ok(PreparedHostRun {
        state,
        dispatch,
        invocation_ui,
    })
}

pub(crate) fn authorized_command_catalog_for(
    auth: &AuthState,
    clients: &AppClients,
) -> Result<Vec<CommandCatalogEntry>> {
    let mut all = clients.plugins().command_catalog();
    all.extend(
        clients
            .native_commands()
            .catalog()
            .into_iter()
            .map(native_catalog_entry_to_command_catalog_entry),
    );
    all.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(all
        .into_iter()
        .filter(|entry| auth.is_external_command_visible(&entry.name))
        .collect())
}

pub(crate) fn resolve_invocation_ui(
    config: &ResolvedConfig,
    ui: &UiState,
    invocation: &InvocationOptions,
) -> ResolvedInvocation {
    let mut render_settings = ui.render_settings.clone();
    render_settings.format_explicit = invocation.format.is_some();
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

    ResolvedInvocation {
        ui: UiState::new(
            render_settings,
            crate::ui::messages::adjust_verbosity(
                ui.message_verbosity,
                invocation.verbose,
                invocation.quiet,
            ),
            if invocation.debug > 0 {
                invocation.debug.min(3)
            } else {
                ui.debug_verbosity
            },
        ),
        plugin_provider: invocation.plugin_provider.clone(),
        help_level: help_level(config, invocation.verbose, invocation.quiet),
    }
}

pub(crate) fn ensure_command_supports_dsl(spec: &ReplCommandSpec, stages: &[String]) -> Result<()> {
    if stages.is_empty() || spec.supports_dsl {
        return Ok(());
    }

    Err(miette!(
        "`{}` does not support DSL pipeline stages",
        spec.name
    ))
}

pub(crate) fn enrich_dispatch_error(err: PluginDispatchError) -> miette::Report {
    report_std_error_with_context(err, "plugin command failed")
}

/// Maps a reported error to the CLI exit code family used by OSP.
pub fn classify_exit_code(err: &miette::Report) -> i32 {
    let known = KnownErrorChain::inspect(err);
    if known.clap.is_some() {
        EXIT_CODE_USAGE
    } else if known.config.is_some() {
        EXIT_CODE_CONFIG
    } else if known.plugin.is_some() {
        EXIT_CODE_PLUGIN
    } else {
        EXIT_CODE_ERROR
    }
}

/// Renders a user-facing error message for the requested message verbosity.
///
/// Higher verbosity levels include more source-chain detail and may append a hint.
pub fn render_report_message(err: &miette::Report, verbosity: MessageLevel) -> String {
    if verbosity >= MessageLevel::Trace {
        return format!("{err:?}");
    }

    let known = KnownErrorChain::inspect(err);
    let mut message = base_error_message(err, &known);

    if verbosity >= MessageLevel::Info {
        let mut next: Option<&(dyn std::error::Error + 'static)> = Some(err.as_ref());
        while let Some(source) = next {
            let source_text = source.to_string();
            if !source_text.is_empty() && !message.contains(&source_text) {
                message.push_str(": ");
                message.push_str(&source_text);
            }
            next = source.source();
        }
    }

    if verbosity >= MessageLevel::Success
        && let Some(hint) = known_error_hint(&known)
        && !message.contains(hint)
    {
        message.push_str("\nHint: ");
        message.push_str(hint);
    }

    message
}

fn known_error_hint(known: &KnownErrorChain<'_>) -> Option<&'static str> {
    if let Some(plugin_err) = known.plugin {
        return Some(match plugin_err {
            PluginDispatchError::CommandNotFound { .. } => {
                "run `osp plugins list` and set --plugin-dir or OSP_PLUGIN_PATH"
            }
            PluginDispatchError::CommandAmbiguous { .. } => {
                "rerun with --plugin-provider <plugin-id> or persist a default with `osp plugins select-provider <command> <plugin-id>`"
            }
            PluginDispatchError::ProviderNotFound { .. } => {
                "pick one of the available providers from `osp plugins commands` or `osp plugins doctor`"
            }
            PluginDispatchError::ExecuteFailed { .. } => {
                "verify the plugin executable exists and is executable"
            }
            PluginDispatchError::TimedOut { .. } => {
                "increase extensions.plugins.timeout_ms or inspect the plugin executable"
            }
            PluginDispatchError::NonZeroExit { .. } => {
                "inspect the plugin stderr output or rerun with -v/-vv for more context"
            }
            PluginDispatchError::InvalidJsonResponse { .. }
            | PluginDispatchError::InvalidResponsePayload { .. } => {
                "inspect the plugin response contract and stderr output"
            }
        });
    }

    if let Some(config_err) = known.config {
        return Some(match config_err {
            crate::config::ConfigError::UnknownProfile { .. } => {
                "run `osp config explain profile.default` or choose a known profile"
            }
            crate::config::ConfigError::InsecureSecretsPermissions { .. } => {
                "restrict the secrets file permissions to 0600"
            }
            _ => "run `osp config explain <key>` to inspect config provenance",
        });
    }

    if known.clap.is_some() {
        return Some("use --help to inspect accepted flags and subcommands");
    }

    None
}

fn base_error_message(err: &miette::Report, known: &KnownErrorChain<'_>) -> String {
    if let Some(plugin_err) = known.plugin {
        return plugin_err.to_string();
    }

    if let Some(config_err) = known.config {
        return config_err.to_string();
    }

    if let Some(clap_err) = known.clap {
        return clap_err.to_string();
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

    match deepest_source {
        // Context wrappers are still useful for verbose output, but at default
        // message levels users usually need the concrete actionable cause.
        Some(source) if outer.starts_with("failed to ") || outer.starts_with("unable to ") => {
            source
        }
        _ => outer,
    }
}

pub(crate) fn report_std_error_with_context<E>(err: E, context: &'static str) -> miette::Report
where
    E: std::error::Error + Send + Sync + 'static,
{
    miette::Report::new(ContextError {
        context,
        source: err,
    })
}

fn find_error_in_chain<E>(err: &miette::Report) -> Option<&E>
where
    E: std::error::Error + 'static,
{
    let mut current: Option<&(dyn std::error::Error + 'static)> = Some(err.as_ref());
    while let Some(source) = current {
        if let Some(found) = source.downcast_ref::<E>() {
            return Some(found);
        }
        current = source.source();
    }
    None
}

fn native_catalog_entry_to_command_catalog_entry(
    entry: NativeCommandCatalogEntry,
) -> CommandCatalogEntry {
    CommandCatalogEntry {
        name: entry.name,
        about: entry.about,
        auth: entry.auth,
        subcommands: entry.subcommands,
        completion: entry.completion,
        provider: None,
        providers: Vec::new(),
        conflicted: false,
        requires_selection: false,
        selected_explicitly: false,
        source: None,
    }
}

fn add_native_command_help(view: &mut GuideView, native_commands: &NativeCommandRegistry) {
    let catalog = native_commands.catalog();
    if catalog.is_empty() {
        return;
    }

    let mut section = GuideSection::new("Native integrations", GuideSectionKind::Custom);
    for entry in catalog {
        section = section.entry(entry.name, entry.about.trim());
    }
    view.sections.push(section);
}

pub(crate) fn resolve_render_settings_with_hint(
    settings: &RenderSettings,
    format_hint: Option<OutputFormat>,
) -> RenderSettings {
    if matches!(settings.format, OutputFormat::Auto)
        && let Some(format) = format_hint
    {
        let mut effective = settings.clone();
        effective.format = format;
        return effective;
    }
    settings.clone()
}
