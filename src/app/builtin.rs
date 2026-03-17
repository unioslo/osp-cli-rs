//! Builtin-command execution owned by the host layer.
//!
//! CLI, inline, and REPL callers all need the same builtin command handlers,
//! but they do not share the same surrounding contracts. This module keeps the
//! per-command decisions in one place while exposing explicit entrypoints for
//! each surface so callers do not encode CLI/REPL differences with nullable
//! context.

use miette::{Result, miette};

use super::{
    AppClients, AppRuntime, AppSession, CMD_CONFIG, CMD_DOCTOR, CMD_HISTORY, CMD_PLUGINS,
    CMD_THEME, CliCommandResult, ResolvedInvocation, UiSink, UiState, ensure_builtin_visible_for,
    ensure_command_supports_dsl, run_cli_command_with_ui,
};
use crate::cli::commands::{
    config as config_cmd, doctor as doctor_cmd, history as history_cmd, intro as intro_cmd,
    plugins as plugins_cmd, theme as theme_cmd,
};
use crate::cli::{Commands, IntroArgs, ReplArgs};
use crate::repl::{self, SharedHistory};

pub(crate) fn run_cli_builtin_command_parts(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    invocation: &ResolvedInvocation,
    command: Commands,
    sink: &mut dyn UiSink,
) -> Result<i32> {
    let result = BuiltinExecutor::new(runtime, session, clients)
        .dispatch(BuiltinSurface::Cli(&invocation.ui), command)?
        .ok_or_else(|| miette!("expected builtin command"))?;
    run_cli_command_with_ui(runtime.config.resolved(), &invocation.ui, result, sink)
}

pub(crate) fn run_inline_builtin_command(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    invocation: Option<&ResolvedInvocation>,
    command: Commands,
    stages: &[String],
) -> Result<Option<CliCommandResult>> {
    if matches!(command, Commands::External(_)) {
        return Ok(None);
    }

    let spec = repl::repl_command_spec(&command);
    ensure_command_supports_dsl(&spec, stages)?;
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
) -> Result<CliCommandResult> {
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
    ) -> Result<Option<CliCommandResult>> {
        match command {
            Commands::Plugins(args) => {
                ensure_builtin_visible_for(&self.runtime.auth, CMD_PLUGINS)?;
                plugins_cmd::run_plugins_command(
                    plugins_cmd::PluginsCommandContext::from_parts(self.runtime, self.clients),
                    args,
                )
                .map(Some)
            }
            Commands::Doctor(args) => {
                ensure_builtin_visible_for(&self.runtime.auth, CMD_DOCTOR)?;
                doctor_cmd::run_doctor_command(
                    doctor_cmd::DoctorCommandContext::from_parts(
                        self.runtime,
                        self.session,
                        self.clients,
                        surface.ui(),
                    ),
                    args,
                )
                .map(Some)
            }
            Commands::Theme(args) => {
                ensure_builtin_visible_for(&self.runtime.auth, CMD_THEME)?;
                theme_cmd::run_theme_command(
                    &mut self.session.config_overrides,
                    theme_cmd::ThemeCommandContext::from_parts(self.runtime, surface.ui()),
                    args,
                )
                .map(Some)
            }
            Commands::Config(args) => {
                ensure_builtin_visible_for(&self.runtime.auth, CMD_CONFIG)?;
                config_cmd::run_config_command(
                    config_cmd::ConfigCommandContext::from_parts(
                        self.runtime,
                        self.session,
                        surface.ui(),
                    ),
                    args,
                )
                .map(Some)
            }
            Commands::History(args) => {
                ensure_builtin_visible_for(&self.runtime.auth, CMD_HISTORY)?;
                self.run_history_command(surface, args).map(Some)
            }
            Commands::Intro(args) => self.run_intro_command(surface, args).map(Some),
            Commands::Repl(args) => self.run_repl_debug_command(surface, args).map(Some),
            Commands::External(_) => Ok(None),
        }
    }

    fn run_history_command(
        &mut self,
        surface: BuiltinSurface<'_>,
        args: crate::cli::HistoryArgs,
    ) -> Result<CliCommandResult> {
        match surface {
            BuiltinSurface::Repl { history, .. } => {
                history_cmd::run_history_repl_command(self.session, args, history)
            }
            BuiltinSurface::Cli(_) | BuiltinSurface::Inline(_) => {
                history_cmd::run_history_command(args)
            }
        }
    }

    fn run_intro_command(
        &mut self,
        surface: BuiltinSurface<'_>,
        args: IntroArgs,
    ) -> Result<CliCommandResult> {
        intro_cmd::run_intro_command(
            intro_cmd::IntroCommandContext::from_parts(
                self.runtime,
                self.session,
                self.clients,
                surface.ui(),
            ),
            args,
        )
    }

    fn run_repl_debug_command(
        &mut self,
        surface: BuiltinSurface<'_>,
        args: ReplArgs,
    ) -> Result<CliCommandResult> {
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
