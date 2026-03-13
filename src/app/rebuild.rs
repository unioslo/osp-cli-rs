//! Internal rebuild authority for runtime/session state after host mutations.
//!
//! This module exists so REPL restarts after config, theme, or plugin changes
//! do not have to reassemble host state ad hoc in lifecycle code. The rebuild
//! path needs to know:
//!
//! - which session-scoped fields are preserved across a rebuild
//! - how to re-resolve config against the same runtime context
//! - how to re-derive UI, themes, and plugin state from the new config
//!
//! It does **not** own normal startup bootstrap. This is specifically the
//! derived-state rebuild path.

use std::collections::{HashMap, VecDeque};

use miette::{Result, WrapErr};

use crate::config::ConfigLayer;
use crate::core::output::OutputFormat;
use crate::core::row::Row;
use crate::native::NativeCommandRegistry;
use crate::plugin::state::PluginCommandPreferences;
use crate::repl::HistoryShellContext;
use crate::ui::RenderSettings;

use super::{
    AppClients, AppRuntime, AppSession, DebugTimingState, LastFailure, LaunchContext,
    ReplScopeStack, RuntimeConfigRequest, RuntimeContext, resolve_runtime_config,
};

#[derive(Debug)]
struct PreservedReplSessionState {
    prompt_prefix: String,
    history_enabled: bool,
    scope: ReplScopeStack,
    history_shell: HistoryShellContext,
    prompt_timing: DebugTimingState,
    startup_prompt_timing_pending: bool,
    result_cache: HashMap<String, Vec<Row>>,
    cache_order: VecDeque<String>,
    last_rows: Vec<Row>,
    last_failure: Option<LastFailure>,
    max_cached_results: usize,
    session_overrides: ConfigLayer,
}

impl PreservedReplSessionState {
    fn capture(session: &AppSession) -> Self {
        Self {
            prompt_prefix: session.prompt_prefix.clone(),
            history_enabled: session.history_enabled,
            scope: session.scope.clone(),
            history_shell: session.history_shell.clone(),
            prompt_timing: session.prompt_timing.clone(),
            startup_prompt_timing_pending: session.startup_prompt_timing_pending,
            result_cache: session.result_cache.clone(),
            cache_order: session.cache_order.clone(),
            last_rows: session.last_rows.clone(),
            last_failure: session.last_failure.clone(),
            max_cached_results: session.max_cached_results,
            session_overrides: session.config_overrides.clone(),
        }
    }

    fn session_layer(&self) -> Option<ConfigLayer> {
        (!self.session_overrides.entries().is_empty()).then(|| self.session_overrides.clone())
    }

    fn restore(self, next: &mut AppSession) {
        next.prompt_prefix = self.prompt_prefix;
        next.history_enabled = self.history_enabled;
        next.config_overrides = self.session_overrides;
        next.scope = self.scope;
        next.prompt_timing = self.prompt_timing;
        next.startup_prompt_timing_pending = self.startup_prompt_timing_pending;
        next.last_rows = self.last_rows;
        next.last_failure = self.last_failure;
        next.result_cache = self.result_cache;
        next.cache_order = self.cache_order;
        next.max_cached_results = self.max_cached_results;
        next.history_shell = self.history_shell;
        next.sync_history_shell_context();
    }
}

/// Rebuilds runtime-derived host state while preserving the intended
/// session-scoped REPL state.
pub(crate) struct ReplStateRebuilder {
    context: RuntimeContext,
    launch: LaunchContext,
    product_defaults: ConfigLayer,
    render_settings: RenderSettings,
    native_commands: NativeCommandRegistry,
    plugin_preferences: PluginCommandPreferences,
    preserved_session: PreservedReplSessionState,
}

impl ReplStateRebuilder {
    /// Captures the current rebuild inputs from the running runtime/session.
    pub(crate) fn capture(
        runtime: &AppRuntime,
        session: &AppSession,
        clients: &AppClients,
    ) -> Self {
        Self {
            context: runtime.context.clone(),
            launch: runtime.launch.clone(),
            product_defaults: runtime.product_defaults().clone(),
            render_settings: runtime.ui.render_settings.clone(),
            native_commands: clients.native_commands().clone(),
            plugin_preferences: clients.plugins().command_preferences_snapshot(),
            preserved_session: PreservedReplSessionState::capture(session),
        }
    }

    /// Rebuilds runtime, session, and client state using the captured inputs.
    pub(crate) fn rebuild(self) -> Result<(AppRuntime, AppSession, AppClients)> {
        tracing::debug!(
            profile_override = ?self.context.profile_override(),
            scoped = !self.preserved_session.scope.is_root(),
            "rebuilding REPL state after config/theme change"
        );
        let config = resolve_runtime_config(
            RuntimeConfigRequest::new(
                self.context.profile_override().map(ToOwned::to_owned),
                Some(self.context.terminal_kind().as_config_terminal()),
            )
            .with_runtime_load(self.launch.runtime_load)
            .with_product_defaults(self.product_defaults.clone())
            .with_session_layer(self.preserved_session.session_layer()),
        )
        .wrap_err("failed to resolve config for REPL rebuild")?;
        let mut render_settings = self.render_settings.clone();
        if !render_settings.format_explicit && config.get_string("ui.format").is_none() {
            render_settings.format = OutputFormat::Auto;
        }
        let host_inputs = super::assembly::ResolvedHostInputs::derive(
            &self.context,
            &config,
            &self.launch,
            super::assembly::RenderSettingsSeed::Existing(Box::new(render_settings)),
            None,
            Some(self.plugin_preferences.clone()),
            self.preserved_session.session_layer(),
        )
        .wrap_err("failed to derive host runtime inputs for REPL rebuild")?;
        let mut next = crate::app::AppStateBuilder::new(self.context, config, host_inputs.ui)
            .with_launch(self.launch)
            .with_plugins(host_inputs.plugins)
            .with_native_commands(self.native_commands)
            .with_session(host_inputs.default_session)
            .with_themes(host_inputs.themes)
            .build();
        next.runtime.set_product_defaults(self.product_defaults);
        self.preserved_session.restore(&mut next.session);
        Ok((next.runtime, next.session, next.clients))
    }
}
