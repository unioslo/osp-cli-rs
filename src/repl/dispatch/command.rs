use std::borrow::Cow;

use crate::repl::{ReplLineResult, ReplReloadKind, SharedHistory};
use crate::ui::{render_document, render_output};
use miette::{Result, miette};

use crate::app;
use crate::app::sink::UiSink;
use crate::app::{AppClients, AppRuntime, AppSession};
use crate::app::{
    CMD_CONFIG, CMD_DOCTOR, CMD_HELP, CMD_HISTORY, CMD_INTRO, CMD_PLUGINS, ReplCommandSpec,
    ResolvedInvocation,
};
use crate::cli::invocation::{append_invocation_help_if_verbose, scan_command_tokens};
use crate::cli::rows::output::{output_to_rows, rows_to_output_result};
use crate::cli::{
    Commands, ConfigArgs, ConfigCommands, DoctorCommands, HistoryCommands, PluginCommandClearArgs,
    PluginCommandStateArgs, PluginProviderClearArgs, PluginProviderSelectArgs, PluginsCommands,
    ThemeArgs, ThemeCommands, parse_inline_command_tokens,
};
use crate::core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use crate::guide::{GuideDoc, HelpDoc, HelpSection, HelpSectionKind};

use crate::repl::{completion, input};

pub(super) struct ParsedReplInvocation {
    pub(super) command: Commands,
    pub(super) effective: ResolvedInvocation,
    pub(super) stages: Vec<String>,
    pub(super) cache_key: Option<String>,
    pub(super) side_effects: CommandSideEffects,
}

pub(super) enum ParsedReplDispatch {
    Help {
        result: crate::app::CliCommandResult,
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
    parsed: &input::ReplParsedLine,
) -> Result<ParsedReplDispatch> {
    let prefixed_tokens = parsed.prefixed_tokens(&session.scope);
    let scanned = scan_command_tokens(&prefixed_tokens)?;
    let command_index = session.scope.commands().len();
    if scanned.tokens.get(command_index).map(String::as_str) == Some(CMD_HELP)
        && !input::has_valid_help_alias_target(&scanned.tokens, command_index)
    {
        return Ok(ParsedReplDispatch::Help {
            result: crate::app::CliCommandResult::guide(render_invalid_help_alias(
                input::help_alias_target_at(&scanned.tokens, command_index),
            )),
            effective: Box::new(app::resolve_invocation_ui(&runtime.ui, &scanned.invocation)),
            stages: parsed.stages.clone(),
        });
    }
    let scoped_tokens = input::rewrite_help_alias_tokens_at(&scanned.tokens, command_index)
        .unwrap_or_else(|| scanned.tokens.clone());
    let command = match parse_inline_command_tokens(&scoped_tokens) {
        Ok(Some(command)) => command,
        Ok(None) => return Err(miette!("missing command")),
        Err(err) => {
            if renders_repl_inline_help(err.kind()) {
                return Ok(ParsedReplDispatch::Help {
                    result: crate::app::CliCommandResult::guide(render_repl_parse_help(
                        &scanned.invocation,
                        &err.to_string(),
                    )),
                    effective: Box::new(app::resolve_invocation_ui(
                        &runtime.ui,
                        &scanned.invocation,
                    )),
                    stages: parsed.stages.clone(),
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
            effective: app::resolve_invocation_ui(&runtime.ui, &scanned.invocation),
            side_effects: command_side_effects(&command),
            command,
            stages: parsed.stages.clone(),
        },
    )))
}

fn render_invalid_help_alias(target: Option<&str>) -> GuideDoc {
    let detail = match target {
        Some(target) => format!("invalid help target: `{target}`"),
        None => "help expects a command target".to_string(),
    };
    HelpDoc {
        preamble: vec![detail],
        sections: vec![
            HelpSection::new("Usage", HelpSectionKind::Usage).paragraph("  help <command>"),
            HelpSection::new("Notes", HelpSectionKind::Notes)
                .paragraph("  Use bare `help` for the REPL overview.")
                .paragraph("  `help help` and `help --help` are not valid.")
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

fn render_repl_parse_help(
    invocation: &crate::cli::invocation::InvocationOptions,
    error_text: &str,
) -> GuideDoc {
    let parsed = parse_clap_help(error_text);
    let mut doc = HelpDoc::from_text(&append_invocation_help_if_verbose(parsed.body, invocation));
    if let Some(summary) = parsed.summary {
        doc.preamble.insert(0, summary.to_string());
    }
    doc
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
        Commands::Plugins(crate::cli::PluginsArgs {
            command:
                PluginsCommands::Enable(PluginCommandStateArgs { .. })
                | PluginsCommands::Disable(PluginCommandStateArgs { .. })
                | PluginsCommands::ClearState(PluginCommandClearArgs { .. })
                | PluginsCommands::Refresh
                | PluginsCommands::SelectProvider(PluginProviderSelectArgs { .. })
                | PluginsCommands::ClearProvider(PluginProviderClearArgs { .. }),
        }) => CommandSideEffects {
            restart_repl: true,
            show_intro_on_reload: false,
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
    result: crate::app::CliCommandResult,
    invocation: &ResolvedInvocation,
    sink: &mut dyn UiSink,
) -> Result<String> {
    let crate::app::CliCommandResult {
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
        Some(crate::app::ReplCommandOutput::Output {
            output,
            format_hint,
        }) => {
            let (output, format_hint) = app::apply_output_stages(output, stages, format_hint)
                .map_err(|err| miette!("{err:#}"))?;

            let render_settings =
                app::resolve_render_settings_with_hint(&invocation.ui.render_settings, format_hint);
            let rendered = render_output(&output, &render_settings);
            session.record_result(line, output_to_rows(&output));
            app::maybe_copy_output_with_runtime(
                &app::CommandRenderRuntime::new(runtime.config.resolved(), &invocation.ui),
                &output,
                sink,
            );
            rendered
        }
        Some(crate::app::ReplCommandOutput::Guide(guide)) => {
            let output = guide.to_output_result();
            let (output, _format_hint) =
                app::apply_output_stages(output, stages, None).map_err(|err| miette!("{err:#}"))?;
            let rendered = crate::repl::help::render_guide_output(
                &output,
                &invocation.ui.render_settings,
                crate::ui::format::help::GuideRenderOptions {
                    title_prefix: None,
                    layout: crate::ui::presentation::help_layout(runtime.config.resolved()),
                    frame_style: invocation.ui.render_settings.chrome_frame,
                    panel_kind: None,
                },
            );
            session.record_result(line, output_to_rows(&output));
            app::maybe_copy_output_with_runtime(
                &app::CommandRenderRuntime::new(runtime.config.resolved(), &invocation.ui),
                &output,
                sink,
            );
            rendered
        }
        Some(crate::app::ReplCommandOutput::Document(document)) => {
            if stages.is_empty() {
                render_document(&document, &invocation.ui.render_settings)
            } else {
                render_staged_textual_output(
                    runtime,
                    session,
                    line,
                    stages,
                    render_document_for_stages(&document, &invocation.ui.render_settings),
                    invocation,
                    sink,
                )?
            }
        }
        Some(crate::app::ReplCommandOutput::Text(text)) => {
            if stages.is_empty() {
                text
            } else {
                render_staged_textual_output(
                    runtime, session, line, stages, text, invocation, sink,
                )?
            }
        }
        None => String::new(),
    };

    if let Some(stderr_text) = stderr_text
        && !stderr_text.is_empty()
    {
        sink.write_stderr(&stderr_text);
    }

    Ok(rendered)
}

fn render_staged_textual_output(
    runtime: &AppRuntime,
    session: &mut AppSession,
    line: &str,
    stages: &[String],
    text: String,
    invocation: &ResolvedInvocation,
    sink: &mut dyn UiSink,
) -> Result<String> {
    let (output, format_hint) = app::apply_output_stages(
        text_output_to_rows(&text),
        stages,
        Some(OutputFormat::Value),
    )
    .map_err(|err| miette!("{err:#}"))?;

    let render_settings =
        app::resolve_render_settings_with_hint(&invocation.ui.render_settings, format_hint);
    let rendered = render_output(&output, &render_settings);
    session.record_result(line, output_to_rows(&output));
    app::maybe_copy_output_with_runtime(
        &app::CommandRenderRuntime::new(runtime.config.resolved(), &invocation.ui),
        &output,
        sink,
    );
    Ok(rendered)
}

fn text_output_to_rows(text: &str) -> crate::core::output_model::OutputResult {
    rows_to_output_result(
        text.lines()
            .filter(|line| !line.is_empty())
            .map(|line| crate::row! { "value" => line })
            .collect(),
    )
}

fn render_document_for_stages(
    document: &crate::ui::Document,
    settings: &crate::ui::RenderSettings,
) -> String {
    let mut plain = settings.clone();
    plain.mode = RenderMode::Plain;
    plain.color = ColorMode::Never;
    plain.unicode = UnicodeMode::Never;
    render_document(document, &plain)
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
            Some(crate::app::ReplCommandOutput::Output { .. })
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
    invocation: &ResolvedInvocation,
) -> Result<crate::app::CliCommandResult> {
    app::run_external_command_with_help_renderer(
        runtime,
        session,
        clients,
        &tokens,
        invocation,
        GuideDoc::from_text,
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
        Commands::Intro(_) => ReplCommandSpec {
            name: Cow::Borrowed(CMD_INTRO),
            supports_dsl: true,
        },
        Commands::Repl(_) => ReplCommandSpec {
            name: Cow::Borrowed("repl"),
            supports_dsl: false,
        },
    }
}
