//! Session facts and access decisions for declared command authentication.
//!
//! Plugin wire metadata is evaluated directly. Products own acquisition and
//! additional visibility through `app::AccessPolicy`.

use super::plugin::{
    DescribeCommandAuthV1, DescribeCredentialRequirementV1, DescribeVisibilityModeV1,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Normalized command path used as the lookup key for policy evaluation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct CommandPath(Vec<String>);

impl CommandPath {
    /// Builds a normalized command path, lowercasing segments and dropping
    /// empty values after trimming.
    ///
    /// # Examples
    ///
    /// ```text
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

/// A product-selected sign-in credential and its user-facing service name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedCredential {
    /// Key in `SessionFacts.credentials`.
    pub name: String,
    /// Product-owned service label, such as "Siteadmin" or "MREG".
    pub label: String,
}

/// Live session facts supplied by the product access hook.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionFacts {
    /// Whether the product has an authenticated session.
    pub authenticated: bool,
    /// Credentials selected for sign-in, in product display order. An empty
    /// selection omits the intro sign-in row; it does not change access policy.
    pub selected_credentials: Vec<SelectedCredential>,
    /// Named credentials currently observed by the product, keyed by trimmed
    /// lowercase service names.
    pub credentials: BTreeMap<String, CredentialState>,
}

/// Reason codes attached to denied or hidden command access.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessReason {
    /// The policy explicitly hides the command.
    HiddenByPolicy,
    /// The product hides the command, optionally explaining why.
    HiddenByProduct(Option<String>),
    /// Authentication is required before the command may run.
    Unauthenticated,
    /// A required credential is missing entirely.
    MissingCredential(String),
    /// A required credential exists but is not currently valid.
    InvalidCredential(String),
}

/// Effective access decision for a command under a specific context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandAccess {
    /// The command is visible and may be executed.
    Runnable,
    /// The command is visible but cannot be executed.
    Denied(AccessReason),
    /// The command is hidden and cannot be executed.
    Hidden(AccessReason),
}

impl CommandAccess {
    /// Returns `true` when the command should be shown to the user.
    pub fn is_visible(&self) -> bool {
        !matches!(self, Self::Hidden(_))
    }

    /// Returns `true` when the command may be executed.
    pub fn is_runnable(&self) -> bool {
        matches!(self, Self::Runnable)
    }

    /// Returns the reason access was restricted, if any.
    pub fn reason(&self) -> Option<&AccessReason> {
        match self {
            Self::Runnable => None,
            Self::Denied(reason) | Self::Hidden(reason) => Some(reason),
        }
    }
}

/// Evaluates supported plugin auth metadata against the current session.
///
/// Legacy capability, feature, visible-session, strength and freshness fields
/// remain accepted on the wire but do not participate in access decisions.
pub fn evaluate_auth(auth: Option<&DescribeCommandAuthV1>, facts: &SessionFacts) -> CommandAccess {
    let Some(auth) = auth else {
        return CommandAccess::Runnable;
    };
    if matches!(auth.visibility, Some(DescribeVisibilityModeV1::Hidden)) {
        return CommandAccess::Hidden(AccessReason::HiddenByPolicy);
    }
    if matches!(
        auth.visibility,
        Some(DescribeVisibilityModeV1::Authenticated)
    ) && !facts.authenticated
    {
        return CommandAccess::Denied(AccessReason::Unauthenticated);
    }
    for requirement in auth
        .run_session
        .iter()
        .flat_map(|session| &session.credentials)
    {
        let (service, valid) = match requirement {
            DescribeCredentialRequirementV1::Present { service } => (service, false),
            DescribeCredentialRequirementV1::Valid { service } => (service, true),
            DescribeCredentialRequirementV1::Fresh { .. } => continue,
        };
        let service = service.trim().to_ascii_lowercase();
        if service.is_empty() {
            continue;
        }
        let Some(credential) = facts.credentials.get(&service) else {
            return CommandAccess::Denied(AccessReason::MissingCredential(service));
        };
        if valid && !credential.valid {
            return CommandAccess::Denied(AccessReason::InvalidCredential(service));
        }
    }
    CommandAccess::Runnable
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {

    #[test]
    fn policy_internal_example_1() {
        use crate::core::command_policy::CommandPath;

        let path = CommandPath::new([" Orch ", "", "Approval", "  Decide  "]);

        assert_eq!(path.as_slice(), &["orch", "approval", "decide"]);
    }
}
