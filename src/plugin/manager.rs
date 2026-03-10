use super::state::PluginCommandPreferences;
use crate::completion::CommandSpec;
use crate::core::plugin::{DescribeCommandAuthV1, DescribeCommandV1};
use crate::core::runtime::RuntimeHints;
use std::collections::HashMap;
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
    /// Loaded from a path specified through an environment variable.
    Env,
    /// Loaded from the CLI's bundled plugin set.
    Bundled,
    /// Loaded from the persisted user configuration.
    UserConfig,
    /// Loaded by scanning the process `PATH`.
    Path,
}

impl Display for PluginSource {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            PluginSource::Explicit => "explicit",
            PluginSource::Env => "env",
            PluginSource::Bundled => "bundled",
            PluginSource::UserConfig => "user",
            PluginSource::Path => "path",
        };
        write!(f, "{value}")
    }
}

/// Full discovery metadata for a single plugin executable.
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
    /// Top-level commands exported by the plugin.
    pub commands: Vec<String>,
    /// Raw describe-command payloads returned by the plugin.
    pub describe_commands: Vec<DescribeCommandV1>,
    /// Normalized completion specs derived from `describe_commands`.
    pub command_specs: Vec<CommandSpec>,
    /// Discovery or validation issue associated with this plugin.
    pub issue: Option<String>,
    /// Whether the plugin should be enabled by default.
    pub default_enabled: bool,
}

/// User-facing summary of a discovered plugin.
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
    /// Whether the plugin is enabled for dispatch.
    pub enabled: bool,
    /// Whether the plugin passed discovery-time validation.
    pub healthy: bool,
    /// Discovery or validation issue associated with this plugin.
    pub issue: Option<String>,
}

/// Reports that multiple plugins provide the same command.
#[derive(Debug, Clone)]
pub struct CommandConflict {
    /// Conflicting command name.
    pub command: String,
    /// Plugin identifiers that provide `command`.
    pub providers: Vec<String>,
}

/// Aggregated health information for plugin discovery and dispatch.
#[derive(Debug, Clone)]
pub struct DoctorReport {
    /// Summary entry for each discovered plugin.
    pub plugins: Vec<PluginSummary>,
    /// Commands that are provided by more than one plugin.
    pub conflicts: Vec<CommandConflict>,
}

/// Catalog entry for a command exposed through the plugin system.
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
    /// Selected provider when dispatch has been resolved.
    pub provider: Option<String>,
    /// All providers that export this command.
    pub providers: Vec<String>,
    /// Whether more than one provider exports this command.
    pub conflicted: bool,
    /// Whether the caller must choose a provider before dispatch.
    pub requires_selection: bool,
    /// Whether the provider was selected explicitly by the caller.
    pub selected_explicitly: bool,
    /// Discovery source for the selected provider, if resolved.
    pub source: Option<PluginSource>,
}

impl CommandCatalogEntry {
    /// Returns the optional auth hint rendered in help and catalog views.
    pub fn auth_hint(&self) -> Option<String> {
        self.auth.as_ref().and_then(|auth| auth.hint())
    }
}

/// Raw stdout/stderr captured from a plugin subprocess invocation.
#[derive(Debug, Clone)]
pub struct RawPluginOutput {
    /// Process exit status code.
    pub status_code: i32,
    /// Captured standard output.
    pub stdout: String,
    /// Captured standard error.
    pub stderr: String,
}

/// Per-dispatch runtime overrides shared with plugin subprocesses.
#[derive(Debug, Clone, Default)]
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
        /// Plugin identifiers that provide `command`.
        providers: Vec<String>,
    },
    /// The requested provider exists, but not for the requested command.
    ProviderNotFound {
        /// Command name requested by the caller.
        command: String,
        /// Provider identifier requested by the caller.
        requested_provider: String,
        /// Plugin identifiers that provide `command`.
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

/// Coordinates plugin discovery, caching, and dispatch settings.
pub struct PluginManager {
    pub(crate) explicit_dirs: Vec<PathBuf>,
    pub(crate) discovered_cache: RwLock<Option<Arc<[DiscoveredPlugin]>>>,
    pub(crate) dispatch_discovered_cache: RwLock<Option<Arc<[DiscoveredPlugin]>>>,
    pub(crate) command_preferences: RwLock<PluginCommandPreferences>,
    pub(crate) config_root: Option<PathBuf>,
    pub(crate) cache_root: Option<PathBuf>,
    pub(crate) process_timeout: Duration,
    pub(crate) allow_path_discovery: bool,
}

impl PluginManager {
    /// Creates a plugin manager with the provided explicit search roots.
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
        }
    }

    /// Sets config and cache roots used for persisted plugin metadata and preferences.
    pub fn with_roots(mut self, config_root: Option<PathBuf>, cache_root: Option<PathBuf>) -> Self {
        self.config_root = config_root;
        self.cache_root = cache_root;
        self
    }

    /// Sets the subprocess timeout used for plugin describe and dispatch calls.
    pub fn with_process_timeout(mut self, timeout: Duration) -> Self {
        self.process_timeout = timeout.max(Duration::from_millis(1));
        self
    }

    /// Enables or disables fallback discovery through the process `PATH`.
    pub fn with_path_discovery(mut self, allow_path_discovery: bool) -> Self {
        self.allow_path_discovery = allow_path_discovery;
        self
    }

    pub(crate) fn with_command_preferences(
        mut self,
        preferences: PluginCommandPreferences,
    ) -> Self {
        self.command_preferences = RwLock::new(preferences);
        self
    }
}
