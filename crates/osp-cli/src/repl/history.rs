use miette::{Result, miette};
use osp_completion::CommandSpec;
use osp_config::{ConfigValue, DEFAULT_REPL_HISTORY_MAX_ENTRIES, ResolvedConfig, RuntimeDefaults};
use osp_core::row::Row;
use osp_repl::{HistoryConfig, HistoryEntry, SharedHistory};
use osp_ui::theme::DEFAULT_THEME_NAME;
use std::path::PathBuf;

use crate::cli::{HistoryArgs, HistoryCommands, HistoryPruneArgs};
use crate::state::AppState;

use crate::app::{CMD_HISTORY, CMD_LIST, DEFAULT_REPL_PROMPT, ReplCommandOutput, config_usize};
use crate::rows::output::rows_to_output_result;

const DEFAULT_REPL_HISTORY_EXCLUDES: [&str; 4] = ["exit", "quit", "help", "history list"];

pub(crate) fn history_command_spec() -> CommandSpec {
    CommandSpec {
        name: CMD_HISTORY.to_string(),
        tooltip: Some("Inspect or prune REPL history".to_string()),
        subcommands: vec![
            CommandSpec {
                name: CMD_LIST.to_string(),
                tooltip: Some("List recent history".to_string()),
                ..CommandSpec::default()
            },
            CommandSpec {
                name: "prune".to_string(),
                tooltip: Some("Keep last N entries".to_string()),
                ..CommandSpec::default()
            },
            CommandSpec {
                name: "clear".to_string(),
                tooltip: Some("Clear history".to_string()),
                ..CommandSpec::default()
            },
        ],
        ..CommandSpec::default()
    }
}

pub(crate) fn build_history_config(state: &mut AppState) -> HistoryConfig {
    let config = state.config.resolved();
    let history_max_entries = config_usize(
        config,
        "repl.history.max_entries",
        DEFAULT_REPL_HISTORY_MAX_ENTRIES as usize,
    );
    let history_enabled =
        config.get_bool("repl.history.enabled").unwrap_or(true) && history_max_entries > 0;
    let history_path = config
        .get_string("repl.history.path")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let defaults =
                RuntimeDefaults::from_process_env(DEFAULT_THEME_NAME, DEFAULT_REPL_PROMPT);
            PathBuf::from(defaults.repl_history_path)
        });
    let history_dedupe = config.get_bool("repl.history.dedupe").unwrap_or(true);
    let history_profile_scoped = config
        .get_bool("repl.history.profile_scoped")
        .unwrap_or(true);
    let history_exclude = repl_history_exclude_patterns(config);
    let history_shell = state.repl.history_shell.clone();
    state.sync_history_shell_context();

    HistoryConfig::new(
        Some(history_path),
        history_max_entries,
        history_enabled,
        history_dedupe,
        history_profile_scoped,
        history_exclude,
        Some(config.active_profile().to_string()),
        Some(
            state
                .context
                .terminal_kind()
                .as_config_terminal()
                .to_string(),
        ),
        history_shell,
    )
}

pub(crate) fn repl_history_enabled(config: &ResolvedConfig) -> bool {
    let max_entries = config_usize(
        config,
        "repl.history.max_entries",
        DEFAULT_REPL_HISTORY_MAX_ENTRIES as usize,
    );
    config.get_bool("repl.history.enabled").unwrap_or(true) && max_entries > 0
}

pub(crate) fn run_history_repl_command(
    _state: &mut AppState,
    args: HistoryArgs,
    history: &SharedHistory,
) -> Result<ReplCommandOutput> {
    if !history.enabled() {
        return Ok(ReplCommandOutput::Text(
            "History is disabled.\n".to_string(),
        ));
    }

    match args.command {
        HistoryCommands::List => {
            let rows = history_entries_rows(history.list_entries());
            Ok(ReplCommandOutput::Output {
                output: rows_to_output_result(rows),
                format_hint: None,
            })
        }
        HistoryCommands::Prune(HistoryPruneArgs { keep }) => {
            let removed = history
                .prune(keep)
                .map_err(|err| miette!(err.to_string()))?;
            Ok(ReplCommandOutput::Text(format!(
                "Removed {removed} entr{}.\n",
                if removed == 1 { "y" } else { "ies" }
            )))
        }
        HistoryCommands::Clear => {
            history
                .clear_scoped()
                .map_err(|err| miette!(err.to_string()))?;
            Ok(ReplCommandOutput::Text("History cleared.\n".to_string()))
        }
    }
}

fn history_entries_rows(entries: Vec<HistoryEntry>) -> Vec<Row> {
    let mut rows = Vec::with_capacity(entries.len());
    for entry in entries {
        let timestamp = entry
            .timestamp_ms
            .map_or(serde_json::Value::Null, |ms| ms.into());
        rows.push(crate::row! {
            "id" => entry.id,
            "timestamp_ms" => timestamp,
            "command" => entry.command,
        });
    }
    rows
}

fn config_string_list(config: &ResolvedConfig, key: &str) -> Vec<String> {
    match config.get(key).map(ConfigValue::reveal) {
        Some(ConfigValue::List(values)) => values
            .iter()
            .filter_map(|value| match value {
                ConfigValue::String(value) => Some(value.clone()),
                ConfigValue::Secret(secret) => match secret.expose() {
                    ConfigValue::String(value) => Some(value.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect(),
        Some(ConfigValue::String(value)) => vec![value.clone()],
        _ => Vec::new(),
    }
}

fn repl_history_exclude_patterns(config: &ResolvedConfig) -> Vec<String> {
    let mut patterns = config_string_list(config, "repl.history.exclude");
    for default in DEFAULT_REPL_HISTORY_EXCLUDES {
        if patterns.iter().any(|pattern| pattern == default) {
            continue;
        }
        patterns.push(default.to_string());
    }
    patterns
}

#[cfg(test)]
mod tests {
    use super::repl_history_exclude_patterns;
    use osp_config::{ConfigLayer, ConfigResolver, ResolveOptions};

    #[test]
    fn history_exclude_patterns_include_repl_defaults() {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        let resolved = resolver
            .resolve(ResolveOptions::default())
            .expect("config should resolve");

        let patterns = repl_history_exclude_patterns(&resolved);

        assert!(patterns.contains(&"exit".to_string()));
        assert!(patterns.contains(&"quit".to_string()));
        assert!(patterns.contains(&"help".to_string()));
        assert!(patterns.contains(&"history list".to_string()));
    }

    #[test]
    fn history_exclude_patterns_do_not_duplicate_defaults() {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        let mut session = ConfigLayer::default();
        session.set("repl.history.exclude", vec!["help".to_string()]);
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        resolver.set_session(session);
        let resolved = resolver
            .resolve(ResolveOptions::default())
            .expect("config should resolve");

        let patterns = repl_history_exclude_patterns(&resolved);
        assert_eq!(
            patterns
                .iter()
                .filter(|pattern| pattern.as_str() == "help")
                .count(),
            1
        );
    }
}
