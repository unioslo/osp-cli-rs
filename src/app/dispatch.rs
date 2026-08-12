//! Top-level command dispatch planning and source selection.
//!
//! This module takes parsed CLI intent and turns it into the coarse action the
//! host should run: builtin command, REPL, or external command surface. It
//! also owns the early native-vs-plugin source choice so later execution paths
//! do not have to repeat visibility and provider-selection logic.

use std::collections::BTreeSet;

use miette::{IntoDiagnostic, Result, WrapErr, miette};

use crate::app::access_recovery::{
    AccessRecoveryOutcome, AccessRecoveryRequest, CommandAccessKind,
};
use crate::app::{AppClients, AuthState, TerminalKind};
use crate::cli::{
    AliasArgs, Cli, Commands, ConfigArgs, DoctorArgs, HistoryArgs, IntroArgs, PluginsArgs,
    ReplArgs, ThemeArgs, parse_inline_command_tokens,
};
use crate::core::command_policy::{AccessReason, CommandAccess, CommandPath};
use crate::normalize::{normalize_identifier, normalize_optional_identifier};
use crate::plugin::CommandCatalogEntry;

#[cfg(test)]
use super::{CMD_CONFIG, CMD_DOCTOR, CMD_HISTORY, CMD_PLUGINS, CMD_THEME};

pub(crate) enum RunAction {
    Repl,
    ReplCommand(ReplArgs),
    Plugins(PluginsArgs),
    Doctor(DoctorArgs),
    Theme(ThemeArgs),
    Config(ConfigArgs),
    Alias(AliasArgs),
    History(HistoryArgs),
    Intro(IntroArgs),
    External(Vec<String>),
}

impl std::fmt::Debug for RunAction {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.name())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExternalCommandSource {
    Native,
    Plugin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExternalPathAccessRequirement {
    Visible,
    Runnable,
}

impl RunAction {
    pub(crate) fn name(&self) -> &'static str {
        match self {
            RunAction::Repl => "repl",
            RunAction::ReplCommand(_) => "repl-command",
            RunAction::Plugins(_) => "plugins",
            RunAction::Doctor(_) => "doctor",
            RunAction::Theme(_) => "theme",
            RunAction::Config(_) => "config",
            RunAction::Alias(_) => "alias",
            RunAction::History(_) => "history",
            RunAction::Intro(_) => "intro",
            RunAction::External(_) => "external",
        }
    }

    pub(crate) fn terminal_kind(&self) -> TerminalKind {
        match self {
            RunAction::Repl | RunAction::ReplCommand(_) | RunAction::Intro(_) => TerminalKind::Repl,
            RunAction::Plugins(_)
            | RunAction::Doctor(_)
            | RunAction::Theme(_)
            | RunAction::Config(_)
            | RunAction::Alias(_)
            | RunAction::History(_)
            | RunAction::External(_) => TerminalKind::Cli,
        }
    }

    pub(crate) fn into_builtin_command(self) -> Option<Commands> {
        match self {
            RunAction::Plugins(args) => Some(Commands::Plugins(args)),
            RunAction::Doctor(args) => Some(Commands::Doctor(args)),
            RunAction::Theme(args) => Some(Commands::Theme(args)),
            RunAction::Config(args) => Some(Commands::Config(args)),
            RunAction::Alias(args) => Some(Commands::Alias(args)),
            RunAction::History(args) => Some(Commands::History(args)),
            RunAction::Intro(args) => Some(Commands::Intro(args)),
            RunAction::ReplCommand(args) => Some(Commands::Repl(args)),
            RunAction::Repl | RunAction::External(_) => None,
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
        Some(Commands::Alias(args)) => {
            Ok(DispatchPlan::new(RunAction::Alias(args), explicit_profile))
        }
        Some(Commands::History(args)) => Ok(DispatchPlan::new(
            RunAction::History(args),
            explicit_profile,
        )),
        Some(Commands::Completions(_)) => Err(miette!(
            "`completions` is available only as a one-shot CLI command"
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

#[cfg(test)]
pub(crate) fn ensure_dispatch_visibility(auth: &AuthState, action: &RunAction) -> Result<()> {
    match action {
        RunAction::Plugins(_) => ensure_builtin_visible_for(auth, CMD_PLUGINS),
        RunAction::Doctor(_) => ensure_builtin_visible_for(auth, CMD_DOCTOR),
        RunAction::Theme(_) => ensure_builtin_visible_for(auth, CMD_THEME),
        RunAction::Config(_) => ensure_builtin_visible_for(auth, CMD_CONFIG),
        RunAction::Alias(_) => ensure_builtin_visible_for(auth, CMD_CONFIG),
        RunAction::History(_) => ensure_builtin_visible_for(auth, CMD_HISTORY),
        RunAction::ReplCommand(_) | RunAction::Repl | RunAction::Intro(_) => Ok(()),
        // External command auth needs provider/native metadata to resolve the
        // real command path. Do that in the runtime dispatch layer instead of
        // guessing from raw tokens here.
        RunAction::External(_) => Ok(()),
    }
}

pub(crate) fn ensure_builtin_visible_for(auth: &AuthState, command: &str) -> Result<()> {
    ensure_command_access(command, "command", auth.builtin_access(command))
}

#[cfg(test)]
pub(crate) fn ensure_plugin_visible_for(auth: &AuthState, command: &str) -> Result<()> {
    ensure_command_access(
        command,
        "plugin command",
        auth.external_command_access(command),
    )
}

pub(crate) fn ensure_builtin_access(
    runtime: &mut crate::app::AppRuntime,
    session: &mut crate::app::AppSession,
    command: &str,
) -> Result<()> {
    ensure_command_access_with_recovery(
        runtime,
        session,
        CommandAccessKind::Builtin,
        command,
        "command",
        |auth| auth.builtin_access(command),
    )
}

pub(crate) fn ensure_external_command_access(
    runtime: &mut crate::app::AppRuntime,
    session: &mut crate::app::AppSession,
    command: &str,
) -> Result<()> {
    ensure_command_access_with_recovery(
        runtime,
        session,
        CommandAccessKind::External,
        command,
        "plugin command",
        |auth| auth.external_command_access(command),
    )
}

pub(crate) fn ensure_external_path_access(
    runtime: &mut crate::app::AppRuntime,
    session: &mut crate::app::AppSession,
    path: &CommandPath,
    requirement: ExternalPathAccessRequirement,
    kind: &'static str,
) -> Result<()> {
    let command = path.as_slice().join(" ");
    match requirement {
        ExternalPathAccessRequirement::Visible => ensure_command_visibility(
            &command,
            kind,
            runtime.auth().external_command_path_access(path),
        ),
        ExternalPathAccessRequirement::Runnable => ensure_command_access_with_recovery(
            runtime,
            session,
            CommandAccessKind::External,
            &command,
            kind,
            |auth| auth.external_command_path_access(path),
        ),
    }
}

pub(crate) fn resolve_external_command_source(
    auth: &AuthState,
    clients: &AppClients,
    command: &str,
    provider_override: Option<&str>,
) -> Result<ExternalCommandSource> {
    if provider_override.is_some() {
        return Ok(ExternalCommandSource::Plugin);
    }

    let catalog = super::authorized_command_catalog_for(auth, clients)?;
    let matching = matching_external_command_entries(&catalog, command);
    let has_native = matching.iter().any(|entry| is_native_command_entry(entry));
    let has_plugin = matching.iter().any(|entry| !is_native_command_entry(entry));

    if matching.is_empty() {
        return Err(match closest_external_command(&catalog, command) {
            Some(suggestion) => {
                miette!("unknown command `{command}`\nTry: use command `{suggestion}`")
            }
            None => miette!("unknown command `{command}`\nTry: run `help` to list commands"),
        });
    }

    if has_native && has_plugin {
        let labels = matching
            .iter()
            .map(|entry| external_command_source_label(entry))
            .collect::<Vec<_>>();
        return Err(miette!(
            "command `{command}` is ambiguous across command sources: {}",
            labels.join(", ")
        ));
    }

    if has_plugin && !has_native && matching.iter().any(|entry| entry.requires_selection) {
        let providers = matching
            .iter()
            .find(|entry| entry.requires_selection)
            .map(|entry| entry.providers.join(", "))
            .unwrap_or_default();
        return Err(miette!(
            "command `{command}` requires provider selection; available: {providers}; use --plugin-provider <plugin-id> or `plugins select-provider {command} <plugin-id>`"
        ));
    }

    if has_native {
        Ok(ExternalCommandSource::Native)
    } else {
        Ok(ExternalCommandSource::Plugin)
    }
}

pub(crate) fn canonical_external_command_name(
    auth: &AuthState,
    clients: &AppClients,
    command: &str,
) -> Result<String> {
    let catalog = super::authorized_command_catalog_for(auth, clients)?;
    Ok(matching_external_command_entries(&catalog, command)
        .first()
        .map(|entry| entry.name.clone())
        .unwrap_or_else(|| command.to_string()))
}

pub(crate) fn external_command_source_label(entry: &CommandCatalogEntry) -> String {
    if entry.requires_selection {
        return format!("plugin providers: {}", entry.providers.join(", "));
    }

    match (&entry.provider, entry.source) {
        (Some(provider), Some(source)) => format!("{provider} ({source})"),
        _ => "native integration".to_string(),
    }
}

fn is_native_command_entry(entry: &CommandCatalogEntry) -> bool {
    entry.provider.is_none() && entry.source.is_none() && entry.providers.is_empty()
}

fn matching_external_command_entries<'a>(
    catalog: &'a [CommandCatalogEntry],
    command: &str,
) -> Vec<&'a CommandCatalogEntry> {
    catalog
        .iter()
        .filter(|entry| entry.name.eq_ignore_ascii_case(command))
        .collect()
}

fn closest_external_command(catalog: &[CommandCatalogEntry], command: &str) -> Option<String> {
    let command = command.trim().to_ascii_lowercase();
    let max_distance = if command.chars().count() >= 4 { 2 } else { 1 };
    catalog
        .iter()
        .map(|entry| {
            (
                levenshtein_distance(&command, &entry.name.to_ascii_lowercase()),
                &entry.name,
            )
        })
        .filter(|(distance, _)| *distance <= max_distance)
        .min_by(|left, right| left.cmp(right))
        .map(|(_, name)| name.clone())
}

fn levenshtein_distance(left: &str, right: &str) -> usize {
    let mut row = (0..=right.chars().count()).collect::<Vec<_>>();
    for (left_index, left_char) in left.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = left_index + 1;
        for (right_index, right_char) in right.chars().enumerate() {
            let above = row[right_index + 1];
            row[right_index + 1] = if left_char == right_char {
                diagonal
            } else {
                1 + diagonal.min(above).min(row[right_index])
            };
            diagonal = above;
        }
    }
    row[right.chars().count()]
}

pub(crate) fn normalize_profile_override(value: Option<String>) -> Option<String> {
    normalize_optional_identifier(value)
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
        .into_diagnostic()
        .wrap_err_with(|| {
            format!("failed to parse command after profile shorthand `{normalized}`")
        })?;
    let action = inline_run_action(parsed)?;
    tracing::debug!(
        profile = %normalized,
        action = %action.name(),
        command = %remaining
            .first()
            .map(String::as_str)
            .unwrap_or("repl"),
        "profile shorthand: routing to command"
    );
    Ok(Some(DispatchPlan::new(action, Some(normalized))))
}

fn inline_run_action(parsed: Option<Commands>) -> Result<RunAction> {
    Ok(match parsed {
        Some(Commands::Plugins(args)) => RunAction::Plugins(args),
        Some(Commands::Doctor(args)) => RunAction::Doctor(args),
        Some(Commands::Theme(args)) => RunAction::Theme(args),
        Some(Commands::Config(args)) => RunAction::Config(args),
        Some(Commands::Alias(args)) => RunAction::Alias(args),
        Some(Commands::History(args)) => RunAction::History(args),
        Some(Commands::Completions(_)) => {
            return Err(miette!(
                "`completions` is available only as a top-level one-shot command"
            ));
        }
        Some(Commands::Intro(args)) => RunAction::Intro(args),
        Some(Commands::Repl(args)) => RunAction::ReplCommand(args),
        Some(Commands::External(external)) => RunAction::External(external),
        None => RunAction::Repl,
    })
}

#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error(
    "{kind} `{command}` requires {required}. Try: authenticate with an identity granted {required}, then retry"
)]
#[diagnostic(
    code(osp::auth::missing_capabilities),
    help(
        "The capability policy gate for `{command}` requires {required}. If authenticating again does not add them, ask the command owner for access."
    )
)]
pub(crate) struct MissingCommandCapabilities {
    kind: &'static str,
    command: String,
    required: String,
}

impl MissingCommandCapabilities {
    fn new(kind: &'static str, command: &str, missing: &BTreeSet<String>) -> Self {
        let required = match missing.iter().next() {
            Some(capability) if missing.len() == 1 => format!("capability `{capability}`"),
            _ => format!(
                "capabilities {}",
                missing
                    .iter()
                    .map(|capability| format!("`{capability}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        Self {
            kind,
            command: command.to_string(),
            required,
        }
    }

    pub(crate) fn normal_detail(&self) -> String {
        format!(
            "Missing requirement: `{}` needs {} for this command path.",
            self.command, self.required
        )
    }
}

fn ensure_command_access(command: &str, kind: &'static str, access: CommandAccess) -> Result<()> {
    if access.is_runnable() {
        return Ok(());
    }

    if !access.missing_capabilities.is_empty() {
        return Err(miette::Report::new(MissingCommandCapabilities::new(
            kind,
            command,
            &access.missing_capabilities,
        )));
    }

    let detail = access
        .reasons
        .first()
        .map(render_access_reason)
        .unwrap_or_else(|| "denied by current auth policy".to_string());
    let action = access
        .reasons
        .first()
        .map(access_reason_action)
        .unwrap_or_else(|| "ask the command owner for access, then retry".to_string());
    Err(miette!("{kind} `{command}` {detail}. Try: {action}"))
}

fn ensure_command_visibility(
    command: &str,
    kind: &'static str,
    access: CommandAccess,
) -> Result<()> {
    if access.is_visible() {
        return Ok(());
    }
    ensure_command_access(command, kind, access)
}

fn ensure_command_access_with_recovery(
    runtime: &mut crate::app::AppRuntime,
    session: &mut crate::app::AppSession,
    command_kind: CommandAccessKind,
    command: &str,
    kind: &'static str,
    access_for: impl Fn(&AuthState) -> CommandAccess,
) -> Result<()> {
    let access = access_for(runtime.auth());
    if access.is_runnable() {
        return Ok(());
    }

    if let Some(recovery) = runtime.access_recovery() {
        let request = AccessRecoveryRequest::new(
            runtime.context.terminal_kind(),
            command_kind,
            command,
            access.clone(),
        );
        if matches!(
            recovery.try_recover(&request, runtime, session)?,
            AccessRecoveryOutcome::Recovered
        ) {
            let recovered = access_for(runtime.auth());
            if recovered.is_runnable() {
                return Ok(());
            }
            return ensure_command_access(command, kind, recovered);
        }
    }

    ensure_command_access(command, kind, access)
}

fn render_access_reason(reason: &AccessReason) -> String {
    match reason {
        AccessReason::HiddenByPolicy => "is hidden by current auth policy".to_string(),
        AccessReason::DisabledByProduct => "is disabled by current product policy".to_string(),
        AccessReason::Unauthenticated => "requires authentication".to_string(),
        AccessReason::MissingCapabilities => "requires additional capabilities".to_string(),
        AccessReason::MissingCredential(service) => {
            format!("requires credential `{service}`")
        }
        AccessReason::InvalidCredential(service) => {
            format!("requires a valid `{service}` credential")
        }
        AccessReason::InsufficientCredentialTtl {
            service,
            required_ttl_seconds,
        } => format!(
            "requires `{service}` credential with at least {required_ttl_seconds}s remaining"
        ),
        AccessReason::InsufficientAuthStrength(required) => {
            format!("requires {} authentication", required.as_label())
        }
        AccessReason::FeatureDisabled(flag) => format!("requires feature `{flag}`"),
        AccessReason::ProfileUnavailable(profile) if profile.is_empty() => {
            "requires an eligible profile".to_string()
        }
        AccessReason::ProfileUnavailable(profile) => {
            format!("is unavailable in profile `{profile}`")
        }
    }
}

fn access_reason_action(reason: &AccessReason) -> String {
    match reason {
        AccessReason::Unauthenticated => "authenticate, then retry".to_string(),
        AccessReason::MissingCapabilities => {
            "authenticate with an identity granted the required capabilities, then retry"
                .to_string()
        }
        AccessReason::MissingCredential(service) => {
            format!("configure the `{service}` credential, then retry")
        }
        AccessReason::InvalidCredential(service)
        | AccessReason::InsufficientCredentialTtl { service, .. } => {
            format!("renew the `{service}` credential, then retry")
        }
        AccessReason::InsufficientAuthStrength(required) => {
            format!(
                "authenticate with {} authentication, then retry",
                required.as_label()
            )
        }
        AccessReason::FeatureDisabled(flag) => {
            format!("enable feature `{flag}`, then retry")
        }
        AccessReason::ProfileUnavailable(profile) if profile.is_empty() => {
            "switch to an eligible profile, then retry".to_string()
        }
        AccessReason::ProfileUnavailable(profile) => {
            format!("switch from profile `{profile}` to an eligible profile, then retry")
        }
        AccessReason::HiddenByPolicy | AccessReason::DisabledByProduct => {
            "ask the command owner to enable access, then retry".to_string()
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
    use crate::cli::{Cli, Commands};

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
        assert!(err.to_string().contains("Try: authenticate, then retry"));

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
    fn run_action_builtin_conversion_covers_builtin_and_non_builtin_paths_unit() {
        let plugins = RunAction::Plugins(crate::cli::PluginsArgs {
            command: crate::cli::PluginsCommands::List,
        });
        let repl = RunAction::Repl;
        let external = RunAction::External(vec!["ldap".to_string()]);

        assert!(matches!(
            plugins.into_builtin_command(),
            Some(Commands::Plugins(_))
        ));
        assert!(repl.into_builtin_command().is_none());
        assert!(external.into_builtin_command().is_none());
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
            .expect_err("active profile outside the allowlist should deny builtin");
        assert!(
            profile_err
                .to_string()
                .contains("is unavailable in profile `default`")
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
        let capability_text = capability_err.to_string();
        assert!(capability_text.contains("plugin command `orch`"));
        assert!(capability_text.contains("capability `orch.approval.decide`"));
        assert!(capability_text.contains("Try: authenticate"));

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
