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

pub const DEFAULT_PLUGIN_PROCESS_TIMEOUT_MS: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginSource {
    Explicit,
    Env,
    Bundled,
    UserConfig,
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

#[derive(Debug, Clone)]
pub struct DiscoveredPlugin {
    pub plugin_id: String,
    pub plugin_version: Option<String>,
    pub executable: PathBuf,
    pub source: PluginSource,
    pub commands: Vec<String>,
    pub describe_commands: Vec<DescribeCommandV1>,
    pub command_specs: Vec<CommandSpec>,
    pub issue: Option<String>,
    pub default_enabled: bool,
}

#[derive(Debug, Clone)]
pub struct PluginSummary {
    pub plugin_id: String,
    pub plugin_version: Option<String>,
    pub executable: PathBuf,
    pub source: PluginSource,
    pub commands: Vec<String>,
    pub enabled: bool,
    pub healthy: bool,
    pub issue: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CommandConflict {
    pub command: String,
    pub providers: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DoctorReport {
    pub plugins: Vec<PluginSummary>,
    pub conflicts: Vec<CommandConflict>,
}

#[derive(Debug, Clone)]
pub struct CommandCatalogEntry {
    pub name: String,
    pub about: String,
    pub auth: Option<DescribeCommandAuthV1>,
    pub subcommands: Vec<String>,
    pub completion: CommandSpec,
    pub provider: Option<String>,
    pub providers: Vec<String>,
    pub conflicted: bool,
    pub requires_selection: bool,
    pub selected_explicitly: bool,
    pub source: Option<PluginSource>,
}

impl CommandCatalogEntry {
    pub fn auth_hint(&self) -> Option<String> {
        self.auth.as_ref().and_then(|auth| auth.hint())
    }
}

#[derive(Debug, Clone)]
pub struct RawPluginOutput {
    pub status_code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Clone, Default)]
pub struct PluginDispatchContext {
    pub runtime_hints: RuntimeHints,
    pub shared_env: Vec<(String, String)>,
    pub plugin_env: HashMap<String, Vec<(String, String)>>,
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

#[derive(Debug)]
pub enum PluginDispatchError {
    CommandNotFound {
        command: String,
    },
    CommandAmbiguous {
        command: String,
        providers: Vec<String>,
    },
    ProviderNotFound {
        command: String,
        requested_provider: String,
        providers: Vec<String>,
    },
    ExecuteFailed {
        plugin_id: String,
        source: std::io::Error,
    },
    TimedOut {
        plugin_id: String,
        timeout: Duration,
        stderr: String,
    },
    NonZeroExit {
        plugin_id: String,
        status_code: i32,
        stderr: String,
    },
    InvalidJsonResponse {
        plugin_id: String,
        source: serde_json::Error,
    },
    InvalidResponsePayload {
        plugin_id: String,
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

    pub fn with_roots(mut self, config_root: Option<PathBuf>, cache_root: Option<PathBuf>) -> Self {
        self.config_root = config_root;
        self.cache_root = cache_root;
        self
    }

    pub fn with_process_timeout(mut self, timeout: Duration) -> Self {
        self.process_timeout = timeout.max(Duration::from_millis(1));
        self
    }

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
