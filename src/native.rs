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
//! - downstream product crates typically build a registry once and pass it
//!   into [`crate::App::with_native_commands`] or
//!   [`crate::AppBuilder::with_native_commands`] as part of their own wrapper
//!   or builder layer

use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use anyhow::Result;
use clap::Command;

use crate::completion::CommandSpec;
use crate::config::ResolvedConfig;
use crate::core::command_policy::CommandPolicyRegistry;
use crate::core::plugin::{
    DescribeCommandAuthV1, DescribeCommandV1, ResponseMessageV1, ResponseMetaV1, ResponseV1,
};
use crate::core::runtime::RuntimeHints;
use crate::plugin::catalog::register_describe_command_policies;

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
    /// Session-scoped native context that may be surfaced by the host REPL.
    pub session_context: NativeSessionContext,
    progress: Option<&'a dyn NativeProgressSink>,
}

impl<'a> NativeCommandContext<'a> {
    /// Creates the runtime context passed to one native-command execution.
    pub fn new(config: &'a ResolvedConfig, runtime_hints: RuntimeHints) -> Self {
        Self {
            config,
            runtime_hints,
            session_context: NativeSessionContext::default(),
            progress: None,
        }
    }

    /// Attaches session-scoped native context to this execution.
    pub fn with_session_context(mut self, session_context: NativeSessionContext) -> Self {
        self.session_context = session_context;
        self
    }

    /// Attaches the host-owned structured progress boundary for this run.
    pub fn with_progress_sink(mut self, progress: &'a dyn NativeProgressSink) -> Self {
        self.progress = Some(progress);
        self
    }

    /// Emits one structured transient progress document immediately.
    ///
    /// A context created outside the host has no sink, in which case emission
    /// is a no-op. Native commands should still return the stable final
    /// document through [`NativeCommandOutcome`].
    pub fn emit_progress(&self, event: NativeProgressEvent) -> Result<()> {
        match self.progress {
            Some(progress) => progress.emit(event),
            None => Ok(()),
        }
    }
}

/// One transient structured document emitted while a native command runs.
#[derive(Debug, Clone)]
pub struct NativeProgressEvent {
    /// Canonical progress data to render immediately.
    pub data: serde_json::Value,
    /// Structured messages attached to this progress document.
    pub messages: Vec<ResponseMessageV1>,
    /// Rendering hints interpreted by the same host pipeline as final output.
    pub meta: ResponseMetaV1,
}

impl NativeProgressEvent {
    /// Creates a progress event with default rendering metadata.
    pub fn new(data: impl Into<serde_json::Value>) -> Self {
        Self {
            data: data.into(),
            messages: Vec::new(),
            meta: ResponseMetaV1::default(),
        }
    }

    /// Attaches rendering metadata to the progress document.
    pub fn with_meta(mut self, meta: ResponseMetaV1) -> Self {
        self.meta = meta;
        self
    }

    /// Attaches structured messages to the progress document.
    pub fn with_messages(mut self, messages: Vec<ResponseMessageV1>) -> Self {
        self.messages = messages;
        self
    }
}

/// Host boundary that renders transient native-command progress.
pub trait NativeProgressSink {
    /// Renders or records one progress document before command execution resumes.
    fn emit(&self, event: NativeProgressEvent) -> Result<()>;
}

/// One prompt-visible session context value owned by a native command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativePromptContextEntry {
    /// Short label rendered in compact prompt surfaces.
    pub label: String,
    /// Current value for the context entry.
    pub value: String,
}

/// Session-scoped context shared between native commands and the REPL host.
///
/// This is deliberately in-memory only. Native commands can retain small
/// workflow values between commands and separately expose prompt-safe values
/// without writing hidden target state to disk.
#[derive(Clone, Default)]
pub struct NativeSessionContext {
    values: Arc<RwLock<BTreeMap<String, String>>>,
    prompt_entries: Arc<RwLock<BTreeMap<String, NativePromptContextEntry>>>,
    completion_refresh_requested: Arc<AtomicBool>,
}

impl fmt::Debug for NativeSessionContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeSessionContext")
            .field("values", &"[redacted]")
            .field("prompt_entries", &self.prompt_entries)
            .field(
                "completion_refresh_requested",
                &self.completion_refresh_requested,
            )
            .finish()
    }
}

impl NativeSessionContext {
    /// Stores one in-memory value for later native commands in this session.
    pub fn set_value(&self, key: impl Into<String>, value: impl Into<String>) {
        if let Ok(mut values) = self.values.write() {
            values.insert(key.into(), value.into());
        }
    }

    /// Returns one previously stored in-memory session value.
    pub fn value(&self, key: &str) -> Option<String> {
        self.values
            .read()
            .ok()
            .and_then(|values| values.get(key).cloned())
    }

    /// Sets one prompt-visible context entry.
    pub fn set_prompt_value(
        &self,
        key: impl Into<String>,
        label: impl Into<String>,
        value: impl Into<String>,
    ) {
        if let Ok(mut entries) = self.prompt_entries.write() {
            entries.insert(
                key.into(),
                NativePromptContextEntry {
                    label: label.into(),
                    value: value.into(),
                },
            );
        }
    }

    /// Removes one prompt-visible context entry.
    pub fn remove_prompt_value(&self, key: &str) {
        if let Ok(mut entries) = self.prompt_entries.write() {
            entries.remove(key);
        }
    }

    /// Returns all prompt-visible context entries in stable key order.
    pub fn prompt_entries(&self) -> Vec<NativePromptContextEntry> {
        self.prompt_entries
            .read()
            .map(|entries| entries.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Requests rebuilding the active REPL completion tree after execution.
    ///
    /// Native commands should call this after replacing completion data held in
    /// their own shared snapshot. Outside an interactive REPL the request has
    /// no observable effect.
    pub fn request_completion_refresh(&self) {
        self.completion_refresh_requested
            .store(true, Ordering::Release);
    }

    pub(crate) fn take_completion_refresh_request(&self) -> bool {
        self.completion_refresh_requested
            .swap(false, Ordering::AcqRel)
    }
}

/// Result of executing a native command.
pub enum NativeCommandOutcome {
    /// Return rendered help text directly.
    Help(String),
    /// Return a protocol response payload.
    Response(Box<ResponseV1>),
    /// Return structured output and then terminate with an explicit status.
    ResponseWithExit {
        /// Final response that still flows through JSON/DSL rendering.
        response: Box<ResponseV1>,
        /// Process status returned after rendering the response.
        exit_code: i32,
    },
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

    /// Adds runtime-owned suggestions to the otherwise static completion tree.
    ///
    /// The host calls this only while preparing an interactive completion
    /// surface. Implementations should cache any remote data they use so REPL
    /// editing never performs network I/O per keystroke.
    fn augment_completion(&self, _completion: &mut CommandSpec) {}

    /// Refreshes runtime completion data on a host-owned background thread.
    ///
    /// The host invokes this once when an authorized interactive REPL starts.
    /// Implementations may perform remote I/O here, then atomically replace
    /// the snapshot read by [`NativeCommand::augment_completion`].
    fn refresh_completion(&self) -> Result<()> {
        Ok(())
    }

    /// Executes the command using already-parsed argument tokens.
    ///
    /// `args` contains the tokens after the registered command name. For a
    /// command registered as `history`, the command line `osp history clear
    /// --all` reaches `execute` as `["clear", "--all"]`.
    ///
    /// The host interprets outcomes as follows:
    ///
    /// - [`NativeCommandOutcome::Help`] is rendered as a help/guide response
    /// - [`NativeCommandOutcome::Exit`] terminates the command immediately with
    ///   that exit code
    /// - [`NativeCommandOutcome::Response`] is treated like plugin protocol
    ///   output and may still flow through trailing DSL stages
    /// - [`NativeCommandOutcome::ResponseWithExit`] follows the same output
    ///   path, then returns the supplied process status
    ///
    /// Long-running commands may call [`NativeCommandContext::emit_progress`].
    /// The host renders each transient document immediately on stderr so the
    /// final stdout document remains safe for JSON and DSL consumers.
    ///
    /// Return `Err` when command execution itself fails. The host formats that
    /// failure like other command errors.
    ///
    /// # Examples
    ///
    /// ```
    /// use anyhow::Result;
    /// use clap::Command;
    /// use osp_cli::{NativeCommand, NativeCommandContext, NativeCommandOutcome};
    ///
    /// struct HistoryCommand;
    ///
    /// impl NativeCommand for HistoryCommand {
    ///     fn command(&self) -> Command {
    ///         Command::new("history").about("Manage local history")
    ///     }
    ///
    ///     fn execute(
    ///         &self,
    ///         args: &[String],
    ///         _context: &NativeCommandContext<'_>,
    ///     ) -> Result<NativeCommandOutcome> {
    ///         match args {
    ///             [subcommand, flag] if subcommand == "clear" && flag == "--all" => {
    ///                 Ok(NativeCommandOutcome::Exit(0))
    ///             }
    ///             _ => Ok(NativeCommandOutcome::Help(
    ///                 "usage: history clear --all".to_string(),
    ///             )),
    ///         }
    ///     }
    /// }
    /// ```
    fn execute(
        &self,
        args: &[String],
        context: &NativeCommandContext<'_>,
    ) -> Result<NativeCommandOutcome>;
}

/// Registry of in-process native commands exposed alongside plugin commands.
#[derive(Clone, Default)]
#[must_use]
pub struct NativeCommandRegistry {
    commands: Arc<BTreeMap<String, Arc<dyn NativeCommand>>>,
}

impl NativeCommandRegistry {
    /// Creates an empty native command registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a registry with one additional registered command.
    ///
    /// # Examples
    ///
    /// ```
    /// use anyhow::Result;
    /// use clap::Command;
    /// use osp_cli::{
    ///     NativeCommand, NativeCommandContext, NativeCommandOutcome, NativeCommandRegistry,
    /// };
    ///
    /// struct VersionCommand;
    ///
    /// impl NativeCommand for VersionCommand {
    ///     fn command(&self) -> Command {
    ///         Command::new("version").about("Show version")
    ///     }
    ///
    ///     fn execute(
    ///         &self,
    ///         _args: &[String],
    ///         _context: &NativeCommandContext<'_>,
    ///     ) -> Result<NativeCommandOutcome> {
    ///         Ok(NativeCommandOutcome::Exit(0))
    ///     }
    /// }
    ///
    /// let registry = NativeCommandRegistry::new().with_command(VersionCommand);
    /// let catalog = registry.catalog();
    ///
    /// assert!(registry.command(" VERSION ").is_some());
    /// assert_eq!(catalog[0].name, "version");
    /// assert_eq!(catalog[0].about, "Show version");
    /// assert!(catalog[0].auth.is_none());
    /// ```
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
        self.catalog_with_completion(false)
    }

    /// Returns catalog metadata with native runtime completion additions.
    pub fn completion_catalog(&self) -> Vec<NativeCommandCatalogEntry> {
        self.catalog_with_completion(true)
    }

    pub(crate) fn refresh_completion_in_background(&self, should_refresh: impl Fn(&str) -> bool) {
        for (name, command) in self.commands.iter() {
            if !should_refresh(name) {
                continue;
            }
            let name = name.clone();
            let command = Arc::clone(command);
            if let Err(error) = std::thread::Builder::new()
                .name(format!("osp-completion-{name}"))
                .spawn(move || {
                    if let Err(error) = command.refresh_completion() {
                        tracing::debug!(command = %name, %error, "native completion refresh failed");
                    }
                })
            {
                tracing::warn!(%error, "failed to start native completion refresh");
            }
        }
    }

    fn catalog_with_completion(&self, augment_completion: bool) -> Vec<NativeCommandCatalogEntry> {
        self.commands
            .values()
            .map(|command| {
                let describe = command.describe();
                let mut completion = crate::plugin::conversion::to_command_spec(&describe);
                if augment_completion {
                    command.augment_completion(&mut completion);
                }
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

fn normalize_name(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests;
