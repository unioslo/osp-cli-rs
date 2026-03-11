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

mod builtins;
mod command;
mod shell;

use crate::repl::{ReplLineResult, SharedHistory};
use miette::Result;
use std::time::Instant;

use crate::app::sink::{StdIoUiSink, UiSink};
use crate::app::{AppClients, AppRuntime, AppSession};
use crate::app::{ResolvedInvocation, resolve_invocation_ui};

use super::{ReplViewContext, completion, input};

use builtins::{ReplBuiltin, execute_repl_builtin, is_repl_bang_request, parse_repl_builtin};
use command::{
    ParsedReplDispatch, ParsedReplInvocation, finalize_repl_command, parse_repl_invocation,
    render_repl_command_output, run_repl_command,
};
use shell::{ReplShortcutPlan, classify_repl_shortcut, execute_repl_shortcut};

#[cfg(test)]
use builtins::{
    BangCommand, current_history_scope, execute_bang_command, parse_bang_command,
    strip_history_scope,
};
pub(crate) use command::repl_command_spec;
#[cfg(test)]
use command::{
    command_side_effects, config_key_change_requires_intro, parse_clap_help,
    renders_repl_inline_help,
};
#[cfg(test)]
pub(crate) use shell::apply_repl_shell_prefix;
#[cfg(test)]
pub(crate) use shell::leave_repl_shell;
#[cfg(test)]
use shell::{enter_repl_shell, handle_repl_exit_request, repl_help_for_scope};

#[derive(Debug)]
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
    Help {
        result: Box<crate::app::CliCommandResult>,
        effective: Box<ResolvedInvocation>,
        stages: Vec<String>,
    },
    Invocation(Box<ParsedReplInvocation>),
}

#[derive(Debug, Clone, Copy)]
enum ReplTimingPlan {
    Flat {
        debug_verbosity: u8,
    },
    ParseOnly {
        debug_verbosity: u8,
    },
    Invocation {
        debug_verbosity: u8,
        parse_finished: Instant,
        execute_finished: Instant,
    },
}

struct ExecutedReplLine {
    result: ReplLineResult,
    timing: ReplTimingPlan,
}

struct ReplExecutionContext<'a, 'sink> {
    runtime: &'a mut AppRuntime,
    session: &'a mut AppSession,
    clients: &'a AppClients,
    history: &'a SharedHistory,
    sink: &'sink mut dyn UiSink,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReplLinePlanKind {
    Builtin,
    Blank,
    DslHelp,
    Shortcut,
    Help,
    Invocation,
}

impl ReplLinePlan {
    #[cfg(test)]
    fn kind(&self) -> ReplLinePlanKind {
        match self {
            ReplLinePlan::Builtin { .. } => ReplLinePlanKind::Builtin,
            ReplLinePlan::Blank => ReplLinePlanKind::Blank,
            ReplLinePlan::DslHelp { .. } => ReplLinePlanKind::DslHelp,
            ReplLinePlan::Shortcut { .. } => ReplLinePlanKind::Shortcut,
            ReplLinePlan::Help { .. } => ReplLinePlanKind::Help,
            ReplLinePlan::Invocation(_) => ReplLinePlanKind::Invocation,
        }
    }
}

pub(crate) fn execute_repl_plugin_line(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    history: &SharedHistory,
    line: &str,
) -> Result<ReplLineResult> {
    let started = Instant::now();
    let mut sink = StdIoUiSink;
    match execute_repl_plugin_line_inner(runtime, session, clients, history, line, &mut sink) {
        Ok(executed) => {
            record_repl_timing(session, started, executed.timing);
            Ok(executed.result)
        }
        Err(err) => {
            if runtime.ui.debug_verbosity > 0 {
                session.record_prompt_timing(
                    runtime.ui.debug_verbosity,
                    started.elapsed(),
                    None,
                    None,
                    None,
                );
            }
            if !is_repl_bang_request(line) {
                let summary = err.to_string();
                let detail = format!("{err:#}");
                session.record_failure(line, summary, detail);
            }
            Err(err)
        }
    }
}

fn execute_repl_plugin_line_inner(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    history: &SharedHistory,
    line: &str,
    sink: &mut dyn UiSink,
) -> Result<ExecutedReplLine> {
    let plan = classify_repl_line(runtime, session, line)?;
    let parse_finished = Instant::now();
    execute_repl_line_plan(
        ReplExecutionContext {
            runtime,
            session,
            clients,
            history,
            sink,
        },
        line,
        plan,
        parse_finished,
    )
}

fn classify_repl_line(
    runtime: &AppRuntime,
    session: &AppSession,
    line: &str,
) -> Result<ReplLinePlan> {
    let raw = line.trim();
    // Builtins and bang expansion must run before full line parsing so REPL
    // control verbs keep working even when the rest of the line is not a valid
    // command invocation.
    if let Some(builtin) = parse_repl_builtin(raw)? {
        return Ok(ReplLinePlan::Builtin {
            raw: raw.to_string(),
            builtin,
        });
    }

    let parsed = input::ReplParsedLine::parse(line, runtime.config.resolved())?;
    if parsed.is_empty() {
        return Ok(ReplLinePlan::Blank);
    }
    // DSL help is a REPL-local affordance over raw stage text, so surface it
    // before command dispatch rather than forcing it through normal execution.
    if let Some(help) = completion::maybe_render_dsl_help(
        ReplViewContext::from_parts(runtime, session),
        &parsed.stages,
    ) {
        return Ok(ReplLinePlan::DslHelp { help });
    }

    let base_invocation = base_repl_invocation(runtime);
    if let Some(shortcut) = classify_repl_shortcut(runtime, session, &parsed, &base_invocation)? {
        return Ok(ReplLinePlan::Shortcut {
            parsed,
            shortcut: Box::new(shortcut),
        });
    }

    match parse_repl_invocation(runtime, session, &parsed)? {
        ParsedReplDispatch::Help {
            result,
            effective,
            stages,
        } => Ok(ReplLinePlan::Help {
            result,
            effective,
            stages,
        }),
        ParsedReplDispatch::Invocation(invocation) => Ok(ReplLinePlan::Invocation(invocation)),
    }
}

fn execute_repl_line_plan(
    context: ReplExecutionContext<'_, '_>,
    line: &str,
    plan: ReplLinePlan,
    parse_finished: Instant,
) -> Result<ExecutedReplLine> {
    let ReplExecutionContext {
        runtime,
        session,
        clients,
        history,
        sink,
    } = context;
    match plan {
        ReplLinePlan::Builtin { raw, builtin } => {
            let result = execute_repl_builtin(runtime, session, clients, history, &raw, builtin)?;
            Ok(ExecutedReplLine {
                result,
                timing: ReplTimingPlan::Flat {
                    debug_verbosity: runtime.ui.debug_verbosity,
                },
            })
        }
        ReplLinePlan::Blank => Ok(ExecutedReplLine {
            result: ReplLineResult::Continue(String::new()),
            timing: ReplTimingPlan::Flat {
                debug_verbosity: runtime.ui.debug_verbosity,
            },
        }),
        ReplLinePlan::DslHelp { help } => {
            session.sync_history_shell_context();
            Ok(ExecutedReplLine {
                result: ReplLineResult::Continue(help),
                timing: ReplTimingPlan::Flat {
                    debug_verbosity: runtime.ui.debug_verbosity,
                },
            })
        }
        ReplLinePlan::Shortcut { parsed, shortcut } => {
            let result =
                execute_repl_shortcut(runtime, session, clients, &parsed, *shortcut, line, sink)?;
            Ok(ExecutedReplLine {
                result,
                timing: ReplTimingPlan::Flat {
                    debug_verbosity: runtime.ui.debug_verbosity,
                },
            })
        }
        ReplLinePlan::Help {
            result,
            effective,
            stages,
        } => {
            let rendered = render_repl_command_output(
                runtime, session, line, &stages, *result, &effective, sink,
            )?;
            Ok(ExecutedReplLine {
                result: ReplLineResult::Continue(rendered),
                timing: ReplTimingPlan::ParseOnly {
                    debug_verbosity: effective.ui.debug_verbosity,
                },
            })
        }
        ReplLinePlan::Invocation(invocation) => {
            let output = run_repl_command(
                runtime,
                session,
                clients,
                invocation.command,
                &invocation.effective,
                history,
                invocation.cache_key.as_deref(),
            )?;
            let execute_finished = Instant::now();
            let rendered = render_repl_command_output(
                runtime,
                session,
                line,
                &invocation.stages,
                output,
                &invocation.effective,
                sink,
            )?;
            Ok(ExecutedReplLine {
                result: finalize_repl_command(
                    session,
                    rendered,
                    invocation.side_effects.restart_repl,
                    invocation.side_effects.show_intro_on_reload,
                ),
                timing: ReplTimingPlan::Invocation {
                    debug_verbosity: invocation.effective.ui.debug_verbosity,
                    parse_finished,
                    execute_finished,
                },
            })
        }
    }
}

fn record_repl_timing(session: &AppSession, started: Instant, timing: ReplTimingPlan) {
    let finished = Instant::now();
    match timing {
        ReplTimingPlan::Flat { debug_verbosity } => {
            session.record_prompt_timing(
                debug_verbosity,
                finished.saturating_duration_since(started),
                None,
                None,
                None,
            );
        }
        ReplTimingPlan::ParseOnly { debug_verbosity } => {
            session.record_prompt_timing(
                debug_verbosity,
                finished.saturating_duration_since(started),
                Some(finished.saturating_duration_since(started)),
                None,
                None,
            );
        }
        ReplTimingPlan::Invocation {
            debug_verbosity,
            parse_finished,
            execute_finished,
        } => {
            session.record_prompt_timing(
                debug_verbosity,
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
pub(crate) fn classify_repl_line_kind(
    runtime: &AppRuntime,
    session: &AppSession,
    line: &str,
) -> Result<ReplLinePlanKind> {
    Ok(classify_repl_line(runtime, session, line)?.kind())
}

#[cfg(test)]
mod tests;
