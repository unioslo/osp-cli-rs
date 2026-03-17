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

use miette::{Result, WrapErr};

use crate::app::session::AppSessionRebuildState;
use crate::config::ConfigLayer;
use crate::core::output::OutputFormat;
use crate::native::NativeCommandRegistry;
use crate::plugin::state::PluginCommandPreferences;
use crate::ui::RenderSettings;

use super::{
    AppClients, AppRuntime, AppSession, LaunchContext, RuntimeConfigRequest, RuntimeContext,
    resolve_runtime_config,
};

/// Rebuilds runtime-derived host state while preserving the intended
/// session-scoped REPL state.
pub(crate) struct ReplStateRebuilder {
    context: RuntimeContext,
    launch: LaunchContext,
    product_defaults: ConfigLayer,
    render_settings: RenderSettings,
    native_commands: NativeCommandRegistry,
    plugin_preferences: PluginCommandPreferences,
    preserved_session: AppSessionRebuildState,
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
            preserved_session: session.capture_rebuild_state(),
        }
    }

    /// Rebuilds runtime, session, and client state using the captured inputs.
    pub(crate) fn rebuild(self) -> Result<(AppRuntime, AppSession, AppClients)> {
        tracing::debug!(
            profile_override = ?self.context.profile_override(),
            scoped = self.preserved_session.is_scoped(),
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
        let mut next =
            crate::app::AppStateBuilder::from_host_inputs(self.context, config, host_inputs)
                .with_launch(self.launch)
                .with_native_commands(self.native_commands)
                .build();
        next.runtime.set_product_defaults(self.product_defaults);
        next.session.restore_rebuild_state(self.preserved_session);
        Ok((next.runtime, next.session, next.clients))
    }
}
