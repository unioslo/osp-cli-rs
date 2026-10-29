//! REPL line dispatch and shell-scope control flow.
//!
//! This module exists to turn an accepted REPL line into the right next action:
//! builtin handling, shell-scope transitions, ordinary command execution, or
//! inline help/DSL affordances.
//!
//! High-level flow:
//!
//! - classify builtins and bang requests before normal command parsing
//! - parse the REPL line into command and stage components
//! - handle shell shortcuts and scoped help before normal dispatch
//! - run the resolved command and package the next REPL action/result
//!
//! Contract:
//!
//! - this is where REPL-specific dispatch semantics live
//! - editor/menu behavior belongs in `repl::engine`
//! - generic non-interactive command execution should not drift into this layer

pub(crate) mod builtins;
mod command;
mod shell;

use crate::repl::{ReplResult, SharedHistory};
use miette::Result;
use std::time::Instant;

use crate::app::sink::{StdIoUiSink, UiSink};
use crate::app::{AppRuntime, AppSession};
use crate::app::{ResolvedInvocation, render_report_message, resolve_invocation_ui};
use crate::cli::invocation::scan_command_tokens;

use super::{ReplViewContext, completion, input};

use builtins::{ReplBuiltin, execute_repl_builtin, is_repl_bang_request, parse_repl_builtin};
use command::{ParsedReplDispatch, execute_repl_command_dispatch, parse_repl_invocation};
use shell::{ReplShortcutPlan, classify_repl_shortcut, execute_repl_shortcut};

#[cfg(test)]
pub(crate) use crate::app::command_plan::repl_command_spec;
#[cfg(test)]
use builtins::{BangCommand, current_history_scope, execute_bang_command, parse_bang_command};
#[cfg(test)]
use command::{
    command_side_effects, config_key_change_requires_intro, finalize_repl_command, parse_clap_help,
    render_repl_command_output, run_repl_command,
};
#[cfg(test)]
pub(crate) use shell::apply_repl_shell_prefix;
#[cfg(test)]
pub(crate) use shell::leave_repl_shell;
#[cfg(test)]
use shell::{handle_repl_exit_request, repl_help_for_scope};

enum ReplLinePlan {
    Builtin {
        raw: String,
        builtin: ReplBuiltin,
    },
    Blank,
    DslHelp {
        help: String,
    },
    Shortcut {
        parsed: input::ReplParsedLine,
        shortcut: Box<ReplShortcutPlan>,
    },
    Command(Box<ParsedReplDispatch>),
}

#[derive(Debug, Clone, Copy)]
enum ReplTimingPlan {
    Flat {
        debug_level: u8,
    },
    ParseOnly {
        debug_level: u8,
    },
    Invocation {
        debug_level: u8,
        parse_finished: Instant,
        execute_finished: Instant,
    },
}

pub(crate) struct ExecutedReplLine {
    pub(crate) result: ReplResult,
    exit_code: i32,
    timing: ReplTimingPlan,
}

impl ExecutedReplLine {
    fn flat(result: ReplResult, debug_level: u8) -> Self {
        let exit_code = match result {
            ReplResult::Exit(code) => code,
            _ => 0,
        };
        Self {
            result,
            exit_code,
            timing: ReplTimingPlan::Flat { debug_level },
        }
    }

    fn parse_only(result: ReplResult, debug_level: u8) -> Self {
        Self {
            result,
            exit_code: 0,
            timing: ReplTimingPlan::ParseOnly { debug_level },
        }
    }

    fn invocation(
        result: ReplResult,
        exit_code: i32,
        debug_level: u8,
        parse_finished: Instant,
        execute_finished: Instant,
    ) -> Self {
        Self {
            result,
            exit_code,
            timing: ReplTimingPlan::Invocation {
                debug_level,
                parse_finished,
                execute_finished,
            },
        }
    }
}

struct ReplExecutionContext<'a, 'sink> {
    state: &'a mut crate::app::AppState,
    history: &'a SharedHistory,
    sink: &'sink mut dyn UiSink,
    accepted: &'a mut bool,
}

#[cfg(test)]
pub(crate) use tests::ReplLinePlanKind;

impl ReplLinePlan {
    #[cfg(test)]
    #[cfg_attr(coverage_nightly, coverage(off))]
    fn kind(&self) -> ReplLinePlanKind {
        match self {
            ReplLinePlan::Builtin { .. } => ReplLinePlanKind::Builtin,
            ReplLinePlan::Blank => ReplLinePlanKind::Blank,
            ReplLinePlan::DslHelp { .. } => ReplLinePlanKind::DslHelp,
            ReplLinePlan::Shortcut { .. } => ReplLinePlanKind::Shortcut,
            ReplLinePlan::Command(dispatch) => match dispatch.as_ref() {
                ParsedReplDispatch::Help { .. } => ReplLinePlanKind::Help,
                ParsedReplDispatch::Invocation(_) => ReplLinePlanKind::Invocation,
            },
        }
    }
}

pub(crate) fn execute_repl_plugin_line(
    state: &mut crate::app::AppState,
    history: &SharedHistory,
    line: &str,
    sink: Option<&mut dyn UiSink>,
) -> Result<ExecutedReplLine> {
    let previous = state.session.last_output.take();
    let result = execute_recorded_repl_line(state, history, line, sink);
    if state.session.last_output.is_none() {
        state.session.last_output = match &result {
            Ok(executed) => match &executed.result {
                ReplResult::Print(text) | ReplResult::Restart { output: text, .. }
                    if !text.is_empty() =>
                {
                    Some((
                        std::sync::Arc::new(crate::app::CommandOutput::text(text.clone())),
                        state.runtime.ui.render_settings.clone(),
                    ))
                }
                _ => previous,
            },
            Err(_) => previous,
        };
    }
    result
}

fn execute_recorded_repl_line(
    state: &mut crate::app::AppState,
    history: &SharedHistory,
    line: &str,
    sink: Option<&mut dyn UiSink>,
) -> Result<ExecutedReplLine> {
    let recording = state.session.recording.clone();
    let Some((_scope, capture)) = recording.enter(line, state.runtime.config.resolved()) else {
        return execute_repl_line_inner(state, history, line, sink);
    };
    let mut stdio = StdIoUiSink;
    let mut sink = super::recording::RecordingSink {
        inner: sink.unwrap_or(&mut stdio),
        capture: capture.clone(),
    };
    let result = execute_repl_line_inner(state, history, line, Some(&mut sink));
    if let Some(capture) = &capture
        && let Ok(executed) = &result
    {
        match &executed.result {
            ReplResult::Print(output) | ReplResult::Restart { output, .. } => {
                capture.output("stdout", output)
            }
            _ => {}
        }
    }
    if let Some(error) = recording.finish(capture) {
        crate::app::sink::write_diagnostics(
            sink.inner,
            &format!("warning: {error}\n"),
            &state.runtime.ui.render_settings,
        );
    }
    result
}

fn execute_repl_line_inner(
    state: &mut crate::app::AppState,
    history: &SharedHistory,
    line: &str,
    sink: Option<&mut dyn UiSink>,
) -> Result<ExecutedReplLine> {
    crate::app::facts::set_command_scope(&mut state.runtime, &state.session.scope);
    let mut stdio = StdIoUiSink;
    let sink = sink.unwrap_or(&mut stdio);
    let mut accepted = false;
    let hint_format =
        crate::repl::command_formats::hint_format(line, state.runtime.config.resolved());
    let started = Instant::now();
    let mut resolved = if line.trim_start().starts_with('|') {
        format!("last {}", line.trim_start())
    } else {
        line.to_string()
    };
    let mut privacy_allows_history = false;
    let mut admitted = false;
    let execution = (|| {
        if let Some(command) = builtins::parse_bang_command(line)? {
            match builtins::execute_bang_command(&mut state.session, history, line, command)? {
                ReplResult::ReplaceInput(expanded) => {
                    sink.write_stderr(&format!("{expanded}\n"));
                    resolved = expanded;
                }
                result => return Ok(ExecutedReplLine::flat(result, state.runtime.ui.debug_level)),
            }
        }
        // Alias expansion needs the live execution config, which the generic
        // history backend does not own. Classify before a command can edit it.
        privacy_allows_history =
            input::ReplParsedLine::parse(&resolved, state.runtime.config.resolved()).is_ok_and(
                |parsed| {
                    if super::history_store::has_sensitive_arguments(&parsed.dispatch_tokens) {
                        return false;
                    }
                    let tokens = parsed.prefixed_tokens(&state.session.scope);
                    let Ok(scanned) = scan_command_tokens(&tokens) else {
                        return false;
                    };
                    let tokens = scanned
                        .tokens
                        .strip_prefix(&["sudo".to_owned()])
                        .unwrap_or(&scanned.tokens);
                    tokens
                        .first()
                        .and_then(|root| state.clients.native_commands().command(root))
                        .is_none_or(|native| !native.has_sensitive_args(&tokens[1..]))
                },
            );
        admitted = super::history::admits_repl_line(state, &resolved);
        if let Some(rest) = line.trim_start().strip_prefix("osp")
            && rest.starts_with(char::is_whitespace)
        {
            let rest = rest.trim_start();
            let root = rest.split_whitespace().next().unwrap_or("");
            let catalog = crate::app::authorized_command_catalog_for(
                &state.runtime.auth,
                &state.clients,
                state.clients.native_commands().catalog(),
            );
            let surface = super::surface::build_repl_surface(
                ReplViewContext::from_parts(&state.runtime, &state.session),
                &catalog,
            );
            if surface.root_words.iter().any(|word| word == root) {
                return Ok(ExecutedReplLine::flat(
                    ReplResult::ReplaceInput(rest.to_string()),
                    state.runtime.ui.debug_level,
                ));
            }
        }
        if !matches!(resolved.trim(), "next" | "prev") {
            state.session.native_context.clear_pagination();
        }
        let plan = classify_repl_line(
            &state.runtime,
            &state.session,
            state.clients.native_commands(),
            &resolved,
        )?;
        let parse_finished = Instant::now();
        execute_repl_line_plan(
            ReplExecutionContext {
                state,
                history,
                sink,
                accepted: &mut accepted,
            },
            &resolved,
            plan,
            parse_finished,
        )
    })();
    if let Some(expanded) = state
        .session
        .native_context
        .values
        .write()
        .ok()
        .and_then(|mut values| values.remove("context.executed"))
        && let Ok(expansion) = serde_json::from_str::<crate::context::HistoryExpansion>(&expanded)
        && let Some(line) = expansion.apply(&resolved, state.runtime.config.resolved())
    {
        resolved = line;
    }
    let exit_code = execution
        .as_ref()
        .map_or_else(crate::app::classify_exit_code, |executed| {
            executed.exit_code
        });
    if exit_code == crate::app::EXIT_CODE_USAGE {
        accepted = false;
    }
    if let Err(err) = history
        .with(|store| store.finalize_execution(line, &resolved, admitted && privacy_allows_history))
        .map_err(anyhow::Error::msg)
        .and_then(|result| result)
    {
        sink.write_stderr(&format!("failed to update REPL history: {err}\n"));
    }
    if accepted && privacy_allows_history && !is_repl_bang_request(&resolved) {
        let _ = history.with(|store| store.remember_command(&resolved));
    }
    for command in state.session.repl_feedback.take_history() {
        let command = state
            .runtime
            .ui
            .render_settings
            .runtime
            .command_scope
            .command(&command);
        if let Err(error) = history
            .with(|store| {
                store.stage_command_line(&command)?;
                store.finalize_execution(&command, &command, true)
            })
            .map_err(anyhow::Error::msg)
            .and_then(|result| result)
        {
            sink.write_stderr(&format!("failed to update REPL history: {error}\n"));
        }
    }

    state.session.finish_repl_line();
    let refresh_completion = state
        .session
        .native_context
        .take_completion_refresh_request()
        | state
            .clients
            .native_commands()
            .take_completion_refresh_request();
    match execution {
        Ok(mut executed) => {
            if executed.exit_code == 0
                && let Some(format) = hint_format
                && state
                    .session
                    .last_output
                    .as_ref()
                    .is_some_and(|(output, settings)| {
                        !matches!(
                            output.output_format(settings),
                            crate::core::OutputFormat::Json
                                | crate::core::OutputFormat::Csv
                                | crate::core::OutputFormat::Tsv
                        )
                    })
            {
                state.session.command_formats.maybe_hint(format, sink);
            }
            if refresh_completion {
                executed.result = match executed.result {
                    ReplResult::Continue => ReplResult::Restart {
                        output: String::new(),
                        reload: crate::repl::ReplReloadKind::Completion,
                    },
                    ReplResult::Print(output) => ReplResult::Restart {
                        output,
                        reload: crate::repl::ReplReloadKind::Completion,
                    },
                    result @ ReplResult::ReplaceInput(_) => {
                        state.session.native_context.request_completion_refresh();
                        result
                    }
                    result => result,
                };
            }
            let debug = match executed.timing {
                ReplTimingPlan::Flat { debug_level }
                | ReplTimingPlan::ParseOnly { debug_level }
                | ReplTimingPlan::Invocation { debug_level, .. } => debug_level,
            };
            let _logging = crate::app::logging::override_stderr_level(debug);
            tracing::info!(target: "osp", exit_code = executed.exit_code,
                elapsed_ms = started.elapsed().as_millis(), "finished");
            record_repl_timing(&state.session, started, executed.timing);
            Ok(executed)
        }
        Err(err) => {
            let fallback_ui =
                input::ReplParsedLine::parse(&resolved, state.runtime.config.resolved())
                    .ok()
                    .and_then(|parsed| scan_command_tokens(&parsed.dispatch_tokens).ok())
                    .map(|scanned| {
                        resolve_invocation_ui(
                            state.runtime.config.resolved(),
                            &state.runtime.ui,
                            &scanned.invocation,
                        )
                        .ui
                    })
                    .unwrap_or_else(|| state.runtime.ui.clone());
            let settings = crate::app::host::error_display_settings(&err)
                .cloned()
                .unwrap_or_else(|| crate::app::host::ErrorDisplaySettings::from(&fallback_ui));
            let err = std::sync::Arc::new(crate::app::host::report_with_invocation(
                err,
                settings.clone(),
            ));
            let ui = &settings;
            if ui.debug_level > 0 {
                state.session.record_prompt_timing(
                    ui.debug_level,
                    started.elapsed(),
                    None,
                    None,
                    None,
                );
            }
            let _logging = crate::app::logging::override_stderr_level(ui.debug_level);
            let plain_settings = ui.render_settings.plain_copy_settings();
            let remember = !is_repl_bang_request(&resolved)
                && !is_doctor_request(&resolved, state.runtime.config.resolved());
            if remember {
                state
                    .session
                    .record_failure_report(&resolved, err.clone(), plain_settings);
            }
            let evidence = crate::app::host::report_evidence(&err, ui.verbosity);
            crate::app::sink::write_diagnostics(sink, &evidence.0, &ui.render_settings);
            let hints = crate::app::host::report_hint_commands(
                &err,
                ui.verbosity,
                Some((&resolved, &ui.render_settings.runtime.command_scope)),
            );
            let hint_count = hints.as_ref().map_or(0, Vec::len);
            if let Some(hints) = hints {
                state.session.repl_feedback.remember_hints(hints);
            }
            let hint_action = crate::repl::lifecycle::hint_action(
                state.runtime.config.resolved(),
                ui.render_settings.requested_format(),
                hint_count,
            );
            let rendered = crate::app::render_process_error(
                &err,
                &ui.render_settings,
                ui.verbosity,
                hint_action.as_deref(),
            );
            if ui.render_settings.requested_format() == crate::core::output::OutputFormat::Json {
                sink.write_stdout(&rendered);
                if ui.debug_level >= 2 {
                    sink.write_stderr(&format!(
                        "{}\n",
                        render_report_message(&err, ui.verbosity, &ui.render_settings).trim_end()
                    ));
                }
            } else {
                crate::app::sink::write_diagnostics(sink, &rendered, &ui.render_settings);
            }
            crate::app::sink::write_diagnostics(sink, &evidence.1, &ui.render_settings);
            if ui.debug_level >= 2
                && let Some(backtrace) =
                    crate::app::host::render_backtrace(&err, &ui.render_settings)
            {
                sink.write_stderr(&format!("{backtrace}\n"));
            }
            let exit_code = crate::app::classify_exit_code(&err);
            tracing::info!(target: "osp", exit_code, elapsed_ms = started.elapsed().as_millis(), "finished");
            let result = if refresh_completion {
                ReplResult::Restart {
                    output: String::new(),
                    reload: crate::repl::ReplReloadKind::Completion,
                }
            } else {
                ReplResult::Continue
            };
            let mut executed = ExecutedReplLine::flat(result, ui.debug_level);
            executed.exit_code = exit_code;
            Ok(executed)
        }
    }
}

fn is_doctor_request(line: &str, config: &crate::config::ResolvedConfig) -> bool {
    input::ReplParsedLine::parse(line, config)
        .ok()
        .and_then(|parsed| scan_command_tokens(&parsed.dispatch_tokens).ok())
        .and_then(|scanned| scanned.tokens.into_iter().next())
        .is_some_and(|command| command == crate::app::CMD_DOCTOR)
}

fn classify_repl_line(
    runtime: &AppRuntime,
    session: &AppSession,
    native_commands: &crate::NativeCommandRegistry,
    line: &str,
) -> Result<ReplLinePlan> {
    // 1. Classify builtins and blank lines before any normal parsing.
    let raw = line.trim();
    if let Some(plan) = classify_builtin_or_blank(raw)? {
        return Ok(plan);
    }

    // 2. Parse the line into REPL-local command/stage shape.
    let mut parsed = parse_repl_dispatch_line(runtime, line)?;
    let scoped = scan_command_tokens(&parsed.prefixed_tokens(&session.scope))?;
    let tokens = scoped
        .tokens
        .strip_prefix(&["sudo".to_owned()])
        .unwrap_or(&scoped.tokens);
    if let Some(native) = tokens
        .first()
        .and_then(|root| native_commands.command(root))
    {
        let segments = crate::dsl::parse::lexer::split_pipeline(line)
            .map_err(|error| miette::miette!("{error}"))?;
        if let Some(command) = segments.first() {
            let prepared = native
                .prepare_repl_command(&command.raw)
                .map_err(|error| miette::miette!("{error}"))?;
            if prepared != command.raw {
                let mut rewritten = line.to_owned();
                rewritten.replace_range(command.span.start..command.span.end, &prepared);
                parsed = parse_repl_dispatch_line(runtime, &rewritten)?;
            }
        }
    }

    let scanned = scan_command_tokens(&parsed.dispatch_tokens)?;
    if crate::cli::is_help_all(&scanned.tokens) {
        return Ok(ReplLinePlan::Builtin {
            raw: line.to_string(),
            builtin: ReplBuiltin::HelpAll,
        });
    }

    // 3. Handle REPL-local affordances before normal command dispatch.
    if let Some(plan) = classify_repl_local_plan(runtime, session, &parsed)? {
        return Ok(plan);
    }

    // Invocation flags and aliases may precede last, which remains global even
    // inside a product shell. Its cached output uses normal parsing/rendering.
    if scan_command_tokens(&parsed.dispatch_tokens)?
        .tokens
        .first()
        .map(String::as_str)
        == Some("last")
    {
        return Ok(ReplLinePlan::Builtin {
            raw: line.to_string(),
            builtin: ReplBuiltin::Last,
        });
    }

    // 4. Resolve the remaining line as ordinary command/help dispatch.
    classify_repl_command_plan(runtime, session, native_commands, parsed)
}

fn classify_builtin_or_blank(raw: &str) -> Result<Option<ReplLinePlan>> {
    // Builtins and bang expansion must run before full line parsing so REPL
    // control verbs keep working even when the rest of the line is not a valid
    // command invocation.
    if let Some(builtin) = parse_repl_builtin(raw)? {
        return Ok(Some(ReplLinePlan::Builtin {
            raw: raw.to_string(),
            builtin,
        }));
    }

    if raw.is_empty() {
        return Ok(Some(ReplLinePlan::Blank));
    }

    Ok(None)
}

fn parse_repl_dispatch_line(runtime: &AppRuntime, line: &str) -> Result<input::ReplParsedLine> {
    input::ReplParsedLine::parse(line, runtime.config.resolved())
}

fn classify_repl_local_plan(
    runtime: &AppRuntime,
    session: &AppSession,
    parsed: &input::ReplParsedLine,
) -> Result<Option<ReplLinePlan>> {
    // DSL help is a REPL-local affordance over raw stage text, so surface it
    // before command dispatch rather than forcing it through normal execution.
    if let Some(help) = completion::maybe_render_dsl_help(
        ReplViewContext::from_parts(runtime, session),
        &parsed.stages,
    ) {
        return Ok(Some(ReplLinePlan::DslHelp { help }));
    }

    let base_invocation = base_repl_invocation(runtime);
    if let Some(shortcut) = classify_repl_shortcut(runtime, session, parsed, &base_invocation)? {
        return Ok(Some(ReplLinePlan::Shortcut {
            parsed: parsed.clone(),
            shortcut: Box::new(shortcut),
        }));
    }

    Ok(None)
}

fn classify_repl_command_plan(
    runtime: &AppRuntime,
    session: &AppSession,
    native_commands: &crate::NativeCommandRegistry,
    parsed: input::ReplParsedLine,
) -> Result<ReplLinePlan> {
    Ok(ReplLinePlan::Command(Box::new(parse_repl_invocation(
        runtime,
        session,
        native_commands,
        &parsed,
    )?)))
}

fn execute_repl_line_plan(
    context: ReplExecutionContext<'_, '_>,
    line: &str,
    plan: ReplLinePlan,
    parse_finished: Instant,
) -> Result<ExecutedReplLine> {
    let ReplExecutionContext {
        state,
        history,
        sink,
        accepted,
    } = context;
    match plan {
        ReplLinePlan::Builtin { raw, builtin } => {
            *accepted = true;
            let debug_level = input::ReplParsedLine::parse(&raw, state.runtime.config.resolved())
                .ok()
                .and_then(|parsed| scan_command_tokens(&parsed.dispatch_tokens).ok())
                .map(|scanned| {
                    resolve_invocation_ui(
                        state.runtime.config.resolved(),
                        &state.runtime.ui,
                        &scanned.invocation,
                    )
                    .ui
                    .debug_level
                })
                .unwrap_or(state.runtime.ui.debug_level);
            let _logging = crate::app::logging::override_stderr_level(debug_level);
            let result = execute_repl_builtin(state, history, &raw, builtin, sink)?;
            Ok(ExecutedReplLine::flat(result, debug_level))
        }
        ReplLinePlan::Blank => Ok(ExecutedReplLine::flat(
            ReplResult::Continue,
            state.runtime.ui.debug_level,
        )),
        ReplLinePlan::DslHelp { help } => {
            *accepted = true;
            Ok(ExecutedReplLine::flat(
                ReplResult::Print(help),
                state.runtime.ui.debug_level,
            ))
        }
        ReplLinePlan::Shortcut { parsed, shortcut } => {
            *accepted = true;
            let invocation = match shortcut.as_ref() {
                ReplShortcutPlan::Help { invocation }
                | ReplShortcutPlan::CurrentShellHelp { invocation, .. }
                | ReplShortcutPlan::ShellEntry { invocation, .. } => invocation,
            };
            let debug_level = invocation.ui.debug_level;
            let error_display = crate::app::host::ErrorDisplaySettings::from(&invocation.ui);
            let _logging = crate::app::logging::override_stderr_level(debug_level);
            let result = execute_repl_shortcut(state, &parsed, *shortcut, line, sink)
                .map_err(|err| crate::app::host::report_with_invocation(err, error_display))?;
            Ok(ExecutedReplLine::flat(result, debug_level))
        }
        ReplLinePlan::Command(dispatch) => {
            let ui = match dispatch.as_ref() {
                ParsedReplDispatch::Help { effective, .. } => &effective.ui,
                ParsedReplDispatch::Invocation(invocation) => &invocation.effective.ui,
            };
            let _logging = crate::app::logging::override_stderr_level(ui.debug_level);
            let error_display = crate::app::host::ErrorDisplaySettings::from(ui);
            execute_repl_command_dispatch(
                ReplExecutionContext {
                    state,
                    history,
                    sink,
                    accepted,
                },
                line,
                *dispatch,
                parse_finished,
            )
            .map_err(|err| crate::app::host::report_with_invocation(err, error_display))
        }
    }
}

fn record_repl_timing(session: &AppSession, started: Instant, timing: ReplTimingPlan) {
    let finished = Instant::now();
    match timing {
        ReplTimingPlan::Flat { debug_level } => {
            session.record_prompt_timing(
                debug_level,
                finished.saturating_duration_since(started),
                None,
                None,
                None,
            );
        }
        ReplTimingPlan::ParseOnly { debug_level } => {
            session.record_prompt_timing(
                debug_level,
                finished.saturating_duration_since(started),
                Some(finished.saturating_duration_since(started)),
                None,
                None,
            );
        }
        ReplTimingPlan::Invocation {
            debug_level,
            parse_finished,
            execute_finished,
        } => {
            session.record_prompt_timing(
                debug_level,
                finished.saturating_duration_since(started),
                Some(parse_finished.saturating_duration_since(started)),
                Some(execute_finished.saturating_duration_since(parse_finished)),
                Some(finished.saturating_duration_since(execute_finished)),
            );
        }
    }
}

fn base_repl_invocation(runtime: &AppRuntime) -> ResolvedInvocation {
    // Shortcut/help rendering starts from the ambient REPL UI state, not from
    // per-command flags, because no concrete command has been resolved yet.
    resolve_invocation_ui(runtime.config.resolved(), &runtime.ui, &Default::default())
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
pub(crate) fn classify_repl_line_kind(
    runtime: &AppRuntime,
    session: &AppSession,
    line: &str,
) -> Result<ReplLinePlanKind> {
    Ok(classify_repl_line(
        runtime,
        session,
        &crate::NativeCommandRegistry::default(),
        line,
    )?
    .kind())
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests;
