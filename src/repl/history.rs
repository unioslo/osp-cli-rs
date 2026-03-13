use crate::config::{
    ConfigValue, DEFAULT_REPL_HISTORY_MAX_ENTRIES, ResolvedConfig, RuntimeDefaults,
};
use crate::core::command_def::CommandDef;
use crate::core::row::Row;
use crate::repl::{HistoryConfig, HistoryEntry, SharedHistory};
use crate::ui::theme::DEFAULT_THEME_NAME;
use miette::{Result, miette};
use std::path::PathBuf;

use crate::app::{AppRuntime, AppSession};
use crate::cli::{HistoryArgs, HistoryCommands, HistoryPruneArgs};

use crate::app::{CMD_HISTORY, CMD_LIST, DEFAULT_REPL_PROMPT, ReplCommandOutput, config_usize};
use crate::cli::rows::output::rows_to_output_result;

const DEFAULT_REPL_HISTORY_EXCLUDES: [&str; 4] = ["exit", "quit", "help", "history list"];

pub(crate) fn history_command_def(sort_key: impl Into<String>) -> CommandDef {
    CommandDef::new(CMD_HISTORY)
        .about("Inspect or prune REPL history")
        .sort(sort_key)
        .subcommands([
            CommandDef::new(CMD_LIST)
                .about("List recent history")
                .sort("10"),
            CommandDef::new("prune")
                .about("Keep last N entries")
                .sort("11"),
            CommandDef::new("clear").about("Clear history").sort("12"),
        ])
}

pub(crate) fn build_history_config(runtime: &AppRuntime, session: &AppSession) -> HistoryConfig {
    let config = runtime.config.resolved();
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
            PathBuf::from(
                defaults
                    .get_string("repl.history.path")
                    .unwrap_or("${user.name}@${profile.active}.history"),
            )
        });
    let history_dedupe = config.get_bool("repl.history.dedupe").unwrap_or(true);
    let history_profile_scoped = config
        .get_bool("repl.history.profile_scoped")
        .unwrap_or(true);
    let history_exclude = repl_history_exclude_patterns(config);
    let history_shell = session.history_shell.clone();
    session.sync_history_shell_context();

    HistoryConfig::builder()
        .with_path(Some(history_path))
        .with_max_entries(history_max_entries)
        .with_enabled(history_enabled)
        .with_dedupe(history_dedupe)
        .with_profile_scoped(history_profile_scoped)
        .with_exclude_patterns(history_exclude)
        .with_profile(Some(config.active_profile().to_string()))
        .with_terminal(Some(
            runtime
                .context
                .terminal_kind()
                .as_config_terminal()
                .to_string(),
        ))
        .with_shell_context(history_shell)
        .build()
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
    session: &mut AppSession,
    args: HistoryArgs,
    history: &SharedHistory,
) -> Result<ReplCommandOutput> {
    if !history.enabled() {
        return Ok(ReplCommandOutput::Text(
            "History is disabled.\n".to_string(),
        ));
    }

    let scope = repl_history_scope(session);
    let scope_label = history_scope_label(session);
    match args.command {
        HistoryCommands::List => {
            let rows = history_entries_rows(history.list_entries_for(scope.as_deref()));
            Ok(ReplCommandOutput::Output {
                output: rows_to_output_result(rows),
                format_hint: None,
            })
        }
        HistoryCommands::Prune(HistoryPruneArgs { keep }) => {
            let removed = history
                .prune_for(keep, scope.as_deref())
                .map_err(|err| miette!(err.to_string()))?;
            Ok(ReplCommandOutput::Text(if removed == 0 {
                format!("No entries removed from {scope_label}.\n")
            } else {
                format!(
                    "Removed {removed} entr{} from {scope_label}.\n",
                    if removed == 1 { "y" } else { "ies" }
                )
            }))
        }
        HistoryCommands::Clear => {
            let removed = history
                .clear_for(scope.as_deref())
                .map_err(|err| miette!(err.to_string()))?;
            Ok(ReplCommandOutput::Text(if removed == 0 {
                format!("{scope_label} is already empty.\n")
            } else {
                format!("Cleared {scope_label}.\n")
            }))
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

fn repl_history_scope(session: &AppSession) -> Option<String> {
    let prefix = session.scope.history_prefix();
    if prefix.is_empty() {
        None
    } else {
        Some(prefix)
    }
}

fn history_scope_label(session: &AppSession) -> String {
    session
        .scope
        .display_label()
        .map(|label| format!("{label} shell history"))
        .unwrap_or_else(|| "root history".to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        history_command_def, history_scope_label, repl_history_enabled,
        repl_history_exclude_patterns, run_history_repl_command,
    };
    use crate::app::AppSession;
    use crate::app::ReplCommandOutput;
    use crate::cli::{HistoryArgs, HistoryCommands, HistoryPruneArgs};
    use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions};
    use crate::repl::{HistoryConfig, SharedHistory};
    use serde_json::Value;

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

    #[test]
    fn history_scope_label_tracks_current_shell_unit() {
        let mut session = AppSession::with_cache_limit(8);
        assert_eq!(history_scope_label(&session), "root history");

        session.scope.enter("orch");
        session.scope.enter("vm");
        assert_eq!(history_scope_label(&session), "orch / vm shell history");
    }

    #[test]
    fn history_command_def_exposes_expected_subcommands_unit() {
        let spec = history_command_def("20");
        let names = spec
            .subcommands
            .iter()
            .map(|child| child.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(spec.name, "history");
        assert_eq!(names, vec!["list", "prune", "clear"]);
    }

    #[test]
    fn repl_history_enabled_obeys_toggle_and_capacity_unit() {
        let disabled = config_with_entries(&[
            ("profile.default", "default"),
            ("repl.history.enabled", "false"),
        ]);
        assert!(!repl_history_enabled(&disabled));

        let zero_capacity_falls_back = config_with_entries(&[
            ("profile.default", "default"),
            ("repl.history.max_entries", "0"),
        ]);
        assert!(repl_history_enabled(&zero_capacity_falls_back));

        let enabled = config_with_entries(&[("profile.default", "default")]);
        assert!(repl_history_enabled(&enabled));
    }

    #[test]
    fn run_history_repl_command_reports_disabled_history_unit() {
        let history = shared_history(false);
        let mut session = AppSession::with_cache_limit(8);

        let output = run_history_repl_command(
            &mut session,
            HistoryArgs {
                command: HistoryCommands::List,
            },
            &history,
        )
        .expect("history command should return a disabled notice");

        match output {
            ReplCommandOutput::Text(text) => assert_eq!(text, "History is disabled.\n"),
            other => panic!("unexpected disabled history output: {other:?}"),
        }
    }

    #[test]
    fn run_history_repl_command_lists_visible_rows_unit() {
        let history = shared_history(true);
        history
            .save_command_line("config show")
            .expect("history seed should save");
        let mut session = AppSession::with_cache_limit(8);

        let output = run_history_repl_command(
            &mut session,
            HistoryArgs {
                command: HistoryCommands::List,
            },
            &history,
        )
        .expect("history list should succeed");

        match output {
            ReplCommandOutput::Output { output, .. } => {
                let rows = output.into_rows().expect("list should produce rows");
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0]["command"], Value::String("config show".to_string()));
                assert!(rows[0].contains_key("timestamp_ms"));
            }
            other => panic!("unexpected history list output: {other:?}"),
        }
    }

    #[test]
    fn run_history_repl_command_prunes_and_clears_with_scope_unit() {
        let history = shared_history(true);
        history
            .save_command_line("ldap user alice")
            .expect("history seed should save");
        history
            .save_command_line("ldap user bob")
            .expect("history seed should save");
        history
            .save_command_line("mreg host a")
            .expect("history seed should save");
        let mut session = AppSession::with_cache_limit(8);
        session.scope.enter("ldap");

        let prune = run_history_repl_command(
            &mut session,
            HistoryArgs {
                command: HistoryCommands::Prune(HistoryPruneArgs { keep: 1 }),
            },
            &history,
        )
        .expect("scoped prune should succeed");
        match prune {
            ReplCommandOutput::Text(text) => {
                assert_eq!(text, "Removed 1 entry from ldap shell history.\n")
            }
            other => panic!("unexpected prune output: {other:?}"),
        }

        let remaining = history.list_entries_for(Some("ldap"));
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].command, "user bob");
        assert_eq!(history.list_entries_for(Some("mreg")).len(), 1);

        let clear = run_history_repl_command(
            &mut session,
            HistoryArgs {
                command: HistoryCommands::Clear,
            },
            &history,
        )
        .expect("scoped clear should succeed");
        match clear {
            ReplCommandOutput::Text(text) => assert_eq!(text, "Cleared ldap shell history.\n"),
            other => panic!("unexpected clear output: {other:?}"),
        }
        assert!(history.list_entries_for(Some("ldap")).is_empty());
        assert_eq!(history.list_entries_for(Some("mreg")).len(), 1);
    }

    fn config_with_entries(entries: &[(&str, &str)]) -> crate::config::ResolvedConfig {
        let mut defaults = ConfigLayer::default();
        for (key, value) in entries {
            defaults.set(*key, *value);
        }
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        resolver
            .resolve(ResolveOptions::default())
            .expect("config should resolve")
    }

    fn shared_history(enabled: bool) -> SharedHistory {
        let temp_dir = make_temp_dir("osp-cli-repl-history");
        SharedHistory::new(
            HistoryConfig::builder()
                .with_path(Some(temp_dir.join("history.jsonl")))
                .with_max_entries(32)
                .with_enabled(enabled)
                .with_dedupe(false)
                .with_profile_scoped(false)
                .with_shell_context(crate::repl::HistoryShellContext::default())
                .build(),
        )
    }

    fn make_temp_dir(prefix: &str) -> crate::tests::TestTempDir {
        crate::tests::make_temp_dir(prefix)
    }
}
