//! Builtin-command execution owned by the host layer.
//!
//! CLI, inline, and REPL callers all need the same builtin command handlers,
//! but they do not share the same surrounding contracts. This module keeps the
//! per-command decisions in one place while exposing explicit entrypoints for
//! each surface so callers do not encode CLI/REPL differences with nullable
//! context.

use miette::{Result, miette};

use super::{
    AppClients, AppRuntime, AppSession, CommandOutput, ResolvedInvocation, UiSink, UiState,
    run_cli_command,
};
use crate::cli::commands::{
    config as config_cmd, context as context_cmd, doctor as doctor_cmd, history as history_cmd,
    intro as intro_cmd, plugins as plugins_cmd, theme as theme_cmd,
};
use crate::cli::{Commands, IntroArgs, ReplArgs};
use crate::repl::{self, SharedHistory};

pub(crate) fn run_cli_builtin_command_parts(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    invocation: &ResolvedInvocation,
    command: Commands,
    stages: &[String],
    sink: &mut dyn UiSink,
) -> Result<i32> {
    super::command_plan::validate_command_pipeline(&command, stages)?;
    let result = BuiltinExecutor::new(runtime, session, clients)
        .dispatch(BuiltinSurface::Cli(&invocation.ui), command)?
        .ok_or_else(|| miette!("expected builtin command"))?;
    let result = super::command_output::apply_stages_to_cli_result(result, stages)?;
    run_cli_command(runtime.config.resolved(), &invocation.ui, &result, sink)
}

pub(crate) fn run_inline_builtin_command(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    invocation: Option<&ResolvedInvocation>,
    command: Commands,
    stages: &[String],
) -> Result<Option<CommandOutput>> {
    if matches!(command, Commands::External(_)) {
        return Ok(None);
    }

    super::command_plan::validate_command_pipeline(&command, stages)?;
    let ui = invocation
        .map(|invocation| invocation.ui.clone())
        .unwrap_or_else(|| runtime.ui.clone());
    BuiltinExecutor::new(runtime, session, clients)
        .dispatch(BuiltinSurface::Inline(Box::new(ui)), command)
}

pub(crate) fn run_repl_builtin_command(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    history: &SharedHistory,
    invocation: &ResolvedInvocation,
    command: Commands,
) -> Result<CommandOutput> {
    BuiltinExecutor::new(runtime, session, clients)
        .dispatch(
            BuiltinSurface::Repl {
                history,
                ui: &invocation.ui,
            },
            command,
        )?
        .ok_or_else(|| miette!("expected builtin command"))
}

struct BuiltinExecutor<'a> {
    runtime: &'a mut AppRuntime,
    session: &'a mut AppSession,
    clients: &'a AppClients,
}

impl<'a> BuiltinExecutor<'a> {
    fn new(
        runtime: &'a mut AppRuntime,
        session: &'a mut AppSession,
        clients: &'a AppClients,
    ) -> Self {
        Self {
            runtime,
            session,
            clients,
        }
    }

    fn dispatch(
        &mut self,
        surface: BuiltinSurface<'_>,
        command: Commands,
    ) -> Result<Option<CommandOutput>> {
        command.validate_dispatch_surface(matches!(surface, BuiltinSurface::Repl { .. }))?;
        let command_path = surface.ui().render_settings.command_path.clone();
        let result = match command {
            Commands::Plugins(args) => {
                plugins_cmd::run_plugins_command(self.runtime, self.clients, args).map(Some)
            }
            Commands::Doctor(args) => doctor_cmd::run_doctor_command(
                self.runtime,
                self.session,
                self.clients,
                surface.ui(),
                args,
            )
            .map(Some),
            Commands::Theme(args) => {
                theme_cmd::run_theme_command(self.session, self.runtime, surface.ui(), args)
                    .map(Some)
            }
            Commands::Config(args) => {
                config_cmd::run_config_command(self.runtime, self.session, surface.ui(), args)
                    .map(Some)
            }
            Commands::Alias(args) => {
                config_cmd::run_alias_command(self.runtime, self.session, surface.ui(), args)
                    .map(Some)
            }
            Commands::Notifications(_) => {
                crate::repl::notices::request_foreground_bell(
                    self.runtime.config.resolved(),
                    surface.ui().render_settings.requested_format(),
                );
                Ok(Some(CommandOutput::exit(0)))
            }
            Commands::History(args) => self.run_history_command(surface, args).map(Some),
            Commands::Context(args) => {
                context_cmd::run_context_command(self.session, self.runtime, args).map(Some)
            }
            Commands::Completions(_) => unreachable!("validated dispatch surface"),
            Commands::Intro(args) => self.run_intro_command(surface, args).map(Some),
            Commands::Repl(args) => self.run_repl_debug_command(surface, args).map(Some),
            Commands::External(_) => Ok(None),
        }?;
        Ok(result.map(|mut output| {
            output.command_path = command_path;
            output
        }))
    }

    fn run_history_command(
        &mut self,
        surface: BuiltinSurface<'_>,
        args: crate::cli::HistoryArgs,
    ) -> Result<CommandOutput> {
        match surface {
            BuiltinSurface::Repl { history, .. } => {
                history_cmd::run_history_repl_command(self.session, args, history)
            }
            BuiltinSurface::Cli(_) | BuiltinSurface::Inline(_) => {
                let history = crate::repl::SharedHistory::new(
                    crate::repl::history::build_history_config(self.runtime, self.session),
                );
                history_cmd::run_history_repl_command(self.session, args, &history)
            }
        }
    }

    fn run_intro_command(
        &mut self,
        surface: BuiltinSurface<'_>,
        args: IntroArgs,
    ) -> Result<CommandOutput> {
        intro_cmd::run_intro_command(self.runtime, self.session, self.clients, surface.ui(), args)
    }

    fn run_repl_debug_command(
        &mut self,
        surface: BuiltinSurface<'_>,
        args: ReplArgs,
    ) -> Result<CommandOutput> {
        match surface {
            BuiltinSurface::Repl { .. } => {
                Err(miette!("`repl` debug commands are not available in REPL"))
            }
            BuiltinSurface::Cli(_) | BuiltinSurface::Inline(_) => {
                repl::run_repl_debug_command_for(self.runtime, self.session, self.clients, args)
            }
        }
    }
}

enum BuiltinSurface<'a> {
    Cli(&'a UiState),
    Inline(Box<UiState>),
    Repl {
        history: &'a SharedHistory,
        ui: &'a UiState,
    },
}

impl BuiltinSurface<'_> {
    fn ui(&self) -> &UiState {
        match self {
            BuiltinSurface::Cli(ui) => ui,
            BuiltinSurface::Inline(ui) => ui,
            BuiltinSurface::Repl { ui, .. } => ui,
        }
    }
}
