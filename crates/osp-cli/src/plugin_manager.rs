use anyhow::{Context, Result, anyhow};
use osp_completion::{ArgNode, CommandSpec, FlagNode, SuggestionEntry, ValueType};
use osp_config::{default_cache_root_dir, default_config_root_dir};
use osp_core::plugin::{
    DescribeArgV1, DescribeCommandV1, DescribeFlagV1, DescribeSuggestionV1, DescribeV1, ResponseV1,
};
use osp_core::runtime::RuntimeHints;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::error::Error as StdError;
use std::fmt::{Display, Formatter, Write as FmtWrite};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PLUGIN_EXECUTABLE_PREFIX: &str = "osp-";
const BUNDLED_MANIFEST_FILE: &str = "manifest.toml";
const ENV_OSP_COMMAND: &str = "OSP_COMMAND";
pub const DEFAULT_PLUGIN_PROCESS_TIMEOUT_MS: usize = 10_000;
const PROCESS_WAIT_POLL_INTERVAL: Duration = Duration::from_millis(10);

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
struct SearchRoot {
    path: PathBuf,
    source: PluginSource,
}

#[derive(Debug, Clone)]
pub struct DiscoveredPlugin {
    pub plugin_id: String,
    pub plugin_version: Option<String>,
    pub executable: PathBuf,
    pub source: PluginSource,
    pub commands: Vec<String>,
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
    pub subcommands: Vec<String>,
    pub completion: CommandSpec,
    pub provider: Option<String>,
    pub providers: Vec<String>,
    pub conflicted: bool,
    pub requires_selection: bool,
    pub selected_explicitly: bool,
    pub source: Option<PluginSource>,
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
    fn env_pairs_for<'a>(&'a self, plugin_id: &'a str) -> impl Iterator<Item = (&'a str, &'a str)> {
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PluginState {
    #[serde(default)]
    enabled: Vec<String>,
    #[serde(default)]
    disabled: Vec<String>,
    #[serde(default)]
    preferred_providers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderSelectionMode {
    Override,
    Preference,
    Unique,
}

#[derive(Debug, Clone, Copy)]
struct ProviderSelection<'a> {
    plugin: &'a DiscoveredPlugin,
    mode: ProviderSelectionMode,
}

enum ProviderResolution<'a> {
    Selected(ProviderSelection<'a>),
    Ambiguous(Vec<&'a DiscoveredPlugin>),
}

#[derive(Debug)]
enum ProviderResolutionError<'a> {
    CommandNotFound,
    RequestedProviderUnavailable {
        requested_provider: String,
        providers: Vec<&'a DiscoveredPlugin>,
    },
}

#[derive(Debug, Clone, Deserialize)]
struct BundledManifest {
    protocol_version: u32,
    #[serde(default)]
    plugin: Vec<ManifestPlugin>,
}

#[derive(Debug, Clone, Deserialize)]
struct ManifestPlugin {
    id: String,
    exe: String,
    version: String,
    #[serde(default = "default_true")]
    enabled_by_default: bool,
    checksum_sha256: Option<String>,
    #[serde(default)]
    commands: Vec<String>,
}

#[derive(Debug, Clone)]
struct ValidatedBundledManifest {
    by_exe: HashMap<String, ManifestPlugin>,
}

enum ManifestState {
    NotBundled,
    Missing,
    Invalid(String),
    Valid(ValidatedBundledManifest),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct DescribeCacheFile {
    #[serde(default)]
    entries: Vec<DescribeCacheEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DescribeCacheEntry {
    path: String,
    size: u64,
    mtime_secs: u64,
    mtime_nanos: u32,
    describe: DescribeV1,
}

pub struct PluginManager {
    explicit_dirs: Vec<PathBuf>,
    discovered_cache: RwLock<Option<Arc<[DiscoveredPlugin]>>>,
    config_root: Option<PathBuf>,
    cache_root: Option<PathBuf>,
    process_timeout: Duration,
}

impl PluginManager {
    pub fn new(explicit_dirs: Vec<PathBuf>) -> Self {
        Self {
            explicit_dirs,
            discovered_cache: RwLock::new(None),
            config_root: None,
            cache_root: None,
            process_timeout: Duration::from_millis(DEFAULT_PLUGIN_PROCESS_TIMEOUT_MS as u64),
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

    pub fn list_plugins(&self) -> Result<Vec<PluginSummary>> {
        let discovered = self.discover();
        let state = self.load_state().unwrap_or_default();

        Ok(discovered
            .iter()
            .map(|plugin| PluginSummary {
                enabled: is_enabled(&state, &plugin.plugin_id, plugin.default_enabled),
                healthy: plugin.issue.is_none(),
                issue: plugin.issue.clone(),
                plugin_id: plugin.plugin_id.clone(),
                plugin_version: plugin.plugin_version.clone(),
                executable: plugin.executable.clone(),
                source: plugin.source,
                commands: plugin.commands.clone(),
            })
            .collect())
    }

    pub fn command_catalog(&self) -> Result<Vec<CommandCatalogEntry>> {
        let state = self.load_state().unwrap_or_default();
        let discovered = self.discover();
        let active = active_plugins(discovered.as_ref(), &state).collect::<Vec<_>>();
        let provider_index = provider_labels_by_command(&active);
        let command_names = active
            .iter()
            .flat_map(|plugin| plugin.command_specs.iter().map(|spec| spec.name.clone()))
            .collect::<BTreeSet<_>>();
        let mut out = Vec::new();

        for command_name in command_names {
            let providers = provider_index
                .get(&command_name)
                .cloned()
                .unwrap_or_default();
            match resolve_provider_for_command(&command_name, &active, &state, None)
                .expect("active command name should resolve to one or more providers")
            {
                ProviderResolution::Selected(selection) => {
                    let spec = selection
                        .plugin
                        .command_specs
                        .iter()
                        .find(|spec| spec.name == command_name)
                        .expect("selected provider should include command spec");
                    out.push(CommandCatalogEntry {
                        name: command_name,
                        about: spec.tooltip.clone().unwrap_or_default(),
                        subcommands: direct_subcommand_names(spec),
                        completion: spec.clone(),
                        provider: Some(selection.plugin.plugin_id.clone()),
                        providers: providers.clone(),
                        conflicted: providers.len() > 1,
                        requires_selection: false,
                        selected_explicitly: matches!(
                            selection.mode,
                            ProviderSelectionMode::Override | ProviderSelectionMode::Preference
                        ),
                        source: Some(selection.plugin.source),
                    });
                }
                ProviderResolution::Ambiguous(_) => {
                    let about = format!(
                        "provider selection required; use --plugin-provider <plugin-id> or `osp plugins select-provider {command_name} <plugin-id>`"
                    );
                    out.push(CommandCatalogEntry {
                        name: command_name.clone(),
                        about: about.clone(),
                        subcommands: Vec::new(),
                        completion: CommandSpec::new(command_name),
                        provider: None,
                        providers: providers.clone(),
                        conflicted: true,
                        requires_selection: true,
                        selected_explicitly: false,
                        source: None,
                    });
                }
            }
        }

        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    pub fn completion_words(&self) -> Result<Vec<String>> {
        let catalog = self.command_catalog()?;
        // These are REPL grammar tokens that stay available even before any
        // plugin commands are added to the completion tree.
        let mut words = vec![
            "help".to_string(),
            "exit".to_string(),
            "quit".to_string(),
            "P".to_string(),
            "F".to_string(),
            "V".to_string(),
            "|".to_string(),
        ];

        for command in catalog {
            words.push(command.name);
            words.extend(collect_completion_words(&command.completion));
        }

        words.sort();
        words.dedup();
        Ok(words)
    }

    pub fn repl_help_text(&self) -> Result<String> {
        let catalog = self.command_catalog()?;
        let mut out = String::new();

        out.push_str("Backbone commands: help, exit, quit\n");
        if catalog.is_empty() {
            out.push_str("No plugin commands available.\n");
            return Ok(out);
        }

        out.push_str("Plugin commands:\n");
        for command in catalog {
            let subs = if command.subcommands.is_empty() {
                "".to_string()
            } else {
                format!(" [{}]", command.subcommands.join(", "))
            };
            let about = if command.about.trim().is_empty() {
                "-".to_string()
            } else {
                command.about.clone()
            };
            if command.requires_selection {
                out.push_str(&format!(
                    "  {name}{subs} - {about} (providers: {providers})\n",
                    name = command.name,
                    providers = command.providers.join(", "),
                ));
            } else {
                let conflict = if command.conflicted {
                    format!(" conflicts: {}", command.providers.join(", "))
                } else {
                    String::new()
                };
                out.push_str(&format!(
                    "  {name}{subs} - {about} ({provider}/{source}){conflict}\n",
                    name = command.name,
                    provider = command.provider.as_deref().unwrap_or("-"),
                    source = command
                        .source
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    conflict = conflict,
                ));
            }
        }

        Ok(out)
    }

    pub fn command_providers(&self, command: &str) -> Vec<String> {
        let state = self.load_state().unwrap_or_default();
        let discovered = self.discover();
        let mut out = Vec::new();
        for plugin in active_plugins(discovered.as_ref(), &state) {
            if plugin.commands.iter().any(|name| name == command) {
                out.push(format!("{} ({})", plugin.plugin_id, plugin.source));
            }
        }
        out
    }

    pub fn selected_provider_label(&self, command: &str) -> Option<String> {
        let state = self.load_state().unwrap_or_default();
        let discovered = self.discover();
        let active = active_plugins(discovered.as_ref(), &state).collect::<Vec<_>>();
        match resolve_provider_for_command(command, &active, &state, None).ok()? {
            ProviderResolution::Selected(selection) => Some(plugin_label(selection.plugin)),
            ProviderResolution::Ambiguous(_) => None,
        }
    }

    pub fn doctor(&self) -> Result<DoctorReport> {
        let plugins = self.list_plugins()?;
        let mut conflicts_index: HashMap<String, Vec<String>> = HashMap::new();

        for plugin in &plugins {
            if !plugin.enabled || !plugin.healthy {
                continue;
            }
            for command in &plugin.commands {
                conflicts_index
                    .entry(command.clone())
                    .or_default()
                    .push(format!("{} ({})", plugin.plugin_id, plugin.source));
            }
        }

        let mut conflicts = conflicts_index
            .into_iter()
            .filter_map(|(command, providers)| {
                if providers.len() > 1 {
                    Some(CommandConflict { command, providers })
                } else {
                    None
                }
            })
            .collect::<Vec<CommandConflict>>();
        conflicts.sort_by(|a, b| a.command.cmp(&b.command));

        Ok(DoctorReport { plugins, conflicts })
    }

    pub fn set_enabled(&self, plugin_id: &str, enabled: bool) -> Result<()> {
        let mut state = self.load_state().unwrap_or_default();
        state.enabled.retain(|id| id != plugin_id);
        state.disabled.retain(|id| id != plugin_id);

        if enabled {
            state.enabled.push(plugin_id.to_string());
        } else {
            state.disabled.push(plugin_id.to_string());
        }

        state.enabled.sort();
        state.enabled.dedup();
        state.disabled.sort();
        state.disabled.dedup();
        self.save_state(&state)
    }

    pub fn set_preferred_provider(&self, command: &str, plugin_id: &str) -> Result<()> {
        let command = command.trim();
        let plugin_id = plugin_id.trim();
        if command.is_empty() {
            return Err(anyhow!("command must not be empty"));
        }
        if plugin_id.is_empty() {
            return Err(anyhow!("plugin id must not be empty"));
        }

        let mut state = self.load_state().unwrap_or_default();
        let discovered = self.discover();
        let active = active_plugins(discovered.as_ref(), &state).collect::<Vec<_>>();
        let available = providers_for_command(command, &active);
        if available.is_empty() {
            return Err(anyhow!("no active plugin provides command `{command}`"));
        }
        if !available.iter().any(|plugin| plugin.plugin_id == plugin_id) {
            return Err(anyhow!(
                "plugin `{plugin_id}` does not provide active command `{command}`; available providers: {}",
                available
                    .iter()
                    .map(|plugin| plugin_label(plugin))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        state
            .preferred_providers
            .insert(command.to_string(), plugin_id.to_string());
        self.save_state(&state)
    }

    pub fn clear_preferred_provider(&self, command: &str) -> Result<bool> {
        let command = command.trim();
        if command.is_empty() {
            return Err(anyhow!("command must not be empty"));
        }

        let mut state = self.load_state().unwrap_or_default();
        let removed = state.preferred_providers.remove(command).is_some();
        if removed {
            self.save_state(&state)?;
        }
        Ok(removed)
    }

    pub fn dispatch(
        &self,
        command: &str,
        args: &[String],
        context: &PluginDispatchContext,
    ) -> std::result::Result<ResponseV1, PluginDispatchError> {
        let provider = self.resolve_provider(command, context.provider_override.as_deref())?;

        let raw = run_provider(&provider, command, args, context, self.process_timeout)?;
        if raw.status_code != 0 {
            tracing::warn!(
                plugin_id = %provider.plugin_id,
                command = %command,
                status_code = raw.status_code,
                stderr = %raw.stderr.trim(),
                "plugin command exited with non-zero status"
            );
            return Err(PluginDispatchError::NonZeroExit {
                plugin_id: provider.plugin_id.clone(),
                status_code: raw.status_code,
                stderr: raw.stderr,
            });
        }

        let response: ResponseV1 = serde_json::from_str(&raw.stdout).map_err(|source| {
            tracing::warn!(
                plugin_id = %provider.plugin_id,
                command = %command,
                error = %source,
                "plugin command returned invalid JSON"
            );
            PluginDispatchError::InvalidJsonResponse {
                plugin_id: provider.plugin_id.clone(),
                source,
            }
        })?;

        response.validate_v1().map_err(|reason| {
            tracing::warn!(
                plugin_id = %provider.plugin_id,
                command = %command,
                reason = %reason,
                "plugin command returned invalid payload"
            );
            PluginDispatchError::InvalidResponsePayload {
                plugin_id: provider.plugin_id.clone(),
                reason,
            }
        })?;

        Ok(response)
    }

    pub fn dispatch_passthrough(
        &self,
        command: &str,
        args: &[String],
        context: &PluginDispatchContext,
    ) -> std::result::Result<RawPluginOutput, PluginDispatchError> {
        let provider = self.resolve_provider(command, context.provider_override.as_deref())?;
        run_provider(&provider, command, args, context, self.process_timeout)
    }

    fn resolve_provider(
        &self,
        command: &str,
        provider_override: Option<&str>,
    ) -> std::result::Result<DiscoveredPlugin, PluginDispatchError> {
        let state = self.load_state().unwrap_or_default();
        let discovered = self.discover();
        let active = active_plugins(discovered.as_ref(), &state).collect::<Vec<_>>();
        match resolve_provider_for_command(command, &active, &state, provider_override) {
            Ok(ProviderResolution::Selected(selection)) => {
                tracing::debug!(
                    command = %command,
                    active_providers = providers_for_command(command, &active).len(),
                    selected_provider = %selection.plugin.plugin_id,
                    selection_mode = ?selection.mode,
                    "resolved plugin provider"
                );
                Ok(selection.plugin.clone())
            }
            Ok(ProviderResolution::Ambiguous(providers)) => {
                let provider_labels = providers
                    .iter()
                    .copied()
                    .map(plugin_label)
                    .collect::<Vec<_>>();
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
                let provider_labels = providers
                    .iter()
                    .copied()
                    .map(plugin_label)
                    .collect::<Vec<_>>();
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
                    active_plugins = active.len(),
                    "no plugin provider found for command"
                );
                Err(PluginDispatchError::CommandNotFound {
                    command: command.to_string(),
                })
            }
        }
    }

    pub fn refresh(&self) {
        let mut guard = self
            .discovered_cache
            .write()
            .unwrap_or_else(|err| err.into_inner());
        *guard = None;
    }

    fn discover(&self) -> Arc<[DiscoveredPlugin]> {
        if let Some(cached) = self
            .discovered_cache
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .clone()
        {
            return cached;
        }

        let mut guard = self
            .discovered_cache
            .write()
            .unwrap_or_else(|err| err.into_inner());
        if let Some(cached) = guard.clone() {
            return cached;
        }
        let discovered = self.discover_uncached();
        let shared = Arc::<[DiscoveredPlugin]>::from(discovered);
        *guard = Some(shared.clone());
        shared
    }

    fn discover_uncached(&self) -> Vec<DiscoveredPlugin> {
        let mut plugins: Vec<DiscoveredPlugin> = Vec::new();
        let mut seen_paths: HashSet<PathBuf> = HashSet::new();
        let mut describe_cache = self.load_describe_cache().unwrap_or_default();
        let mut seen_describe_paths: HashSet<String> = HashSet::new();
        let mut cache_dirty = false;

        for root in self.search_roots() {
            plugins.extend(discover_plugins_in_root(
                &root,
                &mut seen_paths,
                &mut describe_cache,
                &mut seen_describe_paths,
                &mut cache_dirty,
                self.process_timeout,
            ));
        }

        cache_dirty |=
            prune_stale_describe_cache_entries(&mut describe_cache, &seen_describe_paths);
        if cache_dirty {
            let _ = self.save_describe_cache(&describe_cache);
        }

        tracing::debug!(
            discovered_plugins = plugins.len(),
            unhealthy_plugins = plugins
                .iter()
                .filter(|plugin| plugin.issue.is_some())
                .count(),
            search_roots = self.search_roots().len(),
            "completed plugin discovery"
        );

        plugins
    }

    fn search_roots(&self) -> Vec<SearchRoot> {
        let ordered = self.ordered_search_roots();
        let roots = existing_unique_search_roots(ordered);
        tracing::debug!(search_roots = roots.len(), "resolved plugin search roots");
        roots
    }

    fn ordered_search_roots(&self) -> Vec<SearchRoot> {
        let mut ordered = Vec::new();

        ordered.extend(self.explicit_dirs.iter().cloned().map(|path| SearchRoot {
            path,
            source: PluginSource::Explicit,
        }));

        if let Ok(raw) = std::env::var("OSP_PLUGIN_PATH") {
            ordered.extend(
                std::env::split_paths(&raw)
                    .map(|path| SearchRoot {
                        path,
                        source: PluginSource::Env,
                    })
                    .collect::<Vec<SearchRoot>>(),
            );
        }

        ordered.extend(
            bundled_plugin_dirs()
                .into_iter()
                .map(|path| SearchRoot {
                    path,
                    source: PluginSource::Bundled,
                })
                .collect::<Vec<SearchRoot>>(),
        );

        if let Some(user_dir) = self.user_plugin_dir() {
            ordered.push(SearchRoot {
                path: user_dir,
                source: PluginSource::UserConfig,
            });
        }

        if let Ok(raw) = std::env::var("PATH") {
            ordered.extend(
                std::env::split_paths(&raw)
                    .map(|path| SearchRoot {
                        path,
                        source: PluginSource::Path,
                    })
                    .collect::<Vec<SearchRoot>>(),
            );
        }

        tracing::trace!(
            search_roots = ordered.len(),
            "assembled ordered plugin search roots"
        );
        ordered
    }

    fn load_state(&self) -> Result<PluginState> {
        let path = self
            .plugin_state_path()
            .ok_or_else(|| anyhow!("failed to resolve plugin state path"))?;
        if !path.exists() {
            tracing::debug!(path = %path.display(), "plugin state file missing; using defaults");
            return Ok(PluginState::default());
        }

        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read plugin state from {}", path.display()))?;
        let state = serde_json::from_str::<PluginState>(&raw)
            .with_context(|| format!("failed to parse plugin state at {}", path.display()))?;
        tracing::debug!(
            path = %path.display(),
            enabled = state.enabled.len(),
            disabled = state.disabled.len(),
            preferred = state.preferred_providers.len(),
            "loaded plugin state"
        );
        Ok(state)
    }

    fn save_state(&self, state: &PluginState) -> Result<()> {
        let path = self
            .plugin_state_path()
            .ok_or_else(|| anyhow!("failed to resolve plugin state path"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create plugin state dir {}", parent.display())
            })?;
        }

        let payload = serde_json::to_string_pretty(state)?;
        write_text_atomic(&path, &payload)
            .with_context(|| format!("failed to write plugin state to {}", path.display()))
    }

    fn load_describe_cache(&self) -> Result<DescribeCacheFile> {
        let Some(path) = self.describe_cache_path() else {
            tracing::debug!("describe cache path unavailable; using empty cache");
            return Ok(DescribeCacheFile::default());
        };
        if !path.exists() {
            tracing::debug!(path = %path.display(), "describe cache missing; using empty cache");
            return Ok(DescribeCacheFile::default());
        }

        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read describe cache {}", path.display()))?;
        let cache = serde_json::from_str::<DescribeCacheFile>(&raw)
            .with_context(|| format!("failed to parse describe cache {}", path.display()))?;
        tracing::debug!(
            path = %path.display(),
            entries = cache.entries.len(),
            "loaded describe cache"
        );
        Ok(cache)
    }

    fn save_describe_cache(&self, cache: &DescribeCacheFile) -> Result<()> {
        let Some(path) = self.describe_cache_path() else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create describe cache dir {}", parent.display())
            })?;
        }

        let payload = serde_json::to_string_pretty(cache)?;
        write_text_atomic(&path, &payload)
            .with_context(|| format!("failed to write describe cache {}", path.display()))
    }

    fn user_plugin_dir(&self) -> Option<PathBuf> {
        let mut path = self.config_root.clone().or_else(default_config_root_dir)?;
        path.push("plugins");
        Some(path)
    }

    fn plugin_state_path(&self) -> Option<PathBuf> {
        let mut path = self.config_root.clone().or_else(default_config_root_dir)?;
        path.push("plugins.json");
        Some(path)
    }

    fn describe_cache_path(&self) -> Option<PathBuf> {
        let mut path = self.cache_root.clone().or_else(default_cache_root_dir)?;
        path.push("describe-v1.json");
        Some(path)
    }
}

fn to_command_spec(command: &DescribeCommandV1) -> CommandSpec {
    let spec = CommandSpec::new(&command.name)
        .args(command.args.iter().map(to_arg_node))
        .flags(
            command
                .flags
                .iter()
                .map(|(name, flag)| (name.clone(), to_flag_node(flag))),
        )
        .subcommands(command.subcommands.iter().map(to_command_spec));

    if command.about.trim().is_empty() {
        spec
    } else {
        spec.tooltip(&command.about)
    }
}

fn to_arg_node(arg: &DescribeArgV1) -> ArgNode {
    let mut node = ArgNode::default().suggestions(arg.suggestions.iter().map(to_suggestion_entry));
    if let Some(name) = &arg.name {
        node.name = Some(name.clone());
    }
    if let Some(about) = &arg.about {
        node = node.tooltip(about);
    }
    if arg.multi {
        node = node.multi();
    }
    if let Some(value_type) = arg.value_type.and_then(to_value_type) {
        node = node.value_type(value_type);
    }
    node
}

fn to_flag_node(flag: &DescribeFlagV1) -> FlagNode {
    let mut node = FlagNode::new().suggestions(flag.suggestions.iter().map(to_suggestion_entry));
    if let Some(about) = &flag.about {
        node = node.tooltip(about);
    }
    if flag.flag_only {
        node = node.flag_only();
    }
    if flag.multi {
        node = node.multi();
    }
    if let Some(value_type) = flag.value_type.and_then(to_value_type) {
        node = node.value_type(value_type);
    }
    node
}

fn to_suggestion_entry(entry: &DescribeSuggestionV1) -> SuggestionEntry {
    SuggestionEntry {
        value: entry.value.clone(),
        meta: entry.meta.clone(),
        display: entry.display.clone(),
        sort: entry.sort.clone(),
    }
}

fn to_value_type(value_type: osp_core::plugin::DescribeValueTypeV1) -> Option<ValueType> {
    match value_type {
        osp_core::plugin::DescribeValueTypeV1::Path => Some(ValueType::Path),
    }
}

fn direct_subcommand_names(spec: &CommandSpec) -> Vec<String> {
    spec.subcommands
        .iter()
        .map(|subcommand| subcommand.name.clone())
        .collect()
}

fn collect_completion_words(spec: &CommandSpec) -> Vec<String> {
    let mut words = vec![spec.name.clone()];
    for flag in spec.flags.keys() {
        words.push(flag.clone());
    }
    for subcommand in &spec.subcommands {
        words.extend(collect_completion_words(subcommand));
    }
    words
}

fn load_manifest_state(root: &SearchRoot) -> ManifestState {
    let Some(path) = bundled_manifest_path(root) else {
        return ManifestState::NotBundled;
    };
    if !path.exists() {
        return ManifestState::Missing;
    }

    load_manifest_state_from_path(&path)
}

fn bundled_manifest_path(root: &SearchRoot) -> Option<PathBuf> {
    (root.source == PluginSource::Bundled).then(|| root.path.join(BUNDLED_MANIFEST_FILE))
}

fn load_manifest_state_from_path(path: &Path) -> ManifestState {
    match load_and_validate_manifest(path) {
        Ok(manifest) => ManifestState::Valid(manifest),
        Err(err) => ManifestState::Invalid(err.to_string()),
    }
}

fn existing_unique_search_roots(ordered: Vec<SearchRoot>) -> Vec<SearchRoot> {
    let mut deduped_paths: HashSet<PathBuf> = HashSet::new();
    ordered
        .into_iter()
        .filter(|root| {
            if !root.path.is_dir() {
                return false;
            }
            let canonical = root
                .path
                .canonicalize()
                .unwrap_or_else(|_| root.path.clone());
            deduped_paths.insert(canonical)
        })
        .collect()
}

fn discover_root_executables(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };

    let mut executables = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| is_plugin_executable(path))
        .collect::<Vec<PathBuf>>();
    executables.sort();
    executables
}

fn discover_plugins_in_root(
    root: &SearchRoot,
    seen_paths: &mut HashSet<PathBuf>,
    describe_cache: &mut DescribeCacheFile,
    seen_describe_paths: &mut HashSet<String>,
    cache_dirty: &mut bool,
    process_timeout: Duration,
) -> Vec<DiscoveredPlugin> {
    let manifest_state = load_manifest_state(root);
    let plugins = discover_root_executables(&root.path)
        .into_iter()
        .filter(|path| seen_paths.insert(path.clone()))
        .map(|executable| {
            assemble_discovered_plugin(
                root.source,
                executable,
                &manifest_state,
                describe_cache,
                seen_describe_paths,
                cache_dirty,
                process_timeout,
            )
        })
        .collect::<Vec<_>>();

    tracing::debug!(
        root = %root.path.display(),
        source = %root.source,
        discovered_plugins = plugins.len(),
        unhealthy_plugins = plugins.iter().filter(|plugin| plugin.issue.is_some()).count(),
        "scanned plugin search root"
    );

    plugins
}

fn assemble_discovered_plugin(
    source: PluginSource,
    executable: PathBuf,
    manifest_state: &ManifestState,
    describe_cache: &mut DescribeCacheFile,
    seen_describe_paths: &mut HashSet<String>,
    cache_dirty: &mut bool,
    process_timeout: Duration,
) -> DiscoveredPlugin {
    let file_name = executable
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string();
    let manifest_entry = manifest_entry_for_executable(manifest_state, &file_name);
    let mut plugin =
        seeded_discovered_plugin(source, executable.clone(), &file_name, &manifest_entry);

    apply_manifest_discovery_issue(&mut plugin.issue, manifest_state, manifest_entry.as_ref());

    match describe_with_cache(
        &executable,
        describe_cache,
        seen_describe_paths,
        cache_dirty,
        process_timeout,
    ) {
        Ok(describe) => {
            apply_describe_metadata(&mut plugin, &describe, manifest_entry.as_ref(), &executable)
        }
        Err(err) => merge_issue(&mut plugin.issue, err.to_string()),
    }

    tracing::debug!(
        plugin_id = %plugin.plugin_id,
        source = %plugin.source,
        executable = %plugin.executable.display(),
        healthy = plugin.issue.is_none(),
        issue = ?plugin.issue,
        command_count = plugin.commands.len(),
        "assembled discovered plugin"
    );

    plugin
}

fn manifest_entry_for_executable(
    manifest_state: &ManifestState,
    file_name: &str,
) -> Option<ManifestPlugin> {
    match manifest_state {
        ManifestState::Valid(manifest) => manifest.by_exe.get(file_name).cloned(),
        ManifestState::NotBundled | ManifestState::Missing | ManifestState::Invalid(_) => None,
    }
}

fn seeded_discovered_plugin(
    source: PluginSource,
    executable: PathBuf,
    file_name: &str,
    manifest_entry: &Option<ManifestPlugin>,
) -> DiscoveredPlugin {
    let fallback_id = file_name
        .strip_prefix(PLUGIN_EXECUTABLE_PREFIX)
        .unwrap_or("unknown")
        .to_string();
    let commands = manifest_entry
        .as_ref()
        .map(|entry| entry.commands.clone())
        .unwrap_or_default();

    DiscoveredPlugin {
        plugin_id: manifest_entry
            .as_ref()
            .map(|entry| entry.id.clone())
            .unwrap_or(fallback_id),
        plugin_version: manifest_entry.as_ref().map(|entry| entry.version.clone()),
        executable,
        source,
        command_specs: commands
            .iter()
            .map(|name| CommandSpec::new(name.clone()))
            .collect(),
        commands,
        issue: None,
        default_enabled: manifest_entry
            .as_ref()
            .map(|entry| entry.enabled_by_default)
            .unwrap_or(true),
    }
}

fn apply_manifest_discovery_issue(
    issue: &mut Option<String>,
    manifest_state: &ManifestState,
    manifest_entry: Option<&ManifestPlugin>,
) {
    if let Some(message) = manifest_discovery_issue(manifest_state, manifest_entry) {
        merge_issue(issue, message);
    }
}

fn manifest_discovery_issue(
    manifest_state: &ManifestState,
    manifest_entry: Option<&ManifestPlugin>,
) -> Option<String> {
    match manifest_state {
        ManifestState::Missing => Some(format!("bundled {} not found", BUNDLED_MANIFEST_FILE)),
        ManifestState::Invalid(err) => Some(format!("bundled manifest invalid: {err}")),
        ManifestState::Valid(_) if manifest_entry.is_none() => {
            Some("plugin executable not present in bundled manifest".to_string())
        }
        ManifestState::NotBundled | ManifestState::Valid(_) => None,
    }
}

fn apply_describe_metadata(
    plugin: &mut DiscoveredPlugin,
    describe: &DescribeV1,
    manifest_entry: Option<&ManifestPlugin>,
    executable: &Path,
) {
    if let Some(entry) = manifest_entry {
        plugin.default_enabled = entry.enabled_by_default;
        if let Err(err) = validate_manifest_entry(entry, describe, executable) {
            merge_issue(&mut plugin.issue, err.to_string());
            return;
        }
    }

    plugin.plugin_id = describe.plugin_id.clone();
    plugin.plugin_version = Some(describe.plugin_version.clone());
    plugin.commands = describe
        .commands
        .iter()
        .map(|cmd| cmd.name.clone())
        .collect::<Vec<String>>();
    plugin.command_specs = describe
        .commands
        .iter()
        .map(to_command_spec)
        .collect::<Vec<CommandSpec>>();

    if let Some(issue) = min_osp_version_issue(describe) {
        merge_issue(&mut plugin.issue, issue);
    }
}

fn min_osp_version_issue(describe: &DescribeV1) -> Option<String> {
    let min_required = describe
        .min_osp_version
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let current_raw = env!("CARGO_PKG_VERSION");
    let current = match Version::parse(current_raw) {
        Ok(version) => version,
        Err(err) => {
            return Some(format!(
                "osp version `{current_raw}` is invalid for plugin compatibility checks: {err}"
            ));
        }
    };
    let min = match Version::parse(min_required) {
        Ok(version) => version,
        Err(err) => {
            return Some(format!(
                "invalid min_osp_version `{min_required}` declared by plugin {}: {err}",
                describe.plugin_id
            ));
        }
    };

    if current < min {
        Some(format!(
            "plugin {} requires osp >= {min}, current version is {current}",
            describe.plugin_id
        ))
    } else {
        None
    }
}

fn load_and_validate_manifest(path: &Path) -> Result<ValidatedBundledManifest> {
    let manifest = read_bundled_manifest(path)?;
    validate_manifest_protocol(&manifest)?;
    Ok(ValidatedBundledManifest {
        by_exe: index_manifest_plugins(manifest.plugin)?,
    })
}

fn read_bundled_manifest(path: &Path) -> Result<BundledManifest> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read manifest {}", path.display()))?;
    toml::from_str::<BundledManifest>(&raw)
        .with_context(|| format!("failed to parse manifest TOML at {}", path.display()))
}

fn validate_manifest_protocol(manifest: &BundledManifest) -> Result<()> {
    if manifest.protocol_version != 1 {
        return Err(anyhow!(
            "unsupported manifest protocol_version {}",
            manifest.protocol_version
        ));
    }
    Ok(())
}

fn index_manifest_plugins(plugins: Vec<ManifestPlugin>) -> Result<HashMap<String, ManifestPlugin>> {
    let mut by_exe: HashMap<String, ManifestPlugin> = HashMap::new();
    let mut ids = HashSet::new();

    for plugin in plugins {
        validate_manifest_plugin(&plugin)?;
        insert_manifest_plugin(&mut by_exe, &mut ids, plugin)?;
    }

    Ok(by_exe)
}

fn validate_manifest_plugin(plugin: &ManifestPlugin) -> Result<()> {
    if plugin.id.trim().is_empty() {
        return Err(anyhow!("manifest plugin id must not be empty"));
    }
    if plugin.exe.trim().is_empty() {
        return Err(anyhow!("manifest plugin exe must not be empty"));
    }
    if plugin.version.trim().is_empty() {
        return Err(anyhow!("manifest plugin version must not be empty"));
    }
    if plugin.commands.is_empty() {
        return Err(anyhow!(
            "manifest plugin {} must declare at least one command",
            plugin.id
        ));
    }
    Ok(())
}

fn insert_manifest_plugin(
    by_exe: &mut HashMap<String, ManifestPlugin>,
    ids: &mut HashSet<String>,
    plugin: ManifestPlugin,
) -> Result<()> {
    if !ids.insert(plugin.id.clone()) {
        return Err(anyhow!("duplicate plugin id in manifest: {}", plugin.id));
    }
    if by_exe.contains_key(&plugin.exe) {
        return Err(anyhow!("duplicate plugin exe in manifest: {}", plugin.exe));
    }
    by_exe.insert(plugin.exe.clone(), plugin);
    Ok(())
}

fn validate_manifest_entry(
    entry: &ManifestPlugin,
    describe: &DescribeV1,
    path: &Path,
) -> Result<()> {
    if entry.id != describe.plugin_id {
        return Err(anyhow!(
            "manifest id mismatch: expected {}, got {}",
            entry.id,
            describe.plugin_id
        ));
    }

    if entry.version != describe.plugin_version {
        return Err(anyhow!(
            "manifest version mismatch for {}: expected {}, got {}",
            entry.id,
            entry.version,
            describe.plugin_version
        ));
    }

    let mut expected = entry.commands.clone();
    expected.sort();
    expected.dedup();

    let mut actual = describe
        .commands
        .iter()
        .map(|cmd| cmd.name.clone())
        .collect::<Vec<String>>();
    actual.sort();
    actual.dedup();

    if expected != actual {
        return Err(anyhow!(
            "manifest commands mismatch for {}: expected {:?}, got {:?}",
            entry.id,
            expected,
            actual
        ));
    }

    if let Some(expected_checksum) = entry.checksum_sha256.as_deref() {
        let expected_checksum = normalize_checksum(expected_checksum)?;
        let actual_checksum = file_sha256_hex(path)?;
        if expected_checksum != actual_checksum {
            return Err(anyhow!(
                "checksum mismatch for {}: expected {}, got {}",
                entry.id,
                expected_checksum,
                actual_checksum
            ));
        }
    }

    Ok(())
}

enum CommandRunError {
    Execute(std::io::Error),
    TimedOut { timeout: Duration, stderr: Vec<u8> },
}

fn describe_plugin(path: &Path, timeout: Duration) -> Result<DescribeV1> {
    let mut command = Command::new(path);
    command.arg("--describe");
    let started_at = Instant::now();
    tracing::debug!(
        executable = %path.display(),
        timeout_ms = timeout.as_millis(),
        "running plugin describe"
    );
    let output = run_command_with_timeout(command, timeout).map_err(|err| match err {
        CommandRunError::Execute(source) => {
            tracing::warn!(
                executable = %path.display(),
                error = %source,
                "plugin describe execution failed"
            );
            anyhow!(
                "failed to execute --describe for {}: {source}",
                path.display()
            )
        }
        CommandRunError::TimedOut { timeout, stderr } => {
            let stderr = String::from_utf8_lossy(&stderr).trim().to_string();
            tracing::warn!(
                executable = %path.display(),
                timeout_ms = timeout.as_millis(),
                stderr = %stderr,
                "plugin describe timed out"
            );
            if stderr.is_empty() {
                anyhow!(
                    "--describe timed out after {} ms for {}",
                    timeout.as_millis(),
                    path.display()
                )
            } else {
                anyhow!(
                    "--describe timed out after {} ms for {}: {}",
                    timeout.as_millis(),
                    path.display(),
                    stderr
                )
            }
        }
    })?;

    tracing::debug!(
        executable = %path.display(),
        elapsed_ms = started_at.elapsed().as_millis(),
        status = ?output.status.code(),
        "plugin describe completed"
    );

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let message = if stderr.is_empty() {
            format!("--describe failed with status {}", output.status)
        } else {
            format!(
                "--describe failed with status {}: {}",
                output.status, stderr
            )
        };
        return Err(anyhow!(message));
    }

    let describe: DescribeV1 = serde_json::from_slice(&output.stdout)
        .with_context(|| format!("invalid describe JSON from {}", path.display()))?;

    describe
        .validate_v1()
        .map_err(|err| anyhow!("invalid describe payload from {}: {err}", path.display()))?;

    Ok(describe)
}

fn run_provider(
    provider: &DiscoveredPlugin,
    selected_command: &str,
    args: &[String],
    context: &PluginDispatchContext,
    timeout: Duration,
) -> std::result::Result<RawPluginOutput, PluginDispatchError> {
    let mut command = Command::new(&provider.executable);
    let started_at = Instant::now();
    tracing::debug!(
        plugin_id = %provider.plugin_id,
        executable = %provider.executable.display(),
        command = %selected_command,
        arg_count = args.len(),
        timeout_ms = timeout.as_millis(),
        "dispatching plugin command"
    );
    // Pass the selected command in both env and argv so plugin authors can
    // treat plugin executables as ordinary CLIs without losing host context.
    command.arg(selected_command);
    command.args(args);
    command.env(ENV_OSP_COMMAND, selected_command);
    for (key, value) in context.runtime_hints.env_pairs() {
        command.env(key, value);
    }
    // Later env() calls win, so app-owned plugin config can intentionally
    // override shared defaults after runtime hints are injected.
    for (key, value) in context.env_pairs_for(&provider.plugin_id) {
        command.env(key, value);
    }

    let output = run_command_with_timeout(command, timeout).map_err(|err| match err {
        CommandRunError::Execute(source) => {
            tracing::warn!(
                plugin_id = %provider.plugin_id,
                executable = %provider.executable.display(),
                command = %selected_command,
                error = %source,
                "plugin command execution failed"
            );
            PluginDispatchError::ExecuteFailed {
                plugin_id: provider.plugin_id.clone(),
                source,
            }
        }
        CommandRunError::TimedOut { timeout, stderr } => {
            let stderr_text = String::from_utf8_lossy(&stderr).to_string();
            tracing::warn!(
                plugin_id = %provider.plugin_id,
                executable = %provider.executable.display(),
                command = %selected_command,
                timeout_ms = timeout.as_millis(),
                stderr = %stderr_text.trim(),
                "plugin command timed out"
            );
            PluginDispatchError::TimedOut {
                plugin_id: provider.plugin_id.clone(),
                timeout,
                stderr: stderr_text,
            }
        }
    })?;

    tracing::debug!(
        plugin_id = %provider.plugin_id,
        executable = %provider.executable.display(),
        command = %selected_command,
        elapsed_ms = started_at.elapsed().as_millis(),
        status = ?output.status.code(),
        "plugin command completed"
    );

    Ok(RawPluginOutput {
        status_code: output.status.code().unwrap_or(1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn describe_with_cache(
    path: &Path,
    cache: &mut DescribeCacheFile,
    seen_describe_paths: &mut HashSet<String>,
    cache_dirty: &mut bool,
    process_timeout: Duration,
) -> Result<DescribeV1> {
    let key = describe_cache_key(path);
    seen_describe_paths.insert(key.clone());
    let (size, mtime_secs, mtime_nanos) = file_fingerprint(path)?;

    if let Some(entry) = find_cached_describe(cache, &key, size, mtime_secs, mtime_nanos) {
        tracing::trace!(path = %path.display(), "describe cache hit");
        return Ok(entry.describe.clone());
    }

    tracing::trace!(path = %path.display(), "describe cache miss");

    let describe = describe_plugin(path, process_timeout)?;
    upsert_cached_describe(cache, key, size, mtime_secs, mtime_nanos, describe.clone());
    *cache_dirty = true;

    Ok(describe)
}

fn run_command_with_timeout(
    mut command: Command,
    timeout: Duration,
) -> Result<Output, CommandRunError> {
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn().map_err(CommandRunError::Execute)?;
    let deadline = Instant::now() + timeout.max(Duration::from_millis(1));

    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().map_err(CommandRunError::Execute),
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(PROCESS_WAIT_POLL_INTERVAL);
            }
            Ok(None) => {
                let _ = child.kill();
                let output = child.wait_with_output().map_err(CommandRunError::Execute)?;
                return Err(CommandRunError::TimedOut {
                    timeout,
                    stderr: output.stderr,
                });
            }
            Err(source) => return Err(CommandRunError::Execute(source)),
        }
    }
}

fn describe_cache_key(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn find_cached_describe<'a>(
    cache: &'a DescribeCacheFile,
    key: &str,
    size: u64,
    mtime_secs: u64,
    mtime_nanos: u32,
) -> Option<&'a DescribeCacheEntry> {
    cache.entries.iter().find(|entry| {
        entry.path == key
            && entry.size == size
            && entry.mtime_secs == mtime_secs
            && entry.mtime_nanos == mtime_nanos
    })
}

fn upsert_cached_describe(
    cache: &mut DescribeCacheFile,
    key: String,
    size: u64,
    mtime_secs: u64,
    mtime_nanos: u32,
    describe: DescribeV1,
) {
    if let Some(entry) = cache.entries.iter_mut().find(|entry| entry.path == key) {
        entry.size = size;
        entry.mtime_secs = mtime_secs;
        entry.mtime_nanos = mtime_nanos;
        entry.describe = describe;
    } else {
        cache.entries.push(DescribeCacheEntry {
            path: key,
            size,
            mtime_secs,
            mtime_nanos,
            describe,
        });
    }
}

fn prune_stale_describe_cache_entries(
    cache: &mut DescribeCacheFile,
    seen_paths: &HashSet<String>,
) -> bool {
    let before = cache.entries.len();
    cache
        .entries
        .retain(|entry| seen_paths.contains(&entry.path));
    cache.entries.len() != before
}

fn file_fingerprint(path: &Path) -> Result<(u64, u64, u32)> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("failed to read metadata for {}", path.display()))?;
    let size = metadata.len();
    let modified = metadata
        .modified()
        .with_context(|| format!("failed to read mtime for {}", path.display()))?;
    let dur = modified
        .duration_since(UNIX_EPOCH)
        .with_context(|| format!("mtime before unix epoch for {}", path.display()))?;
    Ok((size, dur.as_secs(), dur.subsec_nanos()))
}

fn is_plugin_executable(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if !name.starts_with(PLUGIN_EXECUTABLE_PREFIX) {
        return false;
    }
    if !has_supported_plugin_extension(path) {
        return false;
    }
    if !has_valid_plugin_suffix(name) {
        return false;
    }
    is_executable_file(path)
}

// moved into PluginManager to allow test overrides

fn bundled_plugin_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    if let Ok(path) = std::env::var("OSP_BUNDLED_PLUGIN_DIR") {
        dirs.push(PathBuf::from(path));
    }

    if let Ok(exe_path) = std::env::current_exe()
        && let Some(bin_dir) = exe_path.parent()
    {
        dirs.push(bin_dir.join("plugins"));
        dirs.push(bin_dir.join("../lib/osp/plugins"));
    }

    dirs
}

fn is_active_plugin(plugin: &DiscoveredPlugin, state: &PluginState) -> bool {
    plugin.issue.is_none() && is_enabled(state, &plugin.plugin_id, plugin.default_enabled)
}

fn active_plugins<'a>(
    discovered: &'a [DiscoveredPlugin],
    state: &'a PluginState,
) -> impl Iterator<Item = &'a DiscoveredPlugin> + 'a {
    discovered
        .iter()
        .filter(move |plugin| is_active_plugin(plugin, state))
}

fn plugin_label(plugin: &DiscoveredPlugin) -> String {
    format!("{} ({})", plugin.plugin_id, plugin.source)
}

fn plugin_provides_command(plugin: &DiscoveredPlugin, command: &str) -> bool {
    plugin.commands.iter().any(|name| name == command)
}

fn providers_for_command<'a>(
    command: &str,
    plugins: &[&'a DiscoveredPlugin],
) -> Vec<&'a DiscoveredPlugin> {
    plugins
        .iter()
        .copied()
        .filter(|plugin| plugin_provides_command(plugin, command))
        .collect()
}

fn resolve_provider_for_command<'a>(
    command: &str,
    plugins: &[&'a DiscoveredPlugin],
    state: &PluginState,
    provider_override: Option<&str>,
) -> std::result::Result<ProviderResolution<'a>, ProviderResolutionError<'a>> {
    let providers = providers_for_command(command, plugins);
    if providers.is_empty() {
        return Err(ProviderResolutionError::CommandNotFound);
    }

    if let Some(requested_provider) = provider_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Some(plugin) = providers
            .iter()
            .copied()
            .find(|plugin| plugin.plugin_id == requested_provider)
        {
            return Ok(ProviderResolution::Selected(ProviderSelection {
                plugin,
                mode: ProviderSelectionMode::Override,
            }));
        }
        return Err(ProviderResolutionError::RequestedProviderUnavailable {
            requested_provider: requested_provider.to_string(),
            providers,
        });
    }

    if let Some(preferred) = state.preferred_providers.get(command) {
        if let Some(plugin) = providers
            .iter()
            .copied()
            .find(|plugin| plugin.plugin_id == *preferred)
        {
            return Ok(ProviderResolution::Selected(ProviderSelection {
                plugin,
                mode: ProviderSelectionMode::Preference,
            }));
        }

        tracing::trace!(
            command = %command,
            preferred_provider = %preferred,
            available_providers = providers.len(),
            "preferred provider not available; reevaluating command provider"
        );
    }

    if providers.len() == 1 {
        return Ok(ProviderResolution::Selected(ProviderSelection {
            plugin: providers[0],
            mode: ProviderSelectionMode::Unique,
        }));
    }

    Ok(ProviderResolution::Ambiguous(providers))
}

fn provider_labels_by_command(plugins: &[&DiscoveredPlugin]) -> HashMap<String, Vec<String>> {
    let mut index = HashMap::new();
    for plugin in plugins {
        let label = plugin_label(plugin);
        for command in &plugin.commands {
            index
                .entry(command.clone())
                .or_insert_with(Vec::new)
                .push(label.clone());
        }
    }
    index
}

fn is_enabled(state: &PluginState, plugin_id: &str, default_enabled: bool) -> bool {
    // `enabled`/`disabled` are explicit per-plugin overrides. Plugins without
    // an override fall back to their discovery-time default.
    if state.enabled.iter().any(|id| id == plugin_id) {
        return true;
    }
    if state.disabled.iter().any(|id| id == plugin_id) {
        return false;
    }
    default_enabled
}

fn write_text_atomic(path: &Path, payload: &str) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .ok_or_else(|| anyhow!("path has no file name: {}", path.display()))?;
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut temp_name = std::ffi::OsString::from(".");
    temp_name.push(file_name);
    temp_name.push(format!(".tmp-{}-{suffix}", std::process::id()));
    let temp_path = parent.join(temp_name);
    std::fs::write(&temp_path, payload)?;
    if let Err(err) = std::fs::rename(&temp_path, path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(err.into());
    }
    Ok(())
}

fn merge_issue(target: &mut Option<String>, message: String) {
    if message.trim().is_empty() {
        return;
    }

    match target {
        Some(existing) => {
            existing.push_str("; ");
            existing.push_str(&message);
        }
        None => *target = Some(message),
    }
}

fn normalize_checksum(checksum: &str) -> Result<String> {
    let trimmed = checksum.trim().to_ascii_lowercase();
    if trimmed.len() != 64 || !trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(anyhow!(
            "checksum must be a 64-char lowercase/uppercase hex string"
        ));
    }
    Ok(trimmed)
}

fn file_sha256_hex(path: &Path) -> Result<String> {
    let file = std::fs::File::open(path).with_context(|| {
        format!(
            "failed to read plugin executable for checksum: {}",
            path.display()
        )
    })?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 16 * 1024];

    loop {
        let read = reader.read(&mut buffer).with_context(|| {
            format!(
                "failed to stream plugin executable for checksum: {}",
                path.display()
            )
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    let digest = hasher.finalize();

    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        let _ = write!(&mut out, "{b:02x}");
    }
    Ok(out)
}

fn default_true() -> bool {
    true
}

#[cfg(windows)]
fn has_supported_plugin_extension(path: &Path) -> bool {
    match path.extension().and_then(|ext| ext.to_str()) {
        None => true,
        Some(ext) => ext.eq_ignore_ascii_case("exe"),
    }
}

#[cfg(not(windows))]
fn has_supported_plugin_extension(path: &Path) -> bool {
    path.extension().is_none()
}

#[cfg(windows)]
fn has_valid_plugin_suffix(file_name: &str) -> bool {
    let base = file_name.strip_suffix(".exe").unwrap_or(file_name);
    let Some(suffix) = base.strip_prefix(PLUGIN_EXECUTABLE_PREFIX) else {
        return false;
    };
    !suffix.is_empty()
        && suffix
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(not(windows))]
fn has_valid_plugin_suffix(file_name: &str) -> bool {
    let Some(suffix) = file_name.strip_prefix(PLUGIN_EXECUTABLE_PREFIX) else {
        return false;
    };
    !suffix.is_empty()
        && suffix
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => meta.permissions().mode() & 0o111 != 0,
        _ => false,
    }
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::{
        DescribeV1, PluginDispatchContext, PluginDispatchError, PluginManager, PluginSource,
        PluginState, collect_completion_words, is_enabled, min_osp_version_issue,
    };
    use std::collections::BTreeMap;
    use std::error::Error as _;
    use std::time::Duration;

    #[test]
    fn explicit_enable_overrides_default_disabled() {
        let state = PluginState {
            enabled: vec!["hello".to_string()],
            disabled: Vec::new(),
            preferred_providers: BTreeMap::new(),
        };

        assert!(is_enabled(&state, "hello", false));
    }

    #[test]
    fn explicit_disable_overrides_default_enabled() {
        let state = PluginState {
            enabled: Vec::new(),
            disabled: vec!["hello".to_string()],
            preferred_providers: BTreeMap::new(),
        };

        assert!(!is_enabled(&state, "hello", true));
    }

    #[test]
    fn enabling_one_plugin_does_not_disable_other_default_enabled_plugins() {
        let state = PluginState {
            enabled: vec!["alpha".to_string()],
            disabled: Vec::new(),
            preferred_providers: BTreeMap::new(),
        };

        assert!(is_enabled(&state, "alpha", true));
        assert!(is_enabled(&state, "beta", true));
    }

    #[test]
    fn explicit_enable_wins_if_state_file_contains_conflicting_entries() {
        let state = PluginState {
            enabled: vec!["hello".to_string()],
            disabled: vec!["hello".to_string()],
            preferred_providers: BTreeMap::new(),
        };

        assert!(is_enabled(&state, "hello", false));
    }

    #[cfg(unix)]
    #[test]
    fn ambiguous_command_requires_explicit_selection() {
        let root = make_temp_dir("osp-cli-plugin-manager-ambiguous-command");
        let plugins_dir = root.join("plugins");
        std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");

        write_provider_test_plugin(&plugins_dir, "alpha", "shared");
        write_provider_test_plugin(&plugins_dir, "beta", "shared");
        let manager = PluginManager::new(vec![plugins_dir.clone()]);

        let catalog = manager.command_catalog().expect("catalog should load");
        let entry = catalog
            .iter()
            .find(|entry| entry.name == "shared")
            .expect("shared command should exist");
        assert_eq!(entry.provider, None);
        assert!(entry.requires_selection);
        assert!(!entry.selected_explicitly);
        assert_eq!(manager.selected_provider_label("shared"), None);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn preferred_provider_updates_catalog_and_resolves_command() {
        let root = make_temp_dir("osp-cli-plugin-manager-preferred-provider");
        let plugins_dir = root.join("plugins");
        let config_root = root.join("config");
        let cache_root = root.join("cache");
        std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");

        write_provider_test_plugin(&plugins_dir, "alpha", "shared");
        write_provider_test_plugin(&plugins_dir, "beta", "shared");
        let manager = PluginManager::new(vec![plugins_dir.clone()])
            .with_roots(Some(config_root.clone()), Some(cache_root.clone()));

        manager
            .set_preferred_provider("shared", "beta")
            .expect("preferred provider should be saved");

        let catalog = manager.command_catalog().expect("catalog should load");
        let entry = catalog
            .iter()
            .find(|entry| entry.name == "shared")
            .expect("shared command should exist");
        assert_eq!(entry.provider.as_deref(), Some("beta"));
        assert!(!entry.requires_selection);
        assert!(entry.selected_explicitly);
        assert_eq!(
            manager.selected_provider_label("shared").as_deref(),
            Some("beta (explicit)")
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn clearing_preferred_provider_requires_selection_again() {
        let root = make_temp_dir("osp-cli-plugin-manager-clear-preference");
        let plugins_dir = root.join("plugins");
        let config_root = root.join("config");
        let cache_root = root.join("cache");
        std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");

        write_provider_test_plugin(&plugins_dir, "alpha", "shared");
        write_provider_test_plugin(&plugins_dir, "beta", "shared");
        let manager = PluginManager::new(vec![plugins_dir.clone()])
            .with_roots(Some(config_root.clone()), Some(cache_root.clone()));

        manager
            .set_preferred_provider("shared", "beta")
            .expect("preferred provider should be saved");
        assert!(
            manager
                .clear_preferred_provider("shared")
                .expect("clearing preferred provider should succeed")
        );

        let catalog = manager.command_catalog().expect("catalog should load");
        let entry = catalog
            .iter()
            .find(|entry| entry.name == "shared")
            .expect("shared command should exist");
        assert_eq!(entry.provider, None);
        assert!(entry.requires_selection);
        assert!(!entry.selected_explicitly);
        assert_eq!(manager.selected_provider_label("shared"), None);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn compatible_min_osp_version_has_no_issue() {
        let describe = DescribeV1 {
            protocol_version: 1,
            plugin_id: "hello".to_string(),
            plugin_version: "0.1.0".to_string(),
            min_osp_version: Some("0.1.0".to_string()),
            commands: Vec::new(),
        };

        assert_eq!(min_osp_version_issue(&describe), None);
    }

    #[test]
    fn invalid_min_osp_version_reports_issue() {
        let describe = DescribeV1 {
            protocol_version: 1,
            plugin_id: "hello".to_string(),
            plugin_version: "0.1.0".to_string(),
            min_osp_version: Some("not-a-version".to_string()),
            commands: Vec::new(),
        };

        let issue = min_osp_version_issue(&describe).expect("invalid version should report issue");
        assert!(issue.contains("invalid min_osp_version"));
        assert!(issue.contains("hello"));
    }

    #[cfg(unix)]
    #[test]
    fn refresh_picks_up_filesystem_changes_and_prunes_stale_cache() {
        let root = make_temp_dir("osp-cli-plugin-manager-refresh");
        let plugins_dir = root.join("plugins");
        let config_root = root.join("config");
        let cache_root = root.join("cache");
        std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");

        let alpha_path = write_named_test_plugin(&plugins_dir, "alpha");
        let manager = PluginManager::new(vec![plugins_dir.clone()])
            .with_roots(Some(config_root.clone()), Some(cache_root.clone()));

        let first = manager.list_plugins().expect("plugins should list");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].plugin_id, "alpha");

        std::fs::remove_file(&alpha_path).expect("alpha plugin should be removable");
        write_named_test_plugin(&plugins_dir, "beta");

        let cached = manager.list_plugins().expect("cached plugins should list");
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].plugin_id, "alpha");

        manager.refresh();
        let refreshed = manager
            .list_plugins()
            .expect("refreshed plugins should list");
        assert_eq!(refreshed.len(), 1);
        assert_eq!(refreshed[0].plugin_id, "beta");

        let cache_path = cache_root.join("describe-v1.json");
        let cache_raw =
            std::fs::read_to_string(&cache_path).expect("describe cache should be written");
        assert!(cache_raw.contains("osp-beta"));
        assert!(!cache_raw.contains("osp-alpha"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn incompatible_min_osp_version_marks_plugin_unhealthy() {
        let root = make_temp_dir("osp-cli-plugin-manager-min-version");
        let plugins_dir = root.join("plugins");
        std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");

        write_named_test_plugin_with_min_version(&plugins_dir, "future", "9.9.9");
        let manager = PluginManager::new(vec![plugins_dir.clone()]);

        let plugins = manager.list_plugins().expect("plugins should list");
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].plugin_id, "future");
        assert!(!plugins[0].healthy);
        assert!(
            plugins[0]
                .issue
                .as_deref()
                .expect("issue should be present")
                .contains("requires osp >=")
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn dispatch_times_out_hung_plugin() {
        let root = make_temp_dir("osp-cli-plugin-manager-dispatch-timeout");
        let plugins_dir = root.join("plugins");
        std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");

        write_sleepy_test_plugin(&plugins_dir, "hang", false);
        let manager = PluginManager::new(vec![plugins_dir.clone()])
            .with_process_timeout(Duration::from_millis(50));

        let err = manager
            .dispatch("hang", &[], &PluginDispatchContext::default())
            .expect_err("hung plugin should time out");

        match err {
            PluginDispatchError::TimedOut {
                plugin_id, timeout, ..
            } => {
                assert_eq!(plugin_id, "hang");
                assert_eq!(timeout, Duration::from_millis(50));
            }
            other => panic!("expected timeout error, got {other}"),
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn hung_describe_marks_plugin_unhealthy() {
        let root = make_temp_dir("osp-cli-plugin-manager-describe-timeout");
        let plugins_dir = root.join("plugins");
        std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");

        write_sleepy_test_plugin(&plugins_dir, "hang-describe", true);
        let manager = PluginManager::new(vec![plugins_dir.clone()])
            .with_process_timeout(Duration::from_millis(50));

        let plugins = manager.list_plugins().expect("plugins should list");
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].plugin_id, "hang-describe");
        assert!(!plugins[0].healthy);
        assert!(
            plugins[0]
                .issue
                .as_deref()
                .expect("issue should be present")
                .contains("timed out after 50 ms")
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn preferred_provider_rejects_unknown_command_and_provider_unit() {
        let root = make_temp_dir("osp-cli-plugin-manager-invalid-provider");
        let plugins_dir = root.join("plugins");
        let config_root = root.join("config");
        let cache_root = root.join("cache");
        std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");

        write_provider_test_plugin(&plugins_dir, "alpha", "shared");
        let manager = PluginManager::new(vec![plugins_dir.clone()])
            .with_roots(Some(config_root), Some(cache_root));

        let err = manager
            .set_preferred_provider("missing", "alpha")
            .expect_err("unknown command should fail");
        assert!(
            err.to_string()
                .contains("no active plugin provides command")
        );

        let err = manager
            .set_preferred_provider("shared", "beta")
            .expect_err("unknown provider should fail");
        assert!(err.to_string().contains("does not provide active command"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn clear_preferred_provider_rejects_empty_command_unit() {
        let manager = PluginManager::new(Vec::new());
        let err = manager
            .clear_preferred_provider("   ")
            .expect_err("empty command should fail");
        assert!(err.to_string().contains("command must not be empty"));
    }

    #[test]
    fn plugin_dispatch_context_merges_shared_and_plugin_env_pairs_unit() {
        let context = PluginDispatchContext {
            shared_env: vec![("OSP_FORMAT".to_string(), "json".to_string())],
            plugin_env: std::collections::HashMap::from([(
                "alpha".to_string(),
                vec![("OSP_PLUGIN_FLAG".to_string(), "1".to_string())],
            )]),
            ..PluginDispatchContext::default()
        };

        let pairs = context.env_pairs_for("alpha").collect::<Vec<_>>();
        assert_eq!(
            pairs,
            vec![("OSP_FORMAT", "json"), ("OSP_PLUGIN_FLAG", "1")]
        );
        assert_eq!(
            context.env_pairs_for("missing").collect::<Vec<_>>(),
            vec![("OSP_FORMAT", "json")]
        );
    }

    #[test]
    fn plugin_dispatch_error_formats_cover_terminal_variants_unit() {
        let timeout_plain = PluginDispatchError::TimedOut {
            plugin_id: "alpha".to_string(),
            timeout: Duration::from_millis(25),
            stderr: String::new(),
        };
        assert!(
            timeout_plain
                .to_string()
                .contains("plugin alpha timed out after 25 ms")
        );

        let timeout_stderr = PluginDispatchError::TimedOut {
            plugin_id: "alpha".to_string(),
            timeout: Duration::from_millis(25),
            stderr: "stuck".to_string(),
        };
        assert!(timeout_stderr.to_string().contains("stuck"));

        let nonzero_plain = PluginDispatchError::NonZeroExit {
            plugin_id: "beta".to_string(),
            status_code: 9,
            stderr: String::new(),
        };
        assert_eq!(
            nonzero_plain.to_string(),
            "plugin beta exited with status 9"
        );

        let nonzero_stderr = PluginDispatchError::NonZeroExit {
            plugin_id: "beta".to_string(),
            status_code: 9,
            stderr: "boom".to_string(),
        };
        assert!(nonzero_stderr.to_string().contains("boom"));

        let ambiguous = PluginDispatchError::CommandAmbiguous {
            command: "shared".to_string(),
            providers: vec!["alpha".to_string(), "beta".to_string()],
        };
        assert!(ambiguous.to_string().contains("multiple plugins"));

        let provider_missing = PluginDispatchError::ProviderNotFound {
            command: "shared".to_string(),
            requested_provider: "gamma".to_string(),
            providers: vec!["alpha".to_string(), "beta".to_string()],
        };
        assert!(provider_missing.to_string().contains("available providers"));

        let execute_failed = PluginDispatchError::ExecuteFailed {
            plugin_id: "alpha".to_string(),
            source: std::io::Error::other("spawn failed"),
        };
        assert_eq!(
            execute_failed.source().map(|err| err.to_string()),
            Some("spawn failed".to_string())
        );

        let invalid_json = PluginDispatchError::InvalidJsonResponse {
            plugin_id: "alpha".to_string(),
            source: serde_json::from_str::<serde_json::Value>("not-json").expect_err("invalid"),
        };
        assert!(invalid_json.to_string().contains("invalid JSON response"));
        assert!(invalid_json.source().is_some());

        let invalid_payload = PluginDispatchError::InvalidResponsePayload {
            plugin_id: "alpha".to_string(),
            reason: "missing data".to_string(),
        };
        assert!(invalid_payload.to_string().contains("missing data"));
        assert!(invalid_payload.source().is_none());
    }

    #[test]
    fn completion_words_collect_flags_and_backbone_commands_unit() {
        let spec = osp_completion::CommandSpec::new("ldap")
            .flag("--json", osp_completion::FlagNode::new())
            .subcommand(
                osp_completion::CommandSpec::new("user")
                    .subcommand(osp_completion::CommandSpec::new("show")),
            );

        let words = collect_completion_words(&spec);
        assert!(words.contains(&"ldap".to_string()));
        assert!(words.contains(&"--json".to_string()));
        assert!(words.contains(&"user".to_string()));
        assert!(words.contains(&"show".to_string()));

        let manager = PluginManager::new(Vec::new());
        assert_eq!(
            manager
                .completion_words()
                .expect("backbone completion words should render"),
            vec![
                "F".to_string(),
                "P".to_string(),
                "V".to_string(),
                "exit".to_string(),
                "help".to_string(),
                "quit".to_string(),
                "|".to_string(),
            ]
        );
        assert!(
            manager
                .repl_help_text()
                .expect("empty help should render")
                .contains("No plugin commands available.")
        );
    }

    #[cfg(unix)]
    #[test]
    fn repl_help_and_provider_listing_cover_selected_and_conflicted_commands_unit() {
        let root = make_temp_dir("osp-cli-plugin-manager-help");
        let plugins_dir = root.join("plugins");
        let config_root = root.join("config");
        let cache_root = root.join("cache");
        std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");

        write_provider_test_plugin(&plugins_dir, "alpha", "shared");
        write_provider_test_plugin(&plugins_dir, "beta", "shared");
        write_named_test_plugin(&plugins_dir, "solo");

        let manager = PluginManager::new(vec![plugins_dir.clone()])
            .with_roots(Some(config_root.clone()), Some(cache_root.clone()));

        let ambiguous_help = manager.repl_help_text().expect("help should render");
        assert!(ambiguous_help.contains("shared - provider selection required"));
        assert!(ambiguous_help.contains("solo - solo plugin"));
        assert_eq!(
            manager.command_providers("shared"),
            vec![
                format!("alpha ({})", PluginSource::Explicit),
                format!("beta ({})", PluginSource::Explicit)
            ]
        );

        manager
            .set_preferred_provider("shared", "beta")
            .expect("preferred provider should save");
        let preferred_help = manager
            .repl_help_text()
            .expect("preferred provider help should render");
        assert!(preferred_help.contains("shared - beta plugin (beta/explicit)"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    fn make_temp_dir(prefix: &str) -> std::path::PathBuf {
        let mut dir = std::env::temp_dir();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be valid")
            .as_nanos();
        dir.push(format!("{prefix}-{nonce}"));
        std::fs::create_dir_all(&dir).expect("temp dir should be created");
        dir
    }

    #[cfg(unix)]
    fn write_named_test_plugin(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
        write_named_test_plugin_with_min_version(dir, name, "0.1.0")
    }

    #[cfg(unix)]
    fn write_named_test_plugin_with_min_version(
        dir: &std::path::Path,
        name: &str,
        min_osp_version: &str,
    ) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let plugin_path = dir.join(format!("osp-{name}"));
        let script = format!(
            r#"#!/usr/bin/env bash
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{name}","plugin_version":"0.1.0","min_osp_version":"{min_osp_version}","commands":[{{"name":"{name}","about":"{name} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"ok"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
            name = name,
            min_osp_version = min_osp_version
        );

        std::fs::write(&plugin_path, script).expect("plugin should be written");
        let mut perms = std::fs::metadata(&plugin_path)
            .expect("metadata should be readable")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&plugin_path, perms).expect("plugin should be executable");
        plugin_path
    }

    #[cfg(unix)]
    fn write_provider_test_plugin(
        dir: &std::path::Path,
        plugin_id: &str,
        command_name: &str,
    ) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let plugin_path = dir.join(format!("osp-{plugin_id}"));
        let script = format!(
            r#"#!/usr/bin/env bash
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{plugin_id}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{command_name}","about":"{plugin_id} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"ok"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
            plugin_id = plugin_id,
            command_name = command_name
        );

        std::fs::write(&plugin_path, script).expect("plugin should be written");
        let mut perms = std::fs::metadata(&plugin_path)
            .expect("metadata should be readable")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&plugin_path, perms).expect("plugin should be executable");
        plugin_path
    }

    #[cfg(unix)]
    fn write_sleepy_test_plugin(
        dir: &std::path::Path,
        name: &str,
        sleep_on_describe: bool,
    ) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let plugin_path = dir.join(format!("osp-{name}"));
        let script = format!(
            r#"#!/usr/bin/env bash
if [ "$1" = "--describe" ]; then
  if [ "{sleep_on_describe}" = "true" ]; then
    sleep 1
  fi
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{name}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{name}","about":"{name} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

sleep 1
cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"ok"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
            name = name,
            sleep_on_describe = if sleep_on_describe { "true" } else { "false" }
        );

        std::fs::write(&plugin_path, script).expect("plugin should be written");
        let mut perms = std::fs::metadata(&plugin_path)
            .expect("metadata should be readable")
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&plugin_path, perms).expect("plugin should be executable");
        plugin_path
    }
}
