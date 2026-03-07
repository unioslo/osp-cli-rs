use miette::Result;

use crate::app::{
    DEFAULT_THEME_NAME, RuntimeConfigRequest, build_app_state, build_logging_config,
    effective_debug_verbosity, effective_message_verbosity, plugin_process_timeout,
    resolve_default_render_width, resolve_known_theme_name, resolve_runtime_config,
};
use crate::logging::init_developer_logging;
use crate::plugin_manager::PluginManager;
#[cfg(test)]
use crate::state::AppState;
use crate::state::{AppClients, AppRuntime, AppSession};
use crate::theme_loader;

struct ReplSessionSnapshot {
    context: crate::state::RuntimeContext,
    scope: crate::state::ReplScopeStack,
    history_shell: osp_repl::HistoryShellContext,
    prompt_timing: crate::state::DebugTimingState,
    result_cache: std::collections::HashMap<String, Vec<osp_core::row::Row>>,
    cache_order: std::collections::VecDeque<String>,
    command_cache: std::collections::HashMap<String, crate::app::CliCommandResult>,
    command_cache_order: std::collections::VecDeque<String>,
    last_rows: Vec<osp_core::row::Row>,
    last_failure: Option<crate::state::LastFailure>,
    session_overrides: osp_config::ConfigLayer,
    launch: crate::state::LaunchContext,
}

impl ReplSessionSnapshot {
    fn capture(runtime: &AppRuntime, session: &AppSession) -> Self {
        Self {
            context: runtime.context.clone(),
            scope: session.scope.clone(),
            history_shell: session.history_shell.clone(),
            prompt_timing: session.prompt_timing.clone(),
            result_cache: session.result_cache.clone(),
            cache_order: session.cache_order.clone(),
            command_cache: session.command_cache.clone(),
            command_cache_order: session.command_cache_order.clone(),
            last_rows: session.last_rows.clone(),
            last_failure: session.last_failure.clone(),
            session_overrides: session.config_overrides.clone(),
            launch: runtime.launch.clone(),
        }
    }

    fn session_layer(&self) -> Option<osp_config::ConfigLayer> {
        (!self.session_overrides.entries().is_empty()).then(|| self.session_overrides.clone())
    }

    fn apply_to(self, next: &mut AppSession) {
        next.config_overrides = self.session_overrides;
        next.scope = self.scope;
        next.prompt_timing = self.prompt_timing;
        next.last_rows = self.last_rows;
        next.last_failure = self.last_failure;
        next.result_cache = self.result_cache;
        next.cache_order = self.cache_order;
        next.command_cache = self.command_cache;
        next.command_cache_order = self.command_cache_order;
        next.history_shell = self.history_shell;
        next.sync_history_shell_context();
    }
}

pub(crate) fn rebuild_repl_parts(
    runtime: &AppRuntime,
    session: &AppSession,
) -> Result<(AppRuntime, AppSession, AppClients)> {
    let snapshot = ReplSessionSnapshot::capture(runtime, session);
    let config = resolve_runtime_config(
        RuntimeConfigRequest::new(
            snapshot.context.profile_override().map(ToOwned::to_owned),
            Some(snapshot.context.terminal_kind().as_config_terminal()),
        )
        .with_runtime_load(snapshot.launch.runtime_load)
        .with_session_layer(snapshot.session_layer()),
    )?;
    let theme_catalog = theme_loader::load_theme_catalog(&config);
    let mut render_settings = crate::cli::default_render_settings();
    crate::cli::apply_render_settings_from_config(&mut render_settings, &config);
    render_settings.width = Some(resolve_default_render_width(&config));
    render_settings.theme_name = resolve_known_theme_name(
        config
            .get_string("theme.name")
            .unwrap_or(DEFAULT_THEME_NAME),
        &theme_catalog,
    )?;
    render_settings.theme = theme_catalog
        .resolve(&render_settings.theme_name)
        .map(|entry| entry.theme.clone());

    let message_verbosity = effective_message_verbosity(&config);
    let debug_verbosity = effective_debug_verbosity(&config);

    init_developer_logging(build_logging_config(&config, debug_verbosity));
    theme_loader::log_theme_issues(&theme_catalog.issues);

    let context = snapshot.context.clone();
    let launch = snapshot.launch.clone();
    let plugin_manager = PluginManager::new(launch.plugin_dirs.clone())
        .with_roots(launch.config_root.clone(), launch.cache_root.clone())
        .with_process_timeout(plugin_process_timeout(&config));
    let mut next = build_app_state(crate::state::AppStateInit {
        context,
        config,
        render_settings,
        message_verbosity,
        debug_verbosity,
        plugins: plugin_manager,
        themes: theme_catalog,
        launch,
    });
    snapshot.apply_to(&mut next.session);
    Ok((next.runtime, next.session, next.clients))
}

#[cfg(test)]
pub(crate) fn rebuild_repl_state(current: &AppState) -> Result<AppState> {
    let (runtime, session, clients) = rebuild_repl_parts(&current.runtime, &current.session)?;
    Ok(AppState {
        runtime,
        session,
        clients,
    })
}
