//! Runtime-facing config defaults, path discovery, and loader-pipeline
//! assembly.
//!
//! This module exists to bridge the full layered config system into the smaller
//! runtime surfaces the app actually needs at startup.
//!
//! High-level flow:
//!
//! - define stable default values and path-discovery rules
//! - discover runtime config file locations from the current environment
//! - assemble the standard loader pipeline used by the host
//! - lower resolved config into the compact [`RuntimeConfig`] view used by
//!   callers that do not need the full explanation surface
//!
//! Contract:
//!
//! - this module may depend on config loaders and resolved config types
//! - it should not reimplement precedence rules already owned by the resolver
//! - callers should use this module for runtime bootstrap wiring instead of
//!   inventing their own config path and default logic
//!
//! Public API shape:
//!
//! - small bootstrap toggles like [`RuntimeLoadOptions`] use direct
//!   constructor/`with_*` methods
//! - discovered path/default snapshots stay plain data
//! - loader-pipeline assembly stays centralized here so callers do not invent
//!   incompatible bootstrap rules

use std::{collections::BTreeMap, path::PathBuf};

use directories::{BaseDirs, ProjectDirs};

use crate::config::{
    ChainedLoader, ConfigLayer, EnvSecretsLoader, EnvVarLoader, LoaderPipeline, ResolvedConfig,
    SecretsTomlLoader, StaticLayerLoader, TomlFileLoader,
};

/// Default logical profile name used when no profile override is active.
pub const DEFAULT_PROFILE_NAME: &str = "default";
/// Default maximum number of REPL history entries to keep.
pub const DEFAULT_REPL_HISTORY_MAX_ENTRIES: i64 = 1000;
/// Default toggle for persistent REPL history.
pub const DEFAULT_REPL_HISTORY_ENABLED: bool = true;
/// Default toggle for deduplicating REPL history entries.
pub const DEFAULT_REPL_HISTORY_DEDUPE: bool = true;
/// Default toggle for profile-scoped REPL history storage.
pub const DEFAULT_REPL_HISTORY_PROFILE_SCOPED: bool = true;
/// Default maximum number of rows shown in the REPL history search menu.
pub const DEFAULT_REPL_HISTORY_MENU_ROWS: i64 = 5;
/// Default upper bound for cached session results.
pub const DEFAULT_SESSION_CACHE_MAX_RESULTS: i64 = 64;
/// Default debug verbosity level.
pub const DEFAULT_DEBUG_LEVEL: i64 = 0;
/// Default toggle for file logging.
pub const DEFAULT_LOG_FILE_ENABLED: bool = false;
/// Default log level used for file logging.
pub const DEFAULT_LOG_FILE_LEVEL: &str = "warn";
/// Default render width hint.
pub const DEFAULT_UI_WIDTH: i64 = 72;
/// Default left margin for rendered output.
pub const DEFAULT_UI_MARGIN: i64 = 0;
/// Default indentation width for nested output.
pub const DEFAULT_UI_INDENT: i64 = 2;
/// Default presentation preset name.
pub const DEFAULT_UI_PRESENTATION: &str = "expressive";
/// Default semantic guide-format preference.
pub const DEFAULT_UI_GUIDE_DEFAULT_FORMAT: &str = "guide";
/// Default grouped-message layout mode.
pub const DEFAULT_UI_MESSAGES_LAYOUT: &str = "grouped";
/// Default section chrome frame style.
pub const DEFAULT_UI_CHROME_FRAME: &str = "top";
/// Default rule-sharing policy for sibling section chrome.
pub const DEFAULT_UI_CHROME_RULE_POLICY: &str = "shared";
/// Default table border style.
pub const DEFAULT_UI_TABLE_BORDER: &str = "square";
/// Default REPL intro mode.
pub const DEFAULT_REPL_INTRO: &str = "full";
/// Default threshold for rendering short lists compactly.
pub const DEFAULT_UI_SHORT_LIST_MAX: i64 = 1;
/// Default threshold for rendering medium lists before expanding further.
pub const DEFAULT_UI_MEDIUM_LIST_MAX: i64 = 5;
/// Default grid column padding.
pub const DEFAULT_UI_GRID_PADDING: i64 = 4;
/// Default adaptive grid column weight.
pub const DEFAULT_UI_COLUMN_WEIGHT: i64 = 3;
/// Default minimum width before MREG output stacks columns.
pub const DEFAULT_UI_MREG_STACK_MIN_COL_WIDTH: i64 = 10;
/// Default threshold for stacked MREG overflow behavior.
pub const DEFAULT_UI_MREG_STACK_OVERFLOW_RATIO: i64 = 200;
/// Default table overflow strategy.
pub const DEFAULT_UI_TABLE_OVERFLOW: &str = "clip";

const PROJECT_APPLICATION_NAME: &str = "osp";

/// Options that control which runtime config sources are included.
///
/// # Examples
///
/// ```
/// use osp_cli::config::{RuntimeBootstrapMode, RuntimeLoadOptions};
///
/// let options = RuntimeLoadOptions::default();
///
/// assert!(options.include_env);
/// assert!(options.include_config_file);
/// assert_eq!(options.bootstrap_mode, RuntimeBootstrapMode::Standard);
/// ```
///
/// When callers need a sealed bootstrap path with no environment variables,
/// file loading, or home/XDG-derived discovery, use
/// [`RuntimeLoadOptions::defaults_only`].
///
/// ```
/// use osp_cli::config::{RuntimeBootstrapMode, RuntimeLoadOptions};
///
/// let options = RuntimeLoadOptions::defaults_only();
///
/// assert!(!options.include_env);
/// assert!(!options.include_config_file);
/// assert_eq!(options.bootstrap_mode, RuntimeBootstrapMode::DefaultsOnly);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeBootstrapMode {
    /// Use the crate's normal environment and platform-derived bootstrap
    /// behavior.
    Standard,
    /// Use only built-in defaults plus explicit in-memory inputs.
    ///
    /// This disables environment-derived defaults, HOME/XDG path discovery,
    /// config-file and secrets-file lookup, and env/path override discovery.
    DefaultsOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
#[must_use = "RuntimeLoadOptions builder-style methods return an updated value"]
pub struct RuntimeLoadOptions {
    /// Whether environment-derived layers should be loaded.
    pub include_env: bool,
    /// Whether the ordinary config file should be loaded.
    ///
    /// This does not disable the secrets layer; secrets files and secret
    /// environment overrides still participate through the secrets pipeline.
    pub include_config_file: bool,
    /// Controls whether bootstrap may consult ambient environment and
    /// platform-derived paths before the loader pipeline runs.
    pub bootstrap_mode: RuntimeBootstrapMode,
}

impl Default for RuntimeLoadOptions {
    fn default() -> Self {
        Self {
            include_env: true,
            include_config_file: true,
            bootstrap_mode: RuntimeBootstrapMode::Standard,
        }
    }
}

impl RuntimeLoadOptions {
    /// Creates runtime-load options with the default source set enabled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a sealed bootstrap policy that uses only built-in defaults plus
    /// explicit in-memory layers.
    pub fn defaults_only() -> Self {
        Self {
            include_env: false,
            include_config_file: false,
            bootstrap_mode: RuntimeBootstrapMode::DefaultsOnly,
        }
    }

    /// Sets whether environment-derived layers should be loaded.
    ///
    /// The default is `true`.
    pub fn with_env(mut self, include_env: bool) -> Self {
        self.include_env = include_env;
        if include_env {
            self.bootstrap_mode = RuntimeBootstrapMode::Standard;
        }
        self
    }

    /// Sets whether the ordinary config file should be loaded.
    ///
    /// The default is `true`. This does not disable the secrets layer.
    pub fn with_config_file(mut self, include_config_file: bool) -> Self {
        self.include_config_file = include_config_file;
        if include_config_file {
            self.bootstrap_mode = RuntimeBootstrapMode::Standard;
        }
        self
    }

    /// Sets whether bootstrap may consult ambient environment and
    /// platform-derived paths before the loader pipeline runs.
    ///
    /// Switching to [`RuntimeBootstrapMode::DefaultsOnly`] also disables the
    /// env and config-file loader layers.
    pub fn with_bootstrap_mode(mut self, bootstrap_mode: RuntimeBootstrapMode) -> Self {
        self.bootstrap_mode = bootstrap_mode;
        if matches!(bootstrap_mode, RuntimeBootstrapMode::DefaultsOnly) {
            self.include_env = false;
            self.include_config_file = false;
        }
        self
    }

    /// Returns whether the load options seal bootstrap against ambient process
    /// and home-directory state.
    pub fn is_defaults_only(self) -> bool {
        matches!(self.bootstrap_mode, RuntimeBootstrapMode::DefaultsOnly)
    }
}

impl RuntimeBootstrapMode {
    fn capture_env(self) -> RuntimeEnvironment {
        match self {
            Self::Standard => RuntimeEnvironment::capture(),
            Self::DefaultsOnly => RuntimeEnvironment::defaults_only(),
        }
    }
}

impl RuntimeLoadOptions {
    fn runtime_environment(self) -> RuntimeEnvironment {
        self.bootstrap_mode.capture_env()
    }
}

/// Minimal runtime-derived config that callers often need directly.
///
/// This is intentionally much smaller than [`ResolvedConfig`]. Keep the full
/// [`ResolvedConfig`] when a caller needs arbitrary resolved keys, provenance,
/// terminal selection, or explanation data. Use [`RuntimeConfig`] when the
/// caller only needs the tiny runtime snapshot the host commonly carries
/// around directly.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// Active profile name selected for the current invocation.
    pub active_profile: String,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            active_profile: DEFAULT_PROFILE_NAME.to_string(),
        }
    }
}

impl RuntimeConfig {
    /// Extracts the small runtime snapshot most callers need from a resolved config.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::config::{ConfigLayer, ConfigResolver, ResolveOptions, RuntimeConfig};
    ///
    /// let mut defaults = ConfigLayer::default();
    /// defaults.set("profile.default", "default");
    ///
    /// let mut resolver = ConfigResolver::default();
    /// resolver.set_defaults(defaults);
    /// let resolved = resolver.resolve(ResolveOptions::default()).unwrap();
    ///
    /// let runtime = RuntimeConfig::from_resolved(&resolved);
    /// assert_eq!(runtime.active_profile, "default");
    /// ```
    pub fn from_resolved(resolved: &ResolvedConfig) -> Self {
        Self {
            active_profile: resolved.active_profile().to_string(),
        }
    }
}

/// Discovered filesystem paths for runtime config inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfigPaths {
    /// Path to the ordinary config file, when discovered.
    pub config_file: Option<PathBuf>,
    /// Path to the secrets config file, when discovered.
    pub secrets_file: Option<PathBuf>,
}

impl RuntimeConfigPaths {
    /// Discovers config and secrets paths from the current process environment.
    ///
    /// This is the standard path-discovery entrypoint for host bootstrap. Use
    /// it together with [`RuntimeDefaults`] and [`build_runtime_pipeline`] when
    /// a wrapper crate wants the same platform/env behavior as the stock host.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use osp_cli::config::RuntimeConfigPaths;
    ///
    /// let paths = RuntimeConfigPaths::discover();
    ///
    /// let _config = paths.config_file.as_deref();
    /// let _secrets = paths.secrets_file.as_deref();
    /// ```
    pub fn discover() -> Self {
        Self::discover_with(RuntimeLoadOptions::default())
    }

    /// Discovers config and secrets paths using the supplied runtime bootstrap
    /// policy.
    ///
    /// [`RuntimeLoadOptions::defaults_only`] returns an empty path set here so
    /// callers can build a fully sealed bootstrap path.
    pub fn discover_with(load: RuntimeLoadOptions) -> Self {
        let paths = Self::from_env(&load.runtime_environment());
        tracing::debug!(
            config_file = ?paths.config_file.as_ref().map(|path| path.display().to_string()),
            secrets_file = ?paths.secrets_file.as_ref().map(|path| path.display().to_string()),
            bootstrap_mode = ?load.bootstrap_mode,
            "discovered runtime config paths"
        );
        paths
    }

    fn from_env(env: &RuntimeEnvironment) -> Self {
        Self {
            config_file: env
                .path_override("OSP_CONFIG_FILE")
                .or_else(|| env.config_path("config.toml")),
            secrets_file: env
                .path_override("OSP_SECRETS_FILE")
                .or_else(|| env.config_path("secrets.toml")),
        }
    }
}

/// Built-in default values seeded before user-provided config is loaded.
#[derive(Debug, Clone, Default)]
pub struct RuntimeDefaults {
    layer: ConfigLayer,
}

impl RuntimeDefaults {
    /// Builds the default layer using the current process environment.
    ///
    /// `default_theme_name` and `default_repl_prompt` are the product-level
    /// knobs wrapper crates typically own themselves, while the rest of the
    /// default layer follows the crate's standard runtime bootstrap rules.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::config::RuntimeDefaults;
    ///
    /// let defaults = RuntimeDefaults::from_process_env("dracula", "osp> ");
    ///
    /// assert_eq!(defaults.get_string("theme.name"), Some("dracula"));
    /// assert_eq!(defaults.get_string("repl.prompt"), Some("osp> "));
    /// ```
    pub fn from_process_env(default_theme_name: &str, default_repl_prompt: &str) -> Self {
        Self::from_runtime_load(
            RuntimeLoadOptions::default(),
            default_theme_name,
            default_repl_prompt,
        )
    }

    /// Builds the default layer using the supplied runtime bootstrap policy.
    ///
    /// [`RuntimeLoadOptions::defaults_only`] suppresses environment-derived
    /// identity, theme-path, history-path, and log-path discovery so the
    /// resulting layer depends only on built-in values plus the provided
    /// product defaults.
    pub fn from_runtime_load(
        load: RuntimeLoadOptions,
        default_theme_name: &str,
        default_repl_prompt: &str,
    ) -> Self {
        Self::from_env(
            &load.runtime_environment(),
            default_theme_name,
            default_repl_prompt,
        )
    }

    fn from_env(
        env: &RuntimeEnvironment,
        default_theme_name: &str,
        default_repl_prompt: &str,
    ) -> Self {
        let mut layer = ConfigLayer::default();

        macro_rules! set_defaults {
            ($($key:literal => $value:expr),* $(,)?) => {
                $(layer.set($key, $value);)*
            };
        }

        set_defaults! {
            "profile.default" => DEFAULT_PROFILE_NAME.to_string(),
            "theme.name" => default_theme_name.to_string(),
            "user.name" => env.user_name(),
            "domain" => env.domain_name(),
            "repl.prompt" => default_repl_prompt.to_string(),
            "repl.input_mode" => "auto".to_string(),
            "repl.simple_prompt" => false,
            "repl.shell_indicator" => "[{shell}]".to_string(),
            "repl.intro" => DEFAULT_REPL_INTRO.to_string(),
            "repl.history.path" => env.repl_history_path(),
            "repl.history.max_entries" => DEFAULT_REPL_HISTORY_MAX_ENTRIES,
            "repl.history.enabled" => DEFAULT_REPL_HISTORY_ENABLED,
            "repl.history.dedupe" => DEFAULT_REPL_HISTORY_DEDUPE,
            "repl.history.profile_scoped" => DEFAULT_REPL_HISTORY_PROFILE_SCOPED,
            "repl.history.menu_rows" => DEFAULT_REPL_HISTORY_MENU_ROWS,
            "session.cache.max_results" => DEFAULT_SESSION_CACHE_MAX_RESULTS,
            "debug.level" => DEFAULT_DEBUG_LEVEL,
            "log.file.enabled" => DEFAULT_LOG_FILE_ENABLED,
            "log.file.path" => env.log_file_path(),
            "log.file.level" => DEFAULT_LOG_FILE_LEVEL.to_string(),
            "ui.width" => DEFAULT_UI_WIDTH,
            "ui.margin" => DEFAULT_UI_MARGIN,
            "ui.indent" => DEFAULT_UI_INDENT,
            "ui.presentation" => DEFAULT_UI_PRESENTATION.to_string(),
            "ui.help.level" => "inherit".to_string(),
            "ui.guide.default_format" => DEFAULT_UI_GUIDE_DEFAULT_FORMAT.to_string(),
            "ui.messages.layout" => DEFAULT_UI_MESSAGES_LAYOUT.to_string(),
            "ui.message.verbosity" => "success".to_string(),
            "ui.chrome.frame" => DEFAULT_UI_CHROME_FRAME.to_string(),
            "ui.chrome.rule_policy" => DEFAULT_UI_CHROME_RULE_POLICY.to_string(),
            "ui.table.overflow" => DEFAULT_UI_TABLE_OVERFLOW.to_string(),
            "ui.table.border" => DEFAULT_UI_TABLE_BORDER.to_string(),
            "ui.help.table_chrome" => "none".to_string(),
            "ui.help.entry_indent" => "inherit".to_string(),
            "ui.help.entry_gap" => "inherit".to_string(),
            "ui.help.section_spacing" => "inherit".to_string(),
            "ui.short_list_max" => DEFAULT_UI_SHORT_LIST_MAX,
            "ui.medium_list_max" => DEFAULT_UI_MEDIUM_LIST_MAX,
            "ui.grid_padding" => DEFAULT_UI_GRID_PADDING,
            "ui.column_weight" => DEFAULT_UI_COLUMN_WEIGHT,
            "ui.mreg.stack_min_col_width" => DEFAULT_UI_MREG_STACK_MIN_COL_WIDTH,
            "ui.mreg.stack_overflow_ratio" => DEFAULT_UI_MREG_STACK_OVERFLOW_RATIO,
            "extensions.plugins.timeout_ms" => 10_000,
            "extensions.plugins.discovery.path" => false,
        }

        let theme_path = env.theme_paths();
        if !theme_path.is_empty() {
            layer.set("theme.path", theme_path);
        }

        for key in [
            "color.text",
            "color.text.muted",
            "color.key",
            "color.border",
            "color.prompt.text",
            "color.prompt.command",
            "color.table.header",
            "color.mreg.key",
            "color.value",
            "color.value.number",
            "color.value.bool_true",
            "color.value.bool_false",
            "color.value.null",
            "color.value.ipv4",
            "color.value.ipv6",
            "color.panel.border",
            "color.panel.title",
            "color.code",
            "color.json.key",
        ] {
            layer.set(key, String::new());
        }

        Self { layer }
    }

    /// Returns a default string value by key from the global scope.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::config::RuntimeDefaults;
    ///
    /// let defaults = RuntimeDefaults::from_process_env("dracula", "> ");
    ///
    /// assert_eq!(defaults.get_string("theme.name"), Some("dracula"));
    /// assert_eq!(defaults.get_string("repl.prompt"), Some("> "));
    /// ```
    pub fn get_string(&self, key: &str) -> Option<&str> {
        self.layer
            .entries()
            .iter()
            .find(|entry| entry.key == key && entry.scope == crate::config::Scope::global())
            .and_then(|entry| match entry.value.reveal() {
                crate::config::ConfigValue::String(value) => Some(value.as_str()),
                _ => None,
            })
    }

    /// Clones the defaults as a standalone config layer.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::config::RuntimeDefaults;
    ///
    /// let defaults = RuntimeDefaults::from_process_env("plain", "> ");
    /// let layer = defaults.to_layer();
    ///
    /// assert!(layer.entries().iter().any(|entry| entry.key == "theme.name"));
    /// ```
    pub fn to_layer(&self) -> ConfigLayer {
        self.layer.clone()
    }
}

/// Assembles the runtime loader precedence stack for CLI startup.
///
/// The ordering encoded here is part of the config contract: defaults first,
/// then optional presentation/env/file/secrets layers, then CLI/session
/// overrides last.
///
/// This is the normal bootstrap path for hosts that want the crate's standard
/// platform/env/file loading semantics without manually wiring each loader.
///
/// # Examples
///
/// ```no_run
/// use osp_cli::config::{
///     ResolveOptions, RuntimeConfigPaths, RuntimeDefaults, RuntimeLoadOptions,
///     build_runtime_pipeline,
/// };
///
/// let defaults = RuntimeDefaults::from_process_env("dracula", "osp> ").to_layer();
/// let paths = RuntimeConfigPaths::discover();
/// let presentation = None;
/// let cli = None;
/// let session = None;
///
/// let resolved = build_runtime_pipeline(
///     defaults,
///     presentation,
///     &paths,
///     RuntimeLoadOptions::default(),
///     cli,
///     session,
/// )
/// .resolve(ResolveOptions::new().with_terminal("cli"))?;
///
/// assert_eq!(resolved.terminal(), Some("cli"));
/// # Ok::<(), osp_cli::config::ConfigError>(())
/// ```
pub fn build_runtime_pipeline(
    defaults: ConfigLayer,
    presentation: Option<ConfigLayer>,
    paths: &RuntimeConfigPaths,
    load: RuntimeLoadOptions,
    cli: Option<ConfigLayer>,
    session: Option<ConfigLayer>,
) -> LoaderPipeline {
    tracing::debug!(
        include_env = load.include_env,
        include_config_file = load.include_config_file,
        config_file = ?paths.config_file.as_ref().map(|path| path.display().to_string()),
        secrets_file = ?paths.secrets_file.as_ref().map(|path| path.display().to_string()),
        has_presentation_layer = presentation.is_some(),
        has_cli_layer = cli.is_some(),
        has_session_layer = session.is_some(),
        defaults_entries = defaults.entries().len(),
        "building runtime loader pipeline"
    );
    let mut pipeline = LoaderPipeline::new(StaticLayerLoader::new(defaults));

    if let Some(presentation_layer) = presentation {
        pipeline = pipeline.with_presentation(StaticLayerLoader::new(presentation_layer));
    }

    if load.include_env {
        pipeline = pipeline.with_env(EnvVarLoader::from_process_env());
    }

    if load.include_config_file
        && let Some(path) = &paths.config_file
    {
        pipeline = pipeline.with_file(TomlFileLoader::new(path.clone()).optional());
    }

    if let Some(path) = &paths.secrets_file {
        let mut secret_chain = ChainedLoader::new(SecretsTomlLoader::new(path.clone()).optional());
        if load.include_env {
            secret_chain = secret_chain.with(EnvSecretsLoader::from_process_env());
        }
        pipeline = pipeline.with_secrets(secret_chain);
    } else if load.include_env {
        pipeline = pipeline.with_secrets(ChainedLoader::new(EnvSecretsLoader::from_process_env()));
    }

    if let Some(cli_layer) = cli {
        pipeline = pipeline.with_cli(StaticLayerLoader::new(cli_layer));
    }
    if let Some(session_layer) = session {
        pipeline = pipeline.with_session(StaticLayerLoader::new(session_layer));
    }

    pipeline
}

/// Resolves the default platform config root from the current process environment.
pub fn default_config_root_dir() -> Option<PathBuf> {
    RuntimeEnvironment::capture().config_root_dir()
}

/// Resolves the default platform cache root from the current process environment.
pub fn default_cache_root_dir() -> Option<PathBuf> {
    RuntimeEnvironment::capture().cache_root_dir()
}

/// Resolves the default platform state root from the current process environment.
pub fn default_state_root_dir() -> Option<PathBuf> {
    RuntimeEnvironment::capture().state_root_dir()
}

/// Resolves the current user's home directory from the running platform.
pub fn default_home_dir() -> Option<PathBuf> {
    BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf())
}

#[derive(Debug, Clone, Default)]
struct RuntimeEnvironment {
    vars: BTreeMap<String, String>,
    prefer_platform_dirs: bool,
}

impl RuntimeEnvironment {
    fn capture() -> Self {
        Self {
            vars: std::env::vars().collect(),
            prefer_platform_dirs: true,
        }
    }

    fn defaults_only() -> Self {
        Self {
            vars: BTreeMap::new(),
            prefer_platform_dirs: false,
        }
    }

    #[cfg(test)]
    fn from_pairs<I, K, V>(vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        Self {
            vars: vars
                .into_iter()
                .map(|(key, value)| (key.as_ref().to_string(), value.as_ref().to_string()))
                .collect(),
            prefer_platform_dirs: false,
        }
    }

    fn config_root_dir(&self) -> Option<PathBuf> {
        self.xdg_root_dir("XDG_CONFIG_HOME", &[".config"])
    }

    fn cache_root_dir(&self) -> Option<PathBuf> {
        self.xdg_root_dir("XDG_CACHE_HOME", &[".cache"])
    }

    fn state_root_dir(&self) -> Option<PathBuf> {
        if let Some(path) = self.get_nonempty("XDG_STATE_HOME") {
            return Some(join_path(PathBuf::from(path), &[PROJECT_APPLICATION_NAME]));
        }

        if self.prefer_platform_dirs {
            return project_dirs().map(|dirs| {
                dirs.state_dir()
                    .unwrap_or_else(|| dirs.data_local_dir())
                    .to_path_buf()
            });
        }

        self.home_root_dir(&[".local", "state"])
    }

    fn config_path(&self, leaf: &str) -> Option<PathBuf> {
        self.config_root_dir().map(|root| join_path(root, &[leaf]))
    }

    fn theme_paths(&self) -> Vec<String> {
        self.config_root_dir()
            .map(|root| join_path(root, &["themes"]).to_string_lossy().to_string())
            .into_iter()
            .collect()
    }

    fn user_name(&self) -> String {
        self.get_nonempty("USER")
            .or_else(|| self.get_nonempty("USERNAME"))
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| "anonymous".to_string())
    }

    fn domain_name(&self) -> String {
        self.get_nonempty("HOSTNAME")
            .or_else(|| self.get_nonempty("COMPUTERNAME"))
            .unwrap_or("localhost")
            .split_once('.')
            .map(|(_, domain)| domain.to_string())
            .filter(|domain| !domain.trim().is_empty())
            .unwrap_or_else(|| "local".to_string())
    }

    fn repl_history_path(&self) -> String {
        join_path(
            self.state_root_dir_or_temp(),
            &["history", "${user.name}@${profile.active}.history"],
        )
        .display()
        .to_string()
    }

    fn log_file_path(&self) -> String {
        join_path(self.state_root_dir_or_temp(), &["osp.log"])
            .display()
            .to_string()
    }

    fn path_override(&self, key: &str) -> Option<PathBuf> {
        self.get_nonempty(key).map(PathBuf::from)
    }

    fn state_root_dir_or_temp(&self) -> PathBuf {
        self.state_root_dir().unwrap_or_else(|| {
            let mut path = std::env::temp_dir();
            path.push(PROJECT_APPLICATION_NAME);
            path
        })
    }

    fn xdg_root_dir(&self, xdg_var: &str, home_suffix: &[&str]) -> Option<PathBuf> {
        if let Some(path) = self.get_nonempty(xdg_var) {
            return Some(join_path(PathBuf::from(path), &[PROJECT_APPLICATION_NAME]));
        }

        if self.prefer_platform_dirs {
            return match xdg_var {
                "XDG_CONFIG_HOME" => project_dirs().map(|dirs| dirs.config_dir().to_path_buf()),
                "XDG_CACHE_HOME" => project_dirs().map(|dirs| dirs.cache_dir().to_path_buf()),
                _ => None,
            };
        }

        self.home_root_dir(home_suffix)
    }

    fn home_root_dir(&self, home_suffix: &[&str]) -> Option<PathBuf> {
        let home = self.get_nonempty("HOME")?;
        Some(join_path(PathBuf::from(home), home_suffix).join(PROJECT_APPLICATION_NAME))
    }

    fn get_nonempty(&self, key: &str) -> Option<&str> {
        self.vars
            .get(key)
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }
}

fn join_path(mut root: PathBuf, segments: &[&str]) -> PathBuf {
    for segment in segments {
        root.push(segment);
    }
    root
}

fn project_dirs() -> Option<ProjectDirs> {
    ProjectDirs::from("", "", PROJECT_APPLICATION_NAME)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        DEFAULT_PROFILE_NAME, RuntimeBootstrapMode, RuntimeConfigPaths, RuntimeDefaults,
        RuntimeEnvironment, RuntimeLoadOptions,
    };
    use crate::config::{ConfigLayer, ConfigValue, Scope};

    fn find_value<'a>(layer: &'a ConfigLayer, key: &str) -> Option<&'a ConfigValue> {
        layer
            .entries()
            .iter()
            .find(|entry| entry.key == key && entry.scope == Scope::global())
            .map(|entry| &entry.value)
    }

    #[test]
    fn runtime_defaults_seed_expected_keys_and_history_placeholders_unit() {
        let defaults =
            RuntimeDefaults::from_env(&RuntimeEnvironment::default(), "nord", "osp> ").to_layer();

        assert_eq!(
            find_value(&defaults, "profile.default"),
            Some(&ConfigValue::String(DEFAULT_PROFILE_NAME.to_string()))
        );
        assert_eq!(
            find_value(&defaults, "theme.name"),
            Some(&ConfigValue::String("nord".to_string()))
        );
        assert_eq!(
            find_value(&defaults, "repl.prompt"),
            Some(&ConfigValue::String("osp> ".to_string()))
        );
        assert_eq!(
            find_value(&defaults, "repl.intro"),
            Some(&ConfigValue::String(super::DEFAULT_REPL_INTRO.to_string()))
        );
        assert_eq!(
            find_value(&defaults, "repl.history.max_entries"),
            Some(&ConfigValue::Integer(
                super::DEFAULT_REPL_HISTORY_MAX_ENTRIES
            ))
        );
        assert_eq!(
            find_value(&defaults, "repl.history.menu_rows"),
            Some(&ConfigValue::Integer(super::DEFAULT_REPL_HISTORY_MENU_ROWS))
        );
        assert_eq!(
            find_value(&defaults, "ui.width"),
            Some(&ConfigValue::Integer(super::DEFAULT_UI_WIDTH))
        );
        assert_eq!(
            find_value(&defaults, "ui.presentation"),
            Some(&ConfigValue::String(
                super::DEFAULT_UI_PRESENTATION.to_string()
            ))
        );
        assert_eq!(
            find_value(&defaults, "ui.help.level"),
            Some(&ConfigValue::String("inherit".to_string()))
        );
        assert_eq!(
            find_value(&defaults, "ui.messages.layout"),
            Some(&ConfigValue::String(
                super::DEFAULT_UI_MESSAGES_LAYOUT.to_string()
            ))
        );
        assert_eq!(
            find_value(&defaults, "ui.message.verbosity"),
            Some(&ConfigValue::String("success".to_string()))
        );
        assert_eq!(
            find_value(&defaults, "ui.chrome.frame"),
            Some(&ConfigValue::String(
                super::DEFAULT_UI_CHROME_FRAME.to_string()
            ))
        );
        assert_eq!(
            find_value(&defaults, "ui.table.border"),
            Some(&ConfigValue::String(
                super::DEFAULT_UI_TABLE_BORDER.to_string()
            ))
        );
        assert_eq!(
            find_value(&defaults, "color.prompt.text"),
            Some(&ConfigValue::String(String::new()))
        );
        let path = match find_value(&defaults, "repl.history.path") {
            Some(ConfigValue::String(value)) => value.as_str(),
            other => panic!("unexpected history path value: {other:?}"),
        };

        assert!(path.contains("${user.name}@${profile.active}.history"));
    }

    #[test]
    fn defaults_only_runtime_load_options_disable_ambient_bootstrap_unit() {
        let load = RuntimeLoadOptions::defaults_only();

        assert!(!load.include_env);
        assert!(!load.include_config_file);
        assert_eq!(load.bootstrap_mode, RuntimeBootstrapMode::DefaultsOnly);
        assert!(load.is_defaults_only());
    }

    #[test]
    fn runtime_config_paths_prefer_explicit_file_overrides() {
        let env = RuntimeEnvironment::from_pairs([
            ("OSP_CONFIG_FILE", "/tmp/custom-config.toml"),
            ("OSP_SECRETS_FILE", "/tmp/custom-secrets.toml"),
            ("XDG_CONFIG_HOME", "/ignored"),
        ]);

        let paths = RuntimeConfigPaths::from_env(&env);

        assert_eq!(
            paths.config_file,
            Some(PathBuf::from("/tmp/custom-config.toml"))
        );
        assert_eq!(
            paths.secrets_file,
            Some(PathBuf::from("/tmp/custom-secrets.toml"))
        );

        let env = RuntimeEnvironment::from_pairs([("XDG_CONFIG_HOME", "/var/tmp/xdg-config")]);

        let paths = RuntimeConfigPaths::from_env(&env);

        assert_eq!(
            paths.config_file,
            Some(PathBuf::from("/var/tmp/xdg-config/osp/config.toml"))
        );
        assert_eq!(
            paths.secrets_file,
            Some(PathBuf::from("/var/tmp/xdg-config/osp/secrets.toml"))
        );
    }

    #[test]
    fn runtime_environment_uses_home_and_temp_fallbacks_for_state_paths_unit() {
        let env = RuntimeEnvironment::from_pairs([("HOME", "/home/tester")]);

        assert_eq!(
            env.config_root_dir(),
            Some(PathBuf::from("/home/tester/.config/osp"))
        );
        assert_eq!(
            env.cache_root_dir(),
            Some(PathBuf::from("/home/tester/.cache/osp"))
        );
        assert_eq!(
            env.state_root_dir(),
            Some(PathBuf::from("/home/tester/.local/state/osp"))
        );

        let env = RuntimeEnvironment::default();
        let mut expected_root = std::env::temp_dir();
        expected_root.push("osp");

        assert_eq!(
            env.repl_history_path(),
            expected_root
                .join("history")
                .join("${user.name}@${profile.active}.history")
                .display()
                .to_string()
        );
        assert_eq!(
            env.log_file_path(),
            expected_root.join("osp.log").display().to_string()
        );
    }

    #[test]
    fn defaults_only_bootstrap_skips_home_and_override_discovery_unit() {
        let load = RuntimeLoadOptions::defaults_only();
        let paths = RuntimeConfigPaths::discover_with(load);
        let defaults = RuntimeDefaults::from_runtime_load(load, "nord", "osp> ");

        assert_eq!(paths.config_file, None);
        assert_eq!(paths.secrets_file, None);
        assert_eq!(defaults.get_string("user.name"), Some("anonymous"));
        assert_eq!(defaults.get_string("domain"), Some("local"));
        assert_eq!(defaults.get_string("theme.name"), Some("nord"));
        assert_eq!(defaults.get_string("repl.prompt"), Some("osp> "));
        assert_eq!(defaults.get_string("theme.path"), None);
    }
}
