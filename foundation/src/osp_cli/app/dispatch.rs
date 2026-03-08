use std::collections::BTreeSet;

use miette::{Result, WrapErr, miette};

use crate::osp_cli::cli::{
    Cli, Commands, ConfigArgs, DoctorArgs, HistoryArgs, PluginsArgs, ReplArgs, ThemeArgs,
    parse_inline_command_tokens,
};
use crate::osp_cli::state::{AuthState, TerminalKind};

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

fn run_action_name(action: &RunAction) -> &'static str {
    match action {
        RunAction::Repl => "repl",
        RunAction::ReplCommand(_) => "repl-command",
        RunAction::Plugins(_) => "plugins",
        RunAction::Doctor(_) => "doctor",
        RunAction::Theme(_) => "theme",
        RunAction::Config(_) => "config",
        RunAction::History(_) => "history",
        RunAction::External(_) => "external",
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
        tracing::debug!(profile = %normalized, "profile shorthand: no command, entering REPL");
        return Ok(Some(DispatchPlan::repl(Some(normalized))));
    }

    let parsed = parse_inline_command_tokens(&remaining)
        .map_err(|err| miette!(err.to_string()))
        .wrap_err_with(|| {
            format!("failed to parse command after profile shorthand `{normalized}`")
        })?;
    let action = inline_run_action(parsed);
    tracing::debug!(
        profile = %normalized,
        action = %run_action_name(&action),
        command = %remaining
            .first()
            .map(String::as_str)
            .unwrap_or("repl"),
        "profile shorthand: routing to command"
    );
    Ok(Some(DispatchPlan::new(action, Some(normalized))))
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use clap::Parser;
    use crate::osp_config::{ConfigLayer, ConfigResolver, LoadedLayers, ResolveOptions};

    use super::{
        DispatchPlan, RunAction, build_dispatch_plan, ensure_builtin_visible_for,
        ensure_dispatch_visibility, ensure_plugin_visible_for, normalize_cli_profile,
        normalize_profile_override,
    };
    use crate::osp_cli::cli::Cli;
    use crate::osp_cli::state::{AuthState, TerminalKind};

    fn parse_cli(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).expect("cli args should parse")
    }

    fn auth_state(builtins: Option<&[&str]>, plugins: Option<&[&str]>) -> AuthState {
        let mut file = ConfigLayer::default();
        if let Some(items) = builtins {
            file.set("auth.visible.builtins", items.join(","));
        }
        if let Some(items) = plugins {
            file.set("auth.visible.plugins", items.join(","));
        }

        let resolved = ConfigResolver::from_loaded_layers(LoadedLayers {
            file,
            ..LoadedLayers::default()
        })
        .resolve(ResolveOptions::default())
        .expect("auth visibility config should resolve");
        AuthState::from_resolved(&resolved)
    }

    #[test]
    fn normalize_profile_override_trims_and_rejects_blank_values_unit() {
        assert_eq!(
            normalize_profile_override(Some("  Dev  ".to_string())),
            Some("dev".to_string())
        );
        assert_eq!(normalize_profile_override(Some("   ".to_string())), None);
        assert_eq!(normalize_profile_override(None), None);
    }

    #[test]
    fn build_dispatch_plan_supports_profile_prefixed_external_commands_unit() {
        let profiles = BTreeSet::from(["dev".to_string(), "prod".to_string()]);

        let mut repl_cli = parse_cli(&["osp", "dev"]);
        let DispatchPlan {
            action,
            profile_override,
        } = build_dispatch_plan(&mut repl_cli, &profiles).expect("profile-only repl should work");
        assert!(matches!(action, RunAction::Repl));
        assert_eq!(profile_override.as_deref(), Some("dev"));

        let mut config_cli = parse_cli(&["osp", "dev", "config", "show"]);
        let DispatchPlan {
            action,
            profile_override,
        } = build_dispatch_plan(&mut config_cli, &profiles)
            .expect("profile-prefixed config command should work");
        assert!(matches!(action, RunAction::Config(_)));
        assert_eq!(profile_override.as_deref(), Some("dev"));
    }

    #[test]
    fn build_dispatch_plan_preserves_external_tokens_when_not_shorthand_unit() {
        let profiles = BTreeSet::from(["dev".to_string()]);

        let mut explicit_profile_cli =
            parse_cli(&["osp", "--profile", "prod", "dev", "config", "show"]);
        let DispatchPlan {
            action,
            profile_override,
        } = build_dispatch_plan(&mut explicit_profile_cli, &profiles)
            .expect("explicit profile should bypass shorthand");
        assert!(
            matches!(action, RunAction::External(tokens) if tokens == vec!["dev", "config", "show"])
        );
        assert_eq!(profile_override.as_deref(), Some("prod"));

        let mut unknown_profile_cli = parse_cli(&["osp", "stage", "config", "show"]);
        let DispatchPlan {
            action,
            profile_override,
        } = build_dispatch_plan(&mut unknown_profile_cli, &profiles)
            .expect("unknown prefix should stay external");
        assert!(
            matches!(action, RunAction::External(tokens) if tokens == vec!["stage", "config", "show"])
        );
        assert!(profile_override.is_none());
    }

    #[test]
    fn dispatch_visibility_helpers_enforce_builtin_and_plugin_allowlists_unit() {
        let auth = auth_state(Some(&["config", "history"]), Some(&["ldap"]));

        ensure_builtin_visible_for(&auth, "config").expect("config should be visible");
        ensure_plugin_visible_for(&auth, "ldap").expect("ldap should be visible");
        assert!(
            ensure_builtin_visible_for(&auth, "theme")
                .expect_err("theme should be hidden")
                .to_string()
                .contains("hidden by current auth policy")
        );
        assert!(
            ensure_plugin_visible_for(&auth, "mreg")
                .expect_err("mreg should be hidden")
                .to_string()
                .contains("hidden by current auth policy")
        );
    }

    #[test]
    fn ensure_dispatch_visibility_and_terminal_kind_cover_all_action_families_unit() {
        let profiles = BTreeSet::from(["dev".to_string()]);
        let auth = auth_state(
            Some(&["plugins", "doctor", "theme", "config", "history"]),
            Some(&["ldap"]),
        );

        let mut plugin_cli = parse_cli(&["osp", "plugins", "list"]);
        let plugin_plan =
            build_dispatch_plan(&mut plugin_cli, &profiles).expect("plugins should parse");
        assert_eq!(plugin_plan.action.terminal_kind(), TerminalKind::Cli);
        ensure_dispatch_visibility(&auth, &plugin_plan.action).expect("plugins should be visible");

        let mut theme_cli = parse_cli(&["osp", "theme", "list"]);
        let theme_plan =
            build_dispatch_plan(&mut theme_cli, &profiles).expect("theme should parse");
        assert_eq!(theme_plan.action.terminal_kind(), TerminalKind::Cli);
        ensure_dispatch_visibility(&auth, &theme_plan.action).expect("theme should be visible");

        let mut history_cli = parse_cli(&["osp", "history", "list"]);
        let history_plan =
            build_dispatch_plan(&mut history_cli, &profiles).expect("history should parse");
        assert_eq!(history_plan.action.terminal_kind(), TerminalKind::Cli);
        ensure_dispatch_visibility(&auth, &history_plan.action).expect("history should be visible");

        let mut doctor_cli = parse_cli(&["osp", "doctor", "theme"]);
        let doctor_plan =
            build_dispatch_plan(&mut doctor_cli, &profiles).expect("doctor should parse");
        assert_eq!(doctor_plan.action.terminal_kind(), TerminalKind::Cli);
        ensure_dispatch_visibility(&auth, &doctor_plan.action).expect("doctor should be visible");

        let mut repl_cli = parse_cli(&["osp"]);
        let repl_plan = build_dispatch_plan(&mut repl_cli, &profiles).expect("repl should parse");
        assert_eq!(repl_plan.action.terminal_kind(), TerminalKind::Repl);
        ensure_dispatch_visibility(&auth, &repl_plan.action)
            .expect("repl should always be visible");

        let external = RunAction::External(vec!["ldap".to_string(), "user".to_string()]);
        assert_eq!(external.terminal_kind(), TerminalKind::Cli);
        ensure_dispatch_visibility(&auth, &external).expect("visible plugin should pass");
    }

    #[test]
    fn normalize_cli_profile_rewrites_cli_profile_in_place_unit() {
        let mut cli = parse_cli(&["osp", "--profile", "  DEV  ", "theme", "list"]);

        let normalized = normalize_cli_profile(&mut cli);

        assert_eq!(normalized.as_deref(), Some("dev"));
        assert_eq!(cli.profile.as_deref(), Some("dev"));
    }

    #[test]
    fn build_dispatch_plan_covers_repl_subcommand_and_shorthand_parse_errors_unit() {
        let profiles = BTreeSet::from(["dev".to_string()]);

        let mut repl_cli = parse_cli(&["osp", "repl", "debug-complete", "--line", "ldap"]);
        let DispatchPlan {
            action,
            profile_override,
        } = build_dispatch_plan(&mut repl_cli, &profiles).expect("repl subcommand should parse");
        assert!(matches!(action, RunAction::ReplCommand(_)));
        assert!(profile_override.is_none());

        let mut bad_shorthand_cli = parse_cli(&["osp", "dev", "config", "set", "ui.format"]);
        let err = build_dispatch_plan(&mut bad_shorthand_cli, &profiles)
            .err()
            .expect("invalid shorthand command should fail");
        assert!(
            err.to_string()
                .contains("failed to parse command after profile shorthand `dev`")
        );
    }
}
