use std::path::PathBuf;

use crate::repl::history_store::split_bang_request;
pub(super) use crate::repl::history_store::{BangCommand, parse_bang_command};
use crate::repl::{ReplResult, SharedHistory};
use miette::{Result, WrapErr, miette};

use crate::app::sink::UiSink;
use crate::app::{AppRuntime, AppSession};
use crate::app::{CMD_HELP, EXIT_CODE_WAITING_APPROVAL};
use crate::normalize::expand_home;

use super::shell::{handle_repl_exit_request, render_repl_help_for_scope};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ReplBuiltin {
    Help,
    HelpAll,
    Recording(Option<crate::repl::recording::RecordingCommand>),
    Exit,
    Last,
    Pagination(PaginationDirection),
    Source(SourceCommand),
    Each(EachCommand),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PaginationDirection {
    Previous,
    Next,
}

impl PaginationDirection {
    const COMMANDS: [(&'static str, Self); 2] = [("next", Self::Next), ("prev", Self::Previous)];
}

pub(crate) fn command_names() -> Vec<String> {
    crate::cli::repl_only_commands()
        .iter()
        .flat_map(|command| {
            std::iter::once(command.get_name())
                .chain(command.get_all_aliases())
                .map(str::to_owned)
        })
        .chain(
            PaginationDirection::COMMANDS
                .iter()
                .map(|(name, _)| (*name).to_owned()),
        )
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SourceCommand {
    paths: Vec<PathBuf>,
    ignore_errors: bool,
    help: bool,
}

pub(super) fn execute_repl_builtin(
    state: &mut crate::app::AppState,
    history: &SharedHistory,
    raw: &str,
    builtin: ReplBuiltin,
    sink: &mut dyn UiSink,
) -> Result<ReplResult> {
    match builtin {
        ReplBuiltin::Recording(command) => {
            use clap::CommandFactory;
            Ok(ReplResult::Print(match command {
                Some(command) => command.run(&state.session.recording)?,
                None => crate::repl::recording::RecordingCommand::command()
                    .render_long_help()
                    .to_string(),
            }))
        }
        ReplBuiltin::HelpAll => {
            let parsed =
                crate::repl::input::ReplParsedLine::parse(raw, state.runtime.config.resolved())?;
            let scanned = crate::cli::invocation::scan_command_tokens(&parsed.dispatch_tokens)?;
            let invocation = crate::app::resolve_invocation_ui(
                state.runtime.config.resolved(),
                &state.runtime.ui,
                &scanned.invocation,
            );
            let output = crate::app::help::help_all_output(state);
            Ok(ReplResult::Print(
                super::command::render_repl_command_output(
                    &state.runtime,
                    &mut state.session,
                    raw,
                    &parsed.stages,
                    output,
                    &invocation,
                    sink,
                )?,
            ))
        }
        ReplBuiltin::Help => {
            let invocation = super::base_repl_invocation(&state.runtime);
            Ok(ReplResult::Print(render_repl_help_for_scope(
                state,
                &invocation,
                raw,
                &[],
                sink,
            )?))
        }
        ReplBuiltin::Exit => Ok(handle_repl_exit_request(&mut state.session)),
        ReplBuiltin::Last => {
            execute_last_result_builtin(&mut state.runtime, &mut state.session, raw, sink)
        }
        ReplBuiltin::Pagination(direction) => {
            execute_pagination_builtin(state, history, direction, sink)
        }
        ReplBuiltin::Each(command) => execute_each_command(state, history, command, sink),
        ReplBuiltin::Source(command) => execute_source_command(state, history, command, sink),
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
    if let Some((_, direction)) = PaginationDirection::COMMANDS
        .iter()
        .find(|(name, _)| *name == raw)
    {
        return Ok(Some(ReplBuiltin::Pagination(*direction)));
    }
    if raw.split_whitespace().next() == Some("last") {
        return Ok(Some(ReplBuiltin::Last));
    }
    if raw.split_whitespace().next() == Some("recording") {
        use clap::Parser;
        let words =
            shell_words::split(raw).map_err(|err| miette!("invalid recording command: {err}"))?;
        return match crate::repl::recording::RecordingCommand::try_parse_from(words) {
            Ok(command) => Ok(Some(ReplBuiltin::Recording(Some(command)))),
            Err(err)
                if matches!(
                    err.kind(),
                    clap::error::ErrorKind::DisplayHelp
                        | clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
                ) =>
            {
                Ok(Some(ReplBuiltin::Recording(None)))
            }
            Err(err) => Err(miette!(err.to_string())),
        };
    }
    if raw.split_whitespace().next() == Some("each") {
        use clap::Parser;
        let words =
            shell_words::split(raw).map_err(|err| miette!("invalid each command: {err}"))?;
        return match EachCommand::try_parse_from(words) {
            Ok(command) => Ok(Some(ReplBuiltin::Each(command))),
            Err(err) if err.kind() == clap::error::ErrorKind::DisplayHelp => {
                Ok(Some(ReplBuiltin::Each(EachCommand::default())))
            }
            Err(err) => Err(miette!(err.to_string())),
        };
    }
    if let Some(command) = parse_source_builtin(raw)? {
        return Ok(Some(ReplBuiltin::Source(command)));
    }
    Ok(None)
}

#[derive(Debug, Clone, PartialEq, Eq, Default, clap::Parser)]
#[command(
    name = "each",
    about = "Run a command for every value of the saved result"
)]
pub(crate) struct EachCommand {
    /// Command template containing {}.
    template: Option<String>,
    /// Allow more than 50 values and commands that change data.
    #[arg(long)]
    yes: bool,
    /// Acknowledge exactly this many writes.
    #[arg(long)]
    count: Option<usize>,
    /// Continue after command errors.
    #[arg(long)]
    ignore_errors: bool,
}

fn execute_each_command(
    state: &mut crate::app::AppState,
    history: &SharedHistory,
    command: EachCommand,
    sink: &mut dyn UiSink,
) -> Result<ReplResult> {
    use clap::CommandFactory;
    let Some(template) = command.template else {
        return Ok(ReplResult::Print(
            EachCommand::command().render_long_help().to_string(),
        ));
    };
    if !template.contains("{}") {
        return Err(miette!("each requires a template containing {{}}"));
    }
    let parsed =
        crate::repl::input::ReplParsedLine::parse(&template, state.runtime.config.resolved())?;
    let tokens = parsed.prefixed_tokens(&state.session.scope);
    // Unclassified templates may expand into writes, so require acknowledgement.
    let writes = state
        .clients
        .native_commands()
        .writes(&tokens)
        .unwrap_or(true);
    if writes && !command.yes {
        return Err(miette!("each requires --yes for commands that change data"));
    }
    let last = state
        .session
        .last_success()
        .ok_or_else(|| miette!("No recorded successful REPL result in this session."))?;
    let mut stages = last.stages.clone();
    stages.push("VALUE".to_string());
    let mut ui = state.runtime.ui.clone();
    ui.render_settings.format = crate::core::output::OutputFormat::Json;
    ui.render_settings.format_explicit = true;
    let (rendered, _) = crate::app::render_saved_repl_output(
        state.runtime.config.resolved(),
        &ui,
        &last.output,
        &stages,
        false,
        None,
    )?;
    let values: serde_json::Value =
        serde_json::from_str(&rendered).map_err(|err| miette!("each: {err}"))?;
    let rows = crate::core::output_model::rows_from_value(values);
    if rows.len() > 50 && !command.yes {
        return Err(miette!("each requires --yes for more than 50 values"));
    }
    if writes && command.count != Some(rows.len()) {
        return Err(miette!(
            "each will run {} writes; acknowledge with --count {}",
            rows.len(),
            rows.len()
        ));
    }
    let (mut ok, mut failed) = (0, 0);
    let mut result = Ok(ReplResult::Continue);
    for row in rows {
        let value = row.get("value").unwrap_or(&serde_json::Value::Null);
        let text = value
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| value.to_string());
        let line = template.replace("{}", &shell_words::quote(&text));
        match execute_source_line(state, history, &line, "each", sink) {
            Ok(None) => ok += 1,
            Ok(Some(stop)) => {
                ok += 1;
                result = Ok(stop);
                break;
            }
            Err(err) => {
                failed += 1;
                if !command.ignore_errors {
                    result = Err(err);
                    break;
                }
                sink.write_stderr(&format!("each: {err}\n"));
            }
        }
    }
    sink.write_stderr(&format!("{ok} ok, {failed} failed\n"));
    if failed > 0 && result.is_ok() {
        return Err(miette!("each: {failed} commands failed"));
    }
    result
}

fn parse_source_builtin(raw: &str) -> Result<Option<SourceCommand>> {
    let words = shell_words::split(raw).map_err(|err| miette!("invalid source command: {err}"))?;
    if words.first().map(String::as_str) != Some("source") {
        return Ok(None);
    }

    let mut paths = Vec::new();
    let mut ignore_errors = false;
    let mut help = false;
    let mut options = true;
    for word in words.into_iter().skip(1) {
        match word.as_str() {
            "--" if options => options = false,
            "--ignore-errors" if options => ignore_errors = true,
            "-h" | "--help" if options => help = true,
            _ if options && word.starts_with('-') => {
                return Err(miette!(
                    "unknown source option `{word}`\n\n{}",
                    source_help()
                ));
            }
            _ => paths.push(PathBuf::from(
                expand_home(&word)
                    .map(|path| path.display().to_string())
                    .unwrap_or(word),
            )),
        }
    }
    if !help && paths.is_empty() {
        help = true;
    }
    Ok(Some(SourceCommand {
        paths,
        ignore_errors,
        help,
    }))
}

fn execute_source_command(
    state: &mut crate::app::AppState,
    history: &SharedHistory,
    command: SourceCommand,
    sink: &mut dyn UiSink,
) -> Result<ReplResult> {
    if command.help {
        return Ok(ReplResult::Print(source_help()));
    }

    for path in command.paths {
        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(err) if command.ignore_errors => {
                sink.write_stderr(&format!("{}: {err}\n", path.display()));
                continue;
            }
            Err(err) => {
                return Err(miette!("failed to read `{}`: {err}", path.display()));
            }
        };

        for (index, line) in contents.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let location = format!("{}:{}", path.display(), index + 1);
            match execute_source_line(state, history, line, &location, sink) {
                Ok(None) => {}
                Ok(Some(stop)) => return Ok(stop),
                Err(err) if command.ignore_errors => {
                    sink.write_stderr(&format!("{location}: {err}\n"));
                }
                Err(err) => {
                    return Err(err).wrap_err_with(|| format!("command failed at {location}"));
                }
            }
        }
    }
    Ok(ReplResult::Continue)
}

/// Runs one sourced line, returning the result that stops the batch, if any.
fn execute_source_line(
    state: &mut crate::app::AppState,
    history: &SharedHistory,
    line: &str,
    location: &str,
    sink: &mut dyn UiSink,
) -> Result<Option<ReplResult>> {
    if shell_words::split(line)
        .ok()
        .and_then(|words| words.into_iter().next())
        .as_deref()
        .is_some_and(|word| matches!(word, "source" | "each"))
    {
        return Err(miette!(
            "nested `source` or `each` commands are not supported"
        ));
    }
    let executed = crate::ui::prompt::replay(|| {
        super::execute_repl_plugin_line(state, history, line, Some(sink))
    })?;
    match executed.result {
        ReplResult::Continue => {}
        ReplResult::Print(rendered) => sink.write_stdout(&rendered),
        ReplResult::Restart { output, reload } => {
            sink.write_stdout(&output);
            sink.write_stderr(&format!(
                "{location}: command requires a REPL restart; remaining source lines were not run\n"
            ));
            return Ok(Some(ReplResult::Restart {
                output: String::new(),
                reload,
            }));
        }
        ReplResult::Exit(code) => return Ok(Some(ReplResult::Exit(code))),
        ReplResult::ReplaceInput(_) => {
            return Err(miette!(
                "history expansion is not supported in sourced files"
            ));
        }
    }
    if !matches!(executed.exit_code, 0 | EXIT_CODE_WAITING_APPROVAL) {
        return Err(miette!("command exited with status {}", executed.exit_code));
    }
    Ok(None)
}

fn execute_pagination_builtin(
    state: &mut crate::app::AppState,
    history: &SharedHistory,
    direction: PaginationDirection,
    sink: &mut dyn UiSink,
) -> Result<ReplResult> {
    if state.session.native_context.pagination().is_none() {
        return Ok(ReplResult::Print(
            "No paginated result to continue. Run a paginated list command first.\n".to_string(),
        ));
    }
    let forward = matches!(direction, PaginationDirection::Next);
    let Some(command) = state.session.native_context.step_page(forward) else {
        let message = if forward {
            "Already at the last page.\n"
        } else {
            "Already at the first page.\n"
        };
        return Ok(ReplResult::Print(message.to_string()));
    };

    // NativePagination stores tokenized argv. Re-encode it only at the REPL
    // parser boundary, so a native command cannot smuggle shell syntax into
    // the nested dispatch.
    let line = command
        .iter()
        .map(|argument| crate::core::shell_words::escape_for_shell(argument))
        .collect::<Vec<_>>()
        .join(" ");
    let executed = super::execute_repl_plugin_line(state, history, &line, Some(sink));
    state.session.native_context.finish_page_step();
    Ok(executed?.result)
}

fn source_help() -> String {
    "Run one REPL command per line from one or more files.\n\nUsage: source [--ignore-errors] <FILE>...\n\nBlank lines and # comments are ignored. Waiting for approval is not an error.\nCommands that reload or exit the REPL stop the batch. Paths may start with ~.\n\nOptions:\n  --ignore-errors  Continue after command and file errors\n  -h, --help       Print help\n"
        .to_string()
}

pub(super) fn execute_bang_command(
    session: &mut AppSession,
    history: &SharedHistory,
    raw: &str,
    command: BangCommand,
) -> Result<ReplResult> {
    let elevated = split_bang_request(raw).is_some_and(|(elevated, _)| elevated);
    let scope = current_history_scope(session);
    if matches!(&command, BangCommand::Prefix(prefix) if prefix.is_empty()) {
        return Ok(ReplResult::Print(render_bang_help()));
    }
    let expanded = history
        .with(|store| store.expand(&command, scope.as_deref(), elevated))
        .ok()
        .flatten();

    let Some(expanded) = expanded else {
        return match command {
            BangCommand::Absolute { id, .. } => Err(miette!("no history entry {id}")),
            _ => Err(miette!("no history match for: {raw}")),
        };
    };

    Ok(ReplResult::ReplaceInput(expanded))
}

pub(super) fn current_history_scope(session: &AppSession) -> Option<String> {
    session.scope.history_scope_prefix()
}

fn render_bang_help() -> String {
    let mut out = String::new();
    out.push_str("REPL builtins:\n");
    out.push_str("  last     replay the last successful result\n");
    out.push_str("  last --raw  show the pre-pipeline result\n\n");
    out.push_str("Bang history shortcuts:\n");
    out.push_str("  !! [args] repeat the last command, appending arguments\n");
    out.push_str("  !-N      Nth previous visible command\n");
    out.push_str("  !N [args]  visible history entry by id, with optional appended arguments\n");
    out.push_str("  !prefix  latest visible command starting with prefix\n");
    out.push_str("  !?text   latest visible command containing text\n");
    out
}

fn execute_last_result_builtin(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    line: &str,
    sink: &mut dyn UiSink,
) -> Result<ReplResult> {
    let parsed = crate::repl::input::ReplParsedLine::parse(line, runtime.config.resolved())?;
    let scanned = crate::cli::invocation::scan_command_tokens(&parsed.dispatch_tokens)?;
    let command = clap::Command::new("last")
        .about("Render the last successful result without running its command again")
        .arg(
            clap::Arg::new("raw")
                .long("raw")
                .action(clap::ArgAction::SetTrue)
                .help("Start from the saved result before its original pipeline"),
        )
        .after_help(crate::cli::invocation::invocation_help_text());
    let matches = match command.try_get_matches_from(scanned.tokens) {
        Ok(matches) => matches,
        Err(error) if error.kind() == clap::error::ErrorKind::DisplayHelp => {
            return Ok(ReplResult::Print(error.to_string()));
        }
        Err(error) => return Err(miette!(error.to_string())),
    };
    let Some(last) = session.last_success() else {
        return Ok(ReplResult::Print(
            "No recorded successful REPL result in this session.\n".to_string(),
        ));
    };
    let invocation = crate::app::resolve_invocation_ui(
        runtime.config.resolved(),
        &runtime.ui,
        &scanned.invocation,
    );
    // Replay only saved data. New stages follow the original pipeline unless
    // --raw explicitly selects its pre-pipeline input; neither replaces the cache.
    let mut stages = if matches.get_flag("raw") {
        Vec::new()
    } else {
        last.stages.clone()
    };
    stages.extend(parsed.stages);
    let (rendered, staged) = crate::app::render_saved_repl_output(
        runtime.config.resolved(),
        &invocation.ui,
        &last.output,
        &stages,
        invocation.source,
        Some(sink),
    )?;
    session.last_output = Some((
        std::sync::Arc::new(staged),
        invocation.ui.render_settings.clone(),
    ));
    Ok(ReplResult::Print(rendered))
}

pub(super) fn is_repl_bang_request(raw: &str) -> bool {
    split_bang_request(raw).is_some()
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::{PaginationDirection, execute_repl_builtin, parse_repl_builtin};
    use crate::app::{AppState, CommandOutput, LaunchContext, RuntimeContext, TerminalKind};
    use crate::cli::rows::output::rows_to_output_result;
    use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions};
    use crate::core::output::OutputFormat;
    use crate::native::{NativeCommand, NativeCommandContext, NativeCommandOutcome};
    use crate::repl::{HistoryConfig, ReplResult, SharedHistory};
    use crate::ui::RenderSettings;
    use crate::ui::messages::MessageLevel;
    use clap::Command;

    fn app_state() -> AppState {
        app_state_with_registry(crate::native::NativeCommandRegistry::default())
    }

    fn app_state_with_registry(native_commands: crate::native::NativeCommandRegistry) -> AppState {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        defaults.set("repl.history.path", "/tmp/osp-repl-builtins-history.jsonl");
        defaults.set("theme.path", Vec::<String>::new());
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        let config = resolver
            .resolve(ResolveOptions::default().with_terminal("repl"))
            .expect("test config should resolve");

        crate::app::AppStateBuilder {
            context: RuntimeContext::new(None, TerminalKind::Repl, None),
            config,
            launch: LaunchContext::default(),
            native_commands,
            ui: crate::app::UiState::new(
                RenderSettings::test_plain(OutputFormat::Json),
                MessageLevel::Success,
                0,
            ),
            plugins: Some(
                crate::plugin::PluginManager::new(Vec::new())
                    .with_bundled_roots(false)
                    .with_default_roots(false),
            ),
            themes: Some(crate::ui::theme_catalog::ThemeCatalog::default()),
            session: None,
            definition: Default::default(),
        }
        .build()
    }

    fn history() -> SharedHistory {
        SharedHistory::new(HistoryConfig {
            max_entries: 20,
            enabled: true,
            dedupe: true,
            shell_context: Default::default(),
            ..HistoryConfig::default()
        })
    }

    struct PageCommand;

    struct SourceReplayCommand;

    impl NativeCommand for SourceReplayCommand {
        fn command(&self) -> Command {
            Command::new("source-replay")
        }

        fn execute(
            &self,
            _args: &[String],
            _context: &NativeCommandContext<'_>,
        ) -> anyhow::Result<NativeCommandOutcome> {
            let mut response =
                crate::core::plugin::ResponseV1::ok(serde_json::json!({"name": "display"}));
            response.source = Some(serde_json::json!({"name": "server", "server_field": "kept"}));
            response.meta.preserve_json_document = true;
            Ok(NativeCommandOutcome::Response(Box::new(response)))
        }
    }

    #[test]
    fn last_selects_saved_source_and_combines_it_with_raw_and_new_stages() {
        let history = history();
        for source in [false, true] {
            let mut state = app_state_with_registry(
                crate::native::NativeCommandRegistry::new().with_command(SourceReplayCommand),
            );
            let mut sink = crate::app::sink::BufferedUiSink::default();
            let line = if source {
                "source-replay --source-record | P name"
            } else {
                "source-replay | P name"
            };
            let executed = crate::repl::dispatch::execute_repl_plugin_line(
                &mut state,
                &history,
                line,
                Some(&mut sink),
            )
            .expect("command should render");
            let ReplResult::Print(text) = executed.result else {
                panic!("expected command output")
            };
            let selected = if source { "server" } else { "display" };
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&text).unwrap(),
                serde_json::json!([{"name": selected}])
            );

            for (line, expected) in [
                (
                    "last --source-record --json",
                    serde_json::json!([{"name": "server"}]),
                ),
                (
                    "last --source-record --raw --json",
                    serde_json::json!({"name": "server", "server_field": "kept"}),
                ),
                (
                    "last --raw --source-record --json | P server_field",
                    serde_json::json!([{"server_field": "kept"}]),
                ),
                ("last --json", serde_json::json!([{"name": selected}])),
            ] {
                let executed = crate::repl::dispatch::execute_repl_plugin_line(
                    &mut state,
                    &history,
                    line,
                    Some(&mut sink),
                )
                .expect("last should replay");
                let ReplResult::Print(text) = executed.result else {
                    panic!("expected replay output")
                };
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(&text).unwrap(),
                    expected,
                    "{line}"
                );
            }
        }
    }

    impl NativeCommand for PageCommand {
        fn command(&self) -> Command {
            Command::new("page")
        }

        fn execute(
            &self,
            args: &[String],
            _context: &NativeCommandContext<'_>,
        ) -> anyhow::Result<NativeCommandOutcome> {
            Ok(NativeCommandOutcome::Help(format!(
                "page command received: {}",
                args.join(" ")
            )))
        }
    }

    #[test]
    fn parse_repl_builtin_covers_none_help_exit_and_pagination_unit() {
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
            Some(super::ReplBuiltin::Last)
        ));
        assert!(matches!(
            parse_repl_builtin("last --raw").expect("last raw"),
            Some(super::ReplBuiltin::Last)
        ));
        assert!(matches!(
            parse_repl_builtin("next").expect("next"),
            Some(super::ReplBuiltin::Pagination(PaginationDirection::Next))
        ));
        assert!(matches!(
            parse_repl_builtin("prev").expect("prev"),
            Some(super::ReplBuiltin::Pagination(
                PaginationDirection::Previous
            ))
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
                &mut state,
                &history,
                "exit",
                parse_repl_builtin("exit")
                    .expect("exit should parse")
                    .expect("exit should classify"),
                &mut sink,
            )
            .expect("exit should succeed"),
            ReplResult::Exit(0)
        ));

        let mut state = app_state();
        let mut sink = crate::app::sink::BufferedUiSink::default();
        assert!(matches!(
            execute_repl_builtin(
                &mut state,
                &history,
                "help",
                parse_repl_builtin("help")
                    .expect("help should parse")
                    .expect("help should classify"),
                &mut sink,
            )
            .expect("help should succeed"),
            ReplResult::Print(text) if text.contains("help") || text.contains("config")
        ));

        state.session.record_success_output(
            "config show",
            CommandOutput::text("visible\n".to_string()),
            &[],
        );
        assert!(matches!(
            execute_repl_builtin(
                &mut state,
                &history,
                "last",
                parse_repl_builtin("last")
                    .expect("last should parse")
                    .expect("last should classify"),
                &mut sink,
            )
            .expect("last should succeed"),
            ReplResult::Print(text) if text == "visible\n"
        ));

        state.session.record_success_output(
            "ldap user alice | P name",
            CommandOutput::output(
                rows_to_output_result(vec![crate::row! {
                    "name" => "alice",
                    "role" => "admin"
                }]),
                None,
            ),
            &["P name".to_string()],
        );
        assert!(matches!(
            execute_repl_builtin(
                &mut state,
                &history,
                "last",
                parse_repl_builtin("last")
                    .expect("last should parse")
                    .expect("last should classify"),
                &mut sink,
            )
            .expect("last should succeed"),
            ReplResult::Print(text) if text.contains("alice") && !text.contains("admin")
        ));
        assert!(matches!(
            execute_repl_builtin(
                &mut state,
                &history,
                "last --raw",
                parse_repl_builtin("last --raw")
                    .expect("last raw should parse")
                    .expect("last raw should classify"),
                &mut sink,
            )
            .expect("last raw should succeed"),
            ReplResult::Print(text) if text.contains("alice") && text.contains("admin")
        ));
    }

    #[test]
    fn pagination_builtin_reports_missing_and_boundary_context_unit() {
        let mut state = app_state();
        let history = history();
        let mut sink = crate::app::sink::BufferedUiSink::default();

        let result = execute_repl_builtin(
            &mut state,
            &history,
            "next",
            parse_repl_builtin("next")
                .expect("next should parse")
                .expect("next should classify"),
            &mut sink,
        )
        .expect("missing pagination should be handled");
        assert!(matches!(
            result,
            ReplResult::Print(text) if text.contains("Run a paginated list command first")
        ));

        state
            .session
            .native_context
            .set_pagination(crate::native::NativePagination {
                current: vec!["page".to_string()],
                next: None,
            });
        let result = execute_repl_builtin(
            &mut state,
            &history,
            "prev",
            parse_repl_builtin("prev")
                .expect("prev should parse")
                .expect("prev should classify"),
            &mut sink,
        )
        .expect("first-page boundary should be handled");
        assert!(matches!(
            result,
            ReplResult::Print(text) if text.contains("Already at the first page")
        ));

        let result = execute_repl_builtin(
            &mut state,
            &history,
            "next",
            parse_repl_builtin("next")
                .expect("next should parse")
                .expect("next should classify"),
            &mut sink,
        )
        .expect("last-page boundary should be handled");
        assert!(matches!(
            result,
            ReplResult::Print(text) if text.contains("Already at the last page")
        ));
    }

    #[test]
    fn pagination_builtin_replays_only_the_trusted_native_argv_unit() {
        let mut state = app_state_with_registry(
            crate::native::NativeCommandRegistry::new().with_command(PageCommand),
        );
        let history = history();
        let mut sink = crate::app::sink::BufferedUiSink::default();
        state
            .session
            .native_context
            .set_pagination(crate::native::NativePagination {
                current: vec!["page".to_string()],
                next: Some(vec!["page".to_string(), "next page".to_string()]),
            });

        let result = execute_repl_builtin(
            &mut state,
            &history,
            "next",
            parse_repl_builtin("next")
                .expect("next should parse")
                .expect("next should classify"),
            &mut sink,
        )
        .expect("trusted continuation should dispatch");
        assert!(matches!(
            result,
            ReplResult::Print(text) if text.contains("page command received: next page")
        ));
        assert_eq!(state.session.native_context.pagination(), None);
    }
}
