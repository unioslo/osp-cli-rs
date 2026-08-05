//! Generic command-access recovery hooks for wrapper crates.
//!
//! This module exists so downstream products can recover from denied command
//! access without forking the host runtime. Upstream owns when command access
//! is checked and when one retry is allowed; wrappers own how a session is
//! refreshed or upgraded.

use miette::Result;

use super::{AppRuntime, AppSession, TerminalKind};
use crate::core::command_policy::CommandAccess;

/// Distinguishes which command surface triggered an access check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandAccessKind {
    /// Built-in upstream command such as `config` or `theme`.
    Builtin,
    /// External command path resolved from native/plugin metadata.
    External,
}

/// One denied command-access event that may be recoverable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessRecoveryRequest {
    /// Whether the current host surface is CLI or REPL.
    pub terminal_kind: TerminalKind,
    /// Which command surface triggered the access check.
    pub command_kind: CommandAccessKind,
    /// Human-readable normalized command label.
    pub command: String,
    /// The denied access result that triggered recovery.
    pub access: CommandAccess,
}

impl AccessRecoveryRequest {
    /// Creates a new recovery request.
    pub fn new(
        terminal_kind: TerminalKind,
        command_kind: CommandAccessKind,
        command: impl Into<String>,
        access: CommandAccess,
    ) -> Self {
        Self {
            terminal_kind,
            command_kind,
            command: command.into(),
            access,
        }
    }
}

/// Outcome returned by a recovery hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessRecoveryOutcome {
    /// The hook declined to change runtime/session state.
    NoChange,
    /// The hook refreshed or upgraded the session and the command should retry.
    Recovered,
}

/// Product-owned recovery hook used when command access is denied.
///
/// Wrappers should keep the implementation boring:
/// inspect the denied access reason, refresh local session state when
/// appropriate, mutate the provided runtime/session in place, and return
/// [`AccessRecoveryOutcome::Recovered`] only when a retry is warranted.
pub trait CommandAccessRecovery: Send + Sync {
    /// Attempts to recover from denied access for one command.
    fn try_recover(
        &self,
        request: &AccessRecoveryRequest,
        runtime: &mut AppRuntime,
        session: &mut AppSession,
    ) -> Result<AccessRecoveryOutcome>;
}
