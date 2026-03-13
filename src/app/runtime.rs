//! Runtime-scoped host state shared across invocations.
//!
//! This module exists to hold the long-lived state that belongs to the running
//! host rather than to any single command submission.
//!
//! High-level flow:
//!
//! - capture startup-time runtime context such as terminal kind and profile
//!   override
//! - keep the current resolved config and derived UI/plugin state together
//! - expose one place where host code can read the active runtime snapshot
//!
//! Contract:
//!
//! - runtime state here is broader-lived than session/request state
//! - per-command or per-REPL-line details should not accumulate here unless
//!   they truly affect the whole running host
//!
//! Public API shape:
//!
//! - these types model host machinery, not lightweight semantic payloads
//! - constructors/accessors are the preferred way to create and inspect them
//! - callers usually receive [`AppRuntime`] and [`AppClients`] from host
//!   bootstrap rather than assembling them field-by-field

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

use crate::config::{ConfigLayer, ResolvedConfig, RuntimeLoadOptions};
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

/// Identifies which top-level host surface is currently active.
///
/// This lets config selection and runtime behavior distinguish between
/// one-shot CLI execution and the long-lived REPL host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalKind {
    /// One-shot command execution.
    Cli,
    /// Interactive REPL execution.
    Repl,
}

impl TerminalKind {
    /// Returns the config key fragment used for this terminal mode.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::app::TerminalKind;
    ///
    /// assert_eq!(TerminalKind::Cli.as_config_terminal(), "cli");
    /// assert_eq!(TerminalKind::Repl.as_config_terminal(), "repl");
    /// ```
    pub fn as_config_terminal(self) -> &'static str {
        match self {
            TerminalKind::Cli => "cli",
            TerminalKind::Repl => "repl",
        }
    }
}

/// Startup-time selection inputs that shape runtime config resolution.
///
/// This keeps the profile override and terminal identity together so later
/// runtime rebuilds can resolve config against the same host context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeContext {
    profile_override: Option<String>,
    terminal_kind: TerminalKind,
    terminal_env: Option<String>,
}

impl RuntimeContext {
    /// Creates a runtime context, normalizing the optional profile override.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::app::{RuntimeContext, TerminalKind};
    ///
    /// let ctx = RuntimeContext::new(
    ///     Some("  Work  ".to_string()),
    ///     TerminalKind::Repl,
    ///     Some("xterm-256color".to_string()),
    /// );
    ///
    /// assert_eq!(ctx.profile_override(), Some("work"));
    /// assert_eq!(ctx.terminal_kind(), TerminalKind::Repl);
    /// assert_eq!(ctx.terminal_env(), Some("xterm-256color"));
    /// ```
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

    /// Returns the normalized profile override, if one was supplied.
    pub fn profile_override(&self) -> Option<&str> {
        self.profile_override.as_deref()
    }

    /// Returns the active terminal mode.
    pub fn terminal_kind(&self) -> TerminalKind {
        self.terminal_kind
    }

    /// Returns the detected terminal environment string, if available.
    pub fn terminal_env(&self) -> Option<&str> {
        self.terminal_env.as_deref()
    }
}

/// Holds the current resolved config plus a monotonic in-memory revision.
///
/// The revision gives caches and rebuild logic a cheap way to notice when the
/// effective config actually changed.
pub struct ConfigState {
    resolved: ResolvedConfig,
    revision: u64,
}

impl ConfigState {
    /// Creates configuration state with an initial revision of `1`.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::app::ConfigState;
    /// use osp_cli::config::{ConfigLayer, ConfigResolver, ResolveOptions};
    ///
    /// let mut defaults = ConfigLayer::default();
    /// defaults.set("profile.default", "default");
    ///
    /// let mut resolver = ConfigResolver::default();
    /// resolver.set_defaults(defaults);
    /// let resolved = resolver.resolve(ResolveOptions::default()).unwrap();
    ///
    /// let mut state = ConfigState::new(resolved.clone());
    /// assert_eq!(state.revision(), 1);
    /// assert!(!state.replace_resolved(resolved));
    /// assert_eq!(state.revision(), 1);
    /// ```
    pub fn new(resolved: ResolvedConfig) -> Self {
        Self {
            resolved,
            revision: 1,
        }
    }

    /// Returns the current resolved configuration snapshot.
    pub fn resolved(&self) -> &ResolvedConfig {
        &self.resolved
    }

    /// Returns the current configuration revision.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Replaces the resolved configuration and bumps the revision when it changes.
    pub fn replace_resolved(&mut self, next: ResolvedConfig) -> bool {
        if self.resolved == next {
            return false;
        }

        self.resolved = next;
        self.revision += 1;
        true
    }

    /// Applies a configuration transform atomically against the current snapshot.
    pub fn transaction<F, E>(&mut self, mutator: F) -> Result<bool, E>
    where
        F: FnOnce(&ResolvedConfig) -> Result<ResolvedConfig, E>,
    {
        let current = self.resolved.clone();
        let candidate = mutator(&current)?;
        Ok(self.replace_resolved(candidate))
    }
}

/// Derived presentation/runtime state for the active config snapshot.
///
/// This is cached host state, not a second source of truth. Recompute it when
/// the resolved config changes so renderers and message surfaces all read the
/// same derived values.
#[derive(Debug, Clone)]
#[non_exhaustive]
#[must_use]
pub struct UiState {
    /// Render settings derived from the current config snapshot.
    pub render_settings: RenderSettings,
    /// Default message verbosity derived from the current runtime config.
    pub message_verbosity: MessageLevel,
    /// Numeric debug verbosity used for trace-style host output.
    pub debug_verbosity: u8,
}

impl UiState {
    /// Derives UI state from a resolved config snapshot and runtime context.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::app::{RuntimeContext, TerminalKind, UiState};
    /// use osp_cli::config::{ConfigLayer, ConfigResolver, ResolveOptions};
    ///
    /// let mut defaults = ConfigLayer::default();
    /// defaults.set("profile.default", "default");
    /// defaults.set("ui.message.verbosity", "info");
    ///
    /// let mut resolver = ConfigResolver::default();
    /// resolver.set_defaults(defaults);
    /// let config = resolver.resolve(ResolveOptions::new().with_terminal("cli")).unwrap();
    ///
    /// let ui = UiState::from_resolved_config(
    ///     &RuntimeContext::new(None, TerminalKind::Cli, Some("xterm-256color".to_string())),
    ///     &config,
    /// )
    /// .unwrap();
    ///
    /// assert_eq!(ui.message_verbosity.as_env_str(), "info");
    /// assert_eq!(ui.render_settings.runtime.terminal.as_deref(), Some("xterm-256color"));
    /// ```
    pub fn from_resolved_config(
        context: &RuntimeContext,
        config: &ResolvedConfig,
    ) -> miette::Result<Self> {
        let themes = crate::ui::theme_loader::load_theme_catalog(config);
        crate::app::assembly::derive_ui_state(
            context,
            config,
            &themes,
            crate::app::assembly::RenderSettingsSeed::DefaultAuto,
            None,
        )
    }

    /// Creates the UI state snapshot used for one resolved config revision.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::app::UiState;
    /// use osp_cli::ui::RenderSettings;
    /// use osp_cli::ui::messages::MessageLevel;
    /// use osp_cli::core::output::OutputFormat;
    ///
    /// let ui = UiState::new(
    ///     RenderSettings::test_plain(OutputFormat::Json),
    ///     MessageLevel::Success,
    ///     2,
    /// );
    ///
    /// assert_eq!(ui.message_verbosity, MessageLevel::Success);
    /// assert_eq!(ui.debug_verbosity, 2);
    /// ```
    pub fn new(
        render_settings: RenderSettings,
        message_verbosity: MessageLevel,
        debug_verbosity: u8,
    ) -> Self {
        Self {
            render_settings,
            message_verbosity,
            debug_verbosity,
        }
    }

    /// Replaces the render-settings baseline used by this UI state.
    pub fn with_render_settings(mut self, render_settings: RenderSettings) -> Self {
        self.render_settings = render_settings;
        self
    }

    /// Replaces the message verbosity used for buffered UI messages.
    pub fn with_message_verbosity(mut self, message_verbosity: MessageLevel) -> Self {
        self.message_verbosity = message_verbosity;
        self
    }

    /// Replaces the numeric debug verbosity.
    pub fn with_debug_verbosity(mut self, debug_verbosity: u8) -> Self {
        self.debug_verbosity = debug_verbosity;
        self
    }
}

/// Startup inputs used to assemble runtime services and locate on-disk state.
///
/// This is launch-time provenance for the running host. It is kept separate
/// from [`RuntimeContext`] because callers may need to rebuild caches or plugin
/// services from the same startup inputs after config changes.
#[derive(Debug, Clone)]
#[non_exhaustive]
#[must_use]
pub struct LaunchContext {
    /// Explicit plugin directories requested by the caller at launch time.
    pub plugin_dirs: Vec<PathBuf>,
    /// Optional config-root override for runtime config discovery.
    pub config_root: Option<PathBuf>,
    /// Optional cache-root override for runtime state and caches.
    pub cache_root: Option<PathBuf>,
    /// Flags controlling which runtime config sources are consulted.
    pub runtime_load: RuntimeLoadOptions,
    /// Timestamp captured before startup work begins.
    pub startup_started_at: Instant,
}

impl LaunchContext {
    /// Creates launch-time provenance for one host bootstrap attempt.
    pub fn new(
        plugin_dirs: Vec<PathBuf>,
        config_root: Option<PathBuf>,
        cache_root: Option<PathBuf>,
        runtime_load: RuntimeLoadOptions,
    ) -> Self {
        Self {
            plugin_dirs,
            config_root,
            cache_root,
            runtime_load,
            startup_started_at: Instant::now(),
        }
    }

    /// Appends one explicit plugin directory.
    pub fn with_plugin_dir(mut self, plugin_dir: impl Into<PathBuf>) -> Self {
        self.plugin_dirs.push(plugin_dir.into());
        self
    }

    /// Replaces the explicit plugin directory list.
    pub fn with_plugin_dirs(mut self, plugin_dirs: impl IntoIterator<Item = PathBuf>) -> Self {
        self.plugin_dirs = plugin_dirs.into_iter().collect();
        self
    }

    /// Sets the config root override.
    pub fn with_config_root(mut self, config_root: Option<PathBuf>) -> Self {
        self.config_root = config_root;
        self
    }

    /// Sets the cache root override.
    pub fn with_cache_root(mut self, cache_root: Option<PathBuf>) -> Self {
        self.cache_root = cache_root;
        self
    }

    /// Replaces the runtime-load flags carried by the launch context.
    pub fn with_runtime_load(mut self, runtime_load: RuntimeLoadOptions) -> Self {
        self.runtime_load = runtime_load;
        self
    }

    /// Replaces the captured startup timestamp.
    pub fn with_startup_started_at(mut self, startup_started_at: Instant) -> Self {
        self.startup_started_at = startup_started_at;
        self
    }
}

impl Default for LaunchContext {
    fn default() -> Self {
        Self::new(Vec::new(), None, None, RuntimeLoadOptions::default())
    }
}

/// Long-lived client registries shared across command execution.
///
/// This bundles expensive or stateful clients so they do not have to be
/// recreated on every command dispatch.
///
/// Public API note: this is intentionally constructor/accessor driven. The
/// internal registries stay private so the host can grow additional cached
/// machinery without breaking callers.
#[non_exhaustive]
#[must_use]
pub struct AppClients {
    /// Plugin manager used for discovery, dispatch, and provider metadata.
    plugins: PluginManager,
    /// In-process registry of native commands.
    native_commands: NativeCommandRegistry,
    plugin_config_env: PluginConfigEnvCache,
}

impl AppClients {
    /// Creates the shared client registry used by the application.
    pub fn new(plugins: PluginManager, native_commands: NativeCommandRegistry) -> Self {
        Self {
            plugins,
            native_commands,
            plugin_config_env: PluginConfigEnvCache::default(),
        }
    }

    /// Returns the shared plugin manager.
    pub fn plugins(&self) -> &PluginManager {
        &self.plugins
    }

    /// Returns the shared registry of native commands.
    pub fn native_commands(&self) -> &NativeCommandRegistry {
        &self.native_commands
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

impl Default for AppClients {
    fn default() -> Self {
        Self::new(
            PluginManager::new(Vec::new()),
            NativeCommandRegistry::default(),
        )
    }
}

/// Runtime-scoped application state shared across commands.
///
/// This is the assembled host snapshot that command and REPL code read while
/// the process is running. The fields here are intended to move together: when
/// config changes, callers should rebuild the derived UI/auth/theme state
/// rather than mixing old and new snapshots.
///
/// Public API note: this is a host snapshot you usually receive from app
/// bootstrap, not a semantic DTO meant for arbitrary external construction.
#[non_exhaustive]
pub struct AppRuntime {
    /// Startup-time runtime identity used for config selection and rebuilds.
    pub context: RuntimeContext,
    /// Authoritative resolved config snapshot and its in-memory revision.
    pub config: ConfigState,
    /// UI-facing state derived from the current resolved config.
    pub ui: UiState,
    /// Authorization and command-visibility policy state derived from config.
    pub auth: AuthState,
    pub(crate) themes: ThemeCatalog,
    /// Launch-time inputs used to assemble caches and external services.
    pub launch: LaunchContext,
    product_defaults: ConfigLayer,
}

impl AppRuntime {
    /// Creates the runtime snapshot shared across CLI and REPL execution.
    pub(crate) fn new(
        context: RuntimeContext,
        config: ConfigState,
        ui: UiState,
        auth: AuthState,
        themes: ThemeCatalog,
        launch: LaunchContext,
    ) -> Self {
        Self {
            context,
            config,
            ui,
            auth,
            themes,
            launch,
            product_defaults: ConfigLayer::default(),
        }
    }

    /// Returns the runtime context used for config selection and rebuilds.
    pub fn context(&self) -> &RuntimeContext {
        &self.context
    }

    /// Returns the authoritative resolved-config state.
    pub fn config_state(&self) -> &ConfigState {
        &self.config
    }

    /// Returns mutable resolved-config state.
    pub fn config_state_mut(&mut self) -> &mut ConfigState {
        &mut self.config
    }

    /// Returns the UI state derived from the current config snapshot.
    pub fn ui(&self) -> &UiState {
        &self.ui
    }

    /// Returns mutable UI state for in-process adjustments.
    pub fn ui_mut(&mut self) -> &mut UiState {
        &mut self.ui
    }

    /// Returns the command-visibility/auth state.
    pub fn auth(&self) -> &AuthState {
        &self.auth
    }

    /// Returns mutable command-visibility/auth state.
    pub fn auth_mut(&mut self) -> &mut AuthState {
        &mut self.auth
    }

    /// Returns the launch-time provenance used to assemble the runtime.
    pub fn launch(&self) -> &LaunchContext {
        &self.launch
    }

    pub(crate) fn product_defaults(&self) -> &ConfigLayer {
        &self.product_defaults
    }

    pub(crate) fn set_product_defaults(&mut self, product_defaults: ConfigLayer) {
        self.product_defaults = product_defaults;
    }
}

/// Authorization and command-visibility state derived from configuration.
pub struct AuthState {
    builtins_allowlist: Option<HashSet<String>>,
    external_allowlist: Option<HashSet<String>>,
    policy_context: CommandPolicyContext,
    builtin_policy: CommandPolicyRegistry,
    external_policy: CommandPolicyRegistry,
}

impl AuthState {
    /// Builds authorization state from the resolved configuration.
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

    /// Builds authorization state and external policy from the current config
    /// and active command registries.
    pub(crate) fn from_resolved_with_external_policies(
        config: &ResolvedConfig,
        plugins: &PluginManager,
        native_commands: &NativeCommandRegistry,
    ) -> Self {
        let mut auth = Self::from_resolved(config);
        let plugin_policy = plugins.command_policy_registry();
        let external_policy =
            merge_policy_registries(plugin_policy, native_commands.command_policy_registry());
        auth.replace_external_policy(external_policy);
        auth
    }

    /// Returns the context used when evaluating command policies.
    pub fn policy_context(&self) -> &CommandPolicyContext {
        &self.policy_context
    }

    /// Replaces the context used when evaluating command policies.
    pub fn set_policy_context(&mut self, context: CommandPolicyContext) {
        self.policy_context = context;
    }

    /// Returns the policy registry for built-in commands.
    pub fn builtin_policy(&self) -> &CommandPolicyRegistry {
        &self.builtin_policy
    }

    /// Returns the mutable policy registry for built-in commands.
    pub fn builtin_policy_mut(&mut self) -> &mut CommandPolicyRegistry {
        &mut self.builtin_policy
    }

    /// Returns the policy registry for externally dispatched commands.
    pub fn external_policy(&self) -> &CommandPolicyRegistry {
        &self.external_policy
    }

    /// Returns the mutable policy registry for externally dispatched commands.
    pub fn external_policy_mut(&mut self) -> &mut CommandPolicyRegistry {
        &mut self.external_policy
    }

    /// Replaces the policy registry for externally dispatched commands.
    pub fn replace_external_policy(&mut self, registry: CommandPolicyRegistry) {
        self.external_policy = registry;
    }

    /// Evaluates access for a built-in command.
    pub fn builtin_access(&self, command: &str) -> CommandAccess {
        command_access_for(
            command,
            &self.builtins_allowlist,
            &self.builtin_policy,
            &self.policy_context,
        )
    }

    /// Evaluates access for an external command.
    pub fn external_command_access(&self, command: &str) -> CommandAccess {
        command_access_for(
            command,
            &self.external_allowlist,
            &self.external_policy,
            &self.policy_context,
        )
    }

    /// Returns whether a built-in command should be shown to the user.
    pub fn is_builtin_visible(&self, command: &str) -> bool {
        self.builtin_access(command).is_visible()
    }

    /// Returns whether an external command should be shown to the user.
    pub fn is_external_command_visible(&self, command: &str) -> bool {
        self.external_command_access(command).is_visible()
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

fn merge_policy_registries(
    mut left: CommandPolicyRegistry,
    right: CommandPolicyRegistry,
) -> CommandPolicyRegistry {
    for policy in right.entries() {
        left.register(policy.clone());
    }
    left
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
        auth.replace_external_policy(plugin_registry);

        assert!(auth.external_policy().contains(&CommandPath::new(["ldap"])));
        assert!(
            auth.external_policy_mut()
                .contains(&CommandPath::new(["ldap"]))
        );
        assert!(auth.external_command_access("ldap").is_runnable());
        assert!(auth.is_external_command_visible("ldap"));

        let hidden = auth.external_command_access("orch");
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
