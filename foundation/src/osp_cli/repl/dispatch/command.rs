use std::borrow::Cow;

use miette::{Result, miette};
use crate::osp_repl::{ReplLineResult, ReplReloadKind, SharedHistory};
use crate::osp_ui::{render_document, render_output};

use crate::osp_cli::app;
use crate::osp_cli::app::{
    CMD_CONFIG, CMD_DOCTOR, CMD_HELP, CMD_HISTORY, CMD_PLUGINS, EffectiveInvocation,
    ReplCommandSpec,
};
use crate::osp_cli::cli::{
    Commands, ConfigArgs, ConfigCommands, DoctorCommands, HistoryCommands, PluginsCommands,
    ThemeArgs, ThemeCommands, parse_inline_command_tokens,
};
use crate::osp_cli::invocation::{append_invocation_help_if_verbose, scan_command_tokens};
use crate::osp_cli::rows::output::output_to_rows;
use crate::osp_cli::state::{AppClients, AppRuntime, AppSession};
use crate::osp_cli::ui_sink::UiSink;

use crate::osp_cli::repl::{ReplViewContext, completion, help, input};

pub(super) struct ParsedReplInvocation {
    pub(super) command: Commands,
    pub(super) effective: EffectiveInvocation,
    pub(super) stages: Vec<String>,
    pub(super) cache_key: Option<String>,
    pub(super) side_effects: CommandSideEffects,
}

pub(super) enum ParsedReplDispatch {
    Help {
        rendered: String,
        effective: Box<EffectiveInvocation>,
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
    parsed: &input::ReplParsedLine,
) -> Result<ParsedReplDispatch> {
    let prefixed_tokens = parsed.prefixed_tokens(&session.scope);
    let scanned = scan_command_tokens(&prefixed_tokens)?;
    let command_index = session.scope.commands().len();
    if scanned.tokens.get(command_index).map(String::as_str) == Some(CMD_HELP)
        && !input::has_valid_help_alias_target(&scanned.tokens, command_index)
    {
        return Ok(ParsedReplDispatch::Help {
            rendered: render_invalid_help_alias(
                ReplViewContext::from_parts(runtime, session),
                input::help_alias_target_at(&scanned.tokens, command_index),
            ),
            effective: Box::new(app::resolve_effective_invocation(
                &runtime.ui,
                &scanned.invocation,
            )),
        });
    }
    let scoped_tokens = input::rewrite_help_alias_tokens_at(&scanned.tokens, command_index)
        .unwrap_or_else(|| scanned.tokens.clone());
    let command = match parse_inline_command_tokens(&scoped_tokens) {
        Ok(Some(command)) => command,
        Ok(None) => return Err(miette!("missing command")),
        Err(err) => {
            if renders_repl_inline_help(err.kind()) {
                let rendered = render_repl_parse_help(
                    ReplViewContext::from_parts(runtime, session),
                    &scanned.invocation,
                    &err.to_string(),
                );
                return Ok(ParsedReplDispatch::Help {
                    rendered,
                    effective: Box::new(app::resolve_effective_invocation(
                        &runtime.ui,
                        &scanned.invocation,
                    )),
                });
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
            effective: app::resolve_effective_invocation(&runtime.ui, &scanned.invocation),
            side_effects: command_side_effects(&command),
            command,
            stages: parsed.stages.clone(),
        },
    )))
}

fn render_invalid_help_alias(view: ReplViewContext<'_>, target: Option<&str>) -> String {
    let mut out = String::new();
    let detail = match target {
        Some(target) => format!("invalid help target: `{target}`"),
        None => "help expects a command target".to_string(),
    };
    out.push_str(&detail);
    out.push_str("\n\n");
    out.push_str(&help::render_repl_help_with_chrome(
        view,
        "Usage: help <command>\n\nNotes:\n  Use bare `help` for the REPL overview.\n  `help help` and `help --help` are not valid.\n  Add -v to include common invocation options.\n",
    ));
    out
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

fn render_repl_parse_help(
    view: ReplViewContext<'_>,
    invocation: &crate::osp_cli::invocation::InvocationOptions,
    error_text: &str,
) -> String {
    let parsed = parse_clap_help(error_text);
    let mut out = String::new();
    if let Some(summary) = parsed.summary {
        out.push_str(summary);
        out.push_str("\n\n");
    }
    out.push_str(&help::render_repl_help_with_chrome(
        view,
        &append_invocation_help_if_verbose(parsed.body, invocation),
    ));
    out
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
    invocation: &crate::osp_cli::invocation::InvocationOptions,
) -> Option<String> {
    if !invocation.cache {
        return None;
    }

    let Commands::External(tokens) = command else {
        return None;
    };

    let provider = invocation.plugin_provider.as_deref().unwrap_or_default();
    let encoded_tokens =
        serde_json::to_string(tokens).expect("external command tokens should serialize");

    Some(format!(
        "rev:{}|profile:{}|provider:{}|tokens:{}",
        runtime.config.revision(),
        runtime.config.resolved().active_profile(),
        provider,
        encoded_tokens
    ))
}

pub(super) fn command_side_effects(command: &Commands) -> CommandSideEffects {
    match command {
        Commands::Theme(ThemeArgs {
            command: ThemeCommands::Use(_),
        }) => CommandSideEffects {
            restart_repl: true,
            show_intro_on_reload: true,
        },
        Commands::Config(ConfigArgs {
            command: ConfigCommands::Set(set),
        }) if !set.dry_run => CommandSideEffects {
            restart_repl: true,
            show_intro_on_reload: config_key_change_requires_intro(&set.key),
        },
        Commands::Config(ConfigArgs {
            command: ConfigCommands::Unset(unset),
        }) if !unset.dry_run => CommandSideEffects {
            restart_repl: true,
            show_intro_on_reload: config_key_change_requires_intro(&unset.key),
        },
        _ => CommandSideEffects::default(),
    }
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
    result: crate::osp_cli::app::CliCommandResult,
    invocation: &EffectiveInvocation,
    sink: &mut dyn UiSink,
) -> Result<String> {
    let crate::osp_cli::app::CliCommandResult {
        exit_code,
        messages,
        output,
        stderr_text,
        failure_report,
        ..
    } = result;

    if exit_code != 0
        && let Some(report) = failure_report
    {
        return Err(miette!("{report}"));
    }

    if !messages.is_empty() {
        app::emit_messages_for_ui(
            runtime.config.resolved(),
            &invocation.ui,
            &messages,
            invocation.ui.message_verbosity,
            sink,
        );
    }

    let rendered = match output {
        Some(crate::osp_cli::app::ReplCommandOutput::Output {
            output,
            format_hint,
        }) => {
            let (output, format_hint) = app::apply_output_stages(output, stages, format_hint)
                .map_err(|err| miette!("{err:#}"))?;

            let render_settings =
                app::resolve_effective_render_settings(&invocation.ui.render_settings, format_hint);
            let rendered = render_output(&output, &render_settings);
            session.record_result(line, output_to_rows(&output));
            app::maybe_copy_output_with_runtime(
                &app::CommandRenderRuntime::new(runtime.config.resolved(), &invocation.ui),
                &output,
                sink,
            );
            rendered
        }
        Some(crate::osp_cli::app::ReplCommandOutput::Document(document)) => {
            render_document(&document, &invocation.ui.render_settings)
        }
        Some(crate::osp_cli::app::ReplCommandOutput::Text(text)) => text,
        None => String::new(),
    };

    if let Some(stderr_text) = stderr_text
        && !stderr_text.is_empty()
    {
        sink.write_stderr(&stderr_text);
    }

    Ok(rendered)
}

pub(super) fn finalize_repl_command(
    session: &AppSession,
    rendered: String,
    restart_repl: bool,
    show_intro_on_reload: bool,
) -> ReplLineResult {
    session.sync_history_shell_context();
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

pub(super) fn run_repl_command(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    command: Commands,
    invocation: &EffectiveInvocation,
    history: &SharedHistory,
    cache_key: Option<&str>,
) -> Result<crate::osp_cli::app::CliCommandResult> {
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
        builtin => app::dispatch_builtin_command_parts(
            runtime,
            session,
            clients,
            Some(history),
            Some(invocation),
            builtin,
        )
        .and_then(|result| result.ok_or_else(|| miette!("expected builtin command"))),
    }?;

    if let Some(cache_key) = cache_key
        && result.exit_code == 0
        && matches!(
            &result.output,
            Some(crate::osp_cli::app::ReplCommandOutput::Output { .. })
        )
    {
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
    invocation: &EffectiveInvocation,
) -> Result<crate::osp_cli::app::CliCommandResult> {
    let resolved = invocation.ui.render_settings.resolve_render_settings();
    let layout = crate::osp_cli::ui_presentation::effective_help_layout(runtime.config.resolved());
    app::run_external_command_with_help_renderer(
        runtime,
        session,
        clients,
        &tokens,
        invocation,
        |stdout| help::render_help_with_chrome(stdout, &resolved, layout),
    )
}

pub(crate) fn repl_command_spec(command: &Commands) -> ReplCommandSpec {
    match command {
        Commands::External(tokens) => ReplCommandSpec {
            name: Cow::Owned(
                tokens
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "external".to_string()),
            ),
            supports_dsl: true,
        },
        Commands::Plugins(args) => ReplCommandSpec {
            name: Cow::Borrowed(CMD_PLUGINS),
            supports_dsl: matches!(
                args.command,
                PluginsCommands::List
                    | PluginsCommands::Commands
                    | PluginsCommands::Doctor
                    | PluginsCommands::Config(_)
            ),
        },
        Commands::Doctor(args) => ReplCommandSpec {
            name: Cow::Borrowed(CMD_DOCTOR),
            supports_dsl: matches!(
                args.command,
                Some(DoctorCommands::Config)
                    | Some(DoctorCommands::Plugins)
                    | Some(DoctorCommands::Theme)
            ),
        },
        Commands::History(args) => ReplCommandSpec {
            name: Cow::Borrowed(CMD_HISTORY),
            supports_dsl: matches!(args.command, HistoryCommands::List),
        },
        Commands::Config(args) => ReplCommandSpec {
            name: Cow::Borrowed(CMD_CONFIG),
            supports_dsl: matches!(
                args.command,
                ConfigCommands::Show(_) | ConfigCommands::Get(_) | ConfigCommands::Doctor
            ),
        },
        Commands::Theme(args) => ReplCommandSpec {
            name: Cow::Borrowed("theme"),
            supports_dsl: matches!(args.command, ThemeCommands::List | ThemeCommands::Show(_)),
        },
        Commands::Repl(_) => ReplCommandSpec {
            name: Cow::Borrowed("repl"),
            supports_dsl: false,
        },
    }
}
