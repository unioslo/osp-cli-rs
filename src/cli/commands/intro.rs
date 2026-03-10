use miette::Result;

use crate::app::CliCommandResult;
use crate::cli::IntroArgs;
use crate::repl::presentation::build_repl_intro_payload;
use crate::repl::surface::ReplSurface;
use crate::repl::ReplViewContext;

pub(crate) struct IntroCommandContext<'a> {
    pub(crate) view: ReplViewContext<'a>,
    pub(crate) surface: ReplSurface,
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
