use miette::Result;
use osp_config::{
    ConfigLayer, ConfigValue, ResolveOptions, ResolvedConfig, RuntimeConfigPaths, RuntimeDefaults,
    RuntimeLoadOptions, build_runtime_pipeline,
};
use osp_ui::RenderSettings;
use osp_ui::messages::MessageLevel;

use crate::cli::Cli;
use crate::logging::{DeveloperLoggingConfig, FileLoggingConfig, parse_level_filter};
use crate::plugin_manager::PluginManager;
use crate::state::{AppState, AppStateInit, LaunchContext, RuntimeContext, TerminalKind};
use crate::theme_loader::ThemeCatalog;

use super::{DEFAULT_REPL_PROMPT, DEFAULT_THEME_NAME, report_std_error_with_context};

#[derive(Debug, Clone)]
pub(crate) struct RuntimeConfigRequest {
    pub(crate) profile_override: Option<String>,
    pub(crate) terminal: Option<String>,
    pub(crate) runtime_load: RuntimeLoadOptions,
    pub(crate) session_layer: Option<ConfigLayer>,
}

impl RuntimeConfigRequest {
    pub(crate) fn new(profile_override: Option<String>, terminal: Option<&str>) -> Self {
        Self {
            profile_override,
            terminal: terminal.map(ToOwned::to_owned),
            runtime_load: RuntimeLoadOptions::default(),
            session_layer: None,
        }
    }

    pub(crate) fn with_runtime_load(mut self, runtime_load: RuntimeLoadOptions) -> Self {
        self.runtime_load = runtime_load;
        self
    }

    pub(crate) fn with_session_layer(mut self, session_layer: Option<ConfigLayer>) -> Self {
        self.session_layer = session_layer;
        self
    }
}

pub(crate) fn build_cli_session_layer(
    cli: &Cli,
    profile_override: Option<String>,
    terminal_kind: TerminalKind,
    runtime_load: RuntimeLoadOptions,
) -> Result<Option<ConfigLayer>> {
    let mut layer = ConfigLayer::default();
    cli.append_static_session_overrides(&mut layer);
    let static_override_count = layer.entries().len();
    let bootstrap_layer = if layer.entries().is_empty() {
        None
    } else {
        Some(layer.clone())
    };
    let has_bootstrap_layer = bootstrap_layer.is_some();
    let _config = resolve_runtime_config(
        RuntimeConfigRequest::new(
            profile_override.clone(),
            Some(terminal_kind.as_config_terminal()),
        )
        .with_runtime_load(runtime_load)
        .with_session_layer(bootstrap_layer),
    )?;
    tracing::debug!(
        profile_override = ?profile_override,
        terminal = %terminal_kind.as_config_terminal(),
        static_override_count,
        has_bootstrap_layer,
        "built CLI session layer"
    );

    Ok((!layer.entries().is_empty()).then_some(layer))
}

pub(crate) fn build_runtime_context(
    profile_override: Option<String>,
    terminal_kind: TerminalKind,
) -> RuntimeContext {
    RuntimeContext::new(profile_override, terminal_kind, std::env::var("TERM").ok())
}

pub(crate) fn build_app_state(
    context: RuntimeContext,
    config: ResolvedConfig,
    render_settings: RenderSettings,
    message_verbosity: MessageLevel,
    debug_verbosity: u8,
    plugins: PluginManager,
    themes: ThemeCatalog,
    launch: LaunchContext,
) -> AppState {
    AppState::new(AppStateInit {
        context,
        config,
        render_settings,
        message_verbosity,
        debug_verbosity,
        plugins,
        themes,
        launch,
    })
}

pub(crate) fn build_logging_config(
    config: &ResolvedConfig,
    debug_verbosity: u8,
) -> DeveloperLoggingConfig {
    let file = if config.get_bool("log.file.enabled").unwrap_or(false) {
        let level = config
            .get_string("log.file.level")
            .and_then(parse_level_filter)
            .or_else(|| parse_level_filter("warn"))
            .unwrap_or(tracing_subscriber::filter::LevelFilter::WARN);
        let path = config
            .get_string("log.file.path")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(std::path::PathBuf::from);
        path.map(|path| FileLoggingConfig { path, level })
    } else {
        None
    };

    DeveloperLoggingConfig {
        debug_count: debug_verbosity,
        file,
    }
}

pub(crate) fn effective_message_verbosity(config: &ResolvedConfig) -> MessageLevel {
    config
        .get_string("ui.verbosity.level")
        .and_then(parse_message_level)
        .unwrap_or(MessageLevel::Success)
}

pub(crate) fn effective_debug_verbosity(config: &ResolvedConfig) -> u8 {
    match config.get("debug.level").map(ConfigValue::reveal) {
        Some(ConfigValue::Integer(level)) => (*level).clamp(0, 3) as u8,
        Some(ConfigValue::String(raw)) => raw.trim().parse::<u8>().map_or(0, |level| level.min(3)),
        _ => 0,
    }
}

pub(crate) fn resolve_runtime_config(request: RuntimeConfigRequest) -> Result<ResolvedConfig> {
    let has_session_layer = request.session_layer.is_some();
    tracing::debug!(
        profile_override = ?request.profile_override,
        terminal = ?request.terminal,
        has_session_layer,
        "resolving runtime config"
    );
    let defaults = RuntimeDefaults::from_process_env(DEFAULT_THEME_NAME, DEFAULT_REPL_PROMPT);
    let paths = RuntimeConfigPaths::discover();
    let pipeline = build_runtime_pipeline(
        defaults.to_layer(),
        &paths,
        request.runtime_load,
        None,
        request.session_layer,
    );

    let options = ResolveOptions {
        profile_override: request.profile_override,
        terminal: request.terminal,
    };

    let resolved = pipeline
        .resolve(options)
        .map_err(|err| report_std_error_with_context(err, "config resolution failed"))?;

    tracing::debug!(
        active_profile = %resolved.active_profile(),
        known_profiles = resolved.known_profiles().len(),
        has_session_layer,
        "resolved runtime config"
    );

    Ok(resolved)
}

fn parse_message_level(value: &str) -> Option<MessageLevel> {
    match value.trim().to_ascii_lowercase().as_str() {
        "error" => Some(MessageLevel::Error),
        "warning" | "warn" => Some(MessageLevel::Warning),
        "success" => Some(MessageLevel::Success),
        "info" => Some(MessageLevel::Info),
        "trace" => Some(MessageLevel::Trace),
        _ => None,
    }
}
