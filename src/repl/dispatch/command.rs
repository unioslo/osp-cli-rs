use std::time::Instant;

use crate::repl::{ReplReloadKind, ReplResult, SharedHistory};
use miette::{Result, miette};

use crate::app;
use crate::app::sink::UiSink;
use crate::app::{AppRuntime, AppSession};
use crate::app::{CMD_HELP, ResolvedInvocation};
use crate::cli::invocation::{extend_with_invocation_help, scan_command_tokens};
use crate::cli::{Commands, ConfigCommands, ParsedCommandLine, PluginsCommands, ThemeCommands};
use crate::core::document::{DocBlock, Document};

use crate::repl::input;

pub(super) struct ParsedReplInvocation {
    pub(super) command: Commands,
    pub(super) effective: ResolvedInvocation,
    pub(super) stages: Vec<String>,
    pub(super) side_effects: CommandSideEffects,
}

pub(super) enum ParsedReplDispatch {
    Help {
        result: Box<crate::app::CommandOutput>,
        effective: Box<ResolvedInvocation>,
        stages: Vec<String>,
    },
    Invocation(Box<ParsedReplInvocation>),
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

pub(super) fn parse_repl_invocation(
    runtime: &AppRuntime,
    session: &AppSession,
    native_commands: &crate::NativeCommandRegistry,
    parsed: &input::ReplParsedLine,
) -> Result<ParsedReplDispatch> {
    let mut scanned = scan_command_tokens(&parsed.dispatch_tokens)?;
    let elevated = scanned.tokens.first().map(String::as_str) == Some("sudo");
    let prefix_len = usize::from(elevated);
    let command_tokens = &scanned.tokens[prefix_len..];
    let scope = session.scope.commands();
    let absolute_command = command_tokens.first().is_some_and(|command| {
        input::REPL_GLOBAL_COMMANDS.contains(&command.as_str())
            || (elevated
                && (scope.first() == Some(command)
                    || crate::repl::is_repl_shellable_command(runtime.config.resolved(), command)))
    });
    // Elevation wraps the resolved command; it must not discard shell scope.
    // Keep bare sudo untouched so the ordinary missing-command error applies.
    let scoped = !(absolute_command || elevated && command_tokens.is_empty());
    if scoped {
        let prefixed = session.scope.prefixed_tokens(command_tokens);
        scanned.tokens.splice(prefix_len.., prefixed);
    }
    let mut effective =
        app::resolve_invocation_ui(runtime.config.resolved(), &runtime.ui, &scanned.invocation);
    let command_index = prefix_len + if scoped { scope.len() } else { 0 };
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
            crate::app::CommandOutput::document(
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
    let prefixes =
        crate::command_words::CommandPrefixes::from_config(runtime.config.resolved()).enabled(true);
    let path = app::command_plan::canonical_builtin_path(prefixes, &scoped_tokens)?;
    effective.ui.render_settings.command_path = path;
    let command = match app::command_plan::parse_inline_command(
        &scoped_tokens,
        app::command_plan::CommandSurface::Repl,
        crate::command_words::CommandPrefixes::from_config(runtime.config.resolved()).enabled(true),
    )? {
        app::command_plan::InlineCommandPlan::Command(Some(command)) => command,
        app::command_plan::InlineCommandPlan::Command(None) => {
            return Err(miette!("missing command"));
        }
        app::command_plan::InlineCommandPlan::Help(err) => {
            let (document, long_help) = render_repl_parse_help(
                effective.help_level,
                &scoped_tokens,
                &err.to_string(),
                native_commands,
                &effective.ui.render_settings.runtime.command_scope,
            );
            let mut output = document.to_output_result();
            output.meta.document_long_help = long_help;
            return Ok(repl_help_dispatch(
                parsed,
                &effective,
                crate::app::CommandOutput::output(output, None),
            ));
        }
    };
    app::command_plan::validate_command_pipeline(&command, &parsed.stages)?;

    Ok(ParsedReplDispatch::Invocation(Box::new(
        ParsedReplInvocation {
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
    result: crate::app::CommandOutput,
) -> ParsedReplDispatch {
    ParsedReplDispatch::Help {
        result: Box::new(result),
        effective: Box::new(effective.clone()),
        stages: parsed.stages.clone(),
    }
}

fn render_invalid_help_alias(target: Option<&str>) -> Document {
    let detail = match target {
        Some(target) => format!("invalid help target: {target}"),
        None => "help expects a command target".to_string(),
    };
    Document { blocks: vec![DocBlock::Paragraph { text: detail },
        DocBlock::section("Usage", vec![DocBlock::Paragraph { text: "help <command>".into() }]),
        DocBlock::section("Notes", vec![DocBlock::Paragraph { text:
            "Use bare help for the REPL overview.\nhelp help and help --help are not valid.\nAdd -v to include common invocation options.".into() }]),
    ] }
}

fn render_repl_parse_help(
    help_level: crate::core::document::HelpLevel,
    tokens: &[String],
    error_text: &str,
    native_commands: &crate::NativeCommandRegistry,
    scope: &crate::core::CommandScope,
) -> (Document, bool) {
    let parsed = parse_clap_help(error_text);
    use clap::CommandFactory;
    let command = crate::cli::commands::doctor::with_app_commands(
        crate::cli::InlineCommandCli::command(),
        native_commands,
    );
    let (mut view, _, long_help) = crate::cli::command_help_document(command, tokens, scope);
    extend_with_invocation_help(&mut view, help_level);
    if error_text.starts_with("error:")
        && let Some(summary) = parsed.summary
    {
        view.blocks.insert(
            0,
            DocBlock::Paragraph {
                text: summary.into(),
            },
        );
    }
    (view.filtered_for_help_level(help_level), long_help)
}

pub(super) fn parse_clap_help(error_text: &str) -> ParsedClapHelp<'_> {
    let lines = error_text.lines().collect::<Vec<_>>();
    let body_start = lines
        .iter()
        .position(|line| line.trim_start().starts_with("Usage:"))
        .unwrap_or(0);
    let summary = lines
        .iter()
        .map(|line| line.trim())
        .find_map(|line| line.strip_prefix("error:").map(str::trim))
        .or_else(|| {
            lines[..body_start]
                .iter()
                .map(|line| line.trim())
                .find(|line| !line.is_empty())
        });
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

pub(super) fn command_side_effects(command: &Commands) -> CommandSideEffects {
    let show_intro_on_reload = match command {
        Commands::Config(args) => match &args.command {
            ConfigCommands::Edit(_) => false,
            ConfigCommands::Set(set) if !set.dry_run => config_key_change_requires_intro(&set.key),
            ConfigCommands::Unset(unset) if !unset.dry_run => {
                config_key_change_requires_intro(&unset.key)
            }
            _ => return CommandSideEffects::default(),
        },
        Commands::Plugins(args)
            if matches!(
                args.command,
                PluginsCommands::Enable { .. }
                    | PluginsCommands::Disable { .. }
                    | PluginsCommands::Refresh
            ) =>
        {
            false
        }
        Commands::Alias(args)
            if matches!(
                args.command,
                crate::cli::AliasCommands::Add(crate::cli::AliasAddArgs { dry_run: false, .. })
                    | crate::cli::AliasCommands::Remove(crate::cli::AliasRemoveArgs {
                        dry_run: false,
                        ..
                    })
            ) =>
        {
            false
        }
        Commands::Theme(crate::cli::ThemeArgs {
            command: ThemeCommands::Reset,
        }) => false,
        Commands::Theme(args)
            if matches!(
                args.command,
                ThemeCommands::Use(_) | ThemeCommands::Preview(_)
            ) =>
        {
            true
        }
        _ => return CommandSideEffects::default(),
    };
    CommandSideEffects {
        restart_repl: true,
        show_intro_on_reload,
    }
}

pub(super) fn config_key_change_requires_intro(key: &str) -> bool {
    let key = key.trim().to_ascii_lowercase();
    key == "theme.name" || key.starts_with("theme.") || key.starts_with("palette.")
}

pub(super) fn render_repl_command_output(
    runtime: &AppRuntime,
    session: &mut AppSession,
    line: &str,
    stages: &[String],
    result: crate::app::CommandOutput,
    invocation: &ResolvedInvocation,
    sink: &mut dyn UiSink,
) -> Result<String> {
    app::render_repl_command(
        runtime.config.resolved(),
        &invocation.ui,
        session,
        line,
        stages,
        result,
        sink,
    )
    .map_err(|err| {
        crate::app::report_report_with_context(
            err,
            crate::app::host::ErrorContext::Operation("failed to render REPL command output"),
        )
    })
}

pub(super) fn execute_repl_command_dispatch(
    ctx: super::ReplExecutionContext<'_, '_>,
    line: &str,
    dispatch: ParsedReplDispatch,
    parse_finished: Instant,
) -> Result<super::ExecutedReplLine> {
    let super::ReplExecutionContext {
        state,
        history,
        sink,
        accepted,
    } = ctx;
    match dispatch {
        ParsedReplDispatch::Help {
            result,
            effective,
            stages,
        } => {
            *accepted = true;
            let rendered = render_repl_command_output(
                &state.runtime,
                &mut state.session,
                line,
                &stages,
                *result,
                &effective,
                sink,
            )?;
            Ok(super::ExecutedReplLine::parse_only(
                ReplResult::Print(rendered),
                effective.ui.debug_level,
            ))
        }
        ParsedReplDispatch::Invocation(invocation) => {
            let ParsedReplInvocation {
                command,
                effective,
                stages,
                side_effects,
            } = *invocation;
            let picker = matches!(
                &command,
                Commands::Theme(crate::cli::ThemeArgs {
                    command: ThemeCommands::Preview(crate::cli::ThemePreviewArgs { name: None })
                }) | Commands::Config(crate::cli::ConfigArgs {
                    command: ConfigCommands::Edit(_)
                })
            );
            let output = run_repl_command_with_progress(
                state,
                history,
                ReplRunInput {
                    command,
                    invocation: &effective,
                    stages: &stages,
                    progress_sink: Some(sink),
                    accepted,
                },
            )?;
            let execute_finished = Instant::now();
            let restart_repl = side_effects.restart_repl
                && (!picker || !matches!(output.body, crate::app::CommandBody::None));
            let exit_code = output.exit_code;
            let rendered = render_repl_command_output(
                &state.runtime,
                &mut state.session,
                line,
                &stages,
                output,
                &effective,
                sink,
            )?;
            Ok(super::ExecutedReplLine::invocation(
                finalize_repl_command(rendered, restart_repl, side_effects.show_intro_on_reload),
                exit_code,
                effective.ui.debug_level,
                parse_finished,
                execute_finished,
            ))
        }
    }
}

struct ReplRunInput<'invocation, 'sink, 'accepted> {
    command: Commands,
    invocation: &'invocation ResolvedInvocation,
    stages: &'invocation [String],
    progress_sink: Option<&'sink mut dyn UiSink>,
    accepted: &'accepted mut bool,
}

fn run_repl_command_with_progress(
    state: &mut crate::app::AppState,
    history: &SharedHistory,
    input: ReplRunInput<'_, '_, '_>,
) -> Result<crate::app::CommandOutput> {
    let result = match input.command {
        Commands::External(tokens) => run_repl_external_command_with_progress(
            state,
            ParsedCommandLine {
                tokens,
                stages: input.stages.to_vec(),
            },
            input.invocation,
            input.progress_sink,
            input.accepted,
        ),
        builtin => {
            *input.accepted = true;
            app::run_repl_builtin_command(
                &mut state.runtime,
                &mut state.session,
                &state.clients,
                history,
                input.invocation,
                builtin,
            )
        }
    }?;

    Ok(result)
}

pub(super) fn finalize_repl_command(
    rendered: String,
    restart_repl: bool,
    show_intro_on_reload: bool,
) -> ReplResult {
    if restart_repl {
        tracing::debug!(
            show_intro = show_intro_on_reload,
            "REPL restart requested (config/theme change)"
        );
        ReplResult::Restart {
            output: rendered,
            reload: if show_intro_on_reload {
                ReplReloadKind::WithIntro
            } else {
                ReplReloadKind::Default
            },
        }
    } else {
        ReplResult::Print(rendered)
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests;

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
pub(super) fn run_repl_command(
    state: &mut crate::app::AppState,
    command: Commands,
    invocation: &ResolvedInvocation,
    history: &SharedHistory,
) -> Result<crate::app::CommandOutput> {
    let mut accepted = false;
    run_repl_command_with_progress(
        state,
        history,
        ReplRunInput {
            command,
            invocation,
            stages: &[],
            progress_sink: None,
            accepted: &mut accepted,
        },
    )
}

pub(super) fn run_repl_external_command(
    state: &mut crate::app::AppState,
    tokens: Vec<String>,
    invocation: &ResolvedInvocation,
) -> Result<crate::app::CommandOutput> {
    app::run_external_command_with_help_renderer(
        state,
        &tokens,
        invocation,
        Document::from_help_text,
        None,
    )
}

fn run_repl_external_command_with_progress(
    state: &mut crate::app::AppState,
    command: ParsedCommandLine,
    invocation: &ResolvedInvocation,
    progress_sink: Option<&mut dyn UiSink>,
    accepted: &mut bool,
) -> Result<crate::app::CommandOutput> {
    app::external::run_external_command_for_repl(
        state,
        command,
        invocation,
        Document::from_help_text,
        progress_sink,
        accepted,
    )
}
