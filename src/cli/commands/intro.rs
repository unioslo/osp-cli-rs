use miette::Result;

use crate::app::CliCommandResult;
use crate::cli::IntroArgs;
use crate::core::command_def::CommandDef;
use crate::repl::ReplViewContext;
use crate::repl::presentation::build_repl_intro_payload;
use crate::repl::surface::ReplSurface;

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

#[allow(dead_code)]
pub(crate) fn intro_command_def(sort_key: impl Into<String>) -> CommandDef {
    CommandDef::new("intro")
        .about("Show the REPL intro")
        .sort(sort_key)
        .hidden()
}
