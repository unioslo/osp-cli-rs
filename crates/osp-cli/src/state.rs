use std::collections::{HashMap, VecDeque};

use osp_config::{ConfigLayer, ResolvedConfig};
use osp_core::row::Row;
use osp_ui::RenderSettings;
use osp_ui::messages::MessageLevel;

use crate::plugin_manager::PluginManager;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalKind {
    Cli,
    Repl,
}

impl TerminalKind {
    pub fn as_config_terminal(self) -> &'static str {
        match self {
            TerminalKind::Cli => "cli",
            TerminalKind::Repl => "repl",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeContext {
    profile_override: Option<String>,
    terminal_kind: TerminalKind,
    terminal_env: Option<String>,
}

impl RuntimeContext {
    pub fn new(
        profile_override: Option<String>,
        terminal_kind: TerminalKind,
        terminal_env: Option<String>,
    ) -> Self {
        Self {
            profile_override: profile_override
                .map(|value| value.trim().to_ascii_lowercase())
                .filter(|value| !value.is_empty()),
            terminal_kind,
            terminal_env,
        }
    }

    pub fn profile_override(&self) -> Option<&str> {
        self.profile_override.as_deref()
    }

    pub fn terminal_kind(&self) -> TerminalKind {
        self.terminal_kind
    }

    pub fn terminal_env(&self) -> Option<&str> {
        self.terminal_env.as_deref()
    }
}

pub struct ConfigState {
    resolved: ResolvedConfig,
    revision: u64,
}

impl ConfigState {
    pub fn new(resolved: ResolvedConfig) -> Self {
        Self {
            resolved,
            revision: 1,
        }
    }

    pub fn resolved(&self) -> &ResolvedConfig {
        &self.resolved
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn replace_resolved(&mut self, next: ResolvedConfig) -> bool {
        if self.resolved == next {
            return false;
        }

        self.resolved = next;
        self.revision += 1;
        true
    }

    pub fn transaction<F, E>(&mut self, mutator: F) -> Result<bool, E>
    where
        F: FnOnce(&ResolvedConfig) -> Result<ResolvedConfig, E>,
    {
        let current = self.resolved.clone();
        let candidate = mutator(&current)?;
        Ok(self.replace_resolved(candidate))
    }
}

pub struct UiState {
    pub render_settings: RenderSettings,
    pub message_verbosity: MessageLevel,
    pub debug_verbosity: u8,
}

pub struct ReplState {
    pub prompt_prefix: String,
    pub history_enabled: bool,
}

#[derive(Default)]
pub struct SessionState {
    pub shell_stack: Vec<String>,
    pub last_rows: Vec<Row>,
    pub result_cache: HashMap<String, Vec<Row>>,
    pub cache_order: VecDeque<String>,
    pub max_cached_results: usize,
    pub config_overrides: ConfigLayer,
}

impl SessionState {
    pub fn with_defaults() -> Self {
        Self {
            shell_stack: Vec::new(),
            last_rows: Vec::new(),
            result_cache: HashMap::new(),
            cache_order: VecDeque::new(),
            max_cached_results: 64,
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

    pub fn cached_rows(&self, command_line: &str) -> Option<&[Row]> {
        self.result_cache
            .get(command_line.trim())
            .map(|rows| rows.as_slice())
    }
}

pub struct ClientsState {
    pub plugins: PluginManager,
    config_revision: u64,
}

impl ClientsState {
    pub fn new(plugins: PluginManager, config_revision: u64) -> Self {
        Self {
            plugins,
            config_revision,
        }
    }

    pub fn config_revision(&self) -> u64 {
        self.config_revision
    }

    pub fn sync_config_revision(&mut self, config_revision: u64) {
        self.config_revision = config_revision;
    }
}

pub struct AppState {
    pub context: RuntimeContext,
    pub config: ConfigState,
    pub ui: UiState,
    pub repl: ReplState,
    pub session: SessionState,
    pub clients: ClientsState,
}

impl AppState {
    pub fn new(
        context: RuntimeContext,
        config: ResolvedConfig,
        render_settings: RenderSettings,
        message_verbosity: MessageLevel,
        debug_verbosity: u8,
        plugins: PluginManager,
    ) -> Self {
        let config_state = ConfigState::new(config);
        let config_revision = config_state.revision();

        Self {
            context,
            config: config_state,
            ui: UiState {
                render_settings,
                message_verbosity,
                debug_verbosity,
            },
            repl: ReplState {
                prompt_prefix: "osp".to_string(),
                history_enabled: true,
            },
            session: SessionState::with_defaults(),
            clients: ClientsState::new(plugins, config_revision),
        }
    }

    pub fn prompt_prefix(&self) -> String {
        self.repl.prompt_prefix.clone()
    }

    pub fn record_repl_rows(&mut self, command_line: &str, rows: Vec<Row>) {
        self.session.record_result(command_line, rows);
    }

    pub fn last_repl_rows(&self) -> Vec<Row> {
        self.session.last_rows.clone()
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
