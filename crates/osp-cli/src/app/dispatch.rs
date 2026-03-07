use std::collections::BTreeSet;

use miette::{Result, miette};

use crate::cli::{
    Cli, Commands, ConfigArgs, DoctorArgs, HistoryArgs, PluginsArgs, ReplArgs, ThemeArgs,
    parse_inline_command_tokens,
};
use crate::state::{AuthState, TerminalKind};

use super::{CMD_CONFIG, CMD_DOCTOR, CMD_HISTORY, CMD_PLUGINS, CMD_THEME};

#[derive(Debug)]
pub(crate) enum RunAction {
    Repl,
    ReplCommand(ReplArgs),
    Plugins(PluginsArgs),
    Doctor(DoctorArgs),
    Theme(ThemeArgs),
    Config(ConfigArgs),
    History(HistoryArgs),
    External(Vec<String>),
}

impl RunAction {
    pub(crate) fn terminal_kind(&self) -> TerminalKind {
        match self {
            RunAction::Repl | RunAction::ReplCommand(_) => TerminalKind::Repl,
            RunAction::Plugins(_)
            | RunAction::Doctor(_)
            | RunAction::Theme(_)
            | RunAction::Config(_)
            | RunAction::History(_)
            | RunAction::External(_) => TerminalKind::Cli,
        }
    }
}

pub(crate) struct DispatchPlan {
    pub(crate) action: RunAction,
    pub(crate) profile_override: Option<String>,
}

impl DispatchPlan {
    fn new(action: RunAction, profile_override: Option<String>) -> Self {
        Self {
            action,
            profile_override,
        }
    }

    fn repl(profile_override: Option<String>) -> Self {
        Self::new(RunAction::Repl, profile_override)
    }
}

pub(crate) fn build_dispatch_plan(
    cli: &mut Cli,
    known_profiles: &BTreeSet<String>,
) -> Result<DispatchPlan> {
    let explicit_profile = normalize_cli_profile(cli);
    let command = cli.command.take();
    let normalized_profiles = known_profiles
        .iter()
        .map(|profile| normalize_identifier(profile))
        .collect::<BTreeSet<_>>();

    match command {
        None => Ok(DispatchPlan::repl(explicit_profile)),
        Some(Commands::Plugins(args)) => Ok(DispatchPlan::new(
            RunAction::Plugins(args),
            explicit_profile,
        )),
        Some(Commands::Doctor(args)) => {
            Ok(DispatchPlan::new(RunAction::Doctor(args), explicit_profile))
        }
        Some(Commands::Theme(args)) => {
            Ok(DispatchPlan::new(RunAction::Theme(args), explicit_profile))
        }
        Some(Commands::Config(args)) => {
            Ok(DispatchPlan::new(RunAction::Config(args), explicit_profile))
        }
        Some(Commands::History(args)) => Ok(DispatchPlan::new(
            RunAction::History(args),
            explicit_profile,
        )),
        Some(Commands::Repl(args)) => Ok(DispatchPlan::new(
            RunAction::ReplCommand(args),
            explicit_profile,
        )),
        Some(Commands::External(tokens)) => {
            if let Some(plan) = profile_prefixed_external_plan(
                &tokens,
                explicit_profile.clone(),
                &normalized_profiles,
            )? {
                return Ok(plan);
            }

            Ok(DispatchPlan::new(
                RunAction::External(tokens),
                explicit_profile,
            ))
        }
    }
}

pub(crate) fn normalize_cli_profile(cli: &mut Cli) -> Option<String> {
    let normalized = normalize_profile_override(cli.profile.clone());
    cli.profile = normalized.clone();
    normalized
}

pub(crate) fn ensure_dispatch_visibility(auth: &AuthState, action: &RunAction) -> Result<()> {
    match action {
        RunAction::Plugins(_) => ensure_builtin_visible_for(auth, CMD_PLUGINS),
        RunAction::Doctor(_) => ensure_builtin_visible_for(auth, CMD_DOCTOR),
        RunAction::Theme(_) => ensure_builtin_visible_for(auth, CMD_THEME),
        RunAction::Config(_) => ensure_builtin_visible_for(auth, CMD_CONFIG),
        RunAction::History(_) => ensure_builtin_visible_for(auth, CMD_HISTORY),
        RunAction::ReplCommand(_) | RunAction::Repl => Ok(()),
        RunAction::External(tokens) => {
            if let Some(command) = tokens.first() {
                ensure_plugin_visible_for(auth, command)?;
            }
            Ok(())
        }
    }
}

pub(crate) fn ensure_builtin_visible_for(auth: &AuthState, command: &str) -> Result<()> {
    if auth.is_builtin_visible(command) {
        Ok(())
    } else {
        Err(miette!(
            "command `{command}` is hidden by current auth policy"
        ))
    }
}

pub(crate) fn ensure_plugin_visible_for(auth: &AuthState, command: &str) -> Result<()> {
    if auth.is_plugin_command_visible(command) {
        Ok(())
    } else {
        Err(miette!(
            "plugin command `{command}` is hidden by current auth policy"
        ))
    }
}

pub(crate) fn normalize_profile_override(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let normalized = normalize_identifier(&value);
        if normalized.is_empty() {
            None
        } else {
            Some(normalized)
        }
    })
}

fn normalize_identifier(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

// `osp <profile> <command>` is a supported shorthand for
// `osp --profile <profile> <command>`. Keep the rule here so the
// positional-profile grammar is discoverable in one place.
fn profile_prefixed_external_plan(
    tokens: &[String],
    explicit_profile: Option<String>,
    normalized_profiles: &BTreeSet<String>,
) -> Result<Option<DispatchPlan>> {
    let Some(first) = tokens.first() else {
        return Ok(Some(DispatchPlan::repl(explicit_profile)));
    };
    if explicit_profile.is_some() {
        return Ok(None);
    }

    let normalized = normalize_identifier(first);
    if !normalized_profiles.contains(&normalized) {
        return Ok(None);
    }

    let remaining = tokens[1..].to_vec();
    if remaining.is_empty() {
        return Ok(Some(DispatchPlan::repl(Some(normalized))));
    }

    let parsed = parse_inline_command_tokens(&remaining).map_err(|err| miette!(err.to_string()))?;
    Ok(Some(DispatchPlan::new(
        inline_run_action(parsed),
        Some(normalized),
    )))
}

fn inline_run_action(parsed: Option<Commands>) -> RunAction {
    match parsed {
        Some(Commands::Plugins(args)) => RunAction::Plugins(args),
        Some(Commands::Doctor(args)) => RunAction::Doctor(args),
        Some(Commands::Theme(args)) => RunAction::Theme(args),
        Some(Commands::Config(args)) => RunAction::Config(args),
        Some(Commands::History(args)) => RunAction::History(args),
        Some(Commands::Repl(args)) => RunAction::ReplCommand(args),
        Some(Commands::External(external)) => RunAction::External(external),
        None => RunAction::Repl,
    }
}
