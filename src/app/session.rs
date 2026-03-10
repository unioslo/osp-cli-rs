//! Session-scoped host state.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::config::{ConfigLayer, DEFAULT_SESSION_CACHE_MAX_RESULTS};
use crate::core::command_policy::CommandPolicyRegistry;
use crate::core::row::Row;
use crate::native::NativeCommandRegistry;
use crate::repl::HistoryShellContext;

use super::command_output::CliCommandResult;
use super::runtime::{
    AppClients, AppRuntime, AuthState, ConfigState, LaunchContext, RuntimeContext, UiState,
};
use super::timing::TimingSummary;

#[derive(Debug, Clone, Copy, Default)]
/// Timing badge rendered in the prompt for the most recent command.
pub struct DebugTimingBadge {
    pub level: u8,
    pub(crate) summary: TimingSummary,
}

#[derive(Clone, Default, Debug)]
/// Shared storage for the current prompt timing badge.
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

#[derive(Debug, Clone, PartialEq, Eq)]
/// One scope level in the REPL command stack.
pub struct ReplScopeFrame {
    command: String,
}

impl ReplScopeFrame {
    /// Creates a frame for the given command name.
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Stack of nested REPL command scopes.
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
    pub fn help_tokens(&self) -> Vec<String> {
        let mut tokens = self.commands();
        if !tokens.is_empty() {
            tokens.push("--help".to_string());
        }
        tokens
    }
}

/// Session-scoped REPL state, caches, and prompt metadata.
pub struct AppSession {
    pub prompt_prefix: String,
    pub history_enabled: bool,
    pub history_shell: HistoryShellContext,
    pub prompt_timing: DebugTimingState,
    pub(crate) startup_prompt_timing_pending: bool,
    pub scope: ReplScopeStack,
    pub last_rows: Vec<Row>,
    pub last_failure: Option<LastFailure>,
    pub result_cache: HashMap<String, Vec<Row>>,
    pub cache_order: VecDeque<String>,
    pub(crate) command_cache: HashMap<String, CliCommandResult>,
    pub(crate) command_cache_order: VecDeque<String>,
    pub max_cached_results: usize,
    pub config_overrides: ConfigLayer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Summary of the last failed REPL command.
pub struct LastFailure {
    pub command_line: String,
    pub summary: String,
    pub detail: String,
}

impl AppSession {
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

/// Aggregate application state shared between runtime and session logic.
pub struct AppState {
    pub runtime: AppRuntime,
    pub session: AppSession,
    pub clients: AppClients,
}

impl AppState {
    pub(crate) fn new(init: AppStateInit) -> Self {
        let config_state = ConfigState::new(init.config);
        let mut auth_state = AuthState::from_resolved(config_state.resolved());
        let plugin_policy = init
            .plugins
            .command_policy_registry()
            .unwrap_or_else(|err| {
                tracing::warn!(error = %err, "failed to build plugin command policy registry");
                CommandPolicyRegistry::default()
            });
        let external_policy = merge_policy_registries(
            plugin_policy,
            init.native_commands.command_policy_registry(),
        );
        auth_state.replace_external_policy(external_policy);
        let session_cache_max_results = crate::app::host::config_usize(
            config_state.resolved(),
            "session.cache.max_results",
            DEFAULT_SESSION_CACHE_MAX_RESULTS as usize,
        );

        Self {
            runtime: AppRuntime {
                context: init.context,
                config: config_state,
                ui: UiState {
                    render_settings: init.render_settings,
                    message_verbosity: init.message_verbosity,
                    debug_verbosity: init.debug_verbosity,
                },
                auth: auth_state,
                themes: init.themes,
                launch: init.launch,
            },
            session: AppSession::with_cache_limit(session_cache_max_results),
            clients: AppClients::new(init.plugins, init.native_commands),
        }
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

fn merge_policy_registries(
    mut left: CommandPolicyRegistry,
    right: CommandPolicyRegistry,
) -> CommandPolicyRegistry {
    for policy in right.entries() {
        left.register(policy.clone());
    }
    left
}
