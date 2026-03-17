use miette::Result;

use crate::app::CliCommandResult;
use crate::app::{AppClients, AppRuntime, AppSession, authorized_command_catalog_for};
use crate::cli::IntroArgs;
use crate::repl::ReplViewContext;
use crate::repl::presentation::build_repl_intro_payload;
use crate::repl::surface::ReplSurface;

pub(crate) struct IntroCommandContext<'a> {
    pub(crate) view: ReplViewContext<'a>,
    pub(crate) surface: ReplSurface,
}

impl<'a> IntroCommandContext<'a> {
    pub(crate) fn from_parts(
        runtime: &'a AppRuntime,
        session: &'a AppSession,
        clients: &'a AppClients,
        ui: &'a crate::app::UiState,
    ) -> Self {
        let view = ReplViewContext {
            config: runtime.config.resolved(),
            ui,
            auth: &runtime.auth,
            themes: &runtime.themes,
            scope: &session.scope,
        };
        let surface = authorized_command_catalog_for(&runtime.auth, clients)
            .ok()
            .map(|catalog| crate::repl::surface::build_repl_surface(view, &catalog))
            .unwrap_or_else(|| crate::repl::surface::ReplSurface {
                root_words: Vec::new(),
                intro_commands: Vec::new(),
                specs: Vec::new(),
                aliases: Vec::new(),
                overview_entries: Vec::new(),
            });

        Self { view, surface }
    }
}

pub(crate) fn run_intro_command(
    context: IntroCommandContext<'_>,
    _args: IntroArgs,
) -> Result<CliCommandResult> {
    Ok(CliCommandResult::guide(build_repl_intro_payload(
        context.view,
        &context.surface,
        None,
    )))
}
