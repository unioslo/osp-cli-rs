use miette::{Result, miette};

use crate::app::AppSession;
use crate::app::CliCommandResult;
use crate::cli::HistoryArgs;
use crate::repl::history as repl_history;

pub(crate) fn run_history_command(_args: HistoryArgs) -> Result<CliCommandResult> {
    Err(miette!(
        "history commands are REPL-only (start the REPL with `osp`)"
    ))
}

pub(crate) fn run_history_repl_command(
    session: &mut AppSession,
    args: HistoryArgs,
    history: &crate::repl::SharedHistory,
) -> Result<CliCommandResult> {
    let output = repl_history::run_history_repl_command(session, args, history)?;
    Ok(CliCommandResult {
        exit_code: 0,
        messages: crate::ui::messages::MessageBuffer::default(),
        output: Some(output),
        stderr_text: None,
        failure_report: None,
    })
}

#[cfg(test)]
mod tests {
    use super::{run_history_command, run_history_repl_command};
    use crate::app::AppSession;
    use crate::app::{CliCommandResult, ReplCommandOutput};
    use crate::cli::{HistoryArgs, HistoryCommands};
    use crate::core::row::Row;
    use crate::repl::HistoryConfig;

    fn extract_output_rows(result: CliCommandResult) -> Option<Vec<Row>> {
        let output = match result.output? {
            ReplCommandOutput::Output { output, .. } => output,
            ReplCommandOutput::Guide(_)
            | ReplCommandOutput::Document(_)
            | ReplCommandOutput::Text(_) => return None,
        };
        output.into_rows()
    }

    #[test]
    fn history_command_is_repl_only_unit() {
        let err = run_history_command(HistoryArgs {
            command: HistoryCommands::List,
        })
        .expect_err("history command should be rejected outside the repl");

        assert!(err.to_string().contains("history commands are REPL-only"));
    }

    #[test]
    fn history_repl_command_wraps_repl_output_for_cli_unit() {
        let temp_dir = make_temp_dir("osp-cli-history-wrapper");
        let history = crate::repl::SharedHistory::new(
            HistoryConfig {
                path: Some(temp_dir.join("history.jsonl")),
                max_entries: 32,
                enabled: true,
                dedupe: true,
                profile_scoped: false,
                exclude_patterns: Vec::new(),
                profile: None,
                terminal: None,
                shell_context: crate::repl::HistoryShellContext::default(),
            }
            .normalized(),
        )
        .expect("history should initialize");
        history
            .save_command_line("config show")
            .expect("history seed should save");

        let mut session = AppSession::with_cache_limit(8);
        let result = run_history_repl_command(
            &mut session,
            HistoryArgs {
                command: HistoryCommands::List,
            },
            &history,
        )
        .expect("history list should succeed");

        assert_eq!(result.exit_code, 0);
        let rows = extract_output_rows(result).expect("history list should emit rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0]["command"],
            serde_json::Value::String("config show".into())
        );
    }

    #[test]
    fn extract_output_rows_returns_none_for_text_results_unit() {
        assert!(extract_output_rows(CliCommandResult::text("hello")).is_none());
    }

    fn make_temp_dir(prefix: &str) -> std::path::PathBuf {
        let mut dir = std::env::temp_dir();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be valid")
            .as_nanos();
        dir.push(format!("{prefix}-{nonce}"));
        std::fs::create_dir_all(&dir).expect("temp dir should be created");
        dir
    }
}
