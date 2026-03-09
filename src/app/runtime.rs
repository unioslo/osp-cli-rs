//! Runtime-scoped host state.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

use crate::config::{ResolvedConfig, RuntimeLoadOptions};
use crate::core::command_policy::{
    AccessReason, CommandAccess, CommandPolicy, CommandPolicyContext, CommandPolicyRegistry,
    VisibilityMode,
};
use crate::native::NativeCommandRegistry;
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

#[derive(Debug, Clone)]
pub struct LaunchContext {
    pub plugin_dirs: Vec<PathBuf>,
    pub config_root: Option<PathBuf>,
    pub cache_root: Option<PathBuf>,
    pub runtime_load: RuntimeLoadOptions,
    pub startup_started_at: Instant,
}

impl Default for LaunchContext {
    fn default() -> Self {
        Self {
            plugin_dirs: Vec::new(),
            config_root: None,
            cache_root: None,
            runtime_load: RuntimeLoadOptions::default(),
            startup_started_at: Instant::now(),
        }
    }
}

pub struct AppClients {
    pub plugins: PluginManager,
    pub native_commands: NativeCommandRegistry,
    plugin_config_env: PluginConfigEnvCache,
}

impl AppClients {
    pub fn new(plugins: PluginManager, native_commands: NativeCommandRegistry) -> Self {
        Self {
            plugins,
            native_commands,
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
    external_allowlist: Option<HashSet<String>>,
    policy_context: CommandPolicyContext,
    builtin_policy: CommandPolicyRegistry,
    external_policy: CommandPolicyRegistry,
}

impl AuthState {
    pub fn from_resolved(config: &ResolvedConfig) -> Self {
        Self {
            builtins_allowlist: parse_allowlist(config.get_string("auth.visible.builtins")),
            // Non-builtin top-level commands currently still use the historical
            // `auth.visible.plugins` key. That surface now covers both external
            // plugins and native registered integrations dispatched via the
            // generic external command path.
            external_allowlist: parse_allowlist(config.get_string("auth.visible.plugins")),
            policy_context: CommandPolicyContext::default(),
            builtin_policy: CommandPolicyRegistry::default(),
            external_policy: CommandPolicyRegistry::default(),
        }
    }

    pub fn policy_context(&self) -> &CommandPolicyContext {
        &self.policy_context
    }

    pub fn set_policy_context(&mut self, context: CommandPolicyContext) {
        self.policy_context = context;
    }

    pub fn builtin_policy(&self) -> &CommandPolicyRegistry {
        &self.builtin_policy
    }

    pub fn builtin_policy_mut(&mut self) -> &mut CommandPolicyRegistry {
        &mut self.builtin_policy
    }

    pub fn external_policy(&self) -> &CommandPolicyRegistry {
        &self.external_policy
    }

    pub fn external_policy_mut(&mut self) -> &mut CommandPolicyRegistry {
        &mut self.external_policy
    }

    pub fn replace_external_policy(&mut self, registry: CommandPolicyRegistry) {
        self.external_policy = registry;
    }

    pub fn builtin_access(&self, command: &str) -> CommandAccess {
        command_access_for(
            command,
            &self.builtins_allowlist,
            &self.builtin_policy,
            &self.policy_context,
        )
    }

    pub fn external_command_access(&self, command: &str) -> CommandAccess {
        command_access_for(
            command,
            &self.external_allowlist,
            &self.external_policy,
            &self.policy_context,
        )
    }

    pub fn is_builtin_visible(&self, command: &str) -> bool {
        self.builtin_access(command).is_visible()
    }

    pub fn is_external_command_visible(&self, command: &str) -> bool {
        self.external_command_access(command).is_visible()
    }

    pub fn plugin_policy(&self) -> &CommandPolicyRegistry {
        self.external_policy()
    }

    pub fn plugin_policy_mut(&mut self) -> &mut CommandPolicyRegistry {
        self.external_policy_mut()
    }

    pub fn replace_plugin_policy(&mut self, registry: CommandPolicyRegistry) {
        self.replace_external_policy(registry);
    }

    pub fn plugin_command_access(&self, command: &str) -> CommandAccess {
        self.external_command_access(command)
    }

    pub fn is_plugin_command_visible(&self, command: &str) -> bool {
        self.is_external_command_visible(command)
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

fn command_access_for(
    command: &str,
    allowlist: &Option<HashSet<String>>,
    registry: &CommandPolicyRegistry,
    context: &CommandPolicyContext,
) -> CommandAccess {
    let normalized = command.trim().to_ascii_lowercase();
    let default_policy = CommandPolicy::new(crate::core::command_policy::CommandPath::new([
        normalized.clone(),
    ]))
    .visibility(VisibilityMode::Public);
    let mut access = registry
        .evaluate(&default_policy.path, context)
        .unwrap_or_else(|| crate::core::command_policy::evaluate_policy(&default_policy, context));

    if !is_visible_in_allowlist(allowlist, &normalized) {
        access = CommandAccess::hidden(AccessReason::HiddenByPolicy);
    }

    access
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::config::{ConfigLayer, ConfigResolver, LoadedLayers, ResolveOptions};
    use crate::core::command_policy::{
        AccessReason, CommandPath, CommandPolicy, CommandPolicyContext, CommandPolicyRegistry,
        VisibilityMode,
    };

    use super::{
        AuthState, ConfigState, RuntimeContext, TerminalKind, command_access_for,
        is_visible_in_allowlist, parse_allowlist,
    };

    fn resolved_with(entries: &[(&str, &str)]) -> crate::config::ResolvedConfig {
        let mut file = ConfigLayer::default();
        for (key, value) in entries {
            file.set(*key, (*value).to_string());
        }
        ConfigResolver::from_loaded_layers(LoadedLayers {
            file,
            ..LoadedLayers::default()
        })
        .resolve(ResolveOptions::default())
        .expect("config should resolve")
    }

    #[test]
    fn runtime_context_and_allowlists_normalize_inputs() {
        let context = RuntimeContext::new(
            Some("  Dev  ".to_string()),
            TerminalKind::Repl,
            Some("xterm-256color".to_string()),
        );
        assert_eq!(context.profile_override(), Some("dev"));
        assert_eq!(context.terminal_kind(), TerminalKind::Repl);
        assert_eq!(context.terminal_env(), Some("xterm-256color"));

        assert_eq!(parse_allowlist(None), None);
        assert_eq!(parse_allowlist(Some("   ")), None);
        assert_eq!(parse_allowlist(Some("*")), None);
        assert_eq!(
            parse_allowlist(Some(" LDAP, mreg ldap ")),
            Some(HashSet::from(["ldap".to_string(), "mreg".to_string()]))
        );

        let allowlist = Some(HashSet::from(["ldap".to_string()]));
        assert!(is_visible_in_allowlist(&allowlist, "LDAP"));
        assert!(!is_visible_in_allowlist(&allowlist, "orch"));
    }

    #[test]
    fn config_state_tracks_noops_changes_and_transaction_errors() {
        let resolved = resolved_with(&[]);
        let mut state = ConfigState::new(resolved.clone());
        assert_eq!(state.revision(), 1);
        assert!(!state.replace_resolved(resolved.clone()));
        assert_eq!(state.revision(), 1);

        let changed = resolved_with(&[("ui.format", "json")]);
        assert!(state.replace_resolved(changed));
        assert_eq!(state.revision(), 2);

        let changed = state
            .transaction(|current| {
                let _ = current;
                Ok::<_, &'static str>(resolved_with(&[("ui.format", "mreg")]))
            })
            .expect("transaction should succeed");
        assert!(changed);
        assert_eq!(state.revision(), 3);

        let err = state
            .transaction(|_| Err::<crate::config::ResolvedConfig, _>("boom"))
            .expect_err("transaction error should propagate");
        assert_eq!(err, "boom");
        assert_eq!(state.revision(), 3);
    }

    #[test]
    fn auth_state_and_command_access_layer_policy_overrides_on_allowlists() {
        let resolved = resolved_with(&[
            ("auth.visible.builtins", "config"),
            ("auth.visible.plugins", "ldap"),
        ]);
        let mut auth = AuthState::from_resolved(&resolved);
        auth.set_policy_context(
            CommandPolicyContext::default()
                .authenticated(true)
                .with_capabilities(["orch.approval.decide"]),
        );
        assert!(auth.policy_context().authenticated);

        auth.builtin_policy_mut().register(
            CommandPolicy::new(CommandPath::new(["config"]))
                .visibility(VisibilityMode::Authenticated),
        );
        assert!(auth.builtin_access("config").is_runnable());
        assert!(auth.is_builtin_visible("config"));
        assert!(!auth.is_builtin_visible("theme"));

        let mut plugin_registry = CommandPolicyRegistry::new();
        plugin_registry.register(
            CommandPolicy::new(CommandPath::new(["ldap"]))
                .visibility(VisibilityMode::CapabilityGated)
                .require_capability("orch.approval.decide"),
        );
        plugin_registry.register(
            CommandPolicy::new(CommandPath::new(["orch"]))
                .visibility(VisibilityMode::Authenticated),
        );
        auth.replace_plugin_policy(plugin_registry);

        assert!(auth.plugin_policy().contains(&CommandPath::new(["ldap"])));
        assert!(
            auth.plugin_policy_mut()
                .contains(&CommandPath::new(["ldap"]))
        );
        assert!(auth.plugin_command_access("ldap").is_runnable());
        assert!(auth.is_plugin_command_visible("ldap"));

        let hidden = auth.plugin_command_access("orch");
        assert_eq!(hidden.reasons, vec![AccessReason::HiddenByPolicy]);
        assert!(!hidden.is_visible());
    }

    #[test]
    fn command_access_for_uses_registry_when_present_and_public_default_otherwise() {
        let context = CommandPolicyContext::default();
        let allowlist = Some(HashSet::from(["config".to_string()]));
        let mut registry = CommandPolicyRegistry::new();
        registry.register(
            CommandPolicy::new(CommandPath::new(["config"]))
                .visibility(VisibilityMode::Authenticated),
        );

        let denied = command_access_for("config", &allowlist, &registry, &context);
        assert_eq!(denied.reasons, vec![AccessReason::Unauthenticated]);
        assert!(denied.is_visible());
        assert!(!denied.is_runnable());

        let hidden = command_access_for("theme", &allowlist, &registry, &context);
        assert_eq!(hidden.reasons, vec![AccessReason::HiddenByPolicy]);
        assert!(!hidden.is_visible());

        let fallback =
            command_access_for("config", &None, &CommandPolicyRegistry::default(), &context);
        assert!(fallback.is_visible());
        assert!(fallback.is_runnable());
    }
}
