//! Runtime command visibility and access policy evaluation.
//!
//! This module exists to answer two related questions consistently:
//! should a command be shown, and may the current caller run it? Command
//! metadata can carry coarse auth requirements, but this module owns the
//! normalized runtime evaluation rules.
//!
//! In broad terms:
//!
//! - [`crate::core::command_policy::CommandPolicy`] describes one command's
//!   visibility and prerequisites
//! - [`crate::core::command_policy::CommandPolicyContext`] captures the runtime
//!   facts used during evaluation
//! - [`crate::core::command_policy::evaluate_policy`] turns the two into a
//!   concrete access decision
//! - [`crate::core::command_policy::CommandPolicyRegistry`] stores policies and
//!   applies per-path overrides
//!
//! Contract:
//!
//! - this module owns normalized policy evaluation, not command metadata shape
//! - visibility and runnability are distinct outcomes and should stay distinct
//! - callers should rely on the returned
//!   [`crate::core::command_policy::CommandAccess`] instead of re-deriving
//!   access rules ad hoc
//!
//! Public API shape:
//!
//! - [`crate::core::command_policy::CommandPolicy`] remains a fluent semantic
//!   policy DSL
//! - [`crate::core::command_policy::CommandPolicyOverride`] uses an explicit
//!   constructor plus `with_*` normalization helpers so overrides follow the
//!   same normalization rules as base policies

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// Normalized command path used as the lookup key for policy evaluation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CommandPath(Vec<String>);

impl CommandPath {
    /// Builds a normalized command path, lowercasing segments and dropping
    /// empty values after trimming.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::command_policy::CommandPath;
    ///
    /// let path = CommandPath::new([" Orch ", "", "Approval", "  Decide  "]);
    ///
    /// assert_eq!(path.as_slice(), &["orch", "approval", "decide"]);
    /// ```
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

    /// Returns the normalized path segments.
    pub fn as_slice(&self) -> &[String] {
        self.0.as_slice()
    }

    /// Returns `true` when the path contains no usable segments.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Visibility contract applied before runtime capability checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityMode {
    /// Show and allow the command without authentication.
    Public,
    /// Show the command, but require authentication to run it.
    Authenticated,
    /// Show the command only when capability checks pass.
    CapabilityGated,
    /// Hide the command regardless of runtime context.
    Hidden,
}

/// Relative authentication strength available in the current session.
///
/// This models session quality rather than product authorization. Use
/// capabilities for "may the caller do this?" and auth strength for "how
/// strongly was the caller authenticated for this session?".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthStrength {
    /// Standard authenticated session strength.
    Basic,
    /// Stronger authentication such as MFA-backed login.
    Strong,
}

impl AuthStrength {
    /// Returns the canonical user-facing label for this strength.
    pub fn as_label(self) -> &'static str {
        match self {
            Self::Basic => "basic",
            Self::Strong => "strong",
        }
    }
}

/// Observed runtime state for one named credential or token.
///
/// Presence is modeled by whether the credential exists in the policy context.
/// When present, `valid` indicates whether the host believes the credential is
/// currently usable; `ttl_seconds` can carry any remaining freshness window
/// known by the caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialState {
    /// Whether the credential is currently valid for use.
    pub valid: bool,
    /// Remaining lifetime, when known.
    pub ttl_seconds: Option<u64>,
}

impl CredentialState {
    /// Creates state for a present credential with unknown validity.
    pub fn present() -> Self {
        Self {
            valid: false,
            ttl_seconds: None,
        }
    }

    /// Creates state for a valid credential with unknown remaining TTL.
    pub fn valid() -> Self {
        Self {
            valid: true,
            ttl_seconds: None,
        }
    }

    /// Creates state for a valid credential with a known remaining TTL.
    pub fn valid_for(ttl_seconds: u64) -> Self {
        Self {
            valid: true,
            ttl_seconds: Some(ttl_seconds),
        }
    }
}

/// Requirement applied to one named credential or token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum CredentialRequirement {
    /// Require that the named credential exists.
    Present {
        /// Credential or token namespace, such as `osp` or `mreg`.
        service: String,
    },
    /// Require that the named credential exists and is currently valid.
    Valid {
        /// Credential or token namespace, such as `osp` or `mreg`.
        service: String,
    },
    /// Require that the named credential exists, is valid, and has at least
    /// the specified remaining lifetime.
    Fresh {
        /// Credential or token namespace, such as `osp` or `mreg`.
        service: String,
        /// Minimum remaining lifetime required for the command.
        min_ttl_seconds: u64,
    },
}

impl CredentialRequirement {
    /// Requires that the named credential exists.
    pub fn present(service: impl Into<String>) -> Self {
        Self::Present {
            service: normalize_service_name(service),
        }
    }

    /// Requires that the named credential exists and is valid.
    pub fn valid(service: impl Into<String>) -> Self {
        Self::Valid {
            service: normalize_service_name(service),
        }
    }

    /// Requires that the named credential exists, is valid, and is fresh.
    pub fn fresh(service: impl Into<String>, min_ttl_seconds: u64) -> Self {
        Self::Fresh {
            service: normalize_service_name(service),
            min_ttl_seconds,
        }
    }

    /// Returns the normalized credential namespace used by this requirement.
    pub fn service(&self) -> &str {
        match self {
            Self::Present { service } | Self::Valid { service } | Self::Fresh { service, .. } => {
                service
            }
        }
    }

    fn normalized(self) -> Option<Self> {
        let service = normalize_service_name(self.service());
        if service.is_empty() {
            return None;
        }

        Some(match self {
            Self::Present { .. } => Self::Present { service },
            Self::Valid { .. } => Self::Valid { service },
            Self::Fresh {
                min_ttl_seconds, ..
            } => Self::Fresh {
                service,
                min_ttl_seconds,
            },
        })
    }
}

/// Session-state requirements attached to a command policy.
///
/// These requirements complement capabilities instead of replacing them:
/// capabilities answer whether the caller is allowed, while session
/// requirements answer whether the current session is strong enough right now.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRequirements {
    /// Minimum auth strength required by the command, when any.
    pub auth_strength: Option<AuthStrength>,
    /// Credential or token requirements that must be satisfied.
    pub credentials: Vec<CredentialRequirement>,
}

impl SessionRequirements {
    /// Returns `true` when the requirement set is empty.
    pub fn is_empty(&self) -> bool {
        self.auth_strength.is_none() && self.credentials.is_empty()
    }

    fn require_auth_strength(mut self, auth_strength: AuthStrength) -> Self {
        self.auth_strength = Some(auth_strength);
        self
    }

    fn require_credential(mut self, requirement: CredentialRequirement) -> Self {
        if let Some(requirement) = requirement.normalized() {
            self.credentials.push(requirement);
        }
        self
    }
}

/// Product-level availability state for a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandAvailability {
    /// The product currently exposes the command.
    Available,
    /// The product disables the command entirely.
    Disabled,
}

/// Declarative policy used to decide whether a command is visible and runnable.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct CommandPolicy {
    /// Normalized command path used as the registry key.
    pub path: CommandPath,
    /// Baseline visibility rule for the command.
    pub visibility: VisibilityMode,
    /// Product-level availability state for the command.
    pub availability: CommandAvailability,
    /// Capabilities that must be present for capability-gated commands.
    pub required_capabilities: BTreeSet<String>,
    /// Feature flags that must be enabled before the command is exposed.
    pub feature_flags: BTreeSet<String>,
    /// Session-state requirements that control whether the command is visible.
    pub visible_session_requirements: SessionRequirements,
    /// Session-state requirements that control whether the command may run.
    pub run_session_requirements: SessionRequirements,
    /// Profiles allowed to see the command, when restricted.
    pub allowed_profiles: Option<BTreeSet<String>>,
    /// Optional message shown when the command is visible but denied.
    pub denied_message: Option<String>,
    /// Optional explanation for why the command is hidden.
    pub hidden_reason: Option<String>,
}

impl CommandPolicy {
    /// Creates a public, available policy for the given command path.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::command_policy::{CommandPath, CommandPolicy, VisibilityMode};
    ///
    /// let policy = CommandPolicy::new(CommandPath::new([" Orch ", "Approval "]))
    ///     .visibility(VisibilityMode::CapabilityGated)
    ///     .require_capability(" Orch.Approval.Decide ")
    ///     .feature_flag(" Orch ")
    ///     .allow_profiles([" Dev ", ""]);
    ///
    /// assert_eq!(policy.path.as_slice(), &["orch", "approval"]);
    /// assert!(policy.required_capabilities.contains("orch.approval.decide"));
    /// assert!(policy.feature_flags.contains("orch"));
    /// assert!(policy.allowed_profiles.as_ref().unwrap().contains("dev"));
    /// ```
    pub fn new(path: CommandPath) -> Self {
        Self {
            path,
            visibility: VisibilityMode::Public,
            availability: CommandAvailability::Available,
            required_capabilities: BTreeSet::new(),
            feature_flags: BTreeSet::new(),
            visible_session_requirements: SessionRequirements::default(),
            run_session_requirements: SessionRequirements::default(),
            allowed_profiles: None,
            denied_message: None,
            hidden_reason: None,
        }
    }

    /// Sets the visibility mode applied during policy evaluation.
    pub fn visibility(mut self, visibility: VisibilityMode) -> Self {
        self.visibility = visibility;
        self
    }

    /// Adds a required capability after trimming and lowercasing it.
    pub fn require_capability(mut self, capability: impl Into<String>) -> Self {
        let normalized = capability.into().trim().to_ascii_lowercase();
        if !normalized.is_empty() {
            self.required_capabilities.insert(normalized);
        }
        self
    }

    /// Adds a feature flag prerequisite after trimming and lowercasing it.
    pub fn feature_flag(mut self, flag: impl Into<String>) -> Self {
        let normalized = flag.into().trim().to_ascii_lowercase();
        if !normalized.is_empty() {
            self.feature_flags.insert(normalized);
        }
        self
    }

    /// Requires a minimum auth strength to reveal the command at all.
    pub fn require_visible_auth_strength(mut self, auth_strength: AuthStrength) -> Self {
        self.visible_session_requirements = self
            .visible_session_requirements
            .clone()
            .require_auth_strength(auth_strength);
        self
    }

    /// Requires a minimum auth strength to execute the command.
    pub fn require_auth_strength(mut self, auth_strength: AuthStrength) -> Self {
        self.run_session_requirements = self
            .run_session_requirements
            .clone()
            .require_auth_strength(auth_strength);
        self
    }

    /// Requires a credential state before the command is visible.
    pub fn require_visible_credential(mut self, requirement: CredentialRequirement) -> Self {
        self.visible_session_requirements = self
            .visible_session_requirements
            .clone()
            .require_credential(requirement);
        self
    }

    /// Requires a credential state before the command may run.
    pub fn require_credential(mut self, requirement: CredentialRequirement) -> Self {
        self.run_session_requirements = self
            .run_session_requirements
            .clone()
            .require_credential(requirement);
        self
    }

    /// Restricts the policy to the provided normalized profile names.
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

    /// Sets the user-facing denial message when the command is visible but not runnable.
    pub fn denied_message(mut self, message: impl Into<String>) -> Self {
        let normalized = message.into().trim().to_string();
        self.denied_message = (!normalized.is_empty()).then_some(normalized);
        self
    }

    /// Sets the hidden-reason metadata after trimming empty values away.
    pub fn hidden_reason(mut self, reason: impl Into<String>) -> Self {
        let normalized = reason.into().trim().to_string();
        self.hidden_reason = (!normalized.is_empty()).then_some(normalized);
        self
    }
}

/// Partial override applied on top of a registered [`CommandPolicy`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
#[must_use]
pub struct CommandPolicyOverride {
    /// Replacement visibility mode, when overridden.
    pub visibility: Option<VisibilityMode>,
    /// Replacement availability state, when overridden.
    pub availability: Option<CommandAvailability>,
    /// Additional required capabilities merged into the base policy.
    pub required_capabilities: BTreeSet<String>,
    /// Replacement visible-session requirements, when overridden.
    pub visible_session_requirements: Option<SessionRequirements>,
    /// Replacement run-session requirements, when overridden.
    pub run_session_requirements: Option<SessionRequirements>,
    /// Replacement hidden-reason metadata, when overridden.
    pub hidden_reason: Option<String>,
    /// Replacement denial message, when overridden.
    pub denied_message: Option<String>,
}

impl CommandPolicyOverride {
    /// Creates an empty override.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::command_policy::{
    ///     CommandAvailability, CommandPolicyOverride, VisibilityMode,
    /// };
    ///
    /// let override_policy = CommandPolicyOverride::new()
    ///     .with_visibility(Some(VisibilityMode::CapabilityGated))
    ///     .with_availability(Some(CommandAvailability::Disabled))
    ///     .with_required_capabilities([" Orch.Policy.Write ", ""]);
    ///
    /// assert_eq!(
    ///     override_policy.visibility,
    ///     Some(VisibilityMode::CapabilityGated)
    /// );
    /// assert_eq!(
    ///     override_policy.availability,
    ///     Some(CommandAvailability::Disabled)
    /// );
    /// assert!(override_policy.required_capabilities.contains("orch.policy.write"));
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the overridden visibility mode.
    pub fn with_visibility(mut self, visibility: Option<VisibilityMode>) -> Self {
        self.visibility = visibility;
        self
    }

    /// Replaces the overridden availability state.
    pub fn with_availability(mut self, availability: Option<CommandAvailability>) -> Self {
        self.availability = availability;
        self
    }

    /// Replaces the merged required-capability set with normalized values.
    pub fn with_required_capabilities<I, S>(mut self, required_capabilities: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.required_capabilities = required_capabilities
            .into_iter()
            .map(Into::into)
            .map(|capability| capability.trim().to_ascii_lowercase())
            .filter(|capability| !capability.is_empty())
            .collect();
        self
    }

    /// Replaces the visible-session requirements.
    pub fn with_visible_session_requirements(
        mut self,
        visible_session_requirements: Option<SessionRequirements>,
    ) -> Self {
        self.visible_session_requirements = visible_session_requirements;
        self
    }

    /// Replaces the run-session requirements.
    pub fn with_run_session_requirements(
        mut self,
        run_session_requirements: Option<SessionRequirements>,
    ) -> Self {
        self.run_session_requirements = run_session_requirements;
        self
    }

    /// Replaces the optional hidden-reason metadata.
    pub fn with_hidden_reason(mut self, hidden_reason: Option<String>) -> Self {
        self.hidden_reason = hidden_reason
            .map(|reason| reason.trim().to_string())
            .filter(|reason| !reason.is_empty());
        self
    }

    /// Replaces the optional denial message.
    pub fn with_denied_message(mut self, denied_message: Option<String>) -> Self {
        self.denied_message = denied_message
            .map(|message| message.trim().to_string())
            .filter(|message| !message.is_empty());
        self
    }
}

/// Runtime facts used to evaluate a command policy.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[must_use]
pub struct CommandPolicyContext {
    /// Whether the current caller is authenticated.
    pub authenticated: bool,
    /// Relative auth strength of the current session, when known.
    pub auth_strength: Option<AuthStrength>,
    /// Normalized capabilities available to the caller.
    pub capabilities: BTreeSet<String>,
    /// Named credentials or tokens currently available in the session.
    pub credentials: BTreeMap<String, CredentialState>,
    /// Normalized feature flags enabled in the current product build.
    pub enabled_features: BTreeSet<String>,
    /// Active normalized profile name, when one is selected.
    pub active_profile: Option<String>,
}

impl CommandPolicyContext {
    /// Sets whether the current user is authenticated.
    pub fn authenticated(mut self, value: bool) -> Self {
        self.authenticated = value;
        if !value {
            self.auth_strength = None;
        }
        self
    }

    /// Sets the auth strength and marks the context authenticated.
    pub fn with_auth_strength(mut self, auth_strength: AuthStrength) -> Self {
        self.authenticated = true;
        self.auth_strength = Some(auth_strength);
        self
    }

    /// Inserts or replaces the state for one named credential.
    pub fn with_credential(
        mut self,
        service: impl Into<String>,
        credential: CredentialState,
    ) -> Self {
        let normalized = normalize_service_name(service);
        if !normalized.is_empty() {
            self.credentials.insert(normalized, credential);
        }
        self
    }

    /// Replaces the current capability set with normalized capability names.
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

    /// Replaces the enabled feature set with normalized feature names.
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

    /// Sets the active profile after trimming and lowercasing it.
    pub fn with_profile(mut self, profile: impl Into<String>) -> Self {
        let normalized = profile.into().trim().to_ascii_lowercase();
        self.active_profile = (!normalized.is_empty()).then_some(normalized);
        self
    }
}

/// Visibility outcome produced by policy evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandVisibility {
    /// The command should not be shown to the caller.
    Hidden,
    /// The command should be shown to the caller.
    Visible,
}

/// Runnable outcome produced by policy evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandRunnable {
    /// The caller may execute the command.
    Runnable,
    /// The caller may see the command but not run it.
    Denied,
}

/// Reason codes attached to denied or hidden command access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessReason {
    /// The policy explicitly hides the command.
    HiddenByPolicy,
    /// Product configuration disables the command entirely.
    DisabledByProduct,
    /// Authentication is required before the command may run.
    Unauthenticated,
    /// One or more required capabilities are missing.
    MissingCapabilities,
    /// A required credential is missing entirely.
    MissingCredential(String),
    /// A required credential exists but is not currently valid.
    InvalidCredential(String),
    /// A required credential does not have enough remaining lifetime.
    InsufficientCredentialTtl {
        /// Credential or token namespace, such as `osp` or `mreg`.
        service: String,
        /// Required minimum remaining lifetime.
        required_ttl_seconds: u64,
    },
    /// The current session auth strength is too weak.
    InsufficientAuthStrength(AuthStrength),
    /// A required feature flag is disabled.
    FeatureDisabled(String),
    /// The command is unavailable in the active profile.
    ProfileUnavailable(String),
}

/// Effective access decision for a command under a specific context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandAccess {
    /// Whether the command should be shown to the caller.
    pub visibility: CommandVisibility,
    /// Whether the caller may execute the command.
    pub runnable: CommandRunnable,
    /// Reasons that explain why access was restricted.
    pub reasons: Vec<AccessReason>,
    /// Required capabilities absent from the current context.
    pub missing_capabilities: BTreeSet<String>,
}

impl CommandAccess {
    /// Returns an access result that is visible and runnable.
    pub fn visible_runnable() -> Self {
        Self {
            visibility: CommandVisibility::Visible,
            runnable: CommandRunnable::Runnable,
            reasons: Vec::new(),
            missing_capabilities: BTreeSet::new(),
        }
    }

    /// Returns an access result that is hidden and denied for the given reason.
    pub fn hidden(reason: AccessReason) -> Self {
        Self {
            visibility: CommandVisibility::Hidden,
            runnable: CommandRunnable::Denied,
            reasons: vec![reason],
            missing_capabilities: BTreeSet::new(),
        }
    }

    /// Returns an access result that is visible but denied for the given reason.
    pub fn visible_denied(reason: AccessReason) -> Self {
        Self {
            visibility: CommandVisibility::Visible,
            runnable: CommandRunnable::Denied,
            reasons: vec![reason],
            missing_capabilities: BTreeSet::new(),
        }
    }

    /// Returns `true` when the command should be shown to the user.
    pub fn is_visible(&self) -> bool {
        matches!(self.visibility, CommandVisibility::Visible)
    }

    /// Returns `true` when the command may be executed.
    pub fn is_runnable(&self) -> bool {
        matches!(self.runnable, CommandRunnable::Runnable)
    }
}

/// Registry of command policies and per-path overrides.
#[derive(Debug, Clone, Default)]
pub struct CommandPolicyRegistry {
    entries: BTreeMap<CommandPath, CommandPolicy>,
    overrides: BTreeMap<CommandPath, CommandPolicyOverride>,
}

impl CommandPolicyRegistry {
    /// Creates an empty policy registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` when no base policies are registered.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Registers a policy and returns the previous policy for the same path, if any.
    pub fn register(&mut self, policy: CommandPolicy) -> Option<CommandPolicy> {
        self.entries.insert(policy.path.clone(), policy)
    }

    /// Stores an override for a path and returns the previous override, if any.
    pub fn override_policy(
        &mut self,
        path: CommandPath,
        value: CommandPolicyOverride,
    ) -> Option<CommandPolicyOverride> {
        self.overrides.insert(path, value)
    }

    /// Returns the registered policy merged with any override for the same
    /// path.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::command_policy::{
    ///     CommandAvailability, CommandPath, CommandPolicy, CommandPolicyOverride,
    ///     CommandPolicyRegistry, VisibilityMode,
    /// };
    ///
    /// let path = CommandPath::new(["orch", "policy"]);
    /// let mut registry = CommandPolicyRegistry::new();
    /// registry.register(CommandPolicy::new(path.clone()).visibility(VisibilityMode::Authenticated));
    /// registry.override_policy(
    ///     path.clone(),
    ///     CommandPolicyOverride::new()
    ///         .with_availability(Some(CommandAvailability::Disabled))
    ///         .with_required_capabilities(["orch.policy.write"]),
    /// );
    ///
    /// let resolved = registry.resolved_policy(&path).unwrap();
    ///
    /// assert_eq!(resolved.visibility, VisibilityMode::Authenticated);
    /// assert_eq!(resolved.availability, CommandAvailability::Disabled);
    /// assert!(resolved.required_capabilities.contains("orch.policy.write"));
    /// ```
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
            if let Some(requirements) = &override_policy.visible_session_requirements {
                policy.visible_session_requirements = requirements.clone();
            }
            if let Some(requirements) = &override_policy.run_session_requirements {
                policy.run_session_requirements = requirements.clone();
            }
            if let Some(hidden_reason) = &override_policy.hidden_reason {
                policy.hidden_reason = Some(hidden_reason.clone());
            }
            if let Some(denied_message) = &override_policy.denied_message {
                policy.denied_message = Some(denied_message.clone());
            }
        }
        Some(policy)
    }

    /// Evaluates the resolved policy for `path`, or `None` if the path is unknown.
    pub fn evaluate(
        &self,
        path: &CommandPath,
        context: &CommandPolicyContext,
    ) -> Option<CommandAccess> {
        self.resolved_policy(path)
            .map(|policy| evaluate_policy(&policy, context))
    }

    /// Returns `true` when a policy is registered for `path`.
    pub fn contains(&self, path: &CommandPath) -> bool {
        self.entries.contains_key(path)
    }

    /// Iterates over the registered base policies.
    pub fn entries(&self) -> impl Iterator<Item = &CommandPolicy> {
        self.entries.values()
    }
}

/// Evaluates a single policy against the supplied runtime context.
///
/// Visibility and runnability are evaluated separately. For example, an
/// authenticated-only command stays visible to unauthenticated users, but is
/// denied at execution time.
///
/// # Examples
///
/// ```
/// use osp_cli::core::command_policy::{
///     AccessReason, CommandPath, CommandPolicy, CommandPolicyContext,
///     CommandRunnable, CommandVisibility, VisibilityMode, evaluate_policy,
/// };
///
/// let policy = CommandPolicy::new(CommandPath::new(["orch", "approval", "decide"]))
///     .visibility(VisibilityMode::CapabilityGated)
///     .require_capability("orch.approval.decide");
///
/// let denied = evaluate_policy(
///     &policy,
///     &CommandPolicyContext::default().authenticated(true),
/// );
/// assert_eq!(denied.visibility, CommandVisibility::Visible);
/// assert_eq!(denied.runnable, CommandRunnable::Denied);
/// assert_eq!(denied.reasons, vec![AccessReason::MissingCapabilities]);
///
/// let allowed = evaluate_policy(
///     &policy,
///     &CommandPolicyContext::default()
///         .authenticated(true)
///         .with_capabilities(["orch.approval.decide"]),
/// );
/// assert!(allowed.is_runnable());
/// ```
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
    if let Some(reason) =
        evaluate_session_requirements(&policy.visible_session_requirements, context)
    {
        return CommandAccess::hidden(reason);
    }

    let access = match policy.visibility {
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
    };

    if !access.is_runnable() {
        return access;
    }

    if let Some(reason) = evaluate_session_requirements(&policy.run_session_requirements, context) {
        return CommandAccess::visible_denied(reason);
    }

    access
}

fn normalize_service_name(value: impl Into<String>) -> String {
    value.into().trim().to_ascii_lowercase()
}

fn evaluate_session_requirements(
    requirements: &SessionRequirements,
    context: &CommandPolicyContext,
) -> Option<AccessReason> {
    if let Some(required_strength) = requirements.auth_strength {
        match context.auth_strength {
            Some(actual) if actual >= required_strength => {}
            _ => return Some(AccessReason::InsufficientAuthStrength(required_strength)),
        }
    }

    for requirement in &requirements.credentials {
        let Some(credential) = context.credentials.get(requirement.service()) else {
            return Some(AccessReason::MissingCredential(
                requirement.service().to_string(),
            ));
        };

        match requirement {
            CredentialRequirement::Present { .. } => {}
            CredentialRequirement::Valid { service } => {
                if !credential.valid {
                    return Some(AccessReason::InvalidCredential(service.clone()));
                }
            }
            CredentialRequirement::Fresh {
                service,
                min_ttl_seconds,
            } => {
                if !credential.valid {
                    return Some(AccessReason::InvalidCredential(service.clone()));
                }
                if credential.ttl_seconds.unwrap_or_default() < *min_ttl_seconds {
                    return Some(AccessReason::InsufficientCredentialTtl {
                        service: service.clone(),
                        required_ttl_seconds: *min_ttl_seconds,
                    });
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{
        AccessReason, AuthStrength, CommandAccess, CommandAvailability, CommandPath, CommandPolicy,
        CommandPolicyContext, CommandPolicyOverride, CommandPolicyRegistry, CommandRunnable,
        CommandVisibility, CredentialRequirement, CredentialState, VisibilityMode, evaluate_policy,
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
            .with_profile(" Dev ")
            .with_auth_strength(AuthStrength::Strong)
            .with_credential(" OSP ", CredentialState::valid_for(900));

        assert!(context.authenticated);
        assert_eq!(context.auth_strength, Some(AuthStrength::Strong));
        assert_eq!(
            context.capabilities,
            BTreeSet::from(["orch.read".to_string(), "orch.write".to_string()])
        );
        assert_eq!(
            context.credentials.get("osp"),
            Some(&CredentialState::valid_for(900))
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
            CommandPolicyOverride::new().with_visibility(Some(VisibilityMode::Hidden)),
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
    fn visible_and_run_session_requirements_produce_hidden_denied_and_runnable_states() {
        let policy = CommandPolicy::new(CommandPath::new(["orch", "apply"]))
            .require_visible_auth_strength(AuthStrength::Strong)
            .require_credential(CredentialRequirement::fresh("osp", 600));

        let hidden = evaluate_policy(&policy, &CommandPolicyContext::default());
        assert_eq!(
            hidden,
            CommandAccess::hidden(AccessReason::InsufficientAuthStrength(AuthStrength::Strong))
        );

        let denied = evaluate_policy(
            &policy,
            &CommandPolicyContext::default().with_auth_strength(AuthStrength::Strong),
        );
        assert_eq!(
            denied,
            CommandAccess::visible_denied(AccessReason::MissingCredential("osp".to_string()))
        );

        let allowed = evaluate_policy(
            &policy,
            &CommandPolicyContext::default()
                .with_auth_strength(AuthStrength::Strong)
                .with_credential("osp", CredentialState::valid_for(900)),
        );
        assert!(allowed.is_runnable());
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
            CommandPolicyOverride::new()
                .with_visibility(Some(VisibilityMode::CapabilityGated))
                .with_availability(Some(CommandAvailability::Disabled))
                .with_required_capabilities(["orch.policy.write"])
                .with_hidden_reason(Some("override hidden".to_string()))
                .with_denied_message(Some("override denied".to_string())),
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
