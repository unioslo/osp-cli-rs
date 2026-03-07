use miette::{Result, miette};

use crate::app::CliCommandResult;
use crate::cli::HistoryArgs;
use crate::repl::history as repl_history;
use crate::state::AppSession;

pub(crate) fn run_history_command(_args: HistoryArgs) -> Result<CliCommandResult> {
    Err(miette!(
        "history commands are REPL-only (start the REPL with `osp`)"
    ))
}

pub(crate) fn run_history_repl_command(
    session: &mut AppSession,
    args: HistoryArgs,
    history: &osp_repl::SharedHistory,
) -> Result<CliCommandResult> {
    let output = repl_history::run_history_repl_command(session, args, history)?;
    Ok(CliCommandResult {
        exit_code: 0,
        messages: osp_ui::messages::MessageBuffer::default(),
        output: Some(output),
    })
}
