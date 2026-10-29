//! External command parsing and dispatch.
//!
//! This module owns the path from already-tokenized user input to either a
//! native command, plugin command, inline builtin, or help rewrite on the
//! external command surface. It sits after top-level host setup but before the
//! final native/plugin execution paths.

use std::cell::RefCell;

use miette::{Result, WrapErr, miette};

use crate::app::{AppRuntime, AppSession};
use crate::app::{TerminalKind, UiState};
use crate::cli::Commands;
use crate::cli::invocation::{InvocationOptions, extend_with_invocation_help, scan_command_tokens};
use crate::cli::pipeline::{ParsedCommandLine, parse_command_tokens_with_aliases};
use crate::core::document::Document;
use crate::native::{
    InvocationElevation, NativeCommandContext, NativeCommandOutcome, NativeProgressEvent,
    NativeProgressSink, NativeSessionContext,
};
use crate::repl::completion;
use crate::repl::is_repl_shellable_command;

use super::dispatch::{
    ExternalCommandSource, ExternalPathAccessRequirement, before_external_dispatch,
    before_external_execution, before_external_run, canonical_external_command_name,
    ensure_external_access, resolve_external_command_source,
};
use super::sink::{ProgressUiSink, UiSink};
use super::{
    CommandOutput, ResolvedInvocation, cli_result_from_plugin_response,
    plugin_dispatch_context_for, run_inline_builtin_command, run_progress_command_with_ui,
    runtime_hints_for_invocation,
};

struct ParsedExternalInvocation {
    tokens: Vec<String>,
    stages: Vec<String>,
    apply_output_stages: bool,
    inline_command: Option<Commands>,
    elevation: InvocationElevation,
}

impl ParsedExternalInvocation {
    fn finish(&self, result: CommandOutput) -> Result<CommandOutput> {
        if self.apply_output_stages {
            super::command_output::apply_stages_to_cli_result(result, &self.stages)
        } else {
            Ok(result)
        }
    }
}

enum ExternalParse {
    Handled(Box<CommandOutput>),
    Invocation(Box<ParsedExternalInvocation>),
}

pub(super) fn run_external_command(
    state: &mut crate::app::AppState,
    tokens: &[String],
    options: &InvocationOptions,
    invocation: &mut ResolvedInvocation,
    sink: &mut dyn UiSink,
) -> Result<i32> {
    let expanded = expand_external_tokens(&state.runtime, tokens);
    // Report expansion failures during dispatch, with the same command context
    // as the REPL. Invocation conflicts still fail before inline parsing.
    let scanned = match &expanded {
        Ok((parsed, _)) => Some(scan_command_tokens(&parsed.tokens)?),
        Err(_) => None,
    };
    if let Some(scanned) = &scanned
        && scanned.invocation != InvocationOptions::default()
    {
        let mut options = options.clone();
        options.merge(scanned.invocation.clone())?;
        *invocation = super::resolve_invocation_ui(
            state.runtime.config.resolved(),
            &state.runtime.ui,
            &options,
        );
    }
    if let Some(scanned) = &scanned {
        let prefixes =
            crate::command_words::CommandPrefixes::from_config(state.runtime.config.resolved())
                .enabled(state.runtime.context.terminal_kind() == TerminalKind::Repl);
        let path = super::command_plan::canonical_builtin_path(prefixes, &scanned.tokens)?;
        invocation.ui.render_settings.command_path = path;
    }
    if let (Ok((parsed, _)), Some(scanned)) = (&expanded, &scanned)
        && crate::cli::is_help_all(&scanned.tokens)
    {
        let result = super::help::help_all_output(state);
        let result = super::command_output::apply_stages_to_cli_result(result, &parsed.stages)?;
        return super::run_cli_command(
            state.runtime.config.resolved(),
            &invocation.ui,
            &result,
            sink,
        );
    }
    let parsed = expanded
        .and_then(|(mut parsed, elevation)| {
            if let Some(scanned) = scanned {
                parsed.tokens = scanned.tokens;
            }
            plan_external_invocation(
                &state.runtime,
                &state.session,
                parsed,
                elevation,
                invocation,
            )
        })
        .wrap_err_with(|| external_parse_context(tokens))?;
    let result = dispatch_external_invocation(
        state,
        parsed,
        invocation,
        |stdout| {
            let mut guide = Document::from_help_text(stdout);
            extend_with_invocation_help(&mut guide, invocation.help_level);
            guide.filtered_for_help_level(invocation.help_level)
        },
        Some(sink),
        None,
    )?;
    super::run_cli_command(
        state.runtime.config.resolved(),
        &invocation.ui,
        &result,
        sink,
    )
}

pub(crate) fn run_external_command_with_help_renderer(
    state: &mut crate::app::AppState,
    tokens: &[String],
    invocation: &ResolvedInvocation,
    guide_help: impl Fn(&str) -> Document,
    progress_sink: Option<&mut dyn UiSink>,
) -> Result<CommandOutput> {
    let parsed =
        parse_external_invocation(&state.runtime, &state.session, tokens, invocation, &[])?;
    dispatch_external_invocation(state, parsed, invocation, guide_help, progress_sink, None)
}

pub(crate) fn run_external_command_for_repl(
    state: &mut crate::app::AppState,
    command: ParsedCommandLine,
    invocation: &ResolvedInvocation,
    guide_help: impl Fn(&str) -> Document,
    progress_sink: Option<&mut dyn UiSink>,
    accepted: &mut bool,
) -> Result<CommandOutput> {
    let parsed = parse_external_invocation(
        &state.runtime,
        &state.session,
        &command.tokens,
        invocation,
        &command.stages,
    )?;
    dispatch_external_invocation(
        state,
        parsed,
        invocation,
        guide_help,
        progress_sink,
        Some(accepted),
    )
}

fn dispatch_external_invocation(
    state: &mut crate::app::AppState,
    parsed: ExternalParse,
    invocation: &ResolvedInvocation,
    guide_help: impl Fn(&str) -> Document,
    progress_sink: Option<&mut dyn UiSink>,
    mut accepted: Option<&mut bool>,
) -> Result<CommandOutput> {
    let mut parsed = match parsed {
        ExternalParse::Handled(result) => {
            mark_repl_command_accepted(&mut accepted);
            return Ok(*result);
        }
        ExternalParse::Invocation(parsed) => parsed,
    };

    if parsed.elevation.is_elevated()
        && parsed
            .inline_command
            .as_ref()
            .is_some_and(|command| !matches!(command, Commands::External(_)))
    {
        return Err(miette!(
            "elevation prefix is unsupported for built-in commands"
        ));
    }
    if let Some(command) = parsed.inline_command.take() {
        if !matches!(&command, Commands::External(_)) {
            mark_repl_command_accepted(&mut accepted);
        }
        if let Some(result) = run_inline_builtin_command(
            &mut state.runtime,
            &mut state.session,
            &state.clients,
            Some(invocation),
            command,
            &parsed.stages,
        )? {
            return parsed.finish(result);
        }
    }
    if !parsed.stages.is_empty() {
        completion::validate_dsl_stages(&parsed.stages)
            .wrap_err("failed to validate DSL pipeline stages")?;
    }

    let (command, args) = parsed
        .tokens
        .split_first()
        .ok_or_else(|| miette!("missing external command"))?;
    let command = canonical_external_command_name(&state.runtime.auth, &state.clients, command)?;
    let mut result = match resolve_external_command_source(
        &state.runtime.auth,
        &state.clients,
        &command,
        args,
        &invocation.ui.render_settings.runtime.command_scope,
    )? {
        ExternalCommandSource::Native => {
            let native_command = state
                .clients
                .native_commands()
                .command(&command)
                .cloned()
                .ok_or_else(|| miette!("no native command provides `{command}`"))?;
            run_native_invocation(
                native_command.as_ref(),
                state,
                NativeRunInput {
                    args,
                    source_args: args,
                    action: None,
                    stages: &parsed.stages,
                    invocation,
                    command_path: command.clone(),
                    elevation: parsed.elevation,
                    progress_sink,
                },
                guide_help,
                &mut accepted,
            )
        }
        ExternalCommandSource::Plugin => {
            if parsed.elevation.is_elevated() {
                return Err(miette!(
                    "elevation prefix is unsupported for plugin command `{command}`"
                ));
            }
            run_external_plugin_command(
                state,
                &command,
                &parsed,
                invocation,
                guide_help,
                &mut accepted,
            )
        }
    }?;
    result.use_source = invocation.source;
    parsed.finish(result)
}

struct NativeParse {
    args: Vec<String>,
    grammar: clap::Command,
    matches: std::result::Result<clap::ArgMatches, clap::Error>,
    path: crate::core::command_policy::CommandPath,
    render_help: bool,
    prefixes: bool,
}

impl NativeParse {
    fn new(
        command: &dyn crate::native::NativeCommand,
        name: &str,
        args: &[String],
        prefixes: bool,
    ) -> Result<Self> {
        let args = command
            .normalize_invocation_args(args, prefixes)
            .map_err(|error| {
                crate::app::report_anyhow_with_context(
                    error,
                    crate::app::host::ErrorContext::Operation(
                        "failed to normalize native command arguments",
                    ),
                )
            })?;
        let grammar = command.command();
        let matches = grammar
            .clone()
            .try_get_matches_from(std::iter::once(name.to_string()).chain(args.iter().cloned()));
        let path = native_resolved_path(&grammar, name, &args, matches.as_ref().ok());
        let render_help = matches
            .as_ref()
            .is_err_and(|error| crate::native::is_clap_help_error(error.kind()));
        Ok(Self {
            args,
            grammar,
            matches,
            path,
            render_help,
            prefixes,
        })
    }
}

struct NativeIntent {
    requirement: ExternalPathAccessRequirement,
    render_help: bool,
    refresh_metadata: bool,
}

impl NativeIntent {
    fn from_parse(parsed: &NativeParse) -> Self {
        let render_help = parsed.render_help;
        let help_requested = render_help
            || parsed.args.first().is_some_and(|arg| arg == "help")
            || parsed
                .args
                .iter()
                .take_while(|arg| arg.as_str() != "--")
                .any(|arg| matches!(arg.as_str(), "-h" | "--help" | "-V" | "--version"));
        Self {
            requirement: if help_requested {
                ExternalPathAccessRequirement::Visible
            } else {
                ExternalPathAccessRequirement::Runnable
            },
            render_help,
            refresh_metadata: parsed.matches.is_err(),
        }
    }
}

fn refresh_native_parse(
    command: &dyn crate::native::NativeCommand,
    session_context: &crate::native::NativeSessionContext,
    args: &[String],
    context: &NativeCommandContext<'_>,
    parsed: NativeParse,
    intent: &NativeIntent,
) -> Result<NativeParse> {
    if !intent.refresh_metadata {
        return Ok(parsed);
    }
    let name = parsed.grammar.get_name().to_string();
    let refreshed = command.prepare_invocation_metadata(args, context);
    if matches!(refreshed, Ok(true)) {
        session_context.request_completion_refresh();
    }
    if intent.requirement == ExternalPathAccessRequirement::Visible && parsed.render_help {
        match refreshed {
            Ok(true) => match NativeParse::new(command, &name, args, parsed.prefixes) {
                Ok(parsed) => return Ok(parsed),
                Err(error) => {
                    tracing::debug!(command = %name, error = %error, "refreshed native help arguments could not be normalized; using cached help")
                }
            },
            Ok(false) => {}
            Err(error) => {
                tracing::debug!(command = %name, error = %error, "native help metadata refresh failed; using cached help")
            }
        }
        return Ok(parsed);
    }
    if refreshed.map_err(|error| {
        crate::app::report_anyhow_with_context(
            error,
            crate::app::host::ErrorContext::Operation(
                "failed to prepare native invocation metadata",
            ),
        )
    })? {
        NativeParse::new(command, &name, args, parsed.prefixes)
    } else {
        Ok(parsed)
    }
}

fn run_native_invocation(
    command: &dyn crate::native::NativeCommand,
    state: &mut crate::app::AppState,
    input: NativeRunInput<'_, '_, '_, '_>,
    guide_help: impl Fn(&str) -> Document,
    accepted: &mut Option<&mut bool>,
) -> Result<CommandOutput> {
    let diagnostics = crate::diagnostics::DiagnosticSink::default();
    let _scope = diagnostics.enter();
    let level = input.invocation.ui.verbosity;
    let mut command_path = Vec::new();
    let result: Result<CommandOutput> = (|| {
        let name = input.command_path.as_str();
        let args = input.args;
        let invocation = input.invocation;
        if input.elevation.is_elevated() && !command.supports_invocation_elevation() {
            return Err(miette!(
                "elevation prefix is unsupported for native command `{name}`"
            ));
        }
        let input_path = crate::core::command_policy::CommandPath::new(
            std::iter::once(name.to_string()).chain(args.iter().cloned()),
        );
        before_external_dispatch(&mut state.runtime, &input_path, "command")?;
        state.reload_config_after_access_hook()?;
        let prefixes =
            crate::command_words::CommandPrefixes::from_config(state.runtime.config.resolved())
                .enabled(state.runtime.context.terminal_kind() == crate::app::TerminalKind::Repl);
        let parsed = NativeParse::new(command, name, args, prefixes)?;
        let intent = NativeIntent::from_parse(&parsed);
        before_external_execution(
            &mut state.runtime,
            &parsed.path,
            intent.requirement,
            "command",
        )?;
        state.reload_config_after_access_hook()?;
        let parsed = refresh_native_parse(
            command,
            &state.session.native_context,
            args,
            &NativeCommandContext::new(
                state.runtime.config.resolved(),
                runtime_hints_for_invocation(&state.runtime, invocation),
            )
            .with_invocation_elevation(input.elevation)
            .with_command_scope(invocation.ui.render_settings.runtime.command_scope.clone()),
            parsed,
            &intent,
        )?;
        command_path = parsed.path.as_slice().to_vec();
        let mut invocation = invocation.clone();
        invocation.ui.render_settings.command_path = command_path.clone();
        let invocation = &invocation;
        state.runtime.auth.refresh_native_auth(&command.describe());
        ensure_external_access(&state.runtime, &parsed.path, intent.requirement, "command")?;

        if intent.render_help || parsed.render_help {
            mark_repl_command_accepted(accepted);
            if let Err(error) = &parsed.matches
                && error.kind() == clap::error::ErrorKind::DisplayVersion
            {
                return Ok(CommandOutput::document(guide_help(&error.to_string())));
            }
            let tokens = std::iter::once(name.to_string())
                .chain(parsed.args.iter().cloned())
                .collect::<Vec<_>>();
            let (document, _, _) = crate::cli::command_help_document(
                state
                    .runtime
                    .auth
                    .visible_help_command(parsed.grammar.clone(), &[]),
                &tokens,
                &invocation.ui.render_settings.runtime.command_scope,
            );
            let mut output = document.to_output_result();
            output.meta.document_command_prefixes = true;
            let help_tokens = crate::cli::help_parse_tokens(&tokens);
            let help_path = native_resolved_path(&parsed.grammar, name, &help_tokens[1..], None);
            let path = &help_path.as_slice()[1..];
            let selected = path.iter().try_fold(&parsed.grammar, |grammar, name| {
                grammar.find_subcommand(name)
            });
            if let Some(selected) = selected {
                let names = selected
                    .get_subcommands()
                    .filter(|child| !child.is_hide_set())
                    .map(clap::Command::get_name)
                    .collect::<Vec<_>>();
                output.meta.document_literal_commands =
                    names
                        .iter()
                        .filter(|name| {
                            let prefix = crate::command_words::shortest_prefix(name, &names);
                            let mut args = path.to_vec();
                            args.push(name[..prefix].to_owned());
                            !command.normalize_invocation_args(&args, true).is_ok_and(
                                |normalized| normalized.last().map(String::as_str) == Some(**name),
                            )
                        })
                        .map(|name| (*name).to_owned())
                        .collect();
            }
            return Ok(CommandOutput::output(output, None));
        }
        if let Err(error) = parsed.matches {
            state.session.native_context.activate(&state.runtime);
            let selected_context = if state.runtime.context.terminal_kind() == TerminalKind::Repl {
                state
                    .session
                    .native_context
                    .for_service(command.command().get_name())
            } else {
                NativeSessionContext::default()
            };
            let resolved =
                crate::context::resolve(command.command(), &parsed.args, &selected_context)
                    .map_err(|e| {
                        super::host::report_anyhow_with_context(
                            e,
                            super::host::ErrorContext::NativeExecution,
                        )
                    })?;
            let resolves_missing = resolved.args != parsed.args;
            let mut selected = &parsed.grammar;
            for segment in parsed.path.as_slice().iter().skip(1) {
                if let Some(child) = selected.find_subcommand(segment) {
                    selected = child;
                }
            }
            if !resolves_missing
                && (selected.has_subcommands()
                    || parsed.args.len() + 1 == parsed.path.as_slice().len())
            {
                return Err(super::host::report_clap_error_with_context(
                    error,
                    crate::app::host::ErrorContext::Operation("native command syntax is invalid"),
                    parsed.grammar,
                    std::iter::once(name).chain(parsed.args.iter().map(String::as_str)),
                    None,
                ));
            }
        }
        mark_repl_command_accepted(accepted);
        run_native_command(
            command,
            &mut state.runtime,
            &mut state.session,
            NativeRunInput {
                args: &parsed.args,
                source_args: args,
                action: None,
                stages: input.stages,
                invocation,
                command_path: parsed.path.as_slice().join(" "),
                elevation: input.elevation,
                progress_sink: input.progress_sink,
            },
            guide_help,
            accepted,
        )
    })();
    match result {
        Ok(mut output) => {
            output.command_path = command_path;
            output.retained_evidence = diagnostics
                .render(crate::core::MessageLevel::Evidence, output.exit_code != 0)
                .0;
            output.diagnostics = diagnostics.render(level, output.exit_code != 0);
            Ok(output)
        }
        Err(error) if super::host::classify_exit_code(&error) == super::host::EXIT_CODE_USAGE => {
            Err(error)
        }
        Err(error) => Err(super::host::report_with_evidence(error, diagnostics)),
    }
}

fn mark_repl_command_accepted(accepted: &mut Option<&mut bool>) {
    if let Some(accepted) = accepted.as_mut() {
        **accepted = true;
    }
}

fn mark_repl_command_rejected(accepted: &mut Option<&mut bool>) {
    if let Some(accepted) = accepted.as_mut() {
        **accepted = false;
    }
}

fn native_path_from_matches(
    command: &str,
    matches: &clap::ArgMatches,
) -> crate::core::command_policy::CommandPath {
    let mut segments = vec![command.to_string()];
    let mut current = matches;
    while let Some((subcommand, submatches)) = current.subcommand() {
        segments.push(subcommand.to_string());
        current = submatches;
    }
    crate::core::command_policy::CommandPath::new(segments)
}

fn native_resolved_path(
    grammar: &clap::Command,
    name: &str,
    args: &[String],
    matches: Option<&clap::ArgMatches>,
) -> crate::core::command_policy::CommandPath {
    if let Some(matches) = matches {
        return native_path_from_matches(name, matches);
    }
    let mut tokens = args.iter().map(String::as_str).collect::<Vec<_>>();
    // Clap's help subcommand names a command path rather than executing it.
    if tokens.first() == Some(&"help") {
        tokens.remove(0);
    }
    tokens.retain(|arg| !matches!(*arg, "--help" | "-h"));
    crate::cli::passive_command(grammar.clone())
        .try_get_matches_from(std::iter::once(name).chain(tokens))
        .map(|matches| native_path_from_matches(name, &matches))
        .unwrap_or_else(|_| crate::core::command_policy::CommandPath::new([name]))
}

pub(crate) fn run_repl_action(
    state: &mut crate::app::AppState,
    id: &str,
) -> Result<crate::repl::ReplResult> {
    let invocation = super::resolve_invocation_ui(
        state.runtime.config.resolved(),
        &state.runtime.ui,
        &crate::cli::invocation::InvocationOptions::default(),
    );
    let commands = super::runtime::visible_native_commands(&state.runtime.auth, &state.clients)
        .map(|(name, command)| (name.to_string(), command.clone()))
        .collect::<Vec<_>>();
    for (entry, command) in commands {
        if command
            .repl_actions(state.runtime.config.resolved())
            .map_err(|e| miette!("{e:#}"))?
            .iter()
            .any(|action| action.id == id && action.key.is_some())
        {
            let mut sink = super::sink::StdIoUiSink;
            let result = run_native_command(
                command.as_ref(),
                &mut state.runtime,
                &mut state.session,
                NativeRunInput {
                    args: &[],
                    source_args: &[],
                    action: Some(id),
                    stages: &[],
                    invocation: &invocation,
                    command_path: entry.to_string(),
                    elevation: InvocationElevation::Ordinary,
                    progress_sink: Some(&mut sink),
                },
                Document::from_help_text,
                &mut None,
            )?;
            super::run_cli_command(
                state.runtime.config.resolved(),
                &invocation.ui,
                &result,
                &mut sink,
            )?;
            return Ok(crate::repl::ReplResult::Continue);
        }
    }
    Err(miette!("Editor action is unavailable: {id}"))
}

struct NativeRunInput<'args, 'stages, 'invocation, 'sink> {
    args: &'args [String],
    source_args: &'args [String],
    action: Option<&'args str>,
    stages: &'stages [String],
    invocation: &'invocation ResolvedInvocation,
    command_path: String,
    elevation: InvocationElevation,
    progress_sink: Option<&'sink mut dyn UiSink>,
}

fn run_native_command(
    command: &dyn crate::native::NativeCommand,
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    input: NativeRunInput<'_, '_, '_, '_>,
    guide_help: impl Fn(&str) -> Document,
    accepted: &mut Option<&mut bool>,
) -> Result<CommandOutput> {
    tracing::info!(target: "osp::diagnostic", "debug: Dispatch selected; handler=native:{}.", command.command().get_name());
    if input.action.is_none() {
        session.native_context.clear_pagination();
    }
    let repl = runtime.context.terminal_kind() == TerminalKind::Repl;
    session.native_context.activate(runtime);
    let service = command.command().get_name().to_string();
    session.native_context.register_context(
        &service,
        command.context_origin(runtime.config.resolved()),
        command.persisted_context_kinds(),
        &command.command(),
        command.context_kinds(),
    );
    let selected_context = session.native_context.for_service(&service).for_command(
        std::iter::once(service.clone())
            .chain(input.args.iter().cloned())
            .collect::<Vec<_>>()
            .join(" "),
    );
    let empty_context = NativeSessionContext::default();
    let resolution = if input.action.is_none() {
        Some(
            crate::context::resolve(
                command.command(),
                input.args,
                if repl {
                    &selected_context
                } else {
                    &empty_context
                },
            )
            .map_err(|e| {
                super::host::report_anyhow_with_context(
                    e,
                    super::host::ErrorContext::NativeExecution,
                )
            })?,
        )
    } else {
        None
    };
    let args = resolution
        .as_ref()
        .map(|r| r.args.as_slice())
        .unwrap_or(input.args);
    if let Some(resolution) = &resolution
        && let Some(line) = resolution
            .record_history(
                &session.native_context,
                &service,
                input.args,
                input.source_args,
            )
            .map_err(|e| miette!("{e:#}"))?
    {
        let elevation = if input.elevation.is_elevated() {
            "sudo "
        } else {
            ""
        };
        let mut messages = crate::ui::messages::MessageBuffer::default();
        messages.success(format!("→ {elevation}{line}"));
        super::command_output::emit_messages_for_ui(
            runtime.config.resolved(),
            &input.invocation.ui,
            &messages,
            input.invocation.ui.verbosity,
            &mut super::StdIoUiSink,
        );
    }
    let progress_renderer = input.progress_sink.map(|sink| NativeProgressRenderer {
        config: runtime.config.resolved(),
        ui: &input.invocation.ui,
        stages: input.stages,
        sink: RefCell::new(ProgressUiSink::new(
            sink,
            input
                .invocation
                .ui
                .render_settings
                .resolve_render_settings()
                .unicode,
        )),
        can_present: std::cell::Cell::new(false),
        pending: RefCell::new(None),
        last_draw: std::cell::Cell::new(None),
    });
    let diagnostics = crate::diagnostics::DiagnosticSink::current().unwrap_or_default();
    let _diagnostic_scope = diagnostics.enter();
    let mut context = NativeCommandContext::new(
        runtime.config.resolved(),
        runtime_hints_for_invocation(runtime, input.invocation),
    )
    .with_command_scope(
        input
            .invocation
            .ui
            .render_settings
            .runtime
            .command_scope
            .clone(),
    )
    .with_diagnostic_sink(&diagnostics)
    .with_invocation_elevation(input.elevation)
    .with_session_context(
        if input.action.is_some() {
            session.native_context.for_editor_action()
        } else {
            session.native_context.clone()
        }
        .for_service(&service)
        .for_command(input.command_path.as_str()),
    );
    context.repl_feedback = session.repl_feedback.clone();
    context.output_pipeline = !input.stages.is_empty();
    if let Some(renderer) = progress_renderer.as_ref() {
        context = context.with_progress_sink(renderer);
        renderer.can_present.set(context.can_present());
    }

    let execution = match input.action {
        Some(action) => command.execute_repl_action(action, &context),
        None => command.execute(args, &context),
    }
    .map_err(|err| {
        let structured = err.downcast_ref::<crate::core::CommandError>().cloned();
        match err.downcast::<clap::Error>() {
            Ok(err) => {
                mark_repl_command_rejected(accepted);
                super::host::report_clap_error_with_context(
                    err,
                    crate::app::host::ErrorContext::NativeExecution,
                    command.command(),
                    std::iter::once(command.command().get_name().to_string())
                        .chain(input.args.iter().cloned()),
                    structured,
                )
            }
            Err(err) => crate::app::report_anyhow_with_context(
                err,
                crate::app::host::ErrorContext::NativeExecution,
            ),
        }
    });
    drop(context);
    if let Some(renderer) = progress_renderer.as_ref() {
        renderer.clear();
    }

    let execution = execution.and_then(|mut outcome| {
        if repl && let Some(resolution) = &resolution {
            match &mut outcome {
                NativeCommandOutcome::Response(response)
                | NativeCommandOutcome::ResponseWithExit { response, .. } => {
                    resolution.finish(response, &selected_context);
                }
                _ => {}
            }
            selected_context
                .save_context()
                .map_err(|e| miette!("{e:#}"))?;
        }
        Ok(outcome)
    });
    let mut output = match execution {
        Ok(outcome) => match outcome {
            NativeCommandOutcome::Help(text) => {
                let mut document = guide_help(&text);
                let path = input
                    .command_path
                    .split_whitespace()
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                runtime.auth.filter_help_document(&mut document, &path);
                Ok(CommandOutput::document(document))
            }
            NativeCommandOutcome::Exit(code) => Ok(CommandOutput::exit(code)),
            NativeCommandOutcome::Response(response) => render_native_response_with_context(
                *response,
                input.stages,
                session,
                input.action.is_none() && runtime.context.terminal_kind() == TerminalKind::Repl,
            ),
            NativeCommandOutcome::ResponseWithExit {
                response,
                exit_code,
            } => {
                let mut result = render_native_response_with_context(
                    *response,
                    input.stages,
                    session,
                    input.action.is_none() && runtime.context.terminal_kind() == TerminalKind::Repl,
                )?;
                result.exit_code = exit_code;
                Ok(result)
            }
        },
        Err(report) => {
            let mut output = CommandOutput::exit(super::host::classify_exit_code(&report));
            output.failure_report = Some(report);
            Ok(output)
        }
    }?;
    output.retained_evidence = diagnostics
        .render(crate::core::MessageLevel::Evidence, output.exit_code != 0)
        .0;
    output.diagnostics = diagnostics.render(input.invocation.ui.verbosity, output.exit_code != 0);
    Ok(output)
}

struct NativeProgressRenderer<'config, 'sink> {
    config: &'config crate::config::ResolvedConfig,
    ui: &'config UiState,
    stages: &'config [String],
    sink: RefCell<ProgressUiSink<'sink>>,
    can_present: std::cell::Cell<bool>,
    pending: RefCell<Option<NativeProgressEvent>>,
    last_draw: std::cell::Cell<Option<std::time::Instant>>,
}

impl NativeProgressRenderer<'_, '_> {
    fn clear(&self) {
        self.pending.borrow_mut().take();
        self.sink.borrow_mut().clear_progress();
    }

    fn draw(
        &self,
        mut event: NativeProgressEvent,
        stages: &[String],
        stream: bool,
    ) -> anyhow::Result<()> {
        let human = !matches!(
            self.ui.render_settings.requested_format(),
            crate::core::OutputFormat::Json
                | crate::core::OutputFormat::Csv
                | crate::core::OutputFormat::Tsv
        );
        if stream {
            event.meta.preserve_json_document = true;
            event.meta.row_path = None;
        }
        if human
            && stages.is_empty()
            && self.can_present.get()
            && let Some(frame) = event.frame.take()
        {
            event.data = frame.data;
            event.meta = frame.meta;
            event.document = frame.document;
        }
        let mut result = CommandOutput::from_data(event.data, &event.meta, &event.messages, stages);
        if human
            && stages.is_empty()
            && self.can_present.get()
            && let Some(document) = event.document
        {
            result.body = super::CommandBody::Output(Box::new(document.to_output_result()));
            result.format_hint = None;
        }
        result
            .apply_stages(stages)
            .map_err(|err| anyhow::Error::from_boxed(err.into()))?;
        let mut sink = self.sink.borrow_mut();
        run_progress_command_with_ui(self.config, self.ui, result, &mut *sink, stream)
            .map_err(|err| anyhow::Error::from_boxed(err.into()))?;
        Ok(())
    }
}

impl NativeProgressSink for NativeProgressRenderer<'_, '_> {
    fn stderr_is_terminal(&self) -> bool {
        self.sink.borrow().stderr_is_terminal()
    }

    fn emit_measured(&self, event: NativeProgressEvent) -> anyhow::Result<bool> {
        self.flush()?;
        self.draw(event, self.stages, false)?;
        Ok(self.sink.borrow().can_replace())
    }

    fn present(&self, document: NativeProgressEvent) -> anyhow::Result<()> {
        self.flush()?;
        self.draw(document, &[], false)
    }

    fn emit_stream(&self, event: NativeProgressEvent) -> anyhow::Result<()> {
        anyhow::ensure!(event.data.is_object(), "stream events must be JSON objects");
        self.flush()?;
        self.draw(event, self.stages, true)
    }

    fn emit_stream_measured(&self, event: NativeProgressEvent) -> anyhow::Result<bool> {
        self.emit_stream(event)?;
        Ok(self.can_present.get() && self.sink.borrow().can_replace())
    }

    fn emit(&self, event: NativeProgressEvent) -> anyhow::Result<()> {
        let tree = event.meta.progress_replace && self.can_present.get();
        if !tree {
            return self.draw(event, self.stages, false);
        }
        let replay = event.replay;
        *self.pending.borrow_mut() = Some(event);
        if replay {
            return Ok(());
        }
        if self
            .last_draw
            .get()
            .is_none_or(|last| last.elapsed() >= std::time::Duration::from_millis(250))
        {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&self) -> anyhow::Result<()> {
        // emit() already drops throttled intermediate frames; an explicit flush draws the latest one.
        if let Some(event) = self.pending.borrow_mut().take() {
            self.draw(event, self.stages, false)?;
            self.last_draw.set(Some(std::time::Instant::now()));
        }
        Ok(())
    }
}

fn render_native_response_with_context(
    response: crate::core::plugin::ResponseV1,
    stages: &[String],
    session: &mut AppSession,
    show_pagination_hint: bool,
) -> Result<CommandOutput> {
    let mut result = match cli_result_from_plugin_response(response, stages) {
        Ok(result) => result,
        Err(error) => {
            if show_pagination_hint {
                session.native_context.clear_pagination();
            }
            return Err(error);
        }
    };
    if show_pagination_hint {
        add_native_pagination_hint(&mut result, &session.native_context);
    }
    Ok(result)
}

fn add_native_pagination_hint(result: &mut CommandOutput, context: &NativeSessionContext) {
    let Some(pagination) = context.pagination() else {
        return;
    };
    let text = match (context.has_previous_page(), pagination.next.is_some()) {
        (true, true) => {
            "More results are available. Type `next` or press Shift+→ for the next page, `prev` or Shift+← for the previous."
        }
        (false, true) => {
            "More results are available. Type `next` or press Shift+→ for the next page."
        }
        (true, false) => "Type `prev` or press Shift+← for the previous page.",
        (false, false) => return,
    };
    result.messages.success(text);
}

/// Returns invocation flags that only appear after alias or quoted-line
/// expansion, so the host can apply them like flags typed on the command line.
fn expand_external_tokens(
    runtime: &AppRuntime,
    tokens: &[String],
) -> Result<(ParsedCommandLine, InvocationElevation)> {
    let (tokens, elevation) = parse_invocation_elevation(tokens)?;
    // REPL text has already separated pipes while quote information was present.
    let parsed = if runtime.context.terminal_kind() == TerminalKind::Repl {
        crate::cli::pipeline::finalize_command_with_aliases(
            tokens,
            Vec::new(),
            runtime.config.resolved(),
        )?
    } else {
        parse_command_tokens_with_aliases(&tokens, runtime.config.resolved())?
    };
    Ok((parsed, elevation))
}

fn external_parse_context(tokens: &[String]) -> String {
    format!(
        "failed to parse external command invocation for `{}`",
        tokens.first().map(String::as_str).unwrap_or("external")
    )
}

fn parse_external_invocation(
    runtime: &AppRuntime,
    session: &AppSession,
    tokens: &[String],
    invocation: &ResolvedInvocation,
    stages: &[String],
) -> Result<ExternalParse> {
    expand_external_tokens(runtime, tokens)
        .and_then(|(mut parsed, elevation)| {
            parsed.tokens = scan_command_tokens(&parsed.tokens)?.tokens;
            parsed.stages.extend_from_slice(stages);
            plan_external_invocation(runtime, session, parsed, elevation, invocation)
        })
        .wrap_err_with(|| external_parse_context(tokens))
}

fn plan_external_invocation(
    runtime: &AppRuntime,
    _session: &AppSession,
    mut parsed: ParsedCommandLine,
    elevation: InvocationElevation,
    invocation: &ResolvedInvocation,
) -> Result<ExternalParse> {
    let apply_output_stages = runtime.context.terminal_kind() == TerminalKind::Cli;
    if parsed.tokens.is_empty() {
        return Err(miette!("missing external command"));
    }
    if let Some(spec) = parsed.stages.iter().find_map(|raw| {
        let parsed = crate::dsl::parse::pipeline::parse_stage(raw).ok()?;
        crate::cli::is_cli_help_stage(&parsed).then_some(parsed.spec)
    }) {
        return Ok(ExternalParse::Handled(Box::new(CommandOutput::document(
            crate::dsl::verb_info::help_document(&spec)?,
        ))));
    }

    if let Some(tokens) = crate::repl::input::rewrite_help_alias_tokens(&parsed.tokens) {
        parsed.tokens = tokens;
    }
    let inline_command = match super::command_plan::parse_inline_command(
        &parsed.tokens,
        super::command_plan::CommandSurface::External,
        crate::command_words::CommandPrefixes::from_config(runtime.config.resolved())
            .enabled(runtime.context.terminal_kind() == TerminalKind::Repl),
    )? {
        super::command_plan::InlineCommandPlan::Command(command) => command,
        super::command_plan::InlineCommandPlan::Help(_) => {
            let (mut view, root_help, long_help) = crate::cli::inline_help_document(
                &parsed.tokens,
                &invocation.ui.render_settings.runtime.command_scope,
            );
            if root_help {
                super::host::add_native_command_help(
                    &mut view,
                    &runtime.definition.native_commands,
                    &runtime.auth,
                );
            }
            extend_with_invocation_help(&mut view, invocation.help_level);
            let mut output = view
                .filtered_for_help_level(invocation.help_level)
                .to_output_result();
            output.meta.document_long_help = long_help;
            let result = super::command_output::apply_stages_to_cli_result(
                CommandOutput::output(output, None),
                if apply_output_stages {
                    &parsed.stages
                } else {
                    &[]
                },
            )?;
            return Ok(ExternalParse::Handled(Box::new(result)));
        }
    };

    Ok(ExternalParse::Invocation(Box::new(
        ParsedExternalInvocation {
            tokens: rewrite_shellable_root_help_tokens(parsed.tokens, runtime.config.resolved()),
            stages: parsed.stages,
            apply_output_stages,
            inline_command,
            elevation,
        },
    )))
}

fn parse_invocation_elevation(tokens: &[String]) -> Result<(Vec<String>, InvocationElevation)> {
    if tokens.first().map(String::as_str) != Some("sudo") {
        return Ok((tokens.to_vec(), InvocationElevation::Ordinary));
    }
    if tokens.get(1).is_none() {
        return Err(miette!("sudo prefix must be followed by a command"));
    }

    let mut command_tokens = tokens.to_vec();
    command_tokens.remove(0);
    Ok((command_tokens, InvocationElevation::Elevated))
}

fn rewrite_shellable_root_help_tokens(
    tokens: Vec<String>,
    config: &crate::config::ResolvedConfig,
) -> Vec<String> {
    if tokens.len() == 1
        && let Some(command) = tokens.first()
        && is_repl_shellable_command(config, command)
    {
        return vec![command.clone(), "--help".to_string()];
    }
    tokens
}

fn run_external_plugin_command(
    state: &mut crate::app::AppState,
    command: &str,
    parsed: &ParsedExternalInvocation,
    invocation: &ResolvedInvocation,
    _guide_help: impl Fn(&str) -> Document,
    accepted: &mut Option<&mut bool>,
) -> Result<CommandOutput> {
    let plugin = state
        .clients
        .plugins()
        .find(command)
        .ok_or_else(|| miette!("unknown plugin `{command}`"))?;
    let args = &parsed.tokens[1..];
    let help = args
        .iter()
        .take_while(|arg| arg.as_str() != "--")
        .any(|arg| matches!(arg.as_str(), "--help" | "-h"));
    let resolved = plugin.command().resolve_invocation(args);
    if resolved.ambiguous && !help {
        return Err(miette!(
            "cannot resolve plugin command `{command}`: describe metadata does not cover its arguments"
        ));
    }
    let path = resolved.path;
    let requirement = if help {
        ExternalPathAccessRequirement::Visible
    } else {
        ExternalPathAccessRequirement::Runnable
    };
    before_external_run(&mut state.runtime, &path, requirement, "plugin command")?;
    state.reload_config_after_access_hook()?;
    ensure_external_access(&state.runtime, &path, requirement, "plugin command")?;
    let mut invocation = invocation.clone();
    invocation.ui.render_settings.command_path = path.as_slice().to_vec();
    let invocation = &invocation;
    mark_repl_command_accepted(accepted);
    let capture = !help
        && (plugin
            .describe
            .as_ref()
            .is_some_and(|d| d.output.as_deref() == Some("json"))
            || !parsed.stages.is_empty()
            || invocation.ui.render_settings.format_explicit
            || invocation.ui.render_settings.requested_format() != crate::core::OutputFormat::Auto
            || invocation.source);
    let mut context = plugin_dispatch_context_for(&state.runtime, Some(invocation));
    let selected_context = state
        .session
        .native_context
        .for_service(command)
        .for_command(path.as_slice().join(" "));
    let resolution = if state.runtime.context.terminal_kind() == TerminalKind::Repl && !help {
        state.session.native_context.activate(&state.runtime);
        Some(
            crate::context::resolve(
                crate::context::plugin_grammar(&plugin.command()),
                args,
                &selected_context,
            )
            .map_err(|e| {
                super::host::report_anyhow_with_context(
                    e,
                    super::host::ErrorContext::Operation("failed to resolve plugin context"),
                )
            })?,
        )
    } else {
        None
    };
    if let Some(resolution) = &resolution {
        for consume in &resolution.contract.consumes {
            if let Some(value) = selected_context.context_value(&consume.kind) {
                context.shared_env.push((
                    format!("OSP_CTX_{}", consume.kind.to_ascii_uppercase()),
                    value,
                ));
            }
        }
    }
    if let Some(resolution) = &resolution
        && let Some(line) = resolution
            .record_history(&state.session.native_context, command, args, args)
            .map_err(|e| miette!("{e:#}"))?
    {
        let mut messages = crate::ui::messages::MessageBuffer::default();
        messages.success(format!("→ {line}"));
        super::command_output::emit_messages_for_ui(
            state.runtime.config.resolved(),
            &invocation.ui,
            &messages,
            invocation.ui.verbosity,
            &mut super::StdIoUiSink,
        );
    }
    let args = resolution
        .as_ref()
        .map(|r| r.args.as_slice())
        .unwrap_or(args);
    tracing::info!(target: "osp::diagnostic", "debug: Dispatch selected; handler=plugin:{}.", plugin.name);
    let diagnostics = crate::diagnostics::DiagnosticSink::default();
    let started = std::time::Instant::now();
    let raw = crate::plugin::run::run(&plugin, args, &context, capture)?;
    let elapsed_ms = started.elapsed().as_millis();
    let bytes = raw.stdout.as_ref().map(Vec::len);
    let mut result = if let Some(bytes) = raw.stdout {
        match serde_json::from_slice::<serde_json::Value>(&bytes) {
            Ok(value) => {
                let mut response = if value.get("protocol_version").is_some() {
                    let response: crate::core::plugin::ResponseV1 = serde_json::from_value(value)
                        .map_err(|_| {
                        miette::Report::new(
                            invocation
                                .ui
                                .render_settings
                                .runtime
                                .command_scope
                                .run_hint(
                                    "Invalid plugin response metadata.",
                                    "doctor plugins",
                                    ".",
                                ),
                        )
                    })?;
                    response.validate_v1().map_err(|_| {
                        miette::Report::new(
                            invocation
                                .ui
                                .render_settings
                                .runtime
                                .command_scope
                                .run_hint(
                                    "Invalid plugin response metadata.",
                                    "doctor plugins",
                                    ".",
                                ),
                        )
                    })?;
                    response
                } else {
                    crate::core::plugin::ResponseV1 {
                        source: None,
                        protocol_version: 1,
                        ok: true,
                        data: value,
                        error: None,
                        messages: Vec::new(),
                        context: None,
                        meta: Default::default(),
                    }
                };
                crate::context::protect_response_context(&mut response, &context.secrets);
                if let Some(resolution) = &resolution {
                    resolution.finish(&mut response, &selected_context);
                    selected_context
                        .save_context()
                        .map_err(|e| miette!("{e:#}"))?;
                }
                let plugin_text = |raw: &str| {
                    let mut text = raw.to_owned();
                    for secret in &context.secrets {
                        text = text.replace(secret, "[redacted]");
                    }
                    crate::ui::sanitize_human_text(&text)
                        .replace('\n', " ")
                        .chars()
                        .take(512)
                        .collect::<String>()
                };
                for message in &mut response.messages {
                    message.text = plugin_text(&message.text);
                }
                if !response.ok
                    && let Some(error) = response.error.as_mut()
                {
                    error.code = plugin_text(&error.code);
                    if error.code.len() > 80
                        || !error
                            .code
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
                    {
                        error.code = "plugin_error".into();
                    }
                    let mut details = serde_json::Map::new();
                    if let Some(attempt) = error.details["attempt"].as_u64() {
                        details.insert("attempt".into(), attempt.into());
                    }
                    if let Some(backend) = error.details["backend"]
                        .as_str()
                        .map(plugin_text)
                        .filter(|backend| {
                            !backend.is_empty()
                                && backend.len() <= 80
                                && backend.bytes().all(|byte| {
                                    byte.is_ascii_lowercase()
                                        || byte.is_ascii_digit()
                                        || matches!(byte, b'_' | b'-')
                                })
                        })
                    {
                        details.insert("backend".into(), backend.into());
                    }
                    error.message = format!(
                        "Plugin {} failed: {}.",
                        plugin.name,
                        plugin_text(&error.message).trim_end_matches('.')
                    );
                    if invocation.ui.verbosity < crate::core::MessageLevel::Info {
                        details.insert(
                            "action".into(),
                            serde_json::json!("Rerun with -v for the plugin's details."),
                        );
                    }
                    error.details = details.into();
                }
                for text in response
                    .meta
                    .column_labels
                    .iter_mut()
                    .chain(response.meta.partial_rows.iter_mut())
                {
                    *text = plugin_text(text);
                }
                for rule in &mut response.meta.display_rules {
                    if let crate::core::output_model::DisplayRule::SuffixWhenTrue {
                        suffix, ..
                    } = rule
                    {
                        *suffix = plugin_text(suffix);
                    }
                }
                let mut result = cli_result_from_plugin_response(response, &[])?;
                result.stderr_text = plugin_text(&result.stderr_text);
                result
            }
            Err(_) if !parsed.stages.is_empty() => {
                return Err(miette!(
                    "`{command}` printed text, not JSON; pipes need JSON output"
                ));
            }
            Err(_) => CommandOutput::text(String::from_utf8_lossy(&bytes)),
        }
    } else {
        CommandOutput::exit(raw.status)
    };
    if raw.status != 0 {
        result.exit_code = raw.status;
        result.preserve_process_output = result.failure_report.is_none();
        if result.failure_report.is_none() {
            result.failure_report = Some(miette::Report::new(
                invocation
                    .ui
                    .render_settings
                    .runtime
                    .command_scope
                    .run_hint(
                        format!("Plugin {command} crashed (exit status {}).", raw.status),
                        "doctor plugins",
                        ".",
                    ),
            ));
        }
    }
    let backend = result
        .failure_report
        .as_ref()
        .and_then(super::host::find_error_in_chain::<super::command_output::CommandBackendDetails>);
    let details = backend.map(|backend| &backend.error.details);
    diagnostics.emit(crate::diagnostics::DiagnosticEvent::PluginOutcome {
        plugin: plugin.name,
        exit_code: raw.status,
        elapsed_ms,
        error_code: backend.map(|backend| backend.error.code.clone()),
        attempt: details.and_then(|details| details["attempt"].as_u64()),
        backend: details
            .and_then(|details| details["backend"].as_str())
            .map(str::to_owned),
        bytes,
    });
    result.retained_evidence = diagnostics
        .render(crate::core::MessageLevel::Evidence, result.exit_code != 0)
        .0;
    result.command_path = path.as_slice().to_vec();
    result.diagnostics = diagnostics.render(invocation.ui.verbosity, result.exit_code != 0);
    Ok(result)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::{
        ExternalParse, add_native_pagination_hint, cli_result_from_plugin_response,
        parse_external_invocation, run_external_command_with_help_renderer,
    };
    use crate::app::{
        AppClients, AppRuntime, AppSession, AuthState, ConfigState, LaunchContext, RuntimeContext,
        TerminalKind, UiState, resolve_invocation_ui,
    };
    use crate::app::{BufferedUiSink, CommandOutput};
    use crate::cli::invocation::InvocationOptions;
    use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions};
    use crate::core::MessageLevel;
    use crate::core::document::Document;
    use crate::core::output::OutputFormat;
    use crate::core::plugin::{
        DescribeCommandAuthV1, DescribeCommandV1, DescribeVisibilityModeV1, PLUGIN_PROTOCOL_V1,
        ResponseMessageV1, ResponseMetaV1, ResponseV1,
    };
    use crate::native::{
        NativeCommand, NativeCommandContext, NativeCommandOutcome, NativeCommandRegistry,
        NativePagination, NativeProgressEvent, NativeSessionContext,
    };
    use crate::plugin::PluginManager;
    use crate::ui::RenderSettings;
    use clap::Command;
    #[derive(Clone, Copy)]
    enum NativeOutcomeKind {
        Help,
        Exit,
        Response,
        ResponseWithExit,
    }

    struct TestNativeCommand {
        kind: NativeOutcomeKind,
    }

    impl NativeCommand for TestNativeCommand {
        fn command(&self) -> Command {
            Command::new("ldap").about("Directory lookup")
        }

        fn auth(&self) -> Option<DescribeCommandAuthV1> {
            Some(DescribeCommandAuthV1 {
                visibility: Some(DescribeVisibilityModeV1::Public),
                required_capabilities: Vec::new(),
                feature_flags: Vec::new(),
                ..DescribeCommandAuthV1::default()
            })
        }

        fn execute(
            &self,
            args: &[String],
            _context: &NativeCommandContext<'_>,
        ) -> anyhow::Result<NativeCommandOutcome> {
            Ok(match self.kind {
                NativeOutcomeKind::Help => NativeCommandOutcome::Help(format!(
                    "Usage: osp ldap\n\nArgs: {}\n",
                    args.join(" ")
                )),
                NativeOutcomeKind::Exit => NativeCommandOutcome::Exit(7),
                NativeOutcomeKind::Response => {
                    NativeCommandOutcome::Response(Box::new(ResponseV1 {
                        source: None,
                        protocol_version: PLUGIN_PROTOCOL_V1,
                        ok: true,
                        data: serde_json::json!([{ "command": "ldap", "args": args }]),
                        error: None,
                        messages: vec![ResponseMessageV1 {
                            level: MessageLevel::Info,
                            text: "native ok".to_string(),
                        }],
                        context: None,
                        meta: ResponseMetaV1 {
                            title_field: None,
                            context_not_found: false,
                            detail_columns: Vec::new(),
                            display_notes: Vec::new(),
                            context: Default::default(),
                            format_hint: Some("json".to_string()),
                            columns: None,
                            column_align: Vec::new(),
                            column_labels: Vec::new(),
                            row_path: None,
                            unix_timestamp_columns: Vec::new(),
                            display_rules: Vec::new(),
                            preserve_json_document: false,
                            progress_replace: false,
                            display_limit: None,
                            partial_rows: None,
                        },
                    }))
                }
                NativeOutcomeKind::ResponseWithExit => NativeCommandOutcome::ResponseWithExit {
                    response: Box::new(ResponseV1 {
                        source: None,
                        protocol_version: PLUGIN_PROTOCOL_V1,
                        ok: true,
                        data: serde_json::json!({"status": "waiting_approval"}),
                        error: None,
                        messages: Vec::new(),
                        context: None,
                        meta: ResponseMetaV1 {
                            context_not_found: false,
                            context: Default::default(),
                            format_hint: Some("json".to_string()),
                            unix_timestamp_columns: Vec::new(),
                            display_rules: Vec::new(),
                            preserve_json_document: true,
                            ..ResponseMetaV1::default()
                        },
                    }),
                    exit_code: 3,
                },
            })
        }
    }

    struct ProgressNativeCommand;

    impl NativeCommand for ProgressNativeCommand {
        fn command(&self) -> Command {
            Command::new("progress").about("Emit progress before returning")
        }

        fn auth(&self) -> Option<DescribeCommandAuthV1> {
            Some(DescribeCommandAuthV1 {
                visibility: Some(DescribeVisibilityModeV1::Public),
                ..DescribeCommandAuthV1::default()
            })
        }

        fn execute(
            &self,
            _args: &[String],
            context: &NativeCommandContext<'_>,
        ) -> anyhow::Result<NativeCommandOutcome> {
            context.emit_progress(
                NativeProgressEvent::new(serde_json::json!({
                    "status": "running",
                    "message": "creating virtual machine",
                }))
                .with_meta(ResponseMetaV1 {
                    context_not_found: false,
                    context: Default::default(),
                    format_hint: Some("json".to_string()),
                    unix_timestamp_columns: Vec::new(),
                    display_rules: Vec::new(),
                    preserve_json_document: true,
                    ..ResponseMetaV1::default()
                }),
            )?;
            Ok(NativeCommandOutcome::Response(Box::new(ResponseV1 {
                source: None,
                protocol_version: PLUGIN_PROTOCOL_V1,
                ok: true,
                data: serde_json::json!({"status": "completed"}),
                error: None,
                messages: Vec::new(),
                context: None,
                meta: ResponseMetaV1 {
                    context_not_found: false,
                    context: Default::default(),
                    format_hint: Some("json".to_string()),
                    unix_timestamp_columns: Vec::new(),
                    display_rules: Vec::new(),
                    preserve_json_document: true,
                    ..ResponseMetaV1::default()
                },
            })))
        }
    }

    struct VerbosityNativeCommand;

    impl NativeCommand for VerbosityNativeCommand {
        fn command(&self) -> Command {
            Command::new("verbosity").about("Echo native runtime hints")
        }

        fn auth(&self) -> Option<DescribeCommandAuthV1> {
            Some(DescribeCommandAuthV1 {
                visibility: Some(DescribeVisibilityModeV1::Public),
                ..DescribeCommandAuthV1::default()
            })
        }

        fn execute(
            &self,
            _args: &[String],
            context: &NativeCommandContext<'_>,
        ) -> anyhow::Result<NativeCommandOutcome> {
            Ok(NativeCommandOutcome::Response(Box::new(ResponseV1 {
                source: None,
                protocol_version: PLUGIN_PROTOCOL_V1,
                ok: true,
                data: serde_json::json!({
                    "ui_verbosity": context.runtime_hints.ui_verbosity.as_str(),
                }),
                error: None,
                messages: Vec::new(),
                context: None,
                meta: ResponseMetaV1 {
                    title_field: None,
                    context_not_found: false,
                    detail_columns: Vec::new(),
                    display_notes: Vec::new(),
                    context: Default::default(),
                    format_hint: Some("json".to_string()),
                    columns: None,
                    column_align: Vec::new(),
                    column_labels: Vec::new(),
                    row_path: None,
                    unix_timestamp_columns: Vec::new(),
                    display_rules: Vec::new(),
                    preserve_json_document: false,
                    progress_replace: false,
                    display_limit: None,
                    partial_rows: None,
                },
            })))
        }
    }

    fn make_test_state_with_registry(
        native_commands: NativeCommandRegistry,
    ) -> crate::app::AppState {
        make_test_state_with_parts(
            PluginManager::new(Vec::new())
                .with_bundled_roots(false)
                .with_default_roots(false),
            native_commands,
        )
    }

    fn make_test_state_with_parts(
        plugins: PluginManager,
        native_commands: NativeCommandRegistry,
    ) -> crate::app::AppState {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        defaults.set("theme.path", Vec::<String>::new());
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        let config = resolver
            .resolve(ResolveOptions::default())
            .expect("test config should resolve");
        let themes = crate::ui::theme_catalog::load_theme_catalog(&config);
        let auth = AuthState::from_commands(&plugins, &native_commands);
        let runtime = AppRuntime::new(
            RuntimeContext::new(None, TerminalKind::Cli, None),
            ConfigState::new(config.clone()),
            UiState::new(
                RenderSettings::test_plain(OutputFormat::Json),
                MessageLevel::Success,
                0,
            ),
            auth,
            themes,
            LaunchContext::default(),
        );
        let session = AppSession::default();
        let clients = AppClients::new(plugins, native_commands);
        crate::app::AppState {
            runtime,
            session,
            clients,
        }
    }

    fn make_test_state_with_native(kind: Option<NativeOutcomeKind>) -> crate::app::AppState {
        make_test_state_with_registry(
            kind.map(|kind| NativeCommandRegistry::new().with_command(TestNativeCommand { kind }))
                .unwrap_or_default(),
        )
    }

    struct NestedAuthNativeCommand;

    struct AliasedNestedAuthNativeCommand;

    impl NativeCommand for NestedAuthNativeCommand {
        fn command(&self) -> Command {
            Command::new("ldap")
                .about("Directory lookup")
                .arg(
                    clap::Arg::new("format")
                        .long("format")
                        .value_name("FORMAT")
                        .global(true),
                )
                .subcommand(Command::new("user").about("Look up one user"))
        }

        fn describe(&self) -> DescribeCommandV1 {
            let mut root = DescribeCommandV1::from_clap(self.command());
            root.auth = Some(DescribeCommandAuthV1 {
                visibility: Some(DescribeVisibilityModeV1::Public),
                required_capabilities: Vec::new(),
                feature_flags: Vec::new(),
                ..DescribeCommandAuthV1::default()
            });
            root.subcommands[0].auth = Some(DescribeCommandAuthV1 {
                visibility: Some(DescribeVisibilityModeV1::Authenticated),
                required_capabilities: vec!["ldap.user.read".to_string()],
                feature_flags: Vec::new(),
                ..DescribeCommandAuthV1::default()
            });
            root
        }

        fn execute(
            &self,
            args: &[String],
            _context: &NativeCommandContext<'_>,
        ) -> anyhow::Result<NativeCommandOutcome> {
            let _ = args;
            panic!("nested-auth native command should not execute when auth denies it")
        }
    }

    impl NativeCommand for AliasedNestedAuthNativeCommand {
        fn command(&self) -> Command {
            Command::new("lookup").subcommand(Command::new("user").visible_alias("u"))
        }

        fn describe(&self) -> DescribeCommandV1 {
            let mut root = DescribeCommandV1::from_clap(self.command());
            root.auth = Some(DescribeCommandAuthV1 {
                visibility: Some(DescribeVisibilityModeV1::Public),
                ..DescribeCommandAuthV1::default()
            });
            root.subcommands[0].auth = Some(DescribeCommandAuthV1 {
                visibility: Some(DescribeVisibilityModeV1::Authenticated),
                required_capabilities: vec!["lookup.user.read".to_string()],
                ..DescribeCommandAuthV1::default()
            });
            root
        }

        fn execute(
            &self,
            _args: &[String],
            _context: &NativeCommandContext<'_>,
        ) -> anyhow::Result<NativeCommandOutcome> {
            panic!("aliased nested command should not execute when auth denies it")
        }
    }
    #[test]
    fn external_builtin_help_passthrough_is_handled_unit() {
        let state = make_test_state_with_native(None);
        let tokens = ["config", "--help"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();

        let invocation = crate::app::resolve_invocation_ui(
            state.runtime.config.resolved(),
            &state.runtime.ui,
            &Default::default(),
        );
        let parsed =
            parse_external_invocation(&state.runtime, &state.session, &tokens, &invocation, &[])
                .expect("help should parse");
        assert!(matches!(
            parsed,
            ExternalParse::Handled(result) if matches!(*result, CommandOutput {
                exit_code: 0,
                body: crate::app::CommandBody::Output(_),
                ..
            })
        ));
    }

    #[test]
    fn external_plugin_response_preserves_messages_unit() {
        let response = ResponseV1 {
            source: None,
            protocol_version: 1,
            ok: true,
            data: serde_json::json!({ "message": "hello" }),
            error: None,
            messages: vec![ResponseMessageV1 {
                level: MessageLevel::Warning,
                text: "warning from plugin".to_string(),
            }],
            context: None,
            meta: ResponseMetaV1 {
                title_field: None,
                context_not_found: false,
                detail_columns: Vec::new(),
                display_notes: Vec::new(),
                context: Default::default(),
                format_hint: Some("json".to_string()),
                columns: None,
                column_align: Vec::new(),
                column_labels: Vec::new(),
                row_path: None,
                unix_timestamp_columns: Vec::new(),
                display_rules: Vec::new(),
                preserve_json_document: false,
                progress_replace: false,
                display_limit: None,
                partial_rows: None,
            },
        };

        let result =
            cli_result_from_plugin_response(response, &[]).expect("response should prepare");
        assert!(!result.messages.is_empty());
    }

    #[test]
    fn native_pagination_hint_describes_available_directions_unit() {
        let context = NativeSessionContext::default();
        let mut result = CommandOutput::exit(0);
        add_native_pagination_hint(&mut result, &context);
        assert!(result.messages.is_empty());

        context.set_pagination(NativePagination {
            current: vec!["orch".to_string(), "1".to_string()],
            next: Some(vec!["orch".to_string(), "2".to_string()]),
        });
        add_native_pagination_hint(&mut result, &context);
        assert_eq!(
            result.messages.entries()[0].text,
            "More results are available. Type `next` or press Shift+→ for the next page."
        );

        let mut result = CommandOutput::exit(0);
        context.step_page(true);
        context.set_pagination(NativePagination {
            current: vec!["orch".to_string(), "2".to_string()],
            next: Some(vec!["orch".to_string(), "3".to_string()]),
        });
        add_native_pagination_hint(&mut result, &context);
        assert!(result.messages.entries()[0].text.contains("`prev`"));
        assert!(result.messages.entries()[0].text.contains("`next`"));
    }

    #[test]
    fn external_native_command_help_exit_and_response_paths_unit() {
        for kind in [
            NativeOutcomeKind::Help,
            NativeOutcomeKind::Exit,
            NativeOutcomeKind::Response,
            NativeOutcomeKind::ResponseWithExit,
        ] {
            let mut state = make_test_state_with_native(Some(kind));
            let invocation = resolve_invocation_ui(
                state.runtime.config.resolved(),
                &state.runtime.ui,
                &InvocationOptions::default(),
            );
            let result = run_external_command_with_help_renderer(
                &mut state,
                &["ldap".to_string(), "user".to_string()],
                &invocation,
                |text| Document::from_help_text(&format!("HELP::{text}")),
                None,
            )
            .expect("native command should dispatch");

            match kind {
                NativeOutcomeKind::Help => {
                    assert!(matches!(
                        result.body,
                        crate::app::CommandBody::Output(guide)
                            if guide
                                .document.as_ref().expect("expected document").json_value().to_string().contains("HELP::Usage: osp ldap")
                    ));
                }
                NativeOutcomeKind::Exit => {
                    assert_eq!(result.exit_code, 7);
                    assert!(matches!(result.body, crate::app::CommandBody::None));
                }
                NativeOutcomeKind::Response => {
                    assert_eq!(result.exit_code, 0);
                    assert!(!result.messages.is_empty());
                    assert!(!matches!(result.body, crate::app::CommandBody::None));
                }
                NativeOutcomeKind::ResponseWithExit => {
                    assert_eq!(result.exit_code, 3);
                    assert!(!matches!(result.body, crate::app::CommandBody::None));
                }
            }
        }
    }

    #[test]
    fn external_native_progress_renders_to_stderr_without_corrupting_final_output_unit() {
        let mut state = make_test_state_with_registry(
            NativeCommandRegistry::new().with_command(ProgressNativeCommand),
        );
        let invocation = resolve_invocation_ui(
            state.runtime.config.resolved(),
            &state.runtime.ui,
            &InvocationOptions::default(),
        );
        let mut sink = BufferedUiSink::default();

        let result = run_external_command_with_help_renderer(
            &mut state,
            &["progress".to_string()],
            &invocation,
            Document::from_help_text,
            Some(&mut sink),
        )
        .expect("native command should emit progress and return");

        assert!(sink.stdout.is_empty());
        assert!(sink.stderr.contains("creating virtual machine"));
        assert_eq!(result.exit_code, 0);
        assert!(!matches!(result.body, crate::app::CommandBody::None));
    }

    #[test]
    fn external_native_command_receives_invocation_verbosity_unit() {
        let mut state = make_test_state_with_registry(
            NativeCommandRegistry::new().with_command(VerbosityNativeCommand),
        );
        let options = InvocationOptions {
            verbose: 1,
            ..InvocationOptions::default()
        };
        let invocation =
            resolve_invocation_ui(state.runtime.config.resolved(), &state.runtime.ui, &options);

        let result = run_external_command_with_help_renderer(
            &mut state,
            &["verbosity".to_string()],
            &invocation,
            Document::from_help_text,
            None,
        )
        .expect("native command should dispatch");

        let crate::app::CommandBody::Output(output) = result.body else {
            panic!("expected structured output");
        };
        let rows = output.as_rows().expect("rows");
        assert_eq!(
            rows[0].get("ui_verbosity"),
            Some(&serde_json::json!("info"))
        );
    }

    #[test]
    fn external_bare_shellable_root_rewrites_to_help_unit() {
        let mut state = make_test_state_with_native(Some(NativeOutcomeKind::Help));
        let invocation = resolve_invocation_ui(
            state.runtime.config.resolved(),
            &state.runtime.ui,
            &InvocationOptions::default(),
        );
        let result = run_external_command_with_help_renderer(
            &mut state,
            &["ldap".to_string()],
            &invocation,
            |text| Document::from_help_text(&format!("HELP::{text}")),
            None,
        )
        .expect("bare shellable root should rewrite to help");

        assert!(matches!(
            result.body,
            crate::app::CommandBody::Output(guide)
                if guide
                    .document.as_ref().expect("expected document").json_value().to_string().contains("HELP::")
        ));
    }

    #[test]
    fn external_dispatch_matches_native_commands_case_insensitively_unit() {
        let mut state = make_test_state_with_native(Some(NativeOutcomeKind::Help));
        let invocation = resolve_invocation_ui(
            state.runtime.config.resolved(),
            &state.runtime.ui,
            &InvocationOptions::default(),
        );
        let result = run_external_command_with_help_renderer(
            &mut state,
            &["LDAP".to_string()],
            &invocation,
            |text| Document::from_help_text(&format!("HELP::{text}")),
            None,
        )
        .expect("mixed-case native command should dispatch");

        assert!(matches!(
            result.body,
            crate::app::CommandBody::Output(guide)
                if guide
                    .document.as_ref().expect("expected document").json_value().to_string().contains("HELP::Usage: osp ldap")
        ));
    }

    #[test]
    fn external_dispatch_suggests_nearby_native_command_unit() {
        let mut state = make_test_state_with_native(Some(NativeOutcomeKind::Help));
        let invocation = resolve_invocation_ui(
            state.runtime.config.resolved(),
            &state.runtime.ui,
            &InvocationOptions::default(),
        );
        let err = run_external_command_with_help_renderer(
            &mut state,
            &["lpad".to_string(), "user".to_string(), "oistes".to_string()],
            &invocation,
            Document::from_help_text,
            None,
        )
        .expect_err("misspelled command should fail with a suggestion");

        assert!(err.to_string().contains("Unknown command 'lpad'"));
        assert_eq!(
            super::super::host::find_error_in_chain::<crate::core::CommandError>(&err)
                .expect("structured suggestion")
                .action
                .as_deref(),
            Some("Did you mean osp ldap?")
        );
    }

    #[test]
    fn external_dispatch_enforces_nested_native_command_auth_unit() {
        let mut state = make_test_state_with_registry(
            NativeCommandRegistry::new().with_command(NestedAuthNativeCommand),
        );

        let invocation = resolve_invocation_ui(
            state.runtime.config.resolved(),
            &state.runtime.ui,
            &InvocationOptions::default(),
        );
        let err = run_external_command_with_help_renderer(
            &mut state,
            &[
                "ldap".to_string(),
                "--format".to_string(),
                "json".to_string(),
                "user".to_string(),
                "alice".to_string(),
            ],
            &invocation,
            Document::from_help_text,
            None,
        )
        .expect_err("nested auth should block native external dispatch");

        assert!(err.to_string().contains("command ldap user"));
        assert!(err.to_string().contains("requires authentication"));
        assert_eq!(
            super::super::host::find_error_in_chain::<crate::core::CommandError>(&err)
                .expect("structured recovery")
                .action
                .as_deref(),
            Some("authenticate, then retry")
        );
    }

    #[test]
    fn native_path_uses_clap_canonical_subcommand_for_aliases_unit() {
        let mut state = make_test_state_with_registry(
            NativeCommandRegistry::new().with_command(AliasedNestedAuthNativeCommand),
        );

        let invocation = resolve_invocation_ui(
            state.runtime.config.resolved(),
            &state.runtime.ui,
            &InvocationOptions::default(),
        );
        let err = run_external_command_with_help_renderer(
            &mut state,
            &["lookup".to_string(), "u".to_string()],
            &invocation,
            Document::from_help_text,
            None,
        )
        .expect_err("the canonical aliased child policy should deny execution");

        assert!(err.to_string().contains("command lookup user"));
        assert!(err.to_string().contains("requires authentication"));
    }

    #[test]
    fn external_native_help_requires_visibility_but_not_runnability_unit() {
        let mut state = make_test_state_with_registry(
            NativeCommandRegistry::new().with_command(NestedAuthNativeCommand),
        );
        let invocation = resolve_invocation_ui(
            state.runtime.config.resolved(),
            &state.runtime.ui,
            &InvocationOptions::default(),
        );

        let result = run_external_command_with_help_renderer(
            &mut state,
            &["ldap".to_string(), "help".to_string(), "user".to_string()],
            &invocation,
            Document::from_help_text,
            None,
        )
        .expect("visible nested help should not require its run capability");

        assert!(!matches!(result.body, crate::app::CommandBody::None));

        let denied = run_external_command_with_help_renderer(
            &mut state,
            &["ldap".to_string(), "user".to_string(), "alice".to_string()],
            &invocation,
            Document::from_help_text,
            None,
        )
        .expect_err("real execution should still require runnability");
        assert!(denied.to_string().contains("requires authentication"));
    }

    #[test]
    fn external_native_help_obeys_product_visibility_unit() {
        struct ProductAccess;
        impl crate::app::AccessPolicy for ProductAccess {
            fn visibility(&self, path: &[String]) -> crate::app::Visibility {
                if path == ["ldap", "user"] {
                    crate::app::Visibility::Hidden(Some(
                        "Use the product's operator profile".into(),
                    ))
                } else {
                    crate::app::Visibility::Shown
                }
            }
        }
        let mut state = make_test_state_with_registry(
            NativeCommandRegistry::new().with_command(NestedAuthNativeCommand),
        );
        state
            .runtime
            .set_access_policy(Some(std::sync::Arc::new(ProductAccess)));
        let invocation = resolve_invocation_ui(
            state.runtime.config.resolved(),
            &state.runtime.ui,
            &InvocationOptions::default(),
        );
        let error = run_external_command_with_help_renderer(
            &mut state,
            &["ldap".into(), "user".into(), "--help".into()],
            &invocation,
            Document::from_help_text,
            None,
        )
        .expect_err("hidden command help must remain inaccessible");
        assert_eq!(error.to_string(), "Use the product's operator profile");
    }
}
