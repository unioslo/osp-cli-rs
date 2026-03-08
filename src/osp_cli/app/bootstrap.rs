use crate::osp_config::{
    ConfigLayer, ConfigValue, ResolveOptions, ResolvedConfig, RuntimeConfigPaths, RuntimeDefaults,
    RuntimeLoadOptions, build_runtime_pipeline,
};
use crate::osp_ui::messages::MessageLevel;
use miette::{Result, WrapErr};

use crate::osp_cli::cli::Cli;
use crate::osp_cli::logging::{DeveloperLoggingConfig, FileLoggingConfig, parse_level_filter};
use crate::osp_cli::state::{AppState, AppStateInit, RuntimeContext, TerminalKind};
use crate::osp_cli::ui_presentation::build_presentation_defaults_layer;

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
    )
    .wrap_err("failed to resolve config for CLI session layer")?;
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

pub(crate) fn build_app_state(init: AppStateInit) -> AppState {
    AppState::new(init)
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

pub(crate) fn message_verbosity_from_config(config: &ResolvedConfig) -> MessageLevel {
    config
        .get_string("ui.verbosity.level")
        .and_then(parse_message_level)
        .unwrap_or(MessageLevel::Success)
}

pub(crate) fn debug_verbosity_from_config(config: &ResolvedConfig) -> u8 {
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
    let base_pipeline = build_runtime_pipeline(
        defaults.to_layer(),
        None,
        &paths,
        request.runtime_load,
        None,
        request.session_layer.clone(),
    );

    let options = ResolveOptions {
        profile_override: request.profile_override,
        terminal: request.terminal,
    };

    // Presentation is compiled into a normal config layer instead of being interpreted later in
    // the UI. We first resolve the base config to discover ui.presentation through the normal
    // precedence rules, then synthesize one presentation-defaults layer, then resolve again so
    // downstream code only reads canonical keys like repl.intro and ui.help.layout.
    let base_resolved = base_pipeline
        .resolve(options.clone())
        .map_err(|err| report_std_error_with_context(err, "config resolution failed"))?;

    let presentation_layer = build_presentation_defaults_layer(&base_resolved);
    let resolved = build_runtime_pipeline(
        defaults.to_layer(),
        Some(presentation_layer),
        &paths,
        request.runtime_load,
        None,
        request.session_layer,
    )
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

#[cfg(test)]
mod tests {
    use super::{
        build_logging_config, debug_verbosity_from_config, message_verbosity_from_config,
        parse_message_level,
    };
    use crate::osp_config::{ConfigLayer, ConfigResolver, ResolveOptions};
    use crate::osp_ui::messages::MessageLevel;

    fn resolved(entries: &[(&str, &str)]) -> crate::osp_config::ResolvedConfig {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        for (key, value) in entries {
            defaults.set(*key, *value);
        }

        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        resolver
            .resolve(ResolveOptions::default().with_terminal("cli"))
            .expect("test config should resolve")
    }

    #[test]
    fn parse_message_level_accepts_warn_alias_and_rejects_unknown_values_unit() {
        assert_eq!(parse_message_level(" warn "), Some(MessageLevel::Warning));
        assert_eq!(parse_message_level("TRACE"), Some(MessageLevel::Trace));
        assert_eq!(parse_message_level("loud"), None);
    }

    #[test]
    fn debug_verbosity_from_config_clamps_string_and_integer_inputs_unit() {
        let string_config = resolved(&[("debug.level", "9")]);
        let integer_config = resolved(&[("debug.level", "-2")]);

        assert_eq!(debug_verbosity_from_config(&string_config), 3);
        assert_eq!(debug_verbosity_from_config(&integer_config), 0);
    }

    #[test]
    fn build_logging_config_ignores_blank_paths_even_when_file_logging_is_enabled_unit() {
        let config = resolved(&[
            ("log.file.enabled", "true"),
            ("log.file.level", "debug"),
            ("log.file.path", "   "),
            ("ui.verbosity.level", "warning"),
        ]);

        let logging = build_logging_config(&config, 2);
        assert!(logging.file.is_none());
        assert_eq!(
            message_verbosity_from_config(&config),
            MessageLevel::Warning
        );
        assert_eq!(logging.debug_count, 2);
    }
}
