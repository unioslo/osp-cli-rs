//! Session-scoped host state for one logical app run.
//!
//! This module exists to hold mutable state that should survive across commands
//! within the same session, but should not be promoted to global runtime
//! state.
//!
//! High-level flow:
//!
//! - track prompt timing and last-failure details
//! - maintain REPL scope stack and small in-memory caches
//! - bundle session state that host code needs to carry between dispatches
//!
//! Contract:
//!
//! - session data here is narrower-lived than the runtime state in
//!   [`super::runtime`]
//! - long-lived environment/config/plugin bootstrap state should not drift into
//!   this module
//!
//! Public API shape:
//!
//! - use [`AppSessionBuilder`] for session-scoped REPL state
//! - use [`AppStateBuilder`] when you need a fully assembled runtime/session
//!   snapshot outside the full CLI bootstrap
//! - these types are host machinery, not lightweight semantic DTOs

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::config::{ConfigLayer, DEFAULT_SESSION_CACHE_MAX_RESULTS};
use crate::core::row::Row;
use crate::native::NativeCommandRegistry;
use crate::plugin::PluginManager;
use crate::repl::HistoryShellContext;

use super::command_output::CliCommandResult;
use super::runtime::{AppClients, AppRuntime, LaunchContext, RuntimeContext, UiState};
use super::timing::TimingSummary;

#[derive(Debug, Clone, Copy, Default)]
/// Timing badge rendered in the prompt for the most recent command.
pub struct DebugTimingBadge {
    /// Prompt detail level used when rendering the badge.
    pub level: u8,
    pub(crate) summary: TimingSummary,
}

/// Shared prompt-timing storage that dispatch code can update and prompt
/// rendering can read.
#[derive(Clone, Default, Debug)]
pub struct DebugTimingState {
    inner: Arc<RwLock<Option<DebugTimingBadge>>>,
}

impl DebugTimingState {
    /// Stores the current timing badge.
    pub fn set(&self, badge: DebugTimingBadge) {
        if let Ok(mut guard) = self.inner.write() {
            *guard = Some(badge);
        }
    }

    /// Clears any stored timing badge.
    pub fn clear(&self) {
        if let Ok(mut guard) = self.inner.write() {
            *guard = None;
        }
    }

    /// Returns the current timing badge, if one is available.
    pub fn badge(&self) -> Option<DebugTimingBadge> {
        self.inner.read().map(|value| *value).unwrap_or(None)
    }
}

/// One entered command scope inside the interactive REPL shell stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplScopeFrame {
    command: String,
}

impl ReplScopeFrame {
    /// Creates a frame for the given command name.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::app::ReplScopeFrame;
    ///
    /// let frame = ReplScopeFrame::new("theme");
    /// assert_eq!(frame.command(), "theme");
    /// ```
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
        }
    }

    /// Returns the command name associated with this scope frame.
    pub fn command(&self) -> &str {
        self.command.as_str()
    }
}

/// Nested REPL command-scope stack used for shell-style scoped interaction.
///
/// This is what lets the REPL stay "inside" a command family while still
/// rendering scope labels, help targets, and history prefixes consistently.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReplScopeStack {
    frames: Vec<ReplScopeFrame>,
}

impl ReplScopeStack {
    /// Returns `true` when the REPL is at the top-level scope.
    pub fn is_root(&self) -> bool {
        self.frames.is_empty()
    }

    /// Pushes a new command scope onto the stack.
    pub fn enter(&mut self, command: impl Into<String>) {
        self.frames.push(ReplScopeFrame::new(command));
    }

    /// Pops the current command scope from the stack.
    pub fn leave(&mut self) -> Option<ReplScopeFrame> {
        self.frames.pop()
    }

    /// Returns the command path represented by the current stack.
    pub fn commands(&self) -> Vec<String> {
        self.frames
            .iter()
            .map(|frame| frame.command.clone())
            .collect()
    }

    /// Returns whether the stack already contains the given command.
    pub fn contains_command(&self, command: &str) -> bool {
        self.frames
            .iter()
            .any(|frame| frame.command.eq_ignore_ascii_case(command))
    }

    /// Returns a human-readable label for the current scope path.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::app::ReplScopeStack;
    ///
    /// let mut scope = ReplScopeStack::default();
    /// assert_eq!(scope.display_label(), None);
    ///
    /// scope.enter("theme");
    /// scope.enter("show");
    /// assert_eq!(scope.display_label(), Some("theme / show".to_string()));
    /// ```
    pub fn display_label(&self) -> Option<String> {
        if self.is_root() {
            None
        } else {
            Some(
                self.frames
                    .iter()
                    .map(|frame| frame.command.as_str())
                    .collect::<Vec<_>>()
                    .join(" / "),
            )
        }
    }

    /// Returns the history prefix used for shell-backed history entries.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::app::ReplScopeStack;
    ///
    /// let mut scope = ReplScopeStack::default();
    /// scope.enter("theme");
    /// scope.enter("show");
    ///
    /// assert_eq!(scope.history_prefix(), "theme show ");
    /// ```
    pub fn history_prefix(&self) -> String {
        if self.is_root() {
            String::new()
        } else {
            format!(
                "{} ",
                self.frames
                    .iter()
                    .map(|frame| frame.command.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        }
    }

    /// Prepends the active scope path unless the tokens are already scoped.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::app::ReplScopeStack;
    ///
    /// let mut scope = ReplScopeStack::default();
    /// scope.enter("theme");
    ///
    /// assert_eq!(
    ///     scope.prefixed_tokens(&["show".to_string(), "dracula".to_string()]),
    ///     vec!["theme".to_string(), "show".to_string(), "dracula".to_string()]
    /// );
    /// ```
    pub fn prefixed_tokens(&self, tokens: &[String]) -> Vec<String> {
        let prefix = self.commands();
        if prefix.is_empty() || tokens.starts_with(&prefix) {
            return tokens.to_vec();
        }
        let mut full = prefix;
        full.extend_from_slice(tokens);
        full
    }

    /// Returns help tokens for the current scope.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::app::ReplScopeStack;
    ///
    /// let mut scope = ReplScopeStack::default();
    /// scope.enter("theme");
    ///
    /// assert_eq!(scope.help_tokens(), vec!["theme".to_string(), "--help".to_string()]);
    /// ```
    pub fn help_tokens(&self) -> Vec<String> {
        let mut tokens = self.commands();
        if !tokens.is_empty() {
            tokens.push("--help".to_string());
        }
        tokens
    }
}

/// Session-scoped REPL state, caches, and prompt metadata.
#[non_exhaustive]
pub struct AppSession {
    /// Prompt prefix shown before any scope label.
    pub prompt_prefix: String,
    /// Whether history capture is enabled for this session.
    pub history_enabled: bool,
    /// Shell-scoped history prefix state shared with the history store.
    pub history_shell: HistoryShellContext,
    /// Shared prompt timing badge state.
    pub prompt_timing: DebugTimingState,
    pub(crate) startup_prompt_timing_pending: bool,
    /// Current nested command scope within the REPL.
    pub scope: ReplScopeStack,
    /// Rows returned by the most recent successful REPL command.
    pub last_rows: Vec<Row>,
    /// Summary of the most recent failed REPL command.
    pub last_failure: Option<LastFailure>,
    /// Cached row outputs keyed by command line.
    pub result_cache: HashMap<String, Vec<Row>>,
    /// Eviction order for the row-result cache.
    pub cache_order: VecDeque<String>,
    pub(crate) command_cache: HashMap<String, CliCommandResult>,
    pub(crate) command_cache_order: VecDeque<String>,
    /// Maximum number of cached result sets to retain.
    pub max_cached_results: usize,
    /// Session-scoped config overrides layered above persisted config.
    pub config_overrides: ConfigLayer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Summary of the last failed REPL command.
pub struct LastFailure {
    /// Command line that produced the failure.
    pub command_line: String,
    /// Short failure summary suitable for prompts or status output.
    pub summary: String,
    /// Longer failure detail for follow-up inspection.
    pub detail: String,
}

impl AppSession {
    /// Starts a builder for session-scoped host state.
    pub fn builder() -> AppSessionBuilder {
        AppSessionBuilder::new()
    }

    /// Creates a session with bounded caches for row and command results.
    pub fn with_cache_limit(max_cached_results: usize) -> Self {
        let bounded = max_cached_results.max(1);
        Self {
            prompt_prefix: "osp".to_string(),
            history_enabled: true,
            history_shell: HistoryShellContext::default(),
            prompt_timing: DebugTimingState::default(),
            startup_prompt_timing_pending: true,
            scope: ReplScopeStack::default(),
            last_rows: Vec::new(),
            last_failure: None,
            result_cache: HashMap::new(),
            cache_order: VecDeque::new(),
            command_cache: HashMap::new(),
            command_cache_order: VecDeque::new(),
            max_cached_results: bounded,
            config_overrides: ConfigLayer::default(),
        }
    }

    /// Creates the default session snapshot for the current resolved config.
    pub(crate) fn from_resolved_config(config: &crate::config::ResolvedConfig) -> Self {
        let session_cache_max_results = crate::app::host::config_usize(
            config,
            "session.cache.max_results",
            DEFAULT_SESSION_CACHE_MAX_RESULTS as usize,
        );
        Self::builder()
            .with_cache_limit(session_cache_max_results)
            .build()
    }

    /// Creates the default session snapshot for the current resolved config
    /// and attaches the supplied session-layer overrides.
    pub(crate) fn from_resolved_config_with_overrides(
        config: &crate::config::ResolvedConfig,
        config_overrides: ConfigLayer,
    ) -> Self {
        Self::builder()
            .with_cache_limit(crate::app::host::config_usize(
                config,
                "session.cache.max_results",
                DEFAULT_SESSION_CACHE_MAX_RESULTS as usize,
            ))
            .with_config_overrides(config_overrides)
            .build()
    }

    /// Stores the latest successful row output and updates the result cache.
    pub fn record_result(&mut self, command_line: &str, rows: Vec<Row>) {
        let key = command_line.trim().to_string();
        if key.is_empty() {
            return;
        }

        self.last_rows = rows.clone();
        if !self.result_cache.contains_key(&key)
            && self.result_cache.len() >= self.max_cached_results
            && let Some(evict_key) = self.cache_order.pop_front()
        {
            self.result_cache.remove(&evict_key);
        }

        self.cache_order.retain(|item| item != &key);
        self.cache_order.push_back(key.clone());
        self.result_cache.insert(key, rows);
    }

    /// Records details about the latest failed command.
    pub fn record_failure(
        &mut self,
        command_line: &str,
        summary: impl Into<String>,
        detail: impl Into<String>,
    ) {
        let command_line = command_line.trim().to_string();
        if command_line.is_empty() {
            return;
        }
        self.last_failure = Some(LastFailure {
            command_line,
            summary: summary.into(),
            detail: detail.into(),
        });
    }

    /// Returns cached rows for a previously executed command line.
    pub fn cached_rows(&self, command_line: &str) -> Option<&[Row]> {
        self.result_cache
            .get(command_line.trim())
            .map(|rows| rows.as_slice())
    }

    pub(crate) fn record_cached_command(&mut self, cache_key: &str, result: &CliCommandResult) {
        let cache_key = cache_key.trim().to_string();
        if cache_key.is_empty() {
            return;
        }

        if !self.command_cache.contains_key(&cache_key)
            && self.command_cache.len() >= self.max_cached_results
            && let Some(evict_key) = self.command_cache_order.pop_front()
        {
            self.command_cache.remove(&evict_key);
        }

        self.command_cache_order.retain(|item| item != &cache_key);
        self.command_cache_order.push_back(cache_key.clone());
        self.command_cache.insert(cache_key, result.clone());
    }

    pub(crate) fn cached_command(&self, cache_key: &str) -> Option<CliCommandResult> {
        self.command_cache.get(cache_key.trim()).cloned()
    }

    /// Updates the prompt timing badge for the most recent command.
    pub fn record_prompt_timing(
        &self,
        level: u8,
        total: Duration,
        parse: Option<Duration>,
        execute: Option<Duration>,
        render: Option<Duration>,
    ) {
        if level == 0 {
            self.prompt_timing.clear();
            return;
        }

        self.prompt_timing.set(DebugTimingBadge {
            level,
            summary: TimingSummary {
                total,
                parse,
                execute,
                render,
            },
        });
    }

    /// Seeds the initial prompt timing badge emitted during startup.
    pub fn seed_startup_prompt_timing(&mut self, level: u8, total: Duration) {
        if !self.startup_prompt_timing_pending {
            return;
        }
        self.startup_prompt_timing_pending = false;
        if level == 0 {
            return;
        }

        self.prompt_timing.set(DebugTimingBadge {
            level,
            summary: TimingSummary {
                total,
                parse: None,
                execute: None,
                render: None,
            },
        });
    }

    /// Synchronizes history context with the current REPL scope.
    pub fn sync_history_shell_context(&self) {
        self.history_shell.set_prefix(self.scope.history_prefix());
    }
}

/// Builder for [`AppSession`].
///
/// This is the guided construction path for session-scoped REPL state.
pub struct AppSessionBuilder {
    prompt_prefix: String,
    history_enabled: bool,
    history_shell: HistoryShellContext,
    max_cached_results: usize,
    config_overrides: ConfigLayer,
}

impl Default for AppSessionBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl AppSessionBuilder {
    /// Starts a session builder with the crate's default prompt and cache size.
    pub fn new() -> Self {
        Self {
            prompt_prefix: "osp".to_string(),
            history_enabled: true,
            history_shell: HistoryShellContext::default(),
            max_cached_results: DEFAULT_SESSION_CACHE_MAX_RESULTS as usize,
            config_overrides: ConfigLayer::default(),
        }
    }

    /// Replaces the prompt prefix shown ahead of any scope label.
    pub fn with_prompt_prefix(mut self, prompt_prefix: impl Into<String>) -> Self {
        self.prompt_prefix = prompt_prefix.into();
        self
    }

    /// Enables or disables history capture for the built session.
    pub fn with_history_enabled(mut self, history_enabled: bool) -> Self {
        self.history_enabled = history_enabled;
        self
    }

    /// Replaces the shell-scoped history context shared with the history store.
    pub fn with_history_shell(mut self, history_shell: HistoryShellContext) -> Self {
        self.history_shell = history_shell;
        self
    }

    /// Replaces the maximum number of cached row/command results.
    pub fn with_cache_limit(mut self, max_cached_results: usize) -> Self {
        self.max_cached_results = max_cached_results;
        self
    }

    /// Replaces the session-scoped config overrides layered above persisted config.
    pub fn with_config_overrides(mut self, config_overrides: ConfigLayer) -> Self {
        self.config_overrides = config_overrides;
        self
    }

    /// Builds the configured [`AppSession`].
    pub fn build(self) -> AppSession {
        let mut session = AppSession::with_cache_limit(self.max_cached_results);
        session.prompt_prefix = self.prompt_prefix;
        session.history_enabled = self.history_enabled;
        session.history_shell = self.history_shell;
        session.config_overrides = self.config_overrides;
        session
    }
}

pub(crate) struct AppStateInit {
    pub context: RuntimeContext,
    pub config: crate::config::ResolvedConfig,
    pub render_settings: crate::ui::RenderSettings,
    pub message_verbosity: crate::ui::messages::MessageLevel,
    pub debug_verbosity: u8,
    pub plugins: crate::plugin::PluginManager,
    pub native_commands: NativeCommandRegistry,
    pub themes: crate::ui::theme_loader::ThemeCatalog,
    pub launch: LaunchContext,
}

pub(crate) struct AppStateParts {
    pub runtime: AppRuntime,
    pub session: AppSession,
    pub clients: AppClients,
}

impl AppStateParts {
    fn from_init(init: AppStateInit, session_override: Option<AppSession>) -> Self {
        let clients = AppClients::new(init.plugins, init.native_commands);
        let runtime = AppRuntime::from_resolved_parts(
            init.context,
            init.config,
            init.render_settings,
            init.message_verbosity,
            init.debug_verbosity,
            clients.plugins(),
            clients.native_commands(),
            init.themes,
            init.launch,
        );
        let session = session_override
            .unwrap_or_else(|| AppSession::from_resolved_config(runtime.config.resolved()));

        Self {
            runtime,
            session,
            clients,
        }
    }
}

/// Aggregate application state shared between runtime and session logic.
#[non_exhaustive]
pub struct AppState {
    /// Runtime-scoped services and resolved config state.
    pub runtime: AppRuntime,
    /// Session-scoped REPL caches and prompt metadata.
    pub session: AppSession,
    /// Shared client registries used during command execution.
    pub clients: AppClients,
}

impl AppState {
    /// Starts a builder for a fully assembled application state snapshot.
    pub fn builder(
        context: RuntimeContext,
        config: crate::config::ResolvedConfig,
        ui: UiState,
    ) -> AppStateBuilder {
        AppStateBuilder::new(context, config, ui)
    }

    /// Builds a full application-state snapshot by deriving UI state from the
    /// resolved config and runtime context.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::app::{AppState, RuntimeContext, TerminalKind};
    /// use osp_cli::config::{ConfigLayer, ConfigResolver, ResolveOptions};
    ///
    /// let mut defaults = ConfigLayer::default();
    /// defaults.set("profile.default", "default");
    /// defaults.set("ui.message.verbosity", "warning");
    ///
    /// let mut resolver = ConfigResolver::default();
    /// resolver.set_defaults(defaults);
    /// let config = resolver.resolve(ResolveOptions::new().with_terminal("repl")).unwrap();
    ///
    /// let state = AppState::from_resolved_config(
    ///     RuntimeContext::new(None, TerminalKind::Repl, None),
    ///     config,
    /// )
    /// .unwrap();
    ///
    /// assert_eq!(state.runtime.config.resolved().active_profile(), "default");
    /// assert_eq!(state.runtime.ui.message_verbosity.as_env_str(), "warning");
    /// assert!(state.clients.plugins().explicit_dirs().is_empty());
    /// ```
    pub fn from_resolved_config(
        context: RuntimeContext,
        config: crate::config::ResolvedConfig,
    ) -> miette::Result<Self> {
        AppStateBuilder::from_resolved_config(context, config).map(AppStateBuilder::build)
    }

    #[cfg(test)]
    pub(crate) fn new(init: AppStateInit) -> Self {
        Self::from_parts(AppStateParts::from_init(init, None))
    }

    pub(crate) fn from_parts(parts: AppStateParts) -> Self {
        Self {
            runtime: parts.runtime,
            session: parts.session,
            clients: parts.clients,
        }
    }

    pub(crate) fn replace_parts(&mut self, parts: AppStateParts) {
        self.runtime = parts.runtime;
        self.session = parts.session;
        self.clients = parts.clients;
    }

    /// Returns the prompt prefix configured for the current session.
    pub fn prompt_prefix(&self) -> String {
        self.session.prompt_prefix.clone()
    }

    /// Synchronizes the history shell context with the current session scope.
    pub fn sync_history_shell_context(&self) {
        self.session.sync_history_shell_context();
    }

    /// Records rows produced by a REPL command.
    pub fn record_repl_rows(&mut self, command_line: &str, rows: Vec<Row>) {
        self.session.record_result(command_line, rows);
    }

    /// Records a failed REPL command and its associated messages.
    pub fn record_repl_failure(
        &mut self,
        command_line: &str,
        summary: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.session.record_failure(command_line, summary, detail);
    }

    /// Returns the rows from the most recent successful REPL command.
    pub fn last_repl_rows(&self) -> Vec<Row> {
        self.session.last_rows.clone()
    }

    /// Returns details about the most recent failed REPL command.
    pub fn last_repl_failure(&self) -> Option<LastFailure> {
        self.session.last_failure.clone()
    }

    /// Returns cached rows for a previously executed REPL command.
    pub fn cached_repl_rows(&self, command_line: &str) -> Option<Vec<Row>> {
        self.session
            .cached_rows(command_line)
            .map(ToOwned::to_owned)
    }

    /// Returns the number of cached REPL result sets.
    pub fn repl_cache_size(&self) -> usize {
        self.session.result_cache.len()
    }
}

/// Builder for [`AppState`].
///
/// This is the canonical embedder-facing factory for runtime/session/client
/// state when callers need a snapshot without going through full CLI bootstrap.
pub struct AppStateBuilder {
    context: RuntimeContext,
    config: crate::config::ResolvedConfig,
    ui: UiState,
    launch: LaunchContext,
    plugins: Option<PluginManager>,
    native_commands: NativeCommandRegistry,
    session: Option<AppSession>,
    themes: Option<crate::ui::theme_loader::ThemeCatalog>,
}

impl AppStateBuilder {
    /// Starts building an application-state snapshot from the resolved config
    /// and UI state the caller wants to expose.
    pub fn new(
        context: RuntimeContext,
        config: crate::config::ResolvedConfig,
        ui: UiState,
    ) -> Self {
        Self {
            context,
            config,
            ui,
            launch: LaunchContext::default(),
            plugins: None,
            native_commands: NativeCommandRegistry::default(),
            session: None,
            themes: None,
        }
    }

    /// Starts a builder by deriving UI state from the resolved config and
    /// runtime context.
    pub fn from_resolved_config(
        context: RuntimeContext,
        config: crate::config::ResolvedConfig,
    ) -> miette::Result<Self> {
        // This path is the canonical embedder factory: derive host inputs once
        // and hand callers a coherent runtime/session/client snapshot.
        let host_inputs = crate::app::assembly::ResolvedHostInputs::derive(
            &context,
            &config,
            &LaunchContext::default(),
            crate::app::assembly::RenderSettingsSeed::DefaultAuto,
            None,
            None,
            None,
        )?;
        crate::ui::theme_loader::log_theme_issues(&host_inputs.themes.issues);
        Ok(Self {
            context,
            config,
            ui: host_inputs.ui,
            launch: LaunchContext::default(),
            plugins: None,
            native_commands: NativeCommandRegistry::default(),
            session: Some(host_inputs.default_session),
            themes: Some(host_inputs.themes),
        })
    }

    /// Replaces the launch-time provenance used for cache and plugin setup.
    pub fn with_launch(mut self, launch: LaunchContext) -> Self {
        self.launch = launch;
        self
    }

    /// Replaces the plugin manager used when assembling shared clients.
    pub fn with_plugins(mut self, plugins: PluginManager) -> Self {
        self.plugins = Some(plugins);
        self
    }

    /// Replaces the native command registry used when assembling shared clients.
    pub fn with_native_commands(mut self, native_commands: NativeCommandRegistry) -> Self {
        self.native_commands = native_commands;
        self
    }

    /// Replaces the session snapshot carried by the built app state.
    pub fn with_session(mut self, session: AppSession) -> Self {
        self.session = Some(session);
        self
    }

    /// Replaces the loaded theme catalog used during state assembly.
    pub(crate) fn with_themes(mut self, themes: crate::ui::theme_loader::ThemeCatalog) -> Self {
        self.themes = Some(themes);
        self
    }

    /// Builds the configured [`AppState`].
    pub fn build(self) -> AppState {
        let themes = self.themes.unwrap_or_else(|| {
            let themes = crate::ui::theme_loader::load_theme_catalog(&self.config);
            crate::ui::theme_loader::log_theme_issues(&themes.issues);
            themes
        });
        let plugins = self
            .plugins
            .unwrap_or_else(|| default_plugin_manager(&self.config, &self.launch));

        let crate::app::UiState {
            render_settings,
            message_verbosity,
            debug_verbosity,
            ..
        } = self.ui;

        AppState::from_parts(AppStateParts::from_init(
            AppStateInit {
                context: self.context,
                config: self.config,
                render_settings,
                message_verbosity,
                debug_verbosity,
                plugins,
                native_commands: self.native_commands,
                themes,
                launch: self.launch,
            },
            self.session,
        ))
    }
}

fn default_plugin_manager(
    config: &crate::config::ResolvedConfig,
    launch: &LaunchContext,
) -> PluginManager {
    crate::app::assembly::build_plugin_manager(config, launch, None)
}
