//! Runtime-scoped host state.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::config::{ResolvedConfig, RuntimeLoadOptions};
use crate::plugin::PluginManager;
use crate::plugin::config::{PluginConfigEntry, PluginConfigEnv, PluginConfigEnvCache};
use crate::ui::RenderSettings;
use crate::ui::messages::MessageLevel;
use crate::ui::theme_loader::ThemeCatalog;

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

#[derive(Debug, Clone)]
pub struct UiState {
    pub render_settings: RenderSettings,
    pub message_verbosity: MessageLevel,
    pub debug_verbosity: u8,
}

#[derive(Debug, Clone, Default)]
pub struct LaunchContext {
    pub plugin_dirs: Vec<PathBuf>,
    pub config_root: Option<PathBuf>,
    pub cache_root: Option<PathBuf>,
    pub runtime_load: RuntimeLoadOptions,
}

pub struct AppClients {
    pub plugins: PluginManager,
    plugin_config_env: PluginConfigEnvCache,
}

impl AppClients {
    pub fn new(plugins: PluginManager) -> Self {
        Self {
            plugins,
            plugin_config_env: PluginConfigEnvCache::default(),
        }
    }

    pub(crate) fn plugin_config_env(&self, config: &ConfigState) -> PluginConfigEnv {
        self.plugin_config_env.collect(config)
    }

    pub(crate) fn plugin_config_entries(
        &self,
        config: &ConfigState,
        plugin_id: &str,
    ) -> Vec<PluginConfigEntry> {
        let config_env = self.plugin_config_env(config);
        let mut merged = std::collections::BTreeMap::new();
        for entry in config_env.shared {
            merged.insert(entry.env_key.clone(), entry);
        }
        if let Some(entries) = config_env.by_plugin_id.get(plugin_id) {
            for entry in entries {
                merged.insert(entry.env_key.clone(), entry.clone());
            }
        }
        merged.into_values().collect()
    }
}

pub struct AppRuntime {
    pub context: RuntimeContext,
    pub config: ConfigState,
    pub ui: UiState,
    pub auth: AuthState,
    pub(crate) themes: ThemeCatalog,
    pub launch: LaunchContext,
}

pub struct AuthState {
    builtins_allowlist: Option<HashSet<String>>,
    plugins_allowlist: Option<HashSet<String>>,
}

impl AuthState {
    pub fn from_resolved(config: &ResolvedConfig) -> Self {
        Self {
            builtins_allowlist: parse_allowlist(config.get_string("auth.visible.builtins")),
            plugins_allowlist: parse_allowlist(config.get_string("auth.visible.plugins")),
        }
    }

    pub fn is_builtin_visible(&self, command: &str) -> bool {
        is_visible_in_allowlist(&self.builtins_allowlist, command)
    }

    pub fn is_plugin_command_visible(&self, command: &str) -> bool {
        is_visible_in_allowlist(&self.plugins_allowlist, command)
    }
}

fn parse_allowlist(raw: Option<&str>) -> Option<HashSet<String>> {
    let raw = raw.map(str::trim).filter(|value| !value.is_empty())?;

    if raw == "*" {
        return None;
    }

    let values = raw
        .split([',', ' '])
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .collect::<HashSet<String>>();
    if values.is_empty() {
        None
    } else {
        Some(values)
    }
}

fn is_visible_in_allowlist(allowlist: &Option<HashSet<String>>, command: &str) -> bool {
    match allowlist {
        None => true,
        Some(values) => values.contains(&command.to_ascii_lowercase()),
    }
}
