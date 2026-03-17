use std::borrow::Cow;
use std::time::Instant;

use crate::repl::{ReplLineResult, ReplReloadKind, SharedHistory};
use miette::{Result, miette};

use crate::app;
use crate::app::sink::UiSink;
use crate::app::{AppClients, AppRuntime, AppSession};
use crate::app::{
    CMD_CONFIG, CMD_DOCTOR, CMD_HELP, CMD_HISTORY, CMD_INTRO, CMD_PLUGINS, ReplCommandSpec,
    ResolvedInvocation,
};
use crate::cli::invocation::{extend_with_invocation_help, scan_command_tokens};
use crate::cli::{
    Commands, ConfigCommands, DoctorCommands, HistoryCommands, PluginCommandClearArgs,
    PluginCommandStateArgs, PluginProviderClearArgs, PluginProviderSelectArgs, PluginsCommands,
    ThemeCommands, parse_inline_command_tokens,
};
use crate::guide::{GuideSection, GuideSectionKind, GuideView};

use crate::repl::{completion, input};

#[derive(Debug)]
pub(super) struct ParsedReplInvocation {
    pub(super) command: Commands,
    pub(super) effective: ResolvedInvocation,
    pub(super) stages: Vec<String>,
    pub(super) cache_key: Option<String>,
    pub(super) side_effects: CommandSideEffects,
}

#[derive(Debug)]
pub(super) enum ParsedReplDispatch {
    Help {
        result: Box<crate::app::CliCommandResult>,
        effective: Box<ResolvedInvocation>,
        stages: Vec<String>,
    },
    Invocation(Box<ParsedReplInvocation>),
}

pub(super) struct ExecutedReplCommand {
    pub(super) result: ReplLineResult,
    pub(super) debug_verbosity: u8,
    pub(super) execute_finished: Option<Instant>,
}

impl ExecutedReplCommand {
    fn parse_only(result: ReplLineResult, debug_verbosity: u8) -> Self {
        Self {
            result,
            debug_verbosity,
            execute_finished: None,
        }
    }

    fn invocation(result: ReplLineResult, debug_verbosity: u8, execute_finished: Instant) -> Self {
        Self {
            result,
            debug_verbosity,
            execute_finished: Some(execute_finished),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct CommandSideEffects {
    pub(super) restart_repl: bool,
    pub(super) show_intro_on_reload: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ParsedClapHelp<'a> {
    pub(super) summary: Option<&'a str>,
    pub(super) body: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReplCommandBehavior<'a> {
    name: Cow<'a, str>,
    supports_dsl: bool,
    side_effects: CommandSideEffects,
}

pub(super) fn parse_repl_invocation(
    runtime: &AppRuntime,
    session: &AppSession,
    parsed: &input::ReplParsedLine,
) -> Result<ParsedReplDispatch> {
    let prefixed_tokens = parsed.prefixed_tokens(&session.scope);
    let scanned = scan_command_tokens(&prefixed_tokens)?;
    let effective =
        app::resolve_invocation_ui(runtime.config.resolved(), &runtime.ui, &scanned.invocation);
    let command_index = session.scope.commands().len();
    // `help` is a REPL alias layered on top of shell scope. Handle it before
    // clap parsing so `help user` inside `ldap` resolves as scoped inline help
    // instead of a normal `help` subcommand invocation.
    if scanned.tokens.get(command_index).map(String::as_str) == Some(CMD_HELP)
        && !input::has_valid_help_alias_target(&scanned.tokens, command_index)
    {
        if !parsed.stages.is_empty() {
            let detail = match input::help_alias_target_at(&scanned.tokens, command_index) {
                Some(target) => format!("invalid help target: {target}"),
                None => "help expects a command target".to_string(),
            };
            return Err(miette!("{detail}"));
        }
        return Ok(repl_help_dispatch(
            parsed,
            &effective,
            crate::app::CliCommandResult::guide(
                render_invalid_help_alias(input::help_alias_target_at(
                    &scanned.tokens,
                    command_index,
                ))
                .filtered_for_help_level(effective.help_level),
            ),
        ));
    }
    // Rewrite scoped help aliases back into ordinary command tokens before
    // clap parsing so the rest of dispatch only has to reason about one shape.
    let scoped_tokens = input::rewrite_help_alias_tokens_at(&scanned.tokens, command_index)
        .unwrap_or_else(|| scanned.tokens.clone());
    let command = match parse_inline_command_tokens(&scoped_tokens) {
        Ok(Some(command)) => command,
        Ok(None) => return Err(miette!("missing command")),
        Err(err) => {
            if renders_repl_inline_help(err.kind()) {
                if !parsed.stages.is_empty() {
                    return Err(miette!(err.to_string()));
                }
                return Ok(repl_help_dispatch(
                    parsed,
                    &effective,
                    crate::app::CliCommandResult::guide(render_repl_parse_help(
                        effective.help_level,
                        &err.to_string(),
                    )),
                ));
            }
            return Err(miette!(err.to_string()));
        }
    };
    let spec = repl_command_spec(&command);
    app::ensure_command_supports_dsl(&spec, &parsed.stages)?;
    if !parsed.stages.is_empty() {
        completion::validate_dsl_stages(&parsed.stages)?;
    }

    Ok(ParsedReplDispatch::Invocation(Box::new(
        ParsedReplInvocation {
            cache_key: repl_cache_key_for_command(runtime, &command, &scanned.invocation),
            effective,
            side_effects: command_side_effects(&command),
            command,
            stages: parsed.stages.clone(),
        },
    )))
}

fn repl_help_dispatch(
    parsed: &input::ReplParsedLine,
    effective: &ResolvedInvocation,
    result: crate::app::CliCommandResult,
) -> ParsedReplDispatch {
    ParsedReplDispatch::Help {
        result: Box::new(result),
        effective: Box::new(effective.clone()),
        stages: parsed.stages.clone(),
    }
}

fn render_invalid_help_alias(target: Option<&str>) -> GuideView {
    let detail = match target {
        Some(target) => format!("invalid help target: {target}"),
        None => "help expects a command target".to_string(),
    };
    GuideView {
        preamble: vec![detail],
        usage: Vec::new(),
        commands: Vec::new(),
        arguments: Vec::new(),
        options: Vec::new(),
        common_invocation_options: Vec::new(),
        notes: Vec::new(),
        sections: vec![
            GuideSection::new("Usage", GuideSectionKind::Usage).paragraph("help <command>"),
            GuideSection::new("Notes", GuideSectionKind::Notes)
                .paragraph("  Use bare help for the REPL overview.")
                .paragraph("  help help and help --help are not valid.")
                .paragraph("  Add -v to include common invocation options."),
        ],
        epilogue: Vec::new(),
    }
}

pub(super) fn renders_repl_inline_help(kind: clap::error::ErrorKind) -> bool {
    matches!(
        kind,
        clap::error::ErrorKind::DisplayHelp
            | clap::error::ErrorKind::DisplayVersion
            | clap::error::ErrorKind::InvalidSubcommand
            | clap::error::ErrorKind::UnknownArgument
            | clap::error::ErrorKind::MissingRequiredArgument
    )
}

fn render_repl_parse_help(help_level: crate::guide::HelpLevel, error_text: &str) -> GuideView {
    let parsed = parse_clap_help(error_text);
    let mut view = GuideView::from_text(parsed.body);
    extend_with_invocation_help(&mut view, help_level);
    if let Some(summary) = parsed.summary {
        view.preamble.insert(0, summary.to_string());
    }
    view.filtered_for_help_level(help_level)
}

pub(super) fn parse_clap_help(error_text: &str) -> ParsedClapHelp<'_> {
    let lines = error_text.lines().collect::<Vec<_>>();
    let summary = lines
        .iter()
        .map(|line| line.trim())
        .find_map(|line| line.strip_prefix("error:").map(str::trim));

    let body_start = lines
        .iter()
        .position(|line| line.trim_start().starts_with("Usage:"))
        .unwrap_or(0);
    let mut body_end = lines.len();
    while body_end > body_start {
        let trimmed = lines[body_end - 1].trim();
        if trimmed.is_empty() {
            body_end -= 1;
            continue;
        }
        if trimmed.starts_with("tip:") || trimmed.starts_with("For more information") {
            body_end -= 1;
            continue;
        }
        break;
    }

    let body = if body_start < body_end {
        &error_text[line_start_offset(&lines, body_start)..line_end_offset(&lines, body_end)]
    } else {
        ""
    };

    ParsedClapHelp { summary, body }
}

fn line_start_offset(lines: &[&str], line_index: usize) -> usize {
    lines
        .iter()
        .take(line_index)
        .map(|line| line.len() + 1)
        .sum()
}

fn line_end_offset(lines: &[&str], line_count: usize) -> usize {
    let mut offset = line_start_offset(lines, line_count);
    if line_count > 0 {
        offset = offset.saturating_sub(1);
    }
    offset
}

fn repl_cache_key_for_command(
    runtime: &AppRuntime,
    command: &Commands,
    invocation: &crate::cli::invocation::InvocationOptions,
) -> Option<String> {
    if !invocation.cache {
        return None;
    }

    let Commands::External(tokens) = command else {
        return None;
    };

    let provider = invocation.plugin_provider.as_deref().unwrap_or_default();

    // Cache entries are tied to config revision and active profile so REPL
    // `--cache` never replays output across theme/profile/provider changes that
    // could make the command semantically different.
    Some(format!(
        "rev:{}|profile:{}|provider:{}|tokens:{}",
        runtime.config.revision(),
        runtime.config.resolved().active_profile(),
        provider,
        encode_cache_key_tokens(tokens)
    ))
}

fn encode_cache_key_tokens(tokens: &[String]) -> String {
    let mut encoded = String::new();
    for token in tokens {
        encoded.push_str(&token.len().to_string());
        encoded.push(':');
        encoded.push_str(token);
        encoded.push('|');
    }
    encoded
}

pub(super) fn command_side_effects(command: &Commands) -> CommandSideEffects {
    repl_command_behavior(command).side_effects
}

pub(super) fn config_key_change_requires_intro(key: &str) -> bool {
    let key = key.trim().to_ascii_lowercase();
    key == "theme.name"
        || key.starts_with("theme.")
        || key.starts_with("color.")
        || key.starts_with("palette.")
}

pub(super) fn render_repl_command_output(
    runtime: &AppRuntime,
    session: &mut AppSession,
    line: &str,
    stages: &[String],
    result: crate::app::CliCommandResult,
    invocation: &ResolvedInvocation,
    sink: &mut dyn UiSink,
) -> Result<String> {
    app::render_repl_command_with_runtime(
        &app::CommandRenderRuntime::new(runtime.config.resolved(), &invocation.ui),
        session,
        line,
        stages,
        result,
        sink,
    )
    .map_err(|err| miette!("{err:#}"))
}

pub(super) fn execute_repl_command_dispatch(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    history: Option<&SharedHistory>,
    line: &str,
    dispatch: ParsedReplDispatch,
    sink: &mut dyn UiSink,
) -> Result<ExecutedReplCommand> {
    match dispatch {
        ParsedReplDispatch::Help {
            result,
            effective,
            stages,
        } => {
            let rendered = render_repl_command_output(
                runtime, session, line, &stages, *result, &effective, sink,
            )?;
            Ok(ExecutedReplCommand::parse_only(
                ReplLineResult::Continue(rendered),
                effective.ui.debug_verbosity,
            ))
        }
        ParsedReplDispatch::Invocation(invocation) => {
            let ParsedReplInvocation {
                command,
                effective,
                stages,
                cache_key,
                side_effects,
            } = *invocation;
            let history = history.ok_or_else(|| {
                miette!("REPL command execution requires history when running an invocation")
            })?;
            let output = run_repl_command(
                runtime,
                session,
                clients,
                command,
                &effective,
                history,
                cache_key.as_deref(),
            )?;
            let execute_finished = Instant::now();
            let rendered = render_repl_command_output(
                runtime, session, line, &stages, output, &effective, sink,
            )?;
            Ok(ExecutedReplCommand::invocation(
                finalize_repl_command(
                    rendered,
                    side_effects.restart_repl,
                    side_effects.show_intro_on_reload,
                ),
                effective.ui.debug_verbosity,
                execute_finished,
            ))
        }
    }
}

pub(super) fn finalize_repl_command(
    rendered: String,
    restart_repl: bool,
    show_intro_on_reload: bool,
) -> ReplLineResult {
    if restart_repl {
        tracing::debug!(
            show_intro = show_intro_on_reload,
            "REPL restart requested (config/theme change)"
        );
        ReplLineResult::Restart {
            output: rendered,
            reload: if show_intro_on_reload {
                ReplReloadKind::WithIntro
            } else {
                ReplReloadKind::Default
            },
        }
    } else {
        ReplLineResult::Continue(rendered)
    }
}

#[cfg(test)]
mod tests;

pub(super) fn run_repl_command(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    command: Commands,
    invocation: &ResolvedInvocation,
    history: &SharedHistory,
    cache_key: Option<&str>,
) -> Result<crate::app::CliCommandResult> {
    if let Some(cache_key) = cache_key
        && let Some(cached) = session.cached_command(cache_key)
    {
        tracing::trace!(cache_key = %cache_key, "REPL command cache hit");
        return Ok(cached);
    }

    let result = match command {
        Commands::External(tokens) => {
            run_repl_external_command(runtime, clients, session, tokens, invocation)
        }
        builtin => {
            app::run_repl_builtin_command(runtime, session, clients, history, invocation, builtin)
        }
    }?;

    if let Some(cache_key) = cache_key
        && result.exit_code == 0
        && matches!(
            &result.output,
            Some(crate::app::ReplCommandOutput::Output(_))
        )
    {
        // Only cache successful structured payloads. Text/help/error output is
        // cheap to recompute and too presentation-dependent to be a good cache
        // contract for `--cache`.
        tracing::trace!(cache_key = %cache_key, "REPL command cached");
        session.record_cached_command(cache_key, &result);
    }

    Ok(result)
}

pub(super) fn run_repl_external_command(
    runtime: &mut AppRuntime,
    clients: &AppClients,
    session: &mut AppSession,
    tokens: Vec<String>,
    invocation: &ResolvedInvocation,
) -> Result<crate::app::CliCommandResult> {
    app::run_external_command_with_help_renderer(
        runtime,
        session,
        clients,
        &tokens,
        invocation,
        GuideView::from_text,
    )
}

pub(crate) fn repl_command_spec(command: &Commands) -> ReplCommandSpec {
    let behavior = repl_command_behavior(command);
    ReplCommandSpec {
        name: Cow::Owned(behavior.name.into_owned()),
        supports_dsl: behavior.supports_dsl,
    }
}

fn repl_command_behavior(command: &Commands) -> ReplCommandBehavior<'_> {
    match command {
        Commands::External(tokens) => ReplCommandBehavior {
            name: Cow::Owned(
                tokens
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "external".to_string()),
            ),
            supports_dsl: true,
            side_effects: CommandSideEffects::default(),
        },
        Commands::Plugins(args) => ReplCommandBehavior {
            name: Cow::Borrowed(CMD_PLUGINS),
            supports_dsl: matches!(
                args.command,
                PluginsCommands::List
                    | PluginsCommands::Commands
                    | PluginsCommands::Doctor
                    | PluginsCommands::Config(_)
            ),
            side_effects: if matches!(
                args.command,
                PluginsCommands::Enable(PluginCommandStateArgs { .. })
                    | PluginsCommands::Disable(PluginCommandStateArgs { .. })
                    | PluginsCommands::ClearState(PluginCommandClearArgs { .. })
                    | PluginsCommands::Refresh
                    | PluginsCommands::SelectProvider(PluginProviderSelectArgs { .. })
                    | PluginsCommands::ClearProvider(PluginProviderClearArgs { .. })
            ) {
                CommandSideEffects {
                    restart_repl: true,
                    show_intro_on_reload: false,
                }
            } else {
                CommandSideEffects::default()
            },
        },
        Commands::Doctor(args) => ReplCommandBehavior {
            name: Cow::Borrowed(CMD_DOCTOR),
            supports_dsl: matches!(
                args.command,
                Some(DoctorCommands::Config)
                    | Some(DoctorCommands::Plugins)
                    | Some(DoctorCommands::Theme)
            ),
            side_effects: CommandSideEffects::default(),
        },
        Commands::History(args) => ReplCommandBehavior {
            name: Cow::Borrowed(CMD_HISTORY),
            supports_dsl: matches!(args.command, HistoryCommands::List),
            side_effects: CommandSideEffects::default(),
        },
        Commands::Config(args) => ReplCommandBehavior {
            name: Cow::Borrowed(CMD_CONFIG),
            supports_dsl: matches!(
                args.command,
                ConfigCommands::Show(_) | ConfigCommands::Get(_) | ConfigCommands::Doctor
            ),
            side_effects: match &args.command {
                ConfigCommands::Set(set) if !set.dry_run => CommandSideEffects {
                    restart_repl: true,
                    show_intro_on_reload: config_key_change_requires_intro(&set.key),
                },
                ConfigCommands::Unset(unset) if !unset.dry_run => CommandSideEffects {
                    restart_repl: true,
                    show_intro_on_reload: config_key_change_requires_intro(&unset.key),
                },
                _ => CommandSideEffects::default(),
            },
        },
        Commands::Theme(args) => ReplCommandBehavior {
            name: Cow::Borrowed("theme"),
            supports_dsl: matches!(args.command, ThemeCommands::List | ThemeCommands::Show(_)),
            side_effects: if matches!(args.command, ThemeCommands::Use(_)) {
                CommandSideEffects {
                    restart_repl: true,
                    show_intro_on_reload: true,
                }
            } else {
                CommandSideEffects::default()
            },
        },
        Commands::Intro(_) => ReplCommandBehavior {
            name: Cow::Borrowed(CMD_INTRO),
            supports_dsl: true,
            side_effects: CommandSideEffects::default(),
        },
        Commands::Repl(_) => ReplCommandBehavior {
            name: Cow::Borrowed("repl"),
            supports_dsl: false,
            side_effects: CommandSideEffects::default(),
        },
    }
}
