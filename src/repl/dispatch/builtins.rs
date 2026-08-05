use crate::repl::{ReplLineResult, SharedHistory, expand_history};
use miette::{Result, miette};

use crate::app::CMD_HELP;
use crate::app::sink::UiSink;
use crate::app::{AppClients, AppRuntime, AppSession};

use super::shell::{handle_repl_exit_request, render_repl_help_for_scope};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ReplBuiltin {
    Help,
    Exit,
    Last { raw: bool },
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

pub(super) fn execute_repl_builtin(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    history: &SharedHistory,
    raw: &str,
    builtin: ReplBuiltin,
    sink: &mut dyn UiSink,
) -> Result<ReplLineResult> {
    match builtin {
        ReplBuiltin::Help => Ok(ReplLineResult::Continue(render_repl_help_for_scope(
            runtime,
            session,
            clients,
            &super::base_repl_invocation(runtime),
            raw,
            &[],
            sink,
        )?)),
        ReplBuiltin::Exit => Ok(handle_repl_exit_request(session)),
        ReplBuiltin::Last { raw } => execute_last_result_builtin(runtime, session, raw),
        ReplBuiltin::Bang(command) => execute_bang_command(session, history, raw, command),
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
    if let Some(raw) = parse_last_builtin(raw)? {
        return Ok(Some(ReplBuiltin::Last { raw }));
    }
    if let Some(command) = parse_bang_command(raw)? {
        return Ok(Some(ReplBuiltin::Bang(command)));
    }
    Ok(None)
}

fn parse_last_builtin(raw: &str) -> Result<Option<bool>> {
    let mut parts = raw.split_whitespace();
    if parts.next() != Some("last") {
        return Ok(None);
    }

    match (parts.next(), parts.next()) {
        (None, None) => Ok(Some(false)),
        (Some("--raw"), None) => Ok(Some(true)),
        _ => Err(miette!("`last` only supports the optional `--raw` flag")),
    }
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
    session.scope.history_scope_prefix()
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
    out.push_str("REPL builtins:\n");
    out.push_str("  last     replay the last successful result\n");
    out.push_str("  last --raw  show the pre-pipeline result\n\n");
    out.push_str("Bang history shortcuts:\n");
    out.push_str("  !!       last visible command\n");
    out.push_str("  !-N      Nth previous visible command\n");
    out.push_str("  !N       visible history entry by id\n");
    out.push_str("  !prefix  latest visible command starting with prefix\n");
    out.push_str("  !?text   latest visible command containing text\n");
    out
}

fn execute_last_result_builtin(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    raw: bool,
) -> Result<ReplLineResult> {
    let Some(last) = session.last_success() else {
        return Ok(ReplLineResult::Continue(
            "No recorded successful REPL result in this session.\n".to_string(),
        ));
    };
    let runtime = crate::app::CommandRenderRuntime::new(runtime.config.resolved(), &runtime.ui);
    let rendered = if raw {
        crate::app::render_repl_output_with_runtime(&runtime, &last.output)
    } else {
        crate::app::render_saved_repl_output_with_runtime(&runtime, &last.output, &last.stages)?
    };
    Ok(ReplLineResult::Continue(rendered))
}

pub(super) fn is_repl_bang_request(raw: &str) -> bool {
    raw.trim_start().starts_with('!')
}

#[cfg(test)]
mod tests {
    use super::{BangCommand, execute_repl_builtin, parse_repl_builtin};
    use crate::app::{
        AppState, AppStateInit, LaunchContext, ReplCommandOutput, RuntimeContext,
        StructuredCommandOutput, TerminalKind,
    };
    use crate::cli::rows::output::rows_to_output_result;
    use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions};
    use crate::core::output::OutputFormat;
    use crate::repl::{HistoryConfig, ReplLineResult, SharedHistory};
    use crate::ui::RenderSettings;
    use crate::ui::messages::MessageLevel;

    fn app_state() -> AppState {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        defaults.set("repl.history.path", "/tmp/osp-repl-builtins-history.jsonl");
        defaults.set("theme.path", Vec::<String>::new());
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
            error_detail: crate::app::ErrorDetail::Terse,
            debug_verbosity: 0,
            plugins: crate::plugin::PluginManager::new(Vec::new())
                .with_bundled_roots(false)
                .with_default_roots(false),
            native_commands: crate::native::NativeCommandRegistry::default(),
            themes: crate::ui::theme_catalog::ThemeCatalog::default(),
            launch: LaunchContext::default(),
        })
    }

    fn history() -> SharedHistory {
        SharedHistory::new(
            HistoryConfig::builder()
                .with_max_entries(20)
                .with_enabled(true)
                .with_dedupe(true)
                .with_profile_scoped(false)
                .with_shell_context(Default::default())
                .build(),
        )
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
            parse_repl_builtin("last").expect("last"),
            Some(super::ReplBuiltin::Last { raw: false })
        ));
        assert!(matches!(
            parse_repl_builtin("last --raw").expect("last raw"),
            Some(super::ReplBuiltin::Last { raw: true })
        ));
        assert!(matches!(
            parse_repl_builtin("!!").expect("bang"),
            Some(super::ReplBuiltin::Bang(BangCommand::Last))
        ));
    }

    #[test]
    fn execute_repl_builtin_covers_exit_and_help_unit() {
        let mut state = app_state();
        let history = history();
        let mut sink = crate::app::sink::BufferedUiSink::default();

        assert_eq!(
            parse_repl_builtin("ldap user alice").expect("non-builtin"),
            None
        );

        assert!(matches!(
            execute_repl_builtin(
                &mut state.runtime,
                &mut state.session,
                &state.clients,
                &history,
                "exit",
                parse_repl_builtin("exit")
                    .expect("exit should parse")
                    .expect("exit should classify"),
                &mut sink,
            )
            .expect("exit should succeed"),
            ReplLineResult::Exit(0)
        ));

        let mut state = app_state();
        let mut sink = crate::app::sink::BufferedUiSink::default();
        assert!(matches!(
            execute_repl_builtin(
                &mut state.runtime,
                &mut state.session,
                &state.clients,
                &history,
                "help",
                parse_repl_builtin("help")
                    .expect("help should parse")
                    .expect("help should classify"),
                &mut sink,
            )
            .expect("help should succeed"),
            ReplLineResult::Continue(text) if text.contains("help") || text.contains("config")
        ));

        state.session.record_success_output(
            "config show",
            &ReplCommandOutput::Text("visible\n".to_string()),
            &[],
        );
        assert!(matches!(
            execute_repl_builtin(
                &mut state.runtime,
                &mut state.session,
                &state.clients,
                &history,
                "last",
                parse_repl_builtin("last")
                    .expect("last should parse")
                    .expect("last should classify"),
                &mut sink,
            )
            .expect("last should succeed"),
            ReplLineResult::Continue(text) if text == "visible\n"
        ));

        state.session.record_success_output(
            "ldap user alice | P name",
            &ReplCommandOutput::Output(Box::new(StructuredCommandOutput {
                source_guide: None,
                output: rows_to_output_result(vec![crate::row! {
                    "name" => "alice",
                    "role" => "admin"
                }]),
                format_hint: None,
            })),
            &["P name".to_string()],
        );
        assert!(matches!(
            execute_repl_builtin(
                &mut state.runtime,
                &mut state.session,
                &state.clients,
                &history,
                "last",
                parse_repl_builtin("last")
                    .expect("last should parse")
                    .expect("last should classify"),
                &mut sink,
            )
            .expect("last should succeed"),
            ReplLineResult::Continue(text) if text.contains("alice") && !text.contains("admin")
        ));
        assert!(matches!(
            execute_repl_builtin(
                &mut state.runtime,
                &mut state.session,
                &state.clients,
                &history,
                "last --raw",
                parse_repl_builtin("last --raw")
                    .expect("last raw should parse")
                    .expect("last raw should classify"),
                &mut sink,
            )
            .expect("last raw should succeed"),
            ReplLineResult::Continue(text) if text.contains("alice") && text.contains("admin")
        ));
    }
}
