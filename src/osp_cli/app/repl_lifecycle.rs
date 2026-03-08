use miette::{Result, WrapErr};

use crate::osp_cli::app::{
    DEFAULT_THEME_NAME, RuntimeConfigRequest, build_app_state, build_logging_config,
    debug_verbosity_from_config, message_verbosity_from_config, plugin_process_timeout,
    resolve_default_render_width, resolve_known_theme_name, resolve_runtime_config,
};
use crate::osp_cli::logging::init_developer_logging;
use crate::osp_cli::plugin_manager::PluginManager;
#[cfg(test)]
use crate::osp_cli::state::AppState;
use crate::osp_cli::state::{AppClients, AppRuntime, AppSession};
use crate::osp_cli::theme_loader;

struct ReplSessionSnapshot {
    context: crate::osp_cli::state::RuntimeContext,
    scope: crate::osp_cli::state::ReplScopeStack,
    history_shell: crate::osp_repl::HistoryShellContext,
    prompt_timing: crate::osp_cli::state::DebugTimingState,
    result_cache: std::collections::HashMap<String, Vec<crate::osp_core::row::Row>>,
    cache_order: std::collections::VecDeque<String>,
    command_cache: std::collections::HashMap<String, crate::osp_cli::app::CliCommandResult>,
    command_cache_order: std::collections::VecDeque<String>,
    last_rows: Vec<crate::osp_core::row::Row>,
    last_failure: Option<crate::osp_cli::state::LastFailure>,
    session_overrides: crate::osp_config::ConfigLayer,
    launch: crate::osp_cli::state::LaunchContext,
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

    fn session_layer(&self) -> Option<crate::osp_config::ConfigLayer> {
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
    tracing::debug!(
        profile_override = ?snapshot.context.profile_override(),
        scoped = !snapshot.scope.is_root(),
        "rebuilding REPL state after config/theme change"
    );
    let config = resolve_runtime_config(
        RuntimeConfigRequest::new(
            snapshot.context.profile_override().map(ToOwned::to_owned),
            Some(snapshot.context.terminal_kind().as_config_terminal()),
        )
        .with_runtime_load(snapshot.launch.runtime_load)
        .with_session_layer(snapshot.session_layer()),
    )
    .wrap_err("failed to resolve config for REPL rebuild")?;
    let theme_catalog = theme_loader::load_theme_catalog(&config);
    let mut render_settings = crate::osp_cli::cli::default_render_settings();
    crate::osp_cli::cli::apply_render_settings_from_config(&mut render_settings, &config);
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

    let message_verbosity = message_verbosity_from_config(&config);
    let debug_verbosity = debug_verbosity_from_config(&config);

    init_developer_logging(build_logging_config(&config, debug_verbosity));
    theme_loader::log_theme_issues(&theme_catalog.issues);

    let context = snapshot.context.clone();
    let launch = snapshot.launch.clone();
    let plugin_manager = PluginManager::new(launch.plugin_dirs.clone())
        .with_roots(launch.config_root.clone(), launch.cache_root.clone())
        .with_process_timeout(plugin_process_timeout(&config));
    let mut next = build_app_state(crate::osp_cli::state::AppStateInit {
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
