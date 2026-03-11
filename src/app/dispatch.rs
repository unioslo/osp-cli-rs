use std::collections::BTreeSet;

use miette::{Result, WrapErr, miette};

use crate::app::{AuthState, TerminalKind};
use crate::cli::{
    Cli, Commands, ConfigArgs, DoctorArgs, HistoryArgs, IntroArgs, PluginsArgs, ReplArgs,
    ThemeArgs, parse_inline_command_tokens,
};
use crate::core::command_policy::{AccessReason, CommandAccess};

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
    Intro(IntroArgs),
    External(Vec<String>),
}

impl RunAction {
    pub(crate) fn terminal_kind(&self) -> TerminalKind {
        match self {
            RunAction::Repl | RunAction::ReplCommand(_) | RunAction::Intro(_) => TerminalKind::Repl,
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
        RunAction::Intro(_) => "intro",
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
        Some(Commands::Intro(args)) => {
            Ok(DispatchPlan::new(RunAction::Intro(args), explicit_profile))
        }
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
        RunAction::ReplCommand(_) | RunAction::Repl | RunAction::Intro(_) => Ok(()),
        RunAction::External(tokens) => {
            if let Some(command) = tokens.first() {
                ensure_plugin_visible_for(auth, command)?;
            }
            Ok(())
        }
    }
}

pub(crate) fn ensure_builtin_visible_for(auth: &AuthState, command: &str) -> Result<()> {
    ensure_command_access(command, "command", auth.builtin_access(command))
}

pub(crate) fn ensure_plugin_visible_for(auth: &AuthState, command: &str) -> Result<()> {
    ensure_command_access(
        command,
        "plugin command",
        auth.external_command_access(command),
    )
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
        Some(Commands::Intro(args)) => RunAction::Intro(args),
        Some(Commands::Repl(args)) => RunAction::ReplCommand(args),
        Some(Commands::External(external)) => RunAction::External(external),
        None => RunAction::Repl,
    }
}

fn ensure_command_access(command: &str, kind: &str, access: CommandAccess) -> Result<()> {
    if access.is_runnable() {
        return Ok(());
    }

    let detail = access
        .reasons
        .first()
        .map(render_access_reason)
        .unwrap_or_else(|| "denied by current auth policy".to_string());
    Err(miette!("{kind} `{command}` {detail}"))
}

fn render_access_reason(reason: &AccessReason) -> String {
    match reason {
        AccessReason::HiddenByPolicy => "is hidden by current auth policy".to_string(),
        AccessReason::DisabledByProduct => "is disabled by current product policy".to_string(),
        AccessReason::Unauthenticated => "requires authentication".to_string(),
        AccessReason::MissingCapabilities => "requires additional capabilities".to_string(),
        AccessReason::FeatureDisabled(flag) => format!("requires feature `{flag}`"),
        AccessReason::ProfileUnavailable(profile) if profile.is_empty() => {
            "requires an eligible profile".to_string()
        }
        AccessReason::ProfileUnavailable(profile) => {
            format!("is unavailable in profile `{profile}`")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use crate::config::{ConfigLayer, ConfigResolver, LoadedLayers, ResolveOptions};
    use crate::core::command_policy::{AccessReason, CommandPath, CommandPolicy, VisibilityMode};
    use clap::Parser;

    use super::{
        DispatchPlan, RunAction, build_dispatch_plan, ensure_builtin_visible_for,
        ensure_dispatch_visibility, ensure_plugin_visible_for, normalize_cli_profile,
        normalize_profile_override,
    };
    use crate::app::{AuthState, TerminalKind};
    use crate::cli::Cli;

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
    fn normalize_profile_helpers_trim_blank_values_and_rewrite_cli_unit() {
        assert_eq!(
            normalize_profile_override(Some("  Dev  ".to_string())),
            Some("dev".to_string())
        );
        assert_eq!(normalize_profile_override(Some("   ".to_string())), None);
        assert_eq!(normalize_profile_override(None), None);
        let mut cli = parse_cli(&["osp", "--profile", "  DEV  ", "theme", "list"]);

        let normalized = normalize_cli_profile(&mut cli);

        assert_eq!(normalized.as_deref(), Some("dev"));
        assert_eq!(cli.profile.as_deref(), Some("dev"));
    }

    #[test]
    fn build_dispatch_plan_routes_profiles_builtins_external_and_errors_unit() {
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

    #[test]
    fn dispatch_visibility_helpers_cover_allowlists_unrunnable_commands_and_terminals_unit() {
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

        let mut gated_auth = auth_state(None, None);
        gated_auth.builtin_policy_mut().register(
            CommandPolicy::new(CommandPath::new(["config"]))
                .visibility(VisibilityMode::Authenticated),
        );
        let err = ensure_builtin_visible_for(&gated_auth, "config")
            .expect_err("unauthenticated builtin should be denied");
        assert!(err.to_string().contains("requires authentication"));

        let profiles = BTreeSet::from(["dev".to_string()]);
        let auth = auth_state(
            Some(&["plugins", "doctor", "theme", "config", "history"]),
            Some(&["ldap"]),
        );
        for (args, expected_terminal) in [
            (&["osp", "plugins", "list"][..], TerminalKind::Cli),
            (&["osp", "theme", "list"][..], TerminalKind::Cli),
            (&["osp", "history", "list"][..], TerminalKind::Cli),
            (&["osp", "doctor", "theme"][..], TerminalKind::Cli),
            (&["osp"][..], TerminalKind::Repl),
        ] {
            let mut cli = parse_cli(args);
            let plan = build_dispatch_plan(&mut cli, &profiles).expect("command should parse");
            assert_eq!(plan.action.terminal_kind(), expected_terminal);
            ensure_dispatch_visibility(&auth, &plan.action).expect("command should be visible");
        }

        let external = RunAction::External(vec!["ldap".to_string(), "user".to_string()]);
        assert_eq!(external.terminal_kind(), TerminalKind::Cli);
        ensure_dispatch_visibility(&auth, &external).expect("visible plugin should pass");
    }

    #[test]
    fn dispatch_reason_rendering_covers_feature_profile_and_capability_denials_unit() {
        let mut auth = auth_state(None, Some(&["ldap", "orch"]));
        auth.builtin_policy_mut().register(
            CommandPolicy::new(CommandPath::new(["config"]))
                .visibility(VisibilityMode::Authenticated)
                .allow_profiles(["dev"])
                .feature_flag("config-ui"),
        );
        auth.external_policy_mut().register(
            CommandPolicy::new(CommandPath::new(["orch"]))
                .visibility(VisibilityMode::CapabilityGated)
                .require_capability("orch.approval.decide"),
        );

        let profile_err = ensure_builtin_visible_for(&auth, "config")
            .expect_err("missing profile should deny builtin");
        assert!(
            profile_err
                .to_string()
                .contains("requires an eligible profile")
        );

        auth.set_policy_context(
            crate::core::command_policy::CommandPolicyContext::default().with_profile("dev"),
        );
        let feature_err = ensure_builtin_visible_for(&auth, "config")
            .expect_err("missing feature should deny builtin");
        assert!(
            feature_err
                .to_string()
                .contains("requires feature `config-ui`")
        );

        auth.set_policy_context(
            crate::core::command_policy::CommandPolicyContext::default()
                .authenticated(true)
                .with_profile("dev"),
        );
        let capability_err = ensure_plugin_visible_for(&auth, "orch")
            .expect_err("missing capability should deny plugin");
        assert!(
            capability_err
                .to_string()
                .contains("requires additional capabilities")
        );

        assert_eq!(
            super::render_access_reason(&AccessReason::DisabledByProduct),
            "is disabled by current product policy"
        );
        assert_eq!(
            super::render_access_reason(&AccessReason::ProfileUnavailable("prod".to_string())),
            "is unavailable in profile `prod`"
        );
    }
}
