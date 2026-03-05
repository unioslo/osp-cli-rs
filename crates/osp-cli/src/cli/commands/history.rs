use miette::{Result, miette};

use crate::app::{CliCommandResult, ReplCommandOutput};
use crate::cli::HistoryArgs;
use crate::repl::history as repl_history;
use crate::state::AppState;

pub(crate) fn run_history_command(
    _state: &mut AppState,
    _args: HistoryArgs,
) -> Result<CliCommandResult> {
    Err(miette!(
        "history commands are REPL-only (start the REPL with `osp`)"
    ))
}

pub(crate) fn run_history_repl_command(
    state: &mut AppState,
    args: HistoryArgs,
    history: &osp_repl::SharedHistory,
) -> Result<ReplCommandOutput> {
    repl_history::run_history_repl_command(state, args, history)
}
