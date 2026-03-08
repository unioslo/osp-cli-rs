use crate::osp_repl::{ReplLineResult, SharedHistory, expand_history};
use miette::{Result, miette};

use crate::osp_cli::app::CMD_HELP;
use crate::osp_cli::state::{AppClients, AppRuntime, AppSession};

use super::shell::{handle_repl_exit_request, repl_help_for_scope};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ReplBuiltin {
    Help,
    Exit,
    Bang(BangCommand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum BangCommand {
    Last,
    Relative(usize),
    Absolute(usize),
    Prefix(String),
    Contains(String),
}

pub(super) fn maybe_execute_repl_builtin(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    history: &SharedHistory,
    raw: &str,
) -> Result<Option<ReplLineResult>> {
    let Some(builtin) = parse_repl_builtin(raw)? else {
        return Ok(None);
    };

    match builtin {
        ReplBuiltin::Help => Ok(Some(ReplLineResult::Continue(repl_help_for_scope(
            runtime,
            session,
            clients,
            &super::base_repl_invocation(runtime),
        )?))),
        ReplBuiltin::Exit => Ok(handle_repl_exit_request(session)),
        ReplBuiltin::Bang(command) => {
            execute_bang_command(session, history, raw, command).map(Some)
        }
    }
}

pub(super) fn parse_repl_builtin(raw: &str) -> Result<Option<ReplBuiltin>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    if raw == CMD_HELP || raw == "--help" || raw == "-h" {
        return Ok(Some(ReplBuiltin::Help));
    }
    if raw == "exit" || raw == "quit" {
        return Ok(Some(ReplBuiltin::Exit));
    }
    if let Some(command) = parse_bang_command(raw)? {
        return Ok(Some(ReplBuiltin::Bang(command)));
    }
    Ok(None)
}

pub(super) fn parse_bang_command(raw: &str) -> Result<Option<BangCommand>> {
    let raw = raw.trim();
    if !raw.starts_with('!') {
        return Ok(None);
    }
    if raw == "!" {
        return Ok(Some(BangCommand::Prefix(String::new())));
    }
    if raw == "!!" {
        return Ok(Some(BangCommand::Last));
    }
    if let Some(rest) = raw.strip_prefix("!?") {
        let term = rest.trim();
        if term.is_empty() {
            return Err(miette!("`!?` expects search text"));
        }
        return Ok(Some(BangCommand::Contains(term.to_string())));
    }
    if let Some(rest) = raw.strip_prefix("!-") {
        let offset = rest
            .trim()
            .parse::<usize>()
            .map_err(|_| miette!("`!-N` expects a positive integer"))?;
        if offset == 0 {
            return Err(miette!("`!-N` expects N >= 1"));
        }
        return Ok(Some(BangCommand::Relative(offset)));
    }
    let rest = raw.trim_start_matches('!').trim();
    if rest.is_empty() {
        return Ok(Some(BangCommand::Prefix(String::new())));
    }
    if rest.chars().all(|ch| ch.is_ascii_digit()) {
        let id = rest
            .parse::<usize>()
            .map_err(|_| miette!("`!N` expects a positive integer"))?;
        if id == 0 {
            return Err(miette!("`!N` expects N >= 1"));
        }
        return Ok(Some(BangCommand::Absolute(id)));
    }
    Ok(Some(BangCommand::Prefix(rest.to_string())))
}

pub(super) fn execute_bang_command(
    session: &mut AppSession,
    history: &SharedHistory,
    raw: &str,
    command: BangCommand,
) -> Result<ReplLineResult> {
    let scope = current_history_scope(session);
    let recent = history.recent_commands_for(scope.as_deref());

    let expanded = match command {
        BangCommand::Last => expand_history("!!", &recent, scope.as_deref(), true),
        BangCommand::Relative(offset) => {
            expand_history(&format!("!-{offset}"), &recent, scope.as_deref(), true)
        }
        BangCommand::Absolute(id) => {
            expand_history(&format!("!{id}"), &recent, scope.as_deref(), true)
        }
        BangCommand::Prefix(prefix) => {
            if prefix.is_empty() {
                return Ok(ReplLineResult::Continue(render_bang_help()));
            }
            expand_history(&format!("!{prefix}"), &recent, scope.as_deref(), true)
        }
        BangCommand::Contains(term) => {
            let mut found = None;
            for full in recent.iter().rev() {
                let visible = strip_history_scope(full, scope.as_deref());
                if visible.contains(&term) {
                    found = Some(visible);
                    break;
                }
            }
            found
        }
    };

    let Some(expanded) = expanded else {
        return Ok(ReplLineResult::Continue(format!(
            "No history match for: {raw}\n"
        )));
    };

    Ok(ReplLineResult::ReplaceInput(expanded))
}

pub(super) fn current_history_scope(session: &AppSession) -> Option<String> {
    let prefix = session.scope.history_prefix();
    if prefix.is_empty() {
        None
    } else {
        Some(prefix)
    }
}

pub(super) fn strip_history_scope(command: &str, scope: Option<&str>) -> String {
    let trimmed = command.trim();
    match scope {
        Some(prefix) => trimmed
            .strip_prefix(prefix)
            .map(|rest| rest.trim_start().to_string())
            .unwrap_or_else(|| trimmed.to_string()),
        None => trimmed.to_string(),
    }
}

fn render_bang_help() -> String {
    let mut out = String::new();
    out.push_str("Bang history shortcuts:\n");
    out.push_str("  !!       last visible command\n");
    out.push_str("  !-N      Nth previous visible command\n");
    out.push_str("  !N       visible history entry by id\n");
    out.push_str("  !prefix  latest visible command starting with prefix\n");
    out.push_str("  !?text   latest visible command containing text\n");
    out
}

pub(super) fn is_repl_bang_request(raw: &str) -> bool {
    raw.trim_start().starts_with('!')
}

#[cfg(test)]
mod tests {
    use super::{BangCommand, maybe_execute_repl_builtin, parse_repl_builtin};
    use crate::osp_cli::state::{
        AppState, AppStateInit, LaunchContext, RuntimeContext, TerminalKind,
    };
    use crate::osp_config::{ConfigLayer, ConfigResolver, ResolveOptions};
    use crate::osp_core::output::OutputFormat;
    use crate::osp_repl::{HistoryConfig, ReplLineResult, SharedHistory};
    use crate::osp_ui::RenderSettings;
    use crate::osp_ui::messages::MessageLevel;

    fn app_state() -> AppState {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        let config = resolver
            .resolve(ResolveOptions::default().with_terminal("repl"))
            .expect("test config should resolve");

        AppState::new(AppStateInit {
            context: RuntimeContext::new(None, TerminalKind::Repl, None),
            config,
            render_settings: RenderSettings::test_plain(OutputFormat::Json),
            message_verbosity: MessageLevel::Success,
            debug_verbosity: 0,
            plugins: crate::osp_cli::plugin_manager::PluginManager::new(Vec::new()),
            themes: crate::osp_cli::theme_loader::ThemeCatalog::default(),
            launch: LaunchContext::default(),
        })
    }

    fn history() -> SharedHistory {
        SharedHistory::new(HistoryConfig {
            path: None,
            max_entries: 20,
            enabled: true,
            dedupe: true,
            profile_scoped: false,
            exclude_patterns: Vec::new(),
            profile: None,
            terminal: None,
            shell_context: Default::default(),
        })
        .expect("history should initialize")
    }

    #[test]
    fn parse_repl_builtin_covers_none_help_exit_and_bang_unit() {
        assert_eq!(parse_repl_builtin("   ").expect("blank"), None);
        assert!(matches!(
            parse_repl_builtin("--help").expect("help"),
            Some(super::ReplBuiltin::Help)
        ));
        assert!(matches!(
            parse_repl_builtin("quit").expect("exit"),
            Some(super::ReplBuiltin::Exit)
        ));
        assert!(matches!(
            parse_repl_builtin("!!").expect("bang"),
            Some(super::ReplBuiltin::Bang(BangCommand::Last))
        ));
    }

    #[test]
    fn maybe_execute_repl_builtin_covers_none_exit_and_help_unit() {
        let mut state = app_state();
        let history = history();

        assert_eq!(
            maybe_execute_repl_builtin(
                &mut state.runtime,
                &mut state.session,
                &state.clients,
                &history,
                "ldap user alice",
            )
            .expect("non-builtin should return none"),
            None
        );

        assert!(matches!(
            maybe_execute_repl_builtin(
                &mut state.runtime,
                &mut state.session,
                &state.clients,
                &history,
                "exit",
            )
            .expect("exit should succeed"),
            Some(ReplLineResult::Exit(0))
        ));

        let mut state = app_state();
        assert!(matches!(
            maybe_execute_repl_builtin(
                &mut state.runtime,
                &mut state.session,
                &state.clients,
                &history,
                "help",
            )
            .expect("help should succeed"),
            Some(ReplLineResult::Continue(text)) if text.contains("help") || text.contains("config")
        ));
    }
}
