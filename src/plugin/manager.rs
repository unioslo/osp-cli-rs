//! Public plugin facade and shared plugin data types.
//!
//! This module exists so the rest of the app can depend on one stable plugin
//! entry point while discovery, selection, catalog building, and dispatch live
//! in narrower neighboring modules.
//!
//! High-level flow:
//!
//! - store discovered plugin metadata and process/runtime settings
//! - delegate catalog and selection work to neighboring modules
//! - hand the chosen provider to the dispatch layer when execution is needed
//!
//! Contract:
//!
//! - this file owns the public facade and shared plugin DTOs
//! - catalog building and provider selection logic live in neighboring
//!   modules
//! - subprocess execution and timeout handling belong in `plugin::dispatch`
//!
//! Public API shape:
//!
//! - discovered plugins and catalog entries are semantic payloads
//! - dispatch machinery uses concrete constructors such as
//!   [`PluginDispatchContext::new`] plus `with_*` refinements instead of raw
//!   ad hoc assembly

use super::active::ActivePluginView;
use super::catalog::{
    build_command_catalog, build_command_policy_registry, build_doctor_report,
    command_provider_labels, completion_words_from_catalog, list_plugins, render_repl_help,
    selected_provider_label,
};
use super::conversion::to_command_spec;
use super::selection::{ProviderResolution, ProviderResolutionError, provider_labels};
use super::state::PluginCommandPreferences;
#[cfg(test)]
use super::state::PluginCommandState;
use crate::completion::CommandSpec;
use crate::core::plugin::{DescribeCommandAuthV1, DescribeCommandV1};
use crate::core::runtime::RuntimeHints;
use anyhow::{Result, anyhow};
use std::collections::{BTreeSet, HashMap};
use std::error::Error as StdError;
use std::fmt::{Display, Formatter};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// Default timeout, in milliseconds, for plugin subprocess calls.
pub const DEFAULT_PLUGIN_PROCESS_TIMEOUT_MS: usize = 10_000;

/// Describes how a plugin executable was discovered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginSource {
    /// Loaded from an explicit search directory supplied by the caller.
    Explicit,
    /// Loaded from a path listed in the `OSP_PLUGIN_PATH` environment variable.
    Env,
    /// Loaded from the CLI's bundled plugin set.
    Bundled,
    /// Loaded from the per-user plugin directory under the configured config root.
    UserConfig,
    /// Loaded by scanning the process `PATH`.
    Path,
}

impl Display for PluginSource {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl PluginSource {
    /// Returns the stable label used in diagnostics and persisted metadata.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::plugin::PluginSource;
    ///
    /// assert_eq!(PluginSource::Bundled.to_string(), "bundled");
    /// ```
    pub fn as_str(self) -> &'static str {
        match self {
            PluginSource::Explicit => "explicit",
            PluginSource::Env => "env",
            PluginSource::Bundled => "bundled",
            PluginSource::UserConfig => "user",
            PluginSource::Path => "path",
        }
    }
}

/// Canonical in-memory record for one discovered plugin provider.
///
/// This is the rich internal form used for catalog building, completion, and
/// dispatch decisions after discovery has finished.
#[derive(Debug, Clone)]
pub struct DiscoveredPlugin {
    /// Stable provider identifier returned by the plugin.
    pub plugin_id: String,
    /// Optional plugin version reported during discovery.
    pub plugin_version: Option<String>,
    /// Absolute path to the plugin executable.
    pub executable: PathBuf,
    /// Discovery source used to locate the executable.
    pub source: PluginSource,
    /// Seeded top-level command names from manifest or describe metadata.
    ///
    /// Internal selection and catalog code should prefer the canonical command
    /// helpers on this type so `commands`, `command_specs`, and
    /// `describe_commands` cannot drift independently.
    pub commands: Vec<String>,
    /// Raw describe-command payloads returned by the plugin.
    pub describe_commands: Vec<DescribeCommandV1>,
    /// Normalized completion specs derived from describe metadata or manifest
    /// seed data.
    pub command_specs: Vec<CommandSpec>,
    /// Discovery or validation issue associated with this plugin.
    pub issue: Option<String>,
    /// Whether commands from this plugin default to enabled when no explicit
    /// command-state preference overrides them.
    pub default_enabled: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CanonicalPluginCommand<'a> {
    name: &'a str,
    spec: Option<&'a CommandSpec>,
    describe: Option<&'a DescribeCommandV1>,
}

impl<'a> CanonicalPluginCommand<'a> {
    pub(crate) fn name(self) -> &'a str {
        self.name
    }

    pub(crate) fn completion(self) -> CommandSpec {
        self.spec
            .cloned()
            .or_else(|| self.describe.map(to_command_spec))
            .unwrap_or_else(|| CommandSpec::new(self.name))
    }

    pub(crate) fn auth(self) -> Option<DescribeCommandAuthV1> {
        self.describe.and_then(|command| command.auth.clone())
    }

    pub(crate) fn describe(self) -> Option<&'a DescribeCommandV1> {
        self.describe
    }
}

impl DiscoveredPlugin {
    pub(crate) fn canonical_command_names(&self) -> Vec<&str> {
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();

        for spec in &self.command_specs {
            if seen.insert(spec.name.as_str()) {
                out.push(spec.name.as_str());
            }
        }
        for command in &self.describe_commands {
            if seen.insert(command.name.as_str()) {
                out.push(command.name.as_str());
            }
        }
        for command in &self.commands {
            if seen.insert(command.as_str()) {
                out.push(command.as_str());
            }
        }

        out
    }

    pub(crate) fn canonical_command(
        &self,
        command_name: &str,
    ) -> Option<CanonicalPluginCommand<'_>> {
        let spec = self
            .command_specs
            .iter()
            .find(|spec| spec.name == command_name);
        let describe = self
            .describe_commands
            .iter()
            .find(|command| command.name == command_name);
        let raw_name = self
            .commands
            .iter()
            .find(|command| command.as_str() == command_name);
        let name = spec
            .map(|spec| spec.name.as_str())
            .or_else(|| describe.map(|command| command.name.as_str()))
            .or_else(|| raw_name.map(String::as_str))?;

        Some(CanonicalPluginCommand {
            name,
            spec,
            describe,
        })
    }
}

/// Reduced plugin view for listing, doctor, and status surfaces.
///
/// `enabled` reflects command-state filtering, while `healthy` reflects
/// discovery-time validation and describe-cache status.
#[derive(Debug, Clone)]
pub struct PluginSummary {
    /// Stable provider identifier returned by the plugin.
    pub plugin_id: String,
    /// Optional plugin version reported during discovery.
    pub plugin_version: Option<String>,
    /// Absolute path to the plugin executable.
    pub executable: PathBuf,
    /// Discovery source used to locate the executable.
    pub source: PluginSource,
    /// Top-level commands exported by the plugin.
    pub commands: Vec<String>,
    /// Whether at least one exported command remains enabled after
    /// command-state filtering.
    pub enabled: bool,
    /// Whether the plugin passed discovery-time validation.
    pub healthy: bool,
    /// Discovery or validation issue associated with this plugin.
    pub issue: Option<String>,
}

/// One command-name conflict across multiple plugin providers.
#[derive(Debug, Clone)]
pub struct CommandConflict {
    /// Conflicting command name.
    pub command: String,
    /// Provider labels that provide `command`, such as `alpha (explicit)`.
    pub providers: Vec<String>,
}

/// Aggregated plugin health payload used by diagnostic surfaces.
#[derive(Debug, Clone)]
pub struct DoctorReport {
    /// Summary entry for each discovered plugin.
    pub plugins: Vec<PluginSummary>,
    /// Commands that are provided by more than one provider label.
    pub conflicts: Vec<CommandConflict>,
}

/// Normalized command-level catalog entry derived from the discovered plugin set.
///
/// Help, completion, and dispatch-selection code can share this view without
/// understanding plugin discovery internals.
#[derive(Debug, Clone)]
pub struct CommandCatalogEntry {
    /// Full command path, including parent commands when present.
    pub name: String,
    /// Short description shown in help and catalog output.
    pub about: String,
    /// Optional auth metadata returned by plugin discovery.
    pub auth: Option<DescribeCommandAuthV1>,
    /// Immediate subcommand names beneath `name`.
    pub subcommands: Vec<String>,
    /// Shell completion metadata for this command.
    pub completion: CommandSpec,
    /// Selected provider identifier when dispatch has been resolved.
    pub provider: Option<String>,
    /// Provider labels for every provider that exports this command.
    pub providers: Vec<String>,
    /// Whether more than one provider exports this command.
    pub conflicted: bool,
    /// Whether the caller must choose a provider before dispatch.
    pub requires_selection: bool,
    /// Whether the provider was selected by explicit preference rather than by
    /// unique-provider resolution.
    pub selected_explicitly: bool,
    /// Discovery source for the selected provider, if resolved.
    pub source: Option<PluginSource>,
}

impl CommandCatalogEntry {
    /// Returns the optional auth hint rendered in help and catalog views.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::completion::CommandSpec;
    /// use osp_cli::plugin::CommandCatalogEntry;
    /// use osp_cli::core::plugin::{DescribeCommandAuthV1, DescribeVisibilityModeV1};
    ///
    /// let entry = CommandCatalogEntry {
    ///     name: "ldap user".to_string(),
    ///     about: "lookup users".to_string(),
    ///     auth: Some(DescribeCommandAuthV1 {
    ///         visibility: Some(DescribeVisibilityModeV1::Authenticated),
    ///         required_capabilities: Vec::new(),
    ///         feature_flags: Vec::new(),
    ///     }),
    ///     subcommands: Vec::new(),
    ///     completion: CommandSpec::new("ldap"),
    ///     provider: Some("ldap".to_string()),
    ///     providers: vec!["ldap".to_string()],
    ///     conflicted: false,
    ///     requires_selection: false,
    ///     selected_explicitly: false,
    ///     source: None,
    /// };
    ///
    /// assert_eq!(entry.auth_hint().as_deref(), Some("auth"));
    /// ```
    pub fn auth_hint(&self) -> Option<String> {
        self.auth.as_ref().and_then(|auth| auth.hint())
    }
}

/// Raw stdout/stderr captured from a plugin subprocess invocation.
///
/// This is the payload returned by passthrough dispatch APIs. A non-zero plugin
/// exit code is preserved in `status_code` instead of being converted into a
/// semantic response or validation error.
#[derive(Debug, Clone)]
pub struct RawPluginOutput {
    /// Process exit status code, or `1` when the child ended without a
    /// conventional exit code.
    pub status_code: i32,
    /// Captured standard output.
    pub stdout: String,
    /// Captured standard error.
    pub stderr: String,
}

/// Per-dispatch runtime hints and environment overrides for plugin execution.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
#[must_use]
pub struct PluginDispatchContext {
    /// Runtime hints serialized into plugin requests.
    pub runtime_hints: RuntimeHints,
    /// Environment pairs injected into every plugin process.
    pub shared_env: Vec<(String, String)>,
    /// Additional environment pairs injected for specific plugins.
    pub plugin_env: HashMap<String, Vec<(String, String)>>,
    /// Provider identifier forced by the caller, if any.
    pub provider_override: Option<String>,
}

impl PluginDispatchContext {
    /// Creates dispatch context from the required runtime hint payload.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::output::{ColorMode, OutputFormat, UnicodeMode};
    /// use osp_cli::core::runtime::{RuntimeHints, RuntimeTerminalKind, UiVerbosity};
    /// use osp_cli::plugin::PluginDispatchContext;
    ///
    /// let context = PluginDispatchContext::new(RuntimeHints::new(
    ///     UiVerbosity::Info,
    ///     2,
    ///     OutputFormat::Json,
    ///     ColorMode::Always,
    ///     UnicodeMode::Never,
    /// ))
    /// .with_provider_override(Some("ldap".to_string()))
    /// .with_shared_env([("OSP_FORMAT", "json")]);
    ///
    /// assert_eq!(context.provider_override.as_deref(), Some("ldap"));
    /// assert!(context.shared_env.iter().any(|(key, value)| key == "OSP_FORMAT" && value == "json"));
    /// assert_eq!(context.runtime_hints.terminal_kind, RuntimeTerminalKind::Unknown);
    /// ```
    pub fn new(runtime_hints: RuntimeHints) -> Self {
        Self {
            runtime_hints,
            shared_env: Vec::new(),
            plugin_env: HashMap::new(),
            provider_override: None,
        }
    }

    /// Replaces the environment injected into every plugin process.
    ///
    /// Defaults to no shared environment overrides when omitted.
    pub fn with_shared_env<I, K, V>(mut self, shared_env: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        self.shared_env = shared_env
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect();
        self
    }

    /// Replaces the environment injected for specific plugins.
    ///
    /// Defaults to no plugin-specific environment overrides when omitted.
    /// Matching entries are appended after `shared_env` for the selected
    /// plugin.
    pub fn with_plugin_env(mut self, plugin_env: HashMap<String, Vec<(String, String)>>) -> Self {
        self.plugin_env = plugin_env;
        self
    }

    /// Replaces the optional forced provider identifier.
    ///
    /// Defaults to the manager's normal provider-resolution rules when omitted.
    /// Use this for one-shot dispatch overrides without mutating manager-local
    /// provider selections.
    pub fn with_provider_override(mut self, provider_override: Option<String>) -> Self {
        self.provider_override = provider_override;
        self
    }

    pub(crate) fn env_pairs_for<'a>(
        &'a self,
        plugin_id: &'a str,
    ) -> impl Iterator<Item = (&'a str, &'a str)> {
        self.shared_env
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .chain(
                self.plugin_env
                    .get(plugin_id)
                    .into_iter()
                    .flat_map(|entries| entries.iter())
                    .map(|(key, value)| (key.as_str(), value.as_str())),
            )
    }
}

/// Errors returned when selecting or invoking a plugin command.
///
/// Variants that list `providers` use provider labels as rendered in help and
/// diagnostics, not bare plugin ids.
#[derive(Debug)]
pub enum PluginDispatchError {
    /// No plugin provides the requested command.
    CommandNotFound {
        /// Command name requested by the caller.
        command: String,
    },
    /// More than one plugin provides the requested command.
    CommandAmbiguous {
        /// Command name requested by the caller.
        command: String,
        /// Provider labels that provide `command`.
        providers: Vec<String>,
    },
    /// The requested provider exists, but not for the requested command.
    ProviderNotFound {
        /// Command name requested by the caller.
        command: String,
        /// Provider identifier requested by the caller.
        requested_provider: String,
        /// Provider labels that provide `command`.
        providers: Vec<String>,
    },
    /// Spawning or waiting for the plugin process failed.
    ExecuteFailed {
        /// Plugin identifier being invoked.
        plugin_id: String,
        /// Underlying process execution error.
        source: std::io::Error,
    },
    /// The plugin process exceeded the configured timeout.
    TimedOut {
        /// Plugin identifier being invoked.
        plugin_id: String,
        /// Timeout applied to the subprocess call.
        timeout: Duration,
        /// Captured standard error emitted before timeout.
        stderr: String,
    },
    /// The plugin process exited with a non-zero status code.
    NonZeroExit {
        /// Plugin identifier being invoked.
        plugin_id: String,
        /// Process exit status code.
        status_code: i32,
        /// Captured standard error emitted by the plugin.
        stderr: String,
    },
    /// The plugin returned malformed JSON.
    InvalidJsonResponse {
        /// Plugin identifier being invoked.
        plugin_id: String,
        /// JSON decode error for the response payload.
        source: serde_json::Error,
    },
    /// The plugin returned JSON that failed semantic validation.
    InvalidResponsePayload {
        /// Plugin identifier being invoked.
        plugin_id: String,
        /// Validation failure description.
        reason: String,
    },
}

impl Display for PluginDispatchError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            PluginDispatchError::CommandNotFound { command } => {
                write!(f, "no plugin provides command: {command}")
            }
            PluginDispatchError::CommandAmbiguous { command, providers } => {
                write!(
                    f,
                    "command `{command}` is provided by multiple plugins: {}",
                    providers.join(", ")
                )
            }
            PluginDispatchError::ProviderNotFound {
                command,
                requested_provider,
                providers,
            } => {
                write!(
                    f,
                    "plugin `{requested_provider}` does not provide command `{command}`; available providers: {}",
                    providers.join(", ")
                )
            }
            PluginDispatchError::ExecuteFailed { plugin_id, source } => {
                write!(f, "failed to execute plugin {plugin_id}: {source}")
            }
            PluginDispatchError::TimedOut {
                plugin_id,
                timeout,
                stderr,
            } => {
                if stderr.trim().is_empty() {
                    write!(
                        f,
                        "plugin {plugin_id} timed out after {} ms",
                        timeout.as_millis()
                    )
                } else {
                    write!(
                        f,
                        "plugin {plugin_id} timed out after {} ms: {}",
                        timeout.as_millis(),
                        stderr.trim()
                    )
                }
            }
            PluginDispatchError::NonZeroExit {
                plugin_id,
                status_code,
                stderr,
            } => {
                if stderr.trim().is_empty() {
                    write!(f, "plugin {plugin_id} exited with status {status_code}")
                } else {
                    write!(
                        f,
                        "plugin {plugin_id} exited with status {status_code}: {}",
                        stderr.trim()
                    )
                }
            }
            PluginDispatchError::InvalidJsonResponse { plugin_id, source } => {
                write!(f, "invalid JSON response from plugin {plugin_id}: {source}")
            }
            PluginDispatchError::InvalidResponsePayload { plugin_id, reason } => {
                write!(f, "invalid plugin response from {plugin_id}: {reason}")
            }
        }
    }
}

impl StdError for PluginDispatchError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            PluginDispatchError::ExecuteFailed { source, .. } => Some(source),
            PluginDispatchError::InvalidJsonResponse { source, .. } => Some(source),
            PluginDispatchError::CommandNotFound { .. }
            | PluginDispatchError::CommandAmbiguous { .. }
            | PluginDispatchError::ProviderNotFound { .. }
            | PluginDispatchError::TimedOut { .. }
            | PluginDispatchError::NonZeroExit { .. }
            | PluginDispatchError::InvalidResponsePayload { .. } => None,
        }
    }
}

/// Coordinates plugin discovery, cached metadata, and dispatch settings.
///
/// This is the main host-side facade for plugin integration. A typical caller
/// constructs one manager, points it at explicit roots plus optional config and
/// cache roots, then asks it for one of three things:
///
/// - plugin inventory via [`PluginManager::list_plugins`]
/// - merged command metadata via [`PluginManager::command_catalog`] or
///   [`PluginManager::command_policy_registry`]
/// - dispatch-time configuration such as manager-local provider selections
///
/// If you are implementing the plugin executable itself rather than the host,
/// start in [`crate::core::plugin`] instead of here.
#[must_use]
pub struct PluginManager {
    pub(crate) explicit_dirs: Vec<PathBuf>,
    pub(crate) discovered_cache: RwLock<Option<Arc<[DiscoveredPlugin]>>>,
    pub(crate) dispatch_discovered_cache: RwLock<Option<Arc<[DiscoveredPlugin]>>>,
    pub(crate) command_preferences: RwLock<PluginCommandPreferences>,
    pub(crate) config_root: Option<PathBuf>,
    pub(crate) cache_root: Option<PathBuf>,
    pub(crate) process_timeout: Duration,
    pub(crate) allow_path_discovery: bool,
    pub(crate) allow_default_roots: bool,
}

struct KnownCommandProviders<'a> {
    command: &'a str,
    providers: Vec<&'a DiscoveredPlugin>,
}

impl<'a> KnownCommandProviders<'a> {
    fn collect(view: &ActivePluginView<'a>, command: &'a str) -> Self {
        Self {
            command,
            providers: view.healthy_providers(command),
        }
    }

    fn validate_command(&self) -> Result<()> {
        if self.providers.is_empty() {
            return Err(anyhow!(
                "no healthy plugin provides command `{}`",
                self.command
            ));
        }
        Ok(())
    }
}

struct AvailableCommandProviders<'a> {
    command: &'a str,
    available: Vec<&'a DiscoveredPlugin>,
}

impl<'a> AvailableCommandProviders<'a> {
    fn collect(view: &ActivePluginView<'a>, command: &'a str) -> Self {
        Self {
            command,
            available: view.available_providers(command),
        }
    }

    fn len(&self) -> usize {
        self.available.len()
    }

    fn validate_command(&self) -> Result<()> {
        if self.available.is_empty() {
            return Err(anyhow!(
                "no available plugin provides command `{}`",
                self.command
            ));
        }
        Ok(())
    }

    fn validate_provider(&self, plugin_id: &str) -> Result<()> {
        self.validate_command()?;
        if self
            .available
            .iter()
            .any(|plugin| plugin.plugin_id == plugin_id)
        {
            return Ok(());
        }

        Err(anyhow!(
            "plugin `{plugin_id}` is not currently available for command `{}`; available providers: {}",
            self.command,
            self.labels().join(", ")
        ))
    }

    fn labels(&self) -> Vec<String> {
        provider_labels(&self.available)
    }
}

impl PluginManager {
    /// Creates a plugin manager with the provided explicit search roots.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::plugin::PluginManager;
    /// use std::path::PathBuf;
    ///
    /// let manager = PluginManager::new(vec![PathBuf::from("/plugins")]);
    ///
    /// assert_eq!(manager.explicit_dirs().len(), 1);
    /// ```
    pub fn new(explicit_dirs: Vec<PathBuf>) -> Self {
        Self {
            explicit_dirs,
            discovered_cache: RwLock::new(None),
            dispatch_discovered_cache: RwLock::new(None),
            command_preferences: RwLock::new(PluginCommandPreferences::default()),
            config_root: None,
            cache_root: None,
            process_timeout: Duration::from_millis(DEFAULT_PLUGIN_PROCESS_TIMEOUT_MS as u64),
            allow_path_discovery: false,
            allow_default_roots: true,
        }
    }

    /// Returns the explicit plugin search roots configured for this manager.
    pub fn explicit_dirs(&self) -> &[PathBuf] {
        &self.explicit_dirs
    }

    /// Sets config and cache roots used for user plugin discovery and describe
    /// cache files.
    ///
    /// The config root feeds the per-user plugin directory lookup. The cache
    /// root feeds the on-disk describe cache. This does not make command
    /// provider selections persistent by itself; those remain manager-local
    /// in-memory state.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::plugin::PluginManager;
    /// use std::path::PathBuf;
    ///
    /// let manager = PluginManager::new(Vec::new()).with_roots(
    ///     Some(PathBuf::from("/config")),
    ///     Some(PathBuf::from("/cache")),
    /// );
    ///
    /// assert_eq!(manager.config_root(), Some(PathBuf::from("/config").as_path()));
    /// assert_eq!(manager.cache_root(), Some(PathBuf::from("/cache").as_path()));
    /// ```
    pub fn with_roots(mut self, config_root: Option<PathBuf>, cache_root: Option<PathBuf>) -> Self {
        self.config_root = config_root;
        self.cache_root = cache_root;
        self
    }

    /// Returns the configured config root used to resolve the user plugin
    /// directory.
    pub fn config_root(&self) -> Option<&std::path::Path> {
        self.config_root.as_deref()
    }

    /// Returns the configured cache root used for the describe metadata cache.
    pub fn cache_root(&self) -> Option<&std::path::Path> {
        self.cache_root.as_deref()
    }

    /// Enables or disables fallback to platform config/cache roots when
    /// explicit roots are not configured.
    ///
    /// The default is `true`. Disable this when the caller wants plugin
    /// discovery and describe-cache state to stay fully in-memory unless
    /// explicit roots are provided.
    pub fn with_default_roots(mut self, allow_default_roots: bool) -> Self {
        self.allow_default_roots = allow_default_roots;
        self
    }

    /// Returns whether platform config/cache root fallback is enabled.
    pub fn default_roots_enabled(&self) -> bool {
        self.allow_default_roots
    }

    /// Sets the subprocess timeout used for plugin describe and dispatch calls.
    ///
    /// Timeout values are clamped to at least one millisecond so the manager
    /// never stores a zero-duration subprocess timeout.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::plugin::PluginManager;
    /// use std::time::Duration;
    ///
    /// let manager = PluginManager::new(Vec::new())
    ///     .with_process_timeout(Duration::from_millis(0));
    ///
    /// assert_eq!(manager.process_timeout(), Duration::from_millis(1));
    /// ```
    pub fn with_process_timeout(mut self, timeout: Duration) -> Self {
        self.process_timeout = timeout.max(Duration::from_millis(1));
        self
    }

    /// Returns the subprocess timeout used for describe and dispatch calls.
    pub fn process_timeout(&self) -> Duration {
        self.process_timeout
    }

    /// Enables or disables fallback discovery through the process `PATH`.
    ///
    /// PATH discovery is passive on browse/read surfaces. A PATH-discovered
    /// plugin will not be executed for `--describe` during passive listing or
    /// catalog building, so command metadata is unavailable there until the
    /// first command dispatch to that plugin. Dispatching a command triggers
    /// `--describe` as a cache miss and writes the result to the on-disk
    /// describe cache; subsequent browse and catalog calls will then see the
    /// full command metadata.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::plugin::PluginManager;
    ///
    /// let manager = PluginManager::new(Vec::new()).with_path_discovery(true);
    ///
    /// assert!(manager.path_discovery_enabled());
    /// ```
    pub fn with_path_discovery(mut self, allow_path_discovery: bool) -> Self {
        self.allow_path_discovery = allow_path_discovery;
        self
    }

    /// Returns whether fallback discovery through the process `PATH` is enabled.
    pub fn path_discovery_enabled(&self) -> bool {
        self.allow_path_discovery
    }

    pub(crate) fn with_command_preferences(
        mut self,
        preferences: PluginCommandPreferences,
    ) -> Self {
        self.command_preferences = RwLock::new(preferences);
        self
    }

    /// Lists discovered plugins with health, command, and enablement status.
    ///
    /// When PATH discovery is enabled, PATH-discovered plugins can appear here
    /// before their command metadata is available because passive discovery
    /// does not execute them for `--describe`.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::plugin::PluginManager;
    ///
    /// let plugins = PluginManager::new(Vec::new()).list_plugins();
    ///
    /// assert!(plugins.is_empty());
    /// ```
    pub fn list_plugins(&self) -> Vec<PluginSummary> {
        self.with_passive_view(list_plugins)
    }

    /// Builds the effective command catalog after provider resolution and
    /// health filtering.
    ///
    /// This is the host-facing "what commands exist?" view used by help,
    /// completion, and similar browse/read surfaces. PATH-discovered plugins
    /// only contribute commands here after describe metadata has been cached;
    /// passive discovery alone is not enough.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::plugin::PluginManager;
    ///
    /// let catalog = PluginManager::new(Vec::new()).command_catalog();
    ///
    /// assert!(catalog.is_empty());
    /// ```
    pub fn command_catalog(&self) -> Vec<CommandCatalogEntry> {
        self.with_passive_catalog(|catalog| catalog)
    }

    /// Builds a command policy registry from active plugin describe metadata.
    ///
    /// Use this when plugin auth hints need to participate in the same runtime
    /// visibility and access evaluation as native commands. Commands that
    /// still require provider selection are omitted until one provider is
    /// selected explicitly.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::plugin::PluginManager;
    ///
    /// let registry = PluginManager::new(Vec::new()).command_policy_registry();
    ///
    /// assert!(registry.is_empty());
    /// ```
    pub fn command_policy_registry(&self) -> crate::core::command_policy::CommandPolicyRegistry {
        self.with_passive_view(build_command_policy_registry)
    }

    /// Returns completion words derived from the current plugin command catalog.
    ///
    /// The returned list always includes the REPL backbone words used by the
    /// plugin/completion surface, even when no plugins are currently available.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::plugin::PluginManager;
    ///
    /// let words = PluginManager::new(Vec::new()).completion_words();
    ///
    /// assert!(words.contains(&"help".to_string()));
    /// assert!(words.contains(&"|".to_string()));
    /// ```
    pub fn completion_words(&self) -> Vec<String> {
        self.with_passive_catalog(|catalog| completion_words_from_catalog(&catalog))
    }

    /// Renders a plain-text help view for plugin commands in the REPL.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::plugin::PluginManager;
    ///
    /// let help = PluginManager::new(Vec::new()).repl_help_text();
    ///
    /// assert!(help.contains("Backbone commands: help, exit, quit"));
    /// assert!(help.contains("No plugin commands available."));
    /// ```
    pub fn repl_help_text(&self) -> String {
        self.with_passive_catalog(|catalog| render_repl_help(&catalog))
    }

    /// Returns the available provider labels for a command after health and
    /// enablement filtering.
    ///
    /// Unknown commands and commands with no currently available providers
    /// return an empty list.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::plugin::PluginManager;
    ///
    /// let providers = PluginManager::new(Vec::new()).command_providers("shared");
    ///
    /// assert!(providers.is_empty());
    /// ```
    pub fn command_providers(&self, command: &str) -> Vec<String> {
        self.with_passive_view(|view| command_provider_labels(command, view))
    }

    /// Returns the selected provider label when command resolution is
    /// unambiguous.
    ///
    /// Returns `None` when the command is unknown, ambiguous, or currently
    /// unavailable after health and enablement filtering.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::plugin::PluginManager;
    ///
    /// let provider = PluginManager::new(Vec::new()).selected_provider_label("shared");
    ///
    /// assert_eq!(provider, None);
    /// ```
    pub fn selected_provider_label(&self, command: &str) -> Option<String> {
        self.with_passive_view(|view| selected_provider_label(command, view))
    }

    /// Produces a doctor report with plugin health summaries and command conflicts.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::plugin::PluginManager;
    ///
    /// let report = PluginManager::new(Vec::new()).doctor();
    ///
    /// assert!(report.plugins.is_empty());
    /// assert!(report.conflicts.is_empty());
    /// ```
    pub fn doctor(&self) -> DoctorReport {
        self.with_passive_view(build_doctor_report)
    }

    pub(crate) fn validate_command(&self, command: &str) -> Result<()> {
        let command = command.trim();
        if command.is_empty() {
            return Err(anyhow!("command must not be empty"));
        }

        self.with_dispatch_view(|view| {
            KnownCommandProviders::collect(view, command).validate_command()
        })
    }

    #[cfg(test)]
    pub(crate) fn set_command_state(&self, command: &str, state: PluginCommandState) -> Result<()> {
        self.validate_command(command)?;
        self.update_command_preferences(|preferences| {
            preferences.set_state(command, state);
        });
        Ok(())
    }

    /// Applies an explicit provider selection for a command on this manager.
    ///
    /// The selection is kept in the manager's in-memory command-preference
    /// state and affects subsequent command resolution through this
    /// `PluginManager` value. It is not written to disk.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[cfg(unix)] {
    /// use osp_cli::plugin::PluginManager;
    /// # use std::fs;
    /// # use std::os::unix::fs::PermissionsExt;
    /// # use std::time::{SystemTime, UNIX_EPOCH};
    /// #
    /// # fn write_provider_plugin(dir: &std::path::Path, plugin_id: &str) -> std::io::Result<()> {
    /// #     let plugin_path = dir.join(format!("osp-{plugin_id}"));
    /// #     let script = format!(
    /// #         r#"#!/bin/sh
    /// # PATH=/usr/bin:/bin
    /// # if [ "$1" = "--describe" ]; then
    /// #   cat <<'JSON'
    /// # {{"protocol_version":1,"plugin_id":"{plugin_id}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"shared","about":"{plugin_id} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
    /// # JSON
    /// #   exit 0
    /// # fi
    /// #
    /// # cat <<'JSON'
    /// # {{"protocol_version":1,"ok":true,"data":{{"message":"ok"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
    /// # JSON
    /// # "#,
    /// #         plugin_id = plugin_id
    /// #     );
    /// #     fs::write(&plugin_path, script)?;
    /// #     let mut perms = fs::metadata(&plugin_path)?.permissions();
    /// #     perms.set_mode(0o755);
    /// #     fs::set_permissions(&plugin_path, perms)?;
    /// #     Ok(())
    /// # }
    /// #
    /// # let root = std::env::temp_dir().join(format!(
    /// #     "osp-cli-doc-provider-selection-{}-{}",
    /// #     std::process::id(),
    /// #     SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
    /// # ));
    /// # let plugins_dir = root.join("plugins");
    /// # fs::create_dir_all(&plugins_dir)?;
    /// # write_provider_plugin(&plugins_dir, "alpha")?;
    /// # write_provider_plugin(&plugins_dir, "beta")?;
    /// #
    /// let manager = PluginManager::new(vec![plugins_dir]);
    ///
    /// assert_eq!(manager.selected_provider_label("shared"), None);
    ///
    /// manager.select_provider("shared", "beta")?;
    ///
    /// assert_eq!(
    ///     manager.selected_provider_label("shared").as_deref(),
    ///     Some("beta (explicit)")
    /// );
    /// # fs::remove_dir_all(&root).ok();
    /// # }
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when `command` or `plugin_id` is blank, when no
    /// currently available provider exports `command`, or when `plugin_id` is
    /// not one of the currently available providers for `command`.
    pub fn select_provider(&self, command: &str, plugin_id: &str) -> Result<()> {
        let command = command.trim();
        let plugin_id = plugin_id.trim();
        if command.is_empty() {
            return Err(anyhow!("command must not be empty"));
        }
        if plugin_id.is_empty() {
            return Err(anyhow!("plugin id must not be empty"));
        }

        self.validate_provider_selection(command, plugin_id)?;
        self.update_command_preferences(|preferences| preferences.set_provider(command, plugin_id));
        Ok(())
    }

    /// Clears any explicit in-memory provider selection for a command.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::plugin::PluginManager;
    ///
    /// let removed = PluginManager::new(Vec::new())
    ///     .clear_provider_selection("shared")
    ///     .unwrap();
    ///
    /// assert!(!removed);
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when `command` is blank.
    pub fn clear_provider_selection(&self, command: &str) -> Result<bool> {
        let command = command.trim();
        if command.is_empty() {
            return Err(anyhow!("command must not be empty"));
        }

        let mut removed = false;
        self.update_command_preferences(|preferences| {
            removed = preferences.clear_provider(command);
        });
        Ok(removed)
    }

    /// Verifies that a plugin is a currently available provider candidate for
    /// a command.
    ///
    /// This validates the command/plugin pair against the manager's current
    /// discovery view but does not change selection state or persist anything.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::plugin::PluginManager;
    ///
    /// let err = PluginManager::new(Vec::new())
    ///     .validate_provider_selection("shared", "alpha")
    ///     .unwrap_err();
    ///
    /// assert!(err.to_string().contains("no available plugin provides command"));
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error when no currently available provider exports
    /// `command`, or when `plugin_id` is not one of the currently available
    /// providers for `command`.
    pub fn validate_provider_selection(&self, command: &str, plugin_id: &str) -> Result<()> {
        self.with_dispatch_view(|view| {
            AvailableCommandProviders::collect(view, command).validate_provider(plugin_id)
        })
    }

    pub(super) fn resolve_provider(
        &self,
        command: &str,
        provider_override: Option<&str>,
    ) -> std::result::Result<DiscoveredPlugin, PluginDispatchError> {
        self.with_dispatch_view(|view| {
            let available = AvailableCommandProviders::collect(view, command);
            match view.resolve_provider(command, provider_override) {
                Ok(ProviderResolution::Selected(selection)) => {
                    tracing::debug!(
                        command = %command,
                        active_providers = available.len(),
                        selected_provider = %selection.plugin.plugin_id,
                        selection_mode = ?selection.mode,
                        "resolved plugin provider"
                    );
                    Ok(selection.plugin.clone())
                }
                Ok(ProviderResolution::Ambiguous(providers)) => {
                    let provider_labels = provider_labels(&providers);
                    tracing::warn!(
                        command = %command,
                        providers = provider_labels.join(", "),
                        "plugin command requires explicit provider selection"
                    );
                    Err(PluginDispatchError::CommandAmbiguous {
                        command: command.to_string(),
                        providers: provider_labels,
                    })
                }
                Err(ProviderResolutionError::RequestedProviderUnavailable {
                    requested_provider,
                    providers,
                }) => {
                    let provider_labels = provider_labels(&providers);
                    tracing::warn!(
                        command = %command,
                        requested_provider = %requested_provider,
                        providers = provider_labels.join(", "),
                        "requested plugin provider is not available for command"
                    );
                    Err(PluginDispatchError::ProviderNotFound {
                        command: command.to_string(),
                        requested_provider,
                        providers: provider_labels,
                    })
                }
                Err(ProviderResolutionError::CommandNotFound) => {
                    tracing::warn!(
                        command = %command,
                        active_plugins = view.healthy_plugins().len(),
                        "no plugin provider found for command"
                    );
                    Err(PluginDispatchError::CommandNotFound {
                        command: command.to_string(),
                    })
                }
            }
        })
    }

    // Build the shared passive plugin working set once per operation so read
    // paths stop re-deriving health filtering and provider labels independently.
    fn with_passive_view<R, F>(&self, apply: F) -> R
    where
        F: FnOnce(&ActivePluginView<'_>) -> R,
    {
        self.with_discovered_view(self.discover(), apply)
    }

    // Dispatch paths use the execution-aware discovery snapshot, but the
    // downstream provider-selection rules remain the same shared active view.
    fn with_dispatch_view<R, F>(&self, apply: F) -> R
    where
        F: FnOnce(&ActivePluginView<'_>) -> R,
    {
        self.with_discovered_view(self.discover_for_dispatch(), apply)
    }

    fn with_discovered_view<R, F>(&self, discovered: Arc<[DiscoveredPlugin]>, apply: F) -> R
    where
        F: FnOnce(&ActivePluginView<'_>) -> R,
    {
        let preferences = self.command_preferences();
        let view = ActivePluginView::new(discovered.as_ref(), &preferences);
        apply(&view)
    }

    fn with_passive_catalog<R, F>(&self, apply: F) -> R
    where
        F: FnOnce(Vec<CommandCatalogEntry>) -> R,
    {
        self.with_passive_view(|view| apply(build_command_catalog(view)))
    }

    fn command_preferences(&self) -> PluginCommandPreferences {
        self.command_preferences
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
    }

    pub(crate) fn command_preferences_snapshot(&self) -> PluginCommandPreferences {
        self.command_preferences()
    }

    pub(crate) fn replace_command_preferences(&self, preferences: PluginCommandPreferences) {
        let mut current = self
            .command_preferences
            .write()
            .unwrap_or_else(|err| err.into_inner());
        *current = preferences;
    }

    fn update_command_preferences<F>(&self, update: F)
    where
        F: FnOnce(&mut PluginCommandPreferences),
    {
        let mut preferences = self
            .command_preferences
            .write()
            .unwrap_or_else(|err| err.into_inner());
        update(&mut preferences);
    }
}
