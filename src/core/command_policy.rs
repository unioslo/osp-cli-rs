use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CommandPath(Vec<String>);

impl CommandPath {
    pub fn new<I, S>(segments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self(
            segments
                .into_iter()
                .map(Into::into)
                .map(|segment| segment.trim().to_ascii_lowercase())
                .filter(|segment| !segment.is_empty())
                .collect(),
        )
    }

    pub fn as_slice(&self) -> &[String] {
        self.0.as_slice()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityMode {
    Public,
    Authenticated,
    CapabilityGated,
    Hidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandAvailability {
    Available,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandPolicy {
    pub path: CommandPath,
    pub visibility: VisibilityMode,
    pub availability: CommandAvailability,
    pub required_capabilities: BTreeSet<String>,
    pub feature_flags: BTreeSet<String>,
    pub allowed_profiles: Option<BTreeSet<String>>,
    pub denied_message: Option<String>,
    pub hidden_reason: Option<String>,
}

impl CommandPolicy {
    pub fn new(path: CommandPath) -> Self {
        Self {
            path,
            visibility: VisibilityMode::Public,
            availability: CommandAvailability::Available,
            required_capabilities: BTreeSet::new(),
            feature_flags: BTreeSet::new(),
            allowed_profiles: None,
            denied_message: None,
            hidden_reason: None,
        }
    }

    pub fn visibility(mut self, visibility: VisibilityMode) -> Self {
        self.visibility = visibility;
        self
    }

    pub fn require_capability(mut self, capability: impl Into<String>) -> Self {
        let normalized = capability.into().trim().to_ascii_lowercase();
        if !normalized.is_empty() {
            self.required_capabilities.insert(normalized);
        }
        self
    }

    pub fn feature_flag(mut self, flag: impl Into<String>) -> Self {
        let normalized = flag.into().trim().to_ascii_lowercase();
        if !normalized.is_empty() {
            self.feature_flags.insert(normalized);
        }
        self
    }

    pub fn allow_profiles<I, S>(mut self, profiles: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let values = profiles
            .into_iter()
            .map(Into::into)
            .map(|profile| profile.trim().to_ascii_lowercase())
            .filter(|profile| !profile.is_empty())
            .collect::<BTreeSet<_>>();
        self.allowed_profiles = (!values.is_empty()).then_some(values);
        self
    }

    pub fn denied_message(mut self, message: impl Into<String>) -> Self {
        let normalized = message.into().trim().to_string();
        self.denied_message = (!normalized.is_empty()).then_some(normalized);
        self
    }

    pub fn hidden_reason(mut self, reason: impl Into<String>) -> Self {
        let normalized = reason.into().trim().to_string();
        self.hidden_reason = (!normalized.is_empty()).then_some(normalized);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommandPolicyOverride {
    pub visibility: Option<VisibilityMode>,
    pub availability: Option<CommandAvailability>,
    pub required_capabilities: BTreeSet<String>,
    pub hidden_reason: Option<String>,
    pub denied_message: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandPolicyContext {
    pub authenticated: bool,
    pub capabilities: BTreeSet<String>,
    pub enabled_features: BTreeSet<String>,
    pub active_profile: Option<String>,
}

impl CommandPolicyContext {
    pub fn authenticated(mut self, value: bool) -> Self {
        self.authenticated = value;
        self
    }

    pub fn with_capabilities<I, S>(mut self, capabilities: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.capabilities = capabilities
            .into_iter()
            .map(Into::into)
            .map(|capability| capability.trim().to_ascii_lowercase())
            .filter(|capability| !capability.is_empty())
            .collect();
        self
    }

    pub fn with_features<I, S>(mut self, features: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.enabled_features = features
            .into_iter()
            .map(Into::into)
            .map(|feature| feature.trim().to_ascii_lowercase())
            .filter(|feature| !feature.is_empty())
            .collect();
        self
    }

    pub fn with_profile(mut self, profile: impl Into<String>) -> Self {
        let normalized = profile.into().trim().to_ascii_lowercase();
        self.active_profile = (!normalized.is_empty()).then_some(normalized);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandVisibility {
    Hidden,
    Visible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandRunnable {
    Runnable,
    Denied,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessReason {
    HiddenByPolicy,
    DisabledByProduct,
    Unauthenticated,
    MissingCapabilities,
    FeatureDisabled(String),
    ProfileUnavailable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandAccess {
    pub visibility: CommandVisibility,
    pub runnable: CommandRunnable,
    pub reasons: Vec<AccessReason>,
    pub missing_capabilities: BTreeSet<String>,
}

impl CommandAccess {
    pub fn visible_runnable() -> Self {
        Self {
            visibility: CommandVisibility::Visible,
            runnable: CommandRunnable::Runnable,
            reasons: Vec::new(),
            missing_capabilities: BTreeSet::new(),
        }
    }

    pub fn hidden(reason: AccessReason) -> Self {
        Self {
            visibility: CommandVisibility::Hidden,
            runnable: CommandRunnable::Denied,
            reasons: vec![reason],
            missing_capabilities: BTreeSet::new(),
        }
    }

    pub fn visible_denied(reason: AccessReason) -> Self {
        Self {
            visibility: CommandVisibility::Visible,
            runnable: CommandRunnable::Denied,
            reasons: vec![reason],
            missing_capabilities: BTreeSet::new(),
        }
    }

    pub fn is_visible(&self) -> bool {
        matches!(self.visibility, CommandVisibility::Visible)
    }

    pub fn is_runnable(&self) -> bool {
        matches!(self.runnable, CommandRunnable::Runnable)
    }
}

#[derive(Debug, Clone, Default)]
pub struct CommandPolicyRegistry {
    entries: BTreeMap<CommandPath, CommandPolicy>,
    overrides: BTreeMap<CommandPath, CommandPolicyOverride>,
}

impl CommandPolicyRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, policy: CommandPolicy) -> Option<CommandPolicy> {
        self.entries.insert(policy.path.clone(), policy)
    }

    pub fn override_policy(
        &mut self,
        path: CommandPath,
        value: CommandPolicyOverride,
    ) -> Option<CommandPolicyOverride> {
        self.overrides.insert(path, value)
    }

    pub fn resolved_policy(&self, path: &CommandPath) -> Option<CommandPolicy> {
        let mut policy = self.entries.get(path)?.clone();
        if let Some(override_policy) = self.overrides.get(path) {
            if let Some(visibility) = override_policy.visibility {
                policy.visibility = visibility;
            }
            if let Some(availability) = override_policy.availability {
                policy.availability = availability;
            }
            policy
                .required_capabilities
                .extend(override_policy.required_capabilities.iter().cloned());
            if let Some(hidden_reason) = &override_policy.hidden_reason {
                policy.hidden_reason = Some(hidden_reason.clone());
            }
            if let Some(denied_message) = &override_policy.denied_message {
                policy.denied_message = Some(denied_message.clone());
            }
        }
        Some(policy)
    }

    pub fn evaluate(
        &self,
        path: &CommandPath,
        context: &CommandPolicyContext,
    ) -> Option<CommandAccess> {
        self.resolved_policy(path)
            .map(|policy| evaluate_policy(&policy, context))
    }

    pub fn contains(&self, path: &CommandPath) -> bool {
        self.entries.contains_key(path)
    }

    pub fn entries(&self) -> impl Iterator<Item = &CommandPolicy> {
        self.entries.values()
    }
}

pub fn evaluate_policy(policy: &CommandPolicy, context: &CommandPolicyContext) -> CommandAccess {
    if matches!(policy.availability, CommandAvailability::Disabled) {
        return CommandAccess::hidden(AccessReason::DisabledByProduct);
    }
    if matches!(policy.visibility, VisibilityMode::Hidden) {
        return CommandAccess::hidden(AccessReason::HiddenByPolicy);
    }
    if let Some(allowed_profiles) = &policy.allowed_profiles {
        match context.active_profile.as_ref() {
            Some(profile) if allowed_profiles.contains(profile) => {}
            Some(profile) => {
                return CommandAccess::hidden(AccessReason::ProfileUnavailable(profile.clone()));
            }
            None => return CommandAccess::hidden(AccessReason::ProfileUnavailable(String::new())),
        }
    }
    if let Some(feature) = policy
        .feature_flags
        .iter()
        .find(|feature| !context.enabled_features.contains(*feature))
    {
        return CommandAccess::hidden(AccessReason::FeatureDisabled(feature.clone()));
    }

    match policy.visibility {
        VisibilityMode::Public => CommandAccess::visible_runnable(),
        VisibilityMode::Authenticated => {
            if context.authenticated {
                CommandAccess::visible_runnable()
            } else {
                CommandAccess::visible_denied(AccessReason::Unauthenticated)
            }
        }
        VisibilityMode::CapabilityGated => {
            if !context.authenticated {
                return CommandAccess::visible_denied(AccessReason::Unauthenticated);
            }
            let missing = policy
                .required_capabilities
                .iter()
                .filter(|capability| !context.capabilities.contains(*capability))
                .cloned()
                .collect::<BTreeSet<_>>();
            if missing.is_empty() {
                CommandAccess::visible_runnable()
            } else {
                CommandAccess {
                    visibility: CommandVisibility::Visible,
                    runnable: CommandRunnable::Denied,
                    reasons: vec![AccessReason::MissingCapabilities],
                    missing_capabilities: missing,
                }
            }
        }
        VisibilityMode::Hidden => CommandAccess::hidden(AccessReason::HiddenByPolicy),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{
        AccessReason, CommandAccess, CommandAvailability, CommandPath, CommandPolicy,
        CommandPolicyContext, CommandPolicyOverride, CommandPolicyRegistry, CommandRunnable,
        CommandVisibility, VisibilityMode, evaluate_policy,
    };

    #[test]
    fn command_path_and_policy_builders_normalize_inputs() {
        let path = CommandPath::new([" Orch ", "", "Approval", "  Decide  "]);
        assert_eq!(
            path.as_slice(),
            &[
                "orch".to_string(),
                "approval".to_string(),
                "decide".to_string()
            ]
        );
        assert!(!path.is_empty());
        assert!(CommandPath::new(["", "   "]).is_empty());

        let policy = CommandPolicy::new(path.clone())
            .visibility(VisibilityMode::CapabilityGated)
            .require_capability(" Orch.Approval.Decide ")
            .require_capability("   ")
            .feature_flag(" Orch ")
            .feature_flag("")
            .allow_profiles([" Dev ", " ", "Prod"])
            .denied_message("  Sign in first  ")
            .hidden_reason("  hidden upstream  ");

        assert_eq!(policy.path, path);
        assert_eq!(policy.visibility, VisibilityMode::CapabilityGated);
        assert_eq!(
            policy.required_capabilities,
            BTreeSet::from(["orch.approval.decide".to_string()])
        );
        assert_eq!(policy.feature_flags, BTreeSet::from(["orch".to_string()]));
        assert_eq!(
            policy.allowed_profiles,
            Some(BTreeSet::from(["dev".to_string(), "prod".to_string()]))
        );
        assert_eq!(policy.denied_message.as_deref(), Some("Sign in first"));
        assert_eq!(policy.hidden_reason.as_deref(), Some("hidden upstream"));
    }

    #[test]
    fn policy_context_builders_normalize_inputs() {
        let context = CommandPolicyContext::default()
            .authenticated(true)
            .with_capabilities([" Orch.Read ", "", "orch.write"])
            .with_features([" Orch ", " "])
            .with_profile(" Dev ");

        assert!(context.authenticated);
        assert_eq!(
            context.capabilities,
            BTreeSet::from(["orch.read".to_string(), "orch.write".to_string()])
        );
        assert_eq!(
            context.enabled_features,
            BTreeSet::from(["orch".to_string()])
        );
        assert_eq!(context.active_profile.as_deref(), Some("dev"));
        assert_eq!(
            CommandPolicyContext::default()
                .with_profile("   ")
                .active_profile,
            None
        );
    }

    #[test]
    fn capability_gated_command_is_visible_but_denied_when_capability_missing() {
        let mut registry = CommandPolicyRegistry::new();
        let path = CommandPath::new(["orch", "approval", "decide"]);
        registry.register(
            CommandPolicy::new(path.clone())
                .visibility(VisibilityMode::CapabilityGated)
                .require_capability("orch.approval.decide"),
        );

        let access = registry
            .evaluate(&path, &CommandPolicyContext::default().authenticated(true))
            .expect("policy should exist");

        assert_eq!(access.visibility, CommandVisibility::Visible);
        assert_eq!(access.runnable, CommandRunnable::Denied);
        assert_eq!(access.reasons, vec![AccessReason::MissingCapabilities]);
    }

    #[test]
    fn required_capabilities_are_simple_conjunction() {
        let mut registry = CommandPolicyRegistry::new();
        let path = CommandPath::new(["orch", "policy", "add"]);
        registry.register(
            CommandPolicy::new(path.clone())
                .visibility(VisibilityMode::CapabilityGated)
                .require_capability("orch.policy.read")
                .require_capability("orch.policy.write"),
        );

        let access = registry
            .evaluate(
                &path,
                &CommandPolicyContext::default()
                    .authenticated(true)
                    .with_capabilities(["orch.policy.read"]),
            )
            .expect("policy should exist");

        assert!(access.missing_capabilities.contains("orch.policy.write"));
    }

    #[test]
    fn public_commands_can_remain_unauthenticated() {
        let policy = CommandPolicy::new(CommandPath::new(["help"]));
        let access = evaluate_policy(&policy, &CommandPolicyContext::default());
        assert_eq!(access, CommandAccess::visible_runnable());
    }

    #[test]
    fn overrides_can_hide_commands() {
        let mut registry = CommandPolicyRegistry::new();
        let path = CommandPath::new(["nh", "audit"]);
        registry.register(CommandPolicy::new(path.clone()));
        registry.override_policy(
            path.clone(),
            CommandPolicyOverride {
                visibility: Some(VisibilityMode::Hidden),
                ..CommandPolicyOverride::default()
            },
        );

        let access = registry
            .evaluate(&path, &CommandPolicyContext::default())
            .expect("policy should exist");
        assert_eq!(access.visibility, CommandVisibility::Hidden);
    }

    #[test]
    fn access_helpers_reflect_visibility_and_runnability() {
        let access = CommandAccess::visible_denied(AccessReason::Unauthenticated);
        assert!(access.is_visible());
        assert!(!access.is_runnable());
    }

    #[test]
    fn evaluate_policy_covers_disabled_hidden_feature_profile_and_auth_variants() {
        let disabled = CommandPolicy::new(CommandPath::new(["orch"]))
            .visibility(VisibilityMode::Authenticated);
        let mut disabled = disabled;
        disabled.availability = CommandAvailability::Disabled;
        assert_eq!(
            evaluate_policy(&disabled, &CommandPolicyContext::default()),
            CommandAccess::hidden(AccessReason::DisabledByProduct)
        );

        let hidden =
            CommandPolicy::new(CommandPath::new(["orch"])).visibility(VisibilityMode::Hidden);
        assert_eq!(
            evaluate_policy(&hidden, &CommandPolicyContext::default()),
            CommandAccess::hidden(AccessReason::HiddenByPolicy)
        );

        let profiled = CommandPolicy::new(CommandPath::new(["orch"]))
            .allow_profiles(["dev"])
            .feature_flag("orch");
        assert_eq!(
            evaluate_policy(&profiled, &CommandPolicyContext::default()),
            CommandAccess::hidden(AccessReason::ProfileUnavailable(String::new()))
        );
        assert_eq!(
            evaluate_policy(
                &profiled,
                &CommandPolicyContext::default().with_profile("prod")
            ),
            CommandAccess::hidden(AccessReason::ProfileUnavailable("prod".to_string()))
        );
        assert_eq!(
            evaluate_policy(
                &profiled,
                &CommandPolicyContext::default().with_profile("dev")
            ),
            CommandAccess::hidden(AccessReason::FeatureDisabled("orch".to_string()))
        );

        let auth_only = CommandPolicy::new(CommandPath::new(["auth", "status"]))
            .visibility(VisibilityMode::Authenticated);
        assert_eq!(
            evaluate_policy(&auth_only, &CommandPolicyContext::default()),
            CommandAccess::visible_denied(AccessReason::Unauthenticated)
        );
        assert_eq!(
            evaluate_policy(
                &auth_only,
                &CommandPolicyContext::default().authenticated(true)
            ),
            CommandAccess::visible_runnable()
        );

        let capability = CommandPolicy::new(CommandPath::new(["orch", "approval"]))
            .visibility(VisibilityMode::CapabilityGated)
            .require_capability("orch.approval.decide");
        assert_eq!(
            evaluate_policy(
                &capability,
                &CommandPolicyContext::default()
                    .authenticated(true)
                    .with_capabilities(["orch.approval.decide"])
            ),
            CommandAccess::visible_runnable()
        );
    }

    #[test]
    fn registry_resolution_applies_overrides_and_contains_lookup() {
        let path = CommandPath::new(["orch", "policy"]);
        let mut registry = CommandPolicyRegistry::new();
        assert!(!registry.contains(&path));
        assert!(registry.resolved_policy(&path).is_none());

        registry.register(
            CommandPolicy::new(path.clone())
                .visibility(VisibilityMode::Authenticated)
                .allow_profiles(["dev"])
                .denied_message("sign in")
                .hidden_reason("base hidden"),
        );
        assert!(registry.contains(&path));

        registry.override_policy(
            path.clone(),
            CommandPolicyOverride {
                visibility: Some(VisibilityMode::CapabilityGated),
                availability: Some(CommandAvailability::Disabled),
                required_capabilities: BTreeSet::from(["orch.policy.write".to_string()]),
                hidden_reason: Some("override hidden".to_string()),
                denied_message: Some("override denied".to_string()),
            },
        );

        let resolved = registry
            .resolved_policy(&path)
            .expect("policy should resolve");
        assert_eq!(resolved.visibility, VisibilityMode::CapabilityGated);
        assert_eq!(resolved.availability, CommandAvailability::Disabled);
        assert_eq!(
            resolved.required_capabilities,
            BTreeSet::from(["orch.policy.write".to_string()])
        );
        assert_eq!(resolved.hidden_reason.as_deref(), Some("override hidden"));
        assert_eq!(resolved.denied_message.as_deref(), Some("override denied"));
        assert_eq!(
            registry.evaluate(&path, &CommandPolicyContext::default()),
            Some(CommandAccess::hidden(AccessReason::DisabledByProduct))
        );
    }
}
