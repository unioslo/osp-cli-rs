//! App bootstrap wiring for config, logging, and startup state.
//!
//! This module exists to gather the steps that must happen before the main host
//! can dispatch commands: resolve config, build runtime state, derive logging
//! settings, and assemble the initial app session/runtime objects.
//!
//! High-level flow:
//!
//! - convert CLI/runtime inputs into a config-resolution request
//! - resolve the effective config and derive presentation/logging state
//! - build the startup-time app runtime and session objects
//!
//! Contract:
//!
//! - bootstrap-time assembly lives here
//! - steady-state command execution belongs in `app::dispatch` and `repl`
//! - config precedence still belongs to `crate::config`, not to this module

use std::time::Instant;

use crate::config::{
    ConfigError, ConfigExplain, ConfigLayer, ConfigValue, LoaderPipeline, ResolveOptions,
    ResolvedConfig, RuntimeConfigPaths, RuntimeDefaults, RuntimeLoadOptions,
    build_runtime_pipeline,
};
use crate::ui::messages::MessageLevel;
use miette::{Result, WrapErr};

use crate::app::logging::{DeveloperLoggingConfig, FileLoggingConfig, parse_level_filter};
use crate::app::{LaunchContext, RuntimeContext, TerminalKind};
use crate::cli::Cli;
use crate::ui::build_presentation_defaults_layer;

use super::{DEFAULT_REPL_PROMPT, report_std_error_with_context};
use crate::ui::theme::DEFAULT_THEME_NAME;

#[derive(Debug, Clone)]
pub(crate) struct RuntimeConfigRequest {
    pub(crate) profile_override: Option<String>,
    pub(crate) terminal: Option<String>,
    pub(crate) runtime_load: RuntimeLoadOptions,
    pub(crate) session_layer: Option<ConfigLayer>,
    pub(crate) product_defaults: ConfigLayer,
}

impl RuntimeConfigRequest {
    pub(crate) fn new(profile_override: Option<String>, terminal: Option<&str>) -> Self {
        Self {
            profile_override,
            terminal: terminal.map(ToOwned::to_owned),
            runtime_load: RuntimeLoadOptions::default(),
            session_layer: None,
            product_defaults: ConfigLayer::default(),
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

    pub(crate) fn with_product_defaults(mut self, product_defaults: ConfigLayer) -> Self {
        self.product_defaults = product_defaults;
        self
    }
}

pub(crate) struct PreparedRuntimeConfig {
    pipeline: LoaderPipeline,
    options: ResolveOptions,
}

pub(crate) struct PreparedStartupHost {
    pub(crate) runtime_context: RuntimeContext,
    pub(crate) launch_context: LaunchContext,
    pub(crate) config: ResolvedConfig,
    pub(crate) host_inputs: crate::app::assembly::ResolvedHostInputs,
}

impl PreparedRuntimeConfig {
    pub(crate) fn resolve(self) -> std::result::Result<ResolvedConfig, ConfigError> {
        self.pipeline.resolve(self.options)
    }

    pub(crate) fn explain_key(self, key: &str) -> std::result::Result<ConfigExplain, ConfigError> {
        self.pipeline.resolver()?.explain_key(key, self.options)
    }
}

fn runtime_defaults_layer_with_load(
    runtime_load: RuntimeLoadOptions,
    product_defaults: &ConfigLayer,
) -> ConfigLayer {
    let mut defaults =
        RuntimeDefaults::from_runtime_load(runtime_load, DEFAULT_THEME_NAME, DEFAULT_REPL_PROMPT)
            .to_layer();
    defaults.extend_from_layer(product_defaults);
    defaults
}

fn runtime_resolve_options(request: &RuntimeConfigRequest) -> ResolveOptions {
    ResolveOptions::new()
        .with_profile_override(request.profile_override.clone())
        .with_terminal_override(request.terminal.clone())
}

fn runtime_pipeline_for_request(
    request: &RuntimeConfigRequest,
    paths: &RuntimeConfigPaths,
    presentation_layer: Option<ConfigLayer>,
) -> LoaderPipeline {
    build_runtime_pipeline(
        runtime_defaults_layer_with_load(request.runtime_load, &request.product_defaults),
        presentation_layer,
        paths,
        request.runtime_load,
        None,
        request.session_layer.clone(),
    )
}

pub(crate) fn prepare_runtime_config(
    request: RuntimeConfigRequest,
) -> std::result::Result<PreparedRuntimeConfig, ConfigError> {
    let paths = RuntimeConfigPaths::discover_with(request.runtime_load);
    let options = runtime_resolve_options(&request);
    let base_resolved =
        runtime_pipeline_for_request(&request, &paths, None).resolve(options.clone())?;
    let presentation_layer = build_presentation_defaults_layer(&base_resolved);
    let pipeline = runtime_pipeline_for_request(&request, &paths, Some(presentation_layer));

    Ok(PreparedRuntimeConfig { pipeline, options })
}

pub(crate) fn build_cli_session_layer(cli: &Cli) -> Option<ConfigLayer> {
    let mut layer = ConfigLayer::default();
    cli.append_static_session_overrides(&mut layer);
    let static_override_count = layer.entries().len();
    let session_layer = (!layer.entries().is_empty()).then_some(layer);
    tracing::debug!(
        static_override_count,
        has_session_layer = session_layer.is_some(),
        "built CLI session layer"
    );

    session_layer
}

pub(crate) fn build_runtime_context(
    profile_override: Option<String>,
    terminal_kind: TerminalKind,
) -> RuntimeContext {
    RuntimeContext::new(profile_override, terminal_kind, std::env::var("TERM").ok())
}

pub(crate) fn prepare_startup_host(
    cli: &Cli,
    profile_override: Option<String>,
    terminal_kind: TerminalKind,
    run_started: Instant,
    product_defaults: &ConfigLayer,
) -> Result<PreparedStartupHost> {
    let runtime_load = cli.runtime_load_options();
    let runtime_context = build_runtime_context(profile_override, terminal_kind);
    let session_layer = build_cli_session_layer(cli);
    let launch_context = LaunchContext::default()
        .with_plugin_dirs(cli.plugin_dirs.clone())
        .with_runtime_load(runtime_load)
        .with_startup_started_at(run_started);
    let config = resolve_runtime_config(
        RuntimeConfigRequest::new(
            runtime_context.profile_override().map(ToOwned::to_owned),
            Some(runtime_context.terminal_kind().as_config_terminal()),
        )
        .with_runtime_load(launch_context.runtime_load)
        .with_product_defaults(product_defaults.clone())
        .with_session_layer(session_layer.clone()),
    )
    .wrap_err("failed to resolve config with session layer")?;
    let host_inputs = crate::app::assembly::ResolvedHostInputs::derive(
        &runtime_context,
        &config,
        &launch_context,
        crate::app::assembly::RenderSettingsSeed::DefaultAuto,
        None,
        None,
        session_layer,
    )
    .wrap_err("failed to derive host runtime inputs for startup")?;

    Ok(PreparedStartupHost {
        runtime_context,
        launch_context,
        config,
        host_inputs,
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
        path.map(|path| FileLoggingConfig::new(path, level))
    } else {
        None
    };

    DeveloperLoggingConfig::new(debug_verbosity).with_file(file)
}

pub(crate) fn message_verbosity_from_config(config: &ResolvedConfig) -> MessageLevel {
    config
        .get_string("ui.message.verbosity")
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
        bootstrap_mode = ?request.runtime_load.bootstrap_mode,
        "resolving runtime config"
    );
    let resolved = prepare_runtime_config(request)
        .and_then(PreparedRuntimeConfig::resolve)
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
    use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions};
    use crate::ui::messages::MessageLevel;

    fn resolved(entries: &[(&str, &str)]) -> crate::config::ResolvedConfig {
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
            ("ui.message.verbosity", "warning"),
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
