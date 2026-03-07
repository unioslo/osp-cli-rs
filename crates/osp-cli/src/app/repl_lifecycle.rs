use miette::Result;

use crate::app::{
    DEFAULT_THEME_NAME, RuntimeConfigRequest, build_app_state, build_logging_config,
    effective_debug_verbosity, effective_message_verbosity, resolve_default_render_width,
    resolve_known_theme_name, resolve_runtime_config,
};
use crate::logging::init_developer_logging;
use crate::plugin_manager::PluginManager;
use crate::state::AppState;
use crate::theme_loader;

struct ReplSessionSnapshot {
    context: crate::state::RuntimeContext,
    scope: crate::state::ReplScopeStack,
    history_shell: osp_repl::HistoryShellContext,
    result_cache: std::collections::HashMap<String, Vec<osp_core::row::Row>>,
    cache_order: std::collections::VecDeque<String>,
    last_rows: Vec<osp_core::row::Row>,
    last_failure: Option<crate::state::LastFailure>,
    session_overrides: osp_config::ConfigLayer,
    launch: crate::state::LaunchContext,
}

impl ReplSessionSnapshot {
    fn capture(current: &AppState) -> Self {
        Self {
            context: current.context.clone(),
            scope: current.session.scope.clone(),
            history_shell: current.repl.history_shell.clone(),
            result_cache: current.session.result_cache.clone(),
            cache_order: current.session.cache_order.clone(),
            last_rows: current.session.last_rows.clone(),
            last_failure: current.session.last_failure.clone(),
            session_overrides: current.session.config_overrides.clone(),
            launch: current.launch.clone(),
        }
    }

    fn session_layer(&self) -> Option<osp_config::ConfigLayer> {
        (!self.session_overrides.entries().is_empty()).then(|| self.session_overrides.clone())
    }

    fn apply_to(self, next: &mut AppState) {
        next.session.config_overrides = self.session_overrides;
        next.session.scope = self.scope;
        next.session.last_rows = self.last_rows;
        next.session.last_failure = self.last_failure;
        next.session.result_cache = self.result_cache;
        next.session.cache_order = self.cache_order;
        next.repl.history_shell = self.history_shell;
        next.sync_history_shell_context();
    }
}

pub(crate) fn rebuild_repl_state(current: &AppState) -> Result<AppState> {
    let snapshot = ReplSessionSnapshot::capture(current);
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
        .with_roots(launch.config_root.clone(), launch.cache_root.clone());
    let mut next = build_app_state(
        context,
        config,
        render_settings,
        message_verbosity,
        debug_verbosity,
        plugin_manager,
        theme_catalog,
        launch,
    );
    snapshot.apply_to(&mut next);
    Ok(next)
}
