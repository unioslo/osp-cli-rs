//! In-process native command surface.
//!
//! This module exists so `osp` can expose built-in commands through the same
//! catalog, policy, and dispatch-adjacent shapes that plugin commands use,
//! without spawning a subprocess.
//!
//! High-level flow:
//!
//! - register native command implementations in a [`NativeCommandRegistry`]
//! - describe them through clap-derived metadata
//! - project that metadata into completion, help, and policy surfaces
//! - execute them in-process with a small runtime context
//!
//! Contract:
//!
//! - native commands are the in-process counterpart to plugin commands
//! - catalog and policy projection should stay aligned with the plugin-facing
//!   protocol types in `crate::core::plugin`
//!
//! Public API shape:
//!
//! - [`NativeCommandRegistry`] is the canonical registration surface
//! - catalog/context structs stay plain describe-time or execute-time payloads
//! - commands should expose behavior through the registry rather than by
//!   leaking host-internal runtime state

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Result;
use clap::Command;

use crate::completion::CommandSpec;
use crate::config::ResolvedConfig;
use crate::core::command_policy::CommandPolicyRegistry;
use crate::core::plugin::{DescribeCommandAuthV1, DescribeCommandV1, ResponseV1};
use crate::core::runtime::RuntimeHints;

/// Public metadata snapshot for one registered native command.
///
/// This is the describe-time surface projected into help, completion, and
/// policy code. It is not an execution handle; callers should fetch the command
/// from [`NativeCommandRegistry`] when they need to run it.
#[derive(Debug, Clone)]
pub struct NativeCommandCatalogEntry {
    /// Canonical command path root exposed to CLI and REPL users.
    pub name: String,
    /// Short human-facing summary used in listings and overviews.
    pub about: String,
    /// Optional auth/visibility metadata projected into policy surfaces.
    pub auth: Option<DescribeCommandAuthV1>,
    /// Direct child names available immediately below this command.
    pub subcommands: Vec<String>,
    /// Completion tree rooted at this command's describe-time shape.
    pub completion: CommandSpec,
}

/// Runtime context passed to native command implementations.
///
/// This keeps the command surface small and stable: commands receive the
/// resolved config snapshot and runtime hints they need to behave like the host
/// would, without exposing the whole app runtime for ad hoc coupling.
pub struct NativeCommandContext<'a> {
    /// Current resolved config snapshot for this execution.
    pub config: &'a ResolvedConfig,
    /// Runtime hints that should be propagated to child processes and adapters.
    pub runtime_hints: RuntimeHints,
}

impl<'a> NativeCommandContext<'a> {
    /// Creates the runtime context passed to one native-command execution.
    pub fn new(config: &'a ResolvedConfig, runtime_hints: RuntimeHints) -> Self {
        Self {
            config,
            runtime_hints,
        }
    }
}

/// Result of executing a native command.
pub enum NativeCommandOutcome {
    /// Return rendered help text directly.
    Help(String),
    /// Return a protocol response payload.
    Response(Box<ResponseV1>),
    /// Exit immediately with the given status code.
    Exit(i32),
}

/// Trait implemented by in-process commands registered alongside plugins.
pub trait NativeCommand: Send + Sync {
    /// Returns the clap command definition for this command.
    fn command(&self) -> Command;

    /// Returns optional auth/visibility metadata for the command.
    fn auth(&self) -> Option<DescribeCommandAuthV1> {
        None
    }

    /// Builds the plugin-protocol style description for this command.
    fn describe(&self) -> DescribeCommandV1 {
        let mut describe = DescribeCommandV1::from_clap(self.command());
        describe.auth = self.auth();
        describe
    }

    /// Executes the command using already-parsed argument tokens.
    fn execute(
        &self,
        args: &[String],
        context: &NativeCommandContext<'_>,
    ) -> Result<NativeCommandOutcome>;
}

/// Registry of in-process native commands exposed alongside plugin commands.
#[derive(Clone, Default)]
pub struct NativeCommandRegistry {
    commands: Arc<BTreeMap<String, Arc<dyn NativeCommand>>>,
}

impl NativeCommandRegistry {
    /// Creates an empty native command registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a registry with one additional registered command.
    pub fn with_command(mut self, command: impl NativeCommand + 'static) -> Self {
        self.register(command);
        self
    }

    /// Registers or replaces a native command by normalized command name.
    pub fn register(&mut self, command: impl NativeCommand + 'static) {
        let mut next = (*self.commands).clone();
        let command = Arc::new(command) as Arc<dyn NativeCommand>;
        let name = normalize_name(&command.describe().name);
        next.insert(name, command);
        self.commands = Arc::new(next);
    }

    /// Returns `true` when no native commands are registered.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Returns a registered command by normalized name.
    ///
    /// Lookup is case- and surrounding-whitespace-insensitive so callers can
    /// reuse human-typed names without normalizing them first.
    pub fn command(&self, name: &str) -> Option<&Arc<dyn NativeCommand>> {
        self.commands.get(&normalize_name(name))
    }

    /// Returns catalog metadata for all registered native commands.
    pub fn catalog(&self) -> Vec<NativeCommandCatalogEntry> {
        self.commands
            .values()
            .map(|command| {
                let describe = command.describe();
                let completion = crate::plugin::conversion::to_command_spec(&describe);
                NativeCommandCatalogEntry {
                    name: describe.name.clone(),
                    about: describe.about.clone(),
                    auth: describe.auth.clone(),
                    subcommands: crate::plugin::conversion::direct_subcommand_names(&completion),
                    completion,
                }
            })
            .collect()
    }

    /// Builds a command-policy registry derived from command descriptions.
    pub fn command_policy_registry(&self) -> CommandPolicyRegistry {
        let mut registry = CommandPolicyRegistry::new();
        for command in self.commands.values() {
            let describe = command.describe();
            register_describe_command_policies(&mut registry, &describe, &[]);
        }
        registry
    }
}

fn register_describe_command_policies(
    registry: &mut CommandPolicyRegistry,
    command: &DescribeCommandV1,
    parent: &[String],
) {
    let mut segments = parent.to_vec();
    segments.push(command.name.clone());
    if let Some(policy) = command.command_policy(crate::core::command_policy::CommandPath::new(
        segments.clone(),
    )) {
        registry.register(policy);
    }
    for subcommand in &command.subcommands {
        register_describe_command_policies(registry, subcommand, &segments);
    }
}

fn normalize_name(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests;
