//! Shared inline parsing and command pipeline policy for CLI and REPL hosts.

use std::borrow::Cow;

use clap::CommandFactory;

use miette::{Result, miette};

use super::{
    CMD_CONFIG, CMD_CONTEXT, CMD_DOCTOR, CMD_HISTORY, CMD_INTRO, CMD_PLUGINS, ReplCommandSpec,
};
use crate::cli::{
    Commands, ConfigCommands, ContextCommands, DoctorCommands, HistoryCommands, PluginsCommands,
    ThemeCommands, parse_inline_command_tokens,
};

#[derive(Clone, Copy)]
pub(crate) enum CommandSurface {
    Repl,
    External,
}

pub(crate) enum InlineCommandPlan {
    Command(Option<Commands>),
    Help(clap::Error),
}

/// Resolves command words with the same grammars used for execution, excluding values.
pub(crate) fn canonical_builtin_path(prefixes: bool, tokens: &[String]) -> Result<Vec<String>> {
    let tokens = tokens.strip_prefix(&["sudo".to_owned()]).unwrap_or(tokens);
    let Some((name, args)) = tokens.split_first() else {
        return Ok(Vec::new());
    };
    let builtin = crate::cli::InlineCommandCli::command();
    if let Some(command) = builtin.find_subcommand(name) {
        let args = crate::command_words::normalize(command, args, prefixes, &[])
            .map_err(|error| miette!("{error}"))?;
        let matches = crate::cli::passive_command(builtin)
            .try_get_matches_from(std::iter::once(name.clone()).chain(args));
        let mut path = Vec::new();
        if let Ok(matches) = matches {
            let mut current = &matches;
            while let Some((name, child)) = current.subcommand() {
                path.push(name.to_owned());
                current = child;
            }
        }
        return Ok(path);
    }
    Ok(Vec::new())
}

pub(crate) fn parse_inline_command(
    tokens: &[String],
    surface: CommandSurface,
    prefixes: bool,
) -> Result<InlineCommandPlan> {
    let grammar = crate::cli::InlineCommandCli::command();
    let normalized;
    let tokens = if let Some(command) = tokens
        .first()
        .and_then(|name| grammar.find_subcommand(name))
    {
        normalized = std::iter::once(tokens[0].clone())
            .chain(
                crate::command_words::normalize(command, &tokens[1..], prefixes, &[])
                    .map_err(|error| miette!("{error}"))?,
            )
            .collect::<Vec<_>>();
        &normalized
    } else {
        tokens
    };
    match parse_inline_command_tokens(tokens) {
        Ok(command) => Ok(InlineCommandPlan::Command(command)),
        Err(error) => {
            let help = crate::native::is_clap_help_error(error.kind());
            // Help is a document; callers apply any stages to it once.
            if help {
                return Ok(InlineCommandPlan::Help(error));
            }
            Err(super::host::report_clap_error_with_context(
                error,
                match surface {
                    CommandSurface::Repl => {
                        crate::app::host::ErrorContext::Operation("failed to parse REPL command")
                    }
                    CommandSurface::External => crate::app::host::ErrorContext::Operation(
                        "failed to parse external inline command",
                    ),
                },
                crate::cli::InlineCommandCli::command(),
                tokens,
                None,
            ))
        }
    }
}

pub(crate) fn validate_command_pipeline(command: &Commands, stages: &[String]) -> Result<()> {
    let spec = repl_command_spec(command);
    if stages.is_empty() || spec.supports_dsl {
        Ok(())
    } else {
        Err(miette!(
            "`{}` does not support DSL pipeline stages",
            spec.name
        ))
    }
}

pub(crate) fn repl_command_spec(command: &Commands) -> ReplCommandSpec {
    let (name, supports_dsl) = match command {
        Commands::Context(args) => (
            CMD_CONTEXT,
            matches!(args.command, None | Some(ContextCommands::Clear(_))),
        ),
        Commands::External(tokens) => {
            return ReplCommandSpec {
                name: Cow::Owned(
                    tokens
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "external".to_string()),
                ),
                supports_dsl: true,
            };
        }
        Commands::Plugins(args) => (CMD_PLUGINS, matches!(args.command, PluginsCommands::List)),
        Commands::Doctor(args) => (
            CMD_DOCTOR,
            matches!(
                args.command,
                Some(DoctorCommands::Config)
                    | Some(DoctorCommands::Plugins)
                    | Some(DoctorCommands::Theme)
            ),
        ),
        Commands::History(args) => (
            CMD_HISTORY,
            matches!(args.command, HistoryCommands::List(_)),
        ),
        Commands::Notifications(_) => ("notifications", false),
        Commands::Completions(_) => ("completions", false),
        Commands::Config(args) => (
            CMD_CONFIG,
            matches!(
                args.command,
                ConfigCommands::Show(_) | ConfigCommands::Get(_) | ConfigCommands::Doctor
            ),
        ),
        Commands::Alias(args) => (
            "alias",
            matches!(args.command, crate::cli::AliasCommands::List(_)),
        ),
        Commands::Theme(args) => ("theme", matches!(args.command, ThemeCommands::List)),
        Commands::Intro(_) => (CMD_INTRO, true),
        Commands::Repl(_) => ("repl", false),
    };
    ReplCommandSpec {
        name: Cow::Borrowed(name),
        supports_dsl,
    }
}
