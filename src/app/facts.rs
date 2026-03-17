//! Pure host-side fact derivation shared across app surfaces.
//!
//! This module owns the small projections that several host paths depend on:
//! parsing positive integer config values, resolving render/theme defaults, and
//! lowering runtime/UI state into plugin dispatch hints. Keeping those rules
//! here lets `app::host` stay focused on orchestration instead of becoming a
//! grab bag of reusable derivation helpers.

use crate::config::{ConfigValue, DEFAULT_UI_WIDTH, ResolvedConfig};
use crate::core::runtime::{RuntimeHints, RuntimeTerminalKind, UiVerbosity};
use crate::plugin::{DEFAULT_PLUGIN_PROCESS_TIMEOUT_MS, PluginDispatchContext};
use crate::ui::RenderRuntime;
use crate::ui::messages::MessageLevel;
use crate::ui::theme::normalize_theme_name;
use crate::ui::theme_catalog::ThemeCatalog;
use miette::{Result, miette};
use std::io::IsTerminal;
use terminal_size::{Width, terminal_size};

use super::external::ExternalCommandRuntime;
use super::{
    AppClients, AppRuntime, ConfigState, ResolvedInvocation, RuntimeContext, TerminalKind, UiState,
};

pub(crate) fn resolve_known_theme_name(value: &str, catalog: &ThemeCatalog) -> Result<String> {
    let normalized = normalize_theme_name(value);
    if catalog.resolve(&normalized).is_some() {
        return Ok(normalized);
    }

    let known = catalog.ids().join(", ");
    Err(miette!("unknown theme: {value}. available themes: {known}"))
}

pub(crate) fn config_usize(config: &ResolvedConfig, key: &str, fallback: usize) -> usize {
    match config.get(key).map(ConfigValue::reveal) {
        Some(ConfigValue::Integer(value)) if *value > 0 => *value as usize,
        Some(ConfigValue::String(raw)) => raw
            .trim()
            .parse::<usize>()
            .ok()
            .filter(|value| *value > 0)
            .unwrap_or(fallback),
        _ => fallback,
    }
}

pub(crate) fn plugin_process_timeout(config: &ResolvedConfig) -> std::time::Duration {
    std::time::Duration::from_millis(config_usize(
        config,
        "extensions.plugins.timeout_ms",
        DEFAULT_PLUGIN_PROCESS_TIMEOUT_MS,
    ) as u64)
}

pub(crate) fn plugin_path_discovery_enabled(config: &ResolvedConfig) -> bool {
    config
        .get_bool("extensions.plugins.discovery.path")
        .unwrap_or(false)
}

pub(crate) fn resolve_default_render_width(config: &ResolvedConfig) -> usize {
    let configured = config_usize(config, "ui.width", DEFAULT_UI_WIDTH as usize);
    if configured != DEFAULT_UI_WIDTH as usize {
        return configured;
    }

    detect_terminal_width()
        .or_else(detect_columns_env)
        .unwrap_or(configured)
}

pub(crate) fn build_render_runtime(terminal_env: Option<&str>) -> RenderRuntime {
    RenderRuntime {
        stdout_is_tty: std::io::stdout().is_terminal(),
        terminal: terminal_env.map(ToOwned::to_owned),
        no_color: std::env::var("NO_COLOR").is_ok(),
        width: detect_terminal_width().or_else(detect_columns_env),
        locale_utf8: locale_utf8_hint_from_env(),
    }
}

#[cfg(test)]
pub(crate) fn plugin_dispatch_context_for_runtime(
    runtime: &AppRuntime,
    clients: &AppClients,
    invocation: Option<&ResolvedInvocation>,
) -> PluginDispatchContext {
    build_plugin_dispatch_context(
        &runtime.context,
        &runtime.config,
        clients,
        &runtime.ui,
        invocation,
    )
}

pub(in crate::app) fn plugin_dispatch_context_for(
    runtime: &ExternalCommandRuntime<'_>,
    invocation: Option<&ResolvedInvocation>,
) -> PluginDispatchContext {
    build_plugin_dispatch_context(
        runtime.context,
        runtime.config_state,
        runtime.clients,
        runtime.ui,
        invocation,
    )
}

pub(crate) fn runtime_hints_for_runtime(runtime: &AppRuntime) -> RuntimeHints {
    runtime_hints(
        &runtime.context,
        runtime.config.resolved().active_profile(),
        &runtime.ui,
    )
}

fn build_plugin_dispatch_context(
    context: &RuntimeContext,
    config: &ConfigState,
    clients: &AppClients,
    runtime_ui: &UiState,
    invocation: Option<&ResolvedInvocation>,
) -> PluginDispatchContext {
    let config_env = clients.plugin_config_env(config);
    let ui = invocation.map(|value| &value.ui).unwrap_or(runtime_ui);
    let provider_override = invocation.and_then(|value| value.plugin_provider.clone());

    PluginDispatchContext::new(runtime_hints(
        context,
        config.resolved().active_profile(),
        ui,
    ))
    .with_shared_env(
        config_env
            .shared
            .iter()
            .map(|entry| (entry.env_key.clone(), entry.value.clone()))
            .collect::<Vec<_>>(),
    )
    .with_plugin_env(
        config_env
            .by_plugin_id
            .into_iter()
            .map(|(plugin_id, entries)| {
                (
                    plugin_id,
                    entries
                        .into_iter()
                        .map(|entry| (entry.env_key, entry.value))
                        .collect(),
                )
            })
            .collect(),
    )
    .with_provider_override(provider_override)
}

fn runtime_hints(context: &RuntimeContext, active_profile: &str, ui: &UiState) -> RuntimeHints {
    RuntimeHints::new(
        to_ui_verbosity(ui.message_verbosity),
        ui.debug_verbosity.min(3),
        ui.render_settings.format,
        ui.render_settings.color,
        ui.render_settings.unicode,
    )
    .with_profile(Some(active_profile.to_string()))
    .with_terminal(context.terminal_env().map(ToOwned::to_owned))
    .with_terminal_kind(match context.terminal_kind() {
        TerminalKind::Cli => RuntimeTerminalKind::Cli,
        TerminalKind::Repl => RuntimeTerminalKind::Repl,
    })
}

fn to_ui_verbosity(level: MessageLevel) -> UiVerbosity {
    match level {
        MessageLevel::Error => UiVerbosity::Error,
        MessageLevel::Warning => UiVerbosity::Warning,
        MessageLevel::Success => UiVerbosity::Success,
        MessageLevel::Info => UiVerbosity::Info,
        MessageLevel::Trace => UiVerbosity::Trace,
    }
}

fn detect_terminal_width() -> Option<usize> {
    if !std::io::stdout().is_terminal() {
        return None;
    }
    terminal_size()
        .map(|(Width(columns), _)| columns as usize)
        .filter(|value| *value > 0)
}

fn detect_columns_env() -> Option<usize> {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
}

fn locale_utf8_hint_from_env() -> Option<bool> {
    for key in ["LC_ALL", "LC_CTYPE", "LANG"] {
        if let Ok(value) = std::env::var(key) {
            let lower = value.to_ascii_lowercase();
            if lower.contains("utf-8") || lower.contains("utf8") {
                return Some(true);
            }
            return Some(false);
        }
    }
    None
}
