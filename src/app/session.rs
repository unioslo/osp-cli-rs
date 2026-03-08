//! Session-scoped host state.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::config::{ConfigLayer, DEFAULT_SESSION_CACHE_MAX_RESULTS};
use crate::core::row::Row;
use crate::repl::HistoryShellContext;

use super::command_output::CliCommandResult;
use super::runtime::{
    AppClients, AppRuntime, AuthState, ConfigState, LaunchContext, RuntimeContext, UiState,
};
use super::timing::TimingSummary;

#[derive(Debug, Clone, Copy, Default)]
pub struct DebugTimingBadge {
    pub level: u8,
    pub(crate) summary: TimingSummary,
}

#[derive(Clone, Default, Debug)]
pub struct DebugTimingState {
    inner: Arc<RwLock<Option<DebugTimingBadge>>>,
}

impl DebugTimingState {
    pub fn set(&self, badge: DebugTimingBadge) {
        if let Ok(mut guard) = self.inner.write() {
            *guard = Some(badge);
        }
    }

    pub fn clear(&self) {
        if let Ok(mut guard) = self.inner.write() {
            *guard = None;
        }
    }

    pub fn badge(&self) -> Option<DebugTimingBadge> {
        self.inner.read().map(|value| *value).unwrap_or(None)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplScopeFrame {
    command: String,
}

impl ReplScopeFrame {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
        }
    }

    pub fn command(&self) -> &str {
        self.command.as_str()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReplScopeStack {
    frames: Vec<ReplScopeFrame>,
}

impl ReplScopeStack {
    pub fn is_root(&self) -> bool {
        self.frames.is_empty()
    }

    pub fn enter(&mut self, command: impl Into<String>) {
        self.frames.push(ReplScopeFrame::new(command));
    }

    pub fn leave(&mut self) -> Option<ReplScopeFrame> {
        self.frames.pop()
    }

    pub fn commands(&self) -> Vec<String> {
        self.frames
            .iter()
            .map(|frame| frame.command.clone())
            .collect()
    }

    pub fn contains_command(&self, command: &str) -> bool {
        self.frames
            .iter()
            .any(|frame| frame.command.eq_ignore_ascii_case(command))
    }

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

    pub fn prefixed_tokens(&self, tokens: &[String]) -> Vec<String> {
        let prefix = self.commands();
        if prefix.is_empty() || tokens.starts_with(&prefix) {
            return tokens.to_vec();
        }
        let mut full = prefix;
        full.extend_from_slice(tokens);
        full
    }

    pub fn help_tokens(&self) -> Vec<String> {
        let mut tokens = self.commands();
        if !tokens.is_empty() {
            tokens.push("--help".to_string());
        }
        tokens
    }
}

pub struct AppSession {
    pub prompt_prefix: String,
    pub history_enabled: bool,
    pub history_shell: HistoryShellContext,
    pub prompt_timing: DebugTimingState,
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
pub struct LastFailure {
    pub command_line: String,
    pub summary: String,
    pub detail: String,
}

impl AppSession {
    pub fn with_cache_limit(max_cached_results: usize) -> Self {
        let bounded = max_cached_results.max(1);
        Self {
            prompt_prefix: "osp".to_string(),
            history_enabled: true,
            history_shell: HistoryShellContext::default(),
            prompt_timing: DebugTimingState::default(),
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
    pub themes: crate::ui::theme_loader::ThemeCatalog,
    pub launch: LaunchContext,
}

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
                crate::core::command_policy::CommandPolicyRegistry::default()
            });
        auth_state.replace_plugin_policy(plugin_policy);
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
            clients: AppClients::new(init.plugins),
        }
    }

    pub fn prompt_prefix(&self) -> String {
        self.session.prompt_prefix.clone()
    }

    pub fn sync_history_shell_context(&self) {
        self.session.sync_history_shell_context();
    }

    pub fn record_repl_rows(&mut self, command_line: &str, rows: Vec<Row>) {
        self.session.record_result(command_line, rows);
    }

    pub fn record_repl_failure(
        &mut self,
        command_line: &str,
        summary: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.session.record_failure(command_line, summary, detail);
    }

    pub fn last_repl_rows(&self) -> Vec<Row> {
        self.session.last_rows.clone()
    }

    pub fn last_repl_failure(&self) -> Option<LastFailure> {
        self.session.last_failure.clone()
    }

    pub fn cached_repl_rows(&self, command_line: &str) -> Option<Vec<Row>> {
        self.session
            .cached_rows(command_line)
            .map(ToOwned::to_owned)
    }

    pub fn repl_cache_size(&self) -> usize {
        self.session.result_cache.len()
    }
}
