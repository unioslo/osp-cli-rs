//! Internal assembly of host-derived runtime inputs.
//!
//! This module exists to keep config-derived host assembly on one path instead
//! of re-deriving the same UI/theme/plugin/session decisions in startup,
//! rebuild, and builder code.
//!
//! The important distinction is:
//!
//! - assembly here is pure derivation from resolved config plus launch/runtime
//!   context
//! - side effects such as logging initialization happen outside this module

use miette::Result;

use crate::config::{ConfigSource, ResolvedConfig};
use crate::plugin::PluginManager;
use crate::plugin::state::PluginCommandPreferences;
use crate::ui::RenderSettings;
use crate::ui::theme::DEFAULT_THEME_NAME;
use crate::ui::theme_catalog::ThemeCatalog;

use super::{
    AppSession, LaunchContext, RuntimeContext, TerminalKind, UiState, build_logging_config,
    build_render_runtime, debug_verbosity_from_config, message_verbosity_from_config,
    plugin_path_discovery_enabled, plugin_process_timeout, resolve_default_render_width,
    resolve_known_theme_name,
};

/// Render-settings baseline to use when deriving host-facing UI state.
pub(crate) enum RenderSettingsSeed {
    /// Start from the default auto-render baseline.
    DefaultAuto,
    /// Start from an existing settings baseline and layer config onto it.
    Existing(Box<RenderSettings>),
}

impl RenderSettingsSeed {
    fn into_settings(self, context: &RuntimeContext) -> RenderSettings {
        match self {
            Self::DefaultAuto => {
                let mut settings = crate::ui::RenderSettings::builder()
                    .with_format(crate::core::output::OutputFormat::Auto)
                    .build();
                settings.runtime = build_render_runtime(context.terminal_env());
                settings
            }
            Self::Existing(mut settings) => {
                // Rebuild paths must preserve the editor/runtime facts already
                // observed for this host, otherwise a restart can silently
                // lose TTY/Unicode/color capability state.
                if settings.runtime.terminal.is_none() {
                    settings.runtime.terminal = context.terminal_env().map(str::to_owned);
                }
                *settings
            }
        }
    }
}

/// Pure config-derived host state shared by startup, rebuild, and builders.
pub(crate) struct ResolvedHostInputs {
    pub(crate) themes: ThemeCatalog,
    pub(crate) ui: UiState,
    pub(crate) plugins: PluginManager,
    pub(crate) default_session: AppSession,
}

/// Config-derived host defaults reused when callers need only part of the
/// startup snapshot.
pub(crate) struct ResolvedHostDefaults {
    pub(crate) themes: ThemeCatalog,
    pub(crate) plugins: PluginManager,
    pub(crate) default_session: AppSession,
}

impl ResolvedHostInputs {
    /// Derives the host-facing UI/theme/plugin/session inputs from one
    /// authoritative config snapshot.
    pub(crate) fn derive(
        context: &RuntimeContext,
        config: &ResolvedConfig,
        launch: &LaunchContext,
        render_seed: RenderSettingsSeed,
        theme_name_override: Option<&str>,
        plugin_preferences_override: Option<PluginCommandPreferences>,
        session_overrides: Option<crate::config::ConfigLayer>,
    ) -> Result<Self> {
        let defaults = derive_host_defaults(
            config,
            launch,
            plugin_preferences_override,
            session_overrides,
        );
        let ui = derive_ui_state(
            context,
            config,
            &defaults.themes,
            render_seed,
            theme_name_override,
        )?;

        Ok(Self {
            themes: defaults.themes,
            ui,
            plugins: defaults.plugins,
            default_session: defaults.default_session,
        })
    }
}

pub(crate) fn derive_host_defaults(
    config: &ResolvedConfig,
    launch: &LaunchContext,
    plugin_preferences_override: Option<PluginCommandPreferences>,
    session_overrides: Option<crate::config::ConfigLayer>,
) -> ResolvedHostDefaults {
    let themes = crate::ui::theme_catalog::load_theme_catalog(config);
    let plugins = build_plugin_manager(config, launch, plugin_preferences_override.as_ref());
    let default_session = match session_overrides {
        Some(overrides) => AppSession::from_resolved_config_with_overrides(config, overrides),
        None => AppSession::from_resolved_config(config),
    };

    ResolvedHostDefaults {
        themes,
        plugins,
        default_session,
    }
}

/// Derives UI state from resolved config and a settings baseline.
pub(crate) fn derive_render_settings(
    context: &RuntimeContext,
    config: &ResolvedConfig,
    themes: &ThemeCatalog,
    render_seed: RenderSettingsSeed,
    theme_name_override: Option<&str>,
) -> Result<RenderSettings> {
    let mut render_settings = derive_base_render_settings(context, config, render_seed);
    let selected_theme = selected_theme_name(config, theme_name_override);
    render_settings.theme_name = resolve_known_theme_name(selected_theme, themes)?;
    render_settings.theme = themes
        .resolve(&render_settings.theme_name)
        .map(|entry| entry.theme.clone());
    Ok(render_settings)
}

pub(crate) fn derive_render_settings_or_fallback(
    context: &RuntimeContext,
    config: &ResolvedConfig,
    themes: &ThemeCatalog,
    render_seed: RenderSettingsSeed,
    theme_name_override: Option<&str>,
) -> RenderSettings {
    let mut render_settings = derive_base_render_settings(context, config, render_seed);
    let selected_theme = selected_theme_name(config, theme_name_override);
    render_settings.theme_name = resolve_known_theme_name(selected_theme, themes)
        .unwrap_or_else(|_| DEFAULT_THEME_NAME.to_string());
    render_settings.theme = themes
        .resolve(&render_settings.theme_name)
        .map(|entry| entry.theme.clone());
    render_settings
}

fn derive_base_render_settings(
    context: &RuntimeContext,
    config: &ResolvedConfig,
    render_seed: RenderSettingsSeed,
) -> RenderSettings {
    let mut render_settings = render_seed.into_settings(context);
    crate::ui::settings::apply_render_config_overrides(&mut render_settings, config);
    apply_repl_render_defaults(context, config, &mut render_settings);
    render_settings.width = Some(resolve_default_render_width(config));
    render_settings
}

fn selected_theme_name<'a>(
    config: &'a ResolvedConfig,
    theme_name_override: Option<&'a str>,
) -> &'a str {
    theme_name_override
        .or_else(|| config.get_string("theme.name"))
        .unwrap_or(DEFAULT_THEME_NAME)
}

/// Derives UI state from resolved config and a settings baseline.
pub(crate) fn derive_ui_state(
    context: &RuntimeContext,
    config: &ResolvedConfig,
    themes: &ThemeCatalog,
    render_seed: RenderSettingsSeed,
    theme_name_override: Option<&str>,
) -> Result<UiState> {
    let render_settings =
        derive_render_settings(context, config, themes, render_seed, theme_name_override)?;

    Ok(UiState::new(
        render_settings,
        message_verbosity_from_config(config),
        debug_verbosity_from_config(config),
    ))
}

fn apply_repl_render_defaults(
    context: &RuntimeContext,
    config: &ResolvedConfig,
    render_settings: &mut RenderSettings,
) {
    // REPL output benefits from a small left margin so intro/help/JSON payloads
    // sit under the prompt area instead of hugging the terminal edge. Keep this
    // as a terminal-specific default only when the user did not set `ui.margin`
    // explicitly.
    let margin_is_builtin_default = config
        .get_value_entry("ui.margin")
        .map(|entry| matches!(entry.source, ConfigSource::BuiltinDefaults))
        .unwrap_or(true);
    if matches!(context.terminal_kind(), TerminalKind::Repl) && margin_is_builtin_default {
        render_settings.margin = 2;
    }
}

/// Builds the config-derived plugin manager for one launch context.
pub(crate) fn build_plugin_manager(
    config: &ResolvedConfig,
    launch: &LaunchContext,
    preferences_override: Option<&PluginCommandPreferences>,
) -> PluginManager {
    let manager = PluginManager::new(launch.plugin_dirs.clone())
        .with_roots(launch.config_root.clone(), launch.cache_root.clone())
        .with_default_roots(!launch.runtime_load.is_defaults_only())
        .with_process_timeout(plugin_process_timeout(config))
        .with_path_discovery(plugin_path_discovery_enabled(config))
        .with_command_preferences(
            crate::plugin::state::PluginCommandPreferences::from_resolved(config),
        );
    if let Some(preferences) = preferences_override {
        manager.replace_command_preferences(preferences.clone());
    }
    manager
}

/// Applies side effects associated with an already-derived runtime snapshot.
pub(crate) fn apply_runtime_side_effects(
    config: &ResolvedConfig,
    debug_verbosity: u8,
    themes: &ThemeCatalog,
) {
    super::logging::init_developer_logging(build_logging_config(config, debug_verbosity));
    crate::ui::theme_catalog::log_theme_issues(&themes.issues);
}

#[cfg(test)]
mod tests {
    use super::{
        RenderSettingsSeed, ResolvedHostInputs, build_plugin_manager,
        derive_render_settings_or_fallback, derive_ui_state,
    };
    use crate::app::{LaunchContext, RuntimeContext, TerminalKind};
    use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions};

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
    fn derive_ui_state_layers_config_runtime_and_theme_selection_unit() {
        let config = resolved(&[("theme.name", "plain"), ("ui.margin", "3")]);
        let context = RuntimeContext::new(None, TerminalKind::Cli, Some("xterm".to_string()));
        let themes = crate::ui::theme_catalog::load_theme_catalog(&config);

        let ui = derive_ui_state(
            &context,
            &config,
            &themes,
            RenderSettingsSeed::DefaultAuto,
            None,
        )
        .expect("ui state should derive");

        assert_eq!(ui.render_settings.margin, 3);
        assert_eq!(ui.render_settings.theme_name, "plain");
        assert_eq!(
            ui.render_settings.runtime.terminal.as_deref(),
            Some("xterm")
        );
    }

    #[test]
    fn derive_ui_state_applies_repl_margin_default_without_affecting_cli_unit() {
        let config = resolved(&[]);
        let themes = crate::ui::theme_catalog::load_theme_catalog(&config);

        let repl = derive_ui_state(
            &RuntimeContext::new(None, TerminalKind::Repl, Some("xterm".to_string())),
            &config,
            &themes,
            RenderSettingsSeed::DefaultAuto,
            None,
        )
        .expect("repl ui state should derive");
        let cli = derive_ui_state(
            &RuntimeContext::new(None, TerminalKind::Cli, Some("xterm".to_string())),
            &config,
            &themes,
            RenderSettingsSeed::DefaultAuto,
            None,
        )
        .expect("cli ui state should derive");

        assert_eq!(repl.render_settings.margin, 2);
        assert_eq!(cli.render_settings.margin, 0);
    }

    #[test]
    fn host_inputs_derivation_reuses_one_config_path_for_ui_plugins_and_session_unit() {
        let config = resolved(&[("extensions.plugins.timeout_ms", "42")]);
        let context = RuntimeContext::new(None, TerminalKind::Cli, None);
        let launch = LaunchContext::default();

        let derived = ResolvedHostInputs::derive(
            &context,
            &config,
            &launch,
            RenderSettingsSeed::DefaultAuto,
            None,
            None,
            None,
        )
        .expect("host inputs should derive");

        assert_eq!(derived.ui.debug_verbosity, 0);
        assert_eq!(derived.plugins.process_timeout().as_millis(), 42,);
        assert!(derived.default_session.history_enabled);
    }

    #[test]
    fn build_plugin_manager_applies_launch_roots_and_preference_override_unit() {
        let config = resolved(&[]);
        let launch = LaunchContext::default();
        let mut preferences = crate::plugin::state::PluginCommandPreferences::default();
        preferences.set_provider("ldap user", "demo");

        let manager = build_plugin_manager(&config, &launch, Some(&preferences));
        assert_eq!(
            manager
                .command_preferences_snapshot()
                .preferred_provider_for("ldap user"),
            Some("demo")
        );
    }

    #[test]
    fn derive_render_settings_or_fallback_preserves_existing_seed_runtime_facts_unit() {
        let config = resolved(&[("theme.name", "missing-theme")]);
        let context = RuntimeContext::new(None, TerminalKind::Cli, Some("xterm".to_string()));
        let themes = crate::ui::theme_catalog::load_theme_catalog(&config);
        let mut existing = crate::ui::RenderSettings::default();
        existing.runtime.terminal = Some("preserved-terminal".to_string());

        let derived = derive_render_settings_or_fallback(
            &context,
            &config,
            &themes,
            RenderSettingsSeed::Existing(Box::new(existing)),
            None,
        );

        assert_eq!(
            derived.runtime.terminal.as_deref(),
            Some("preserved-terminal")
        );
        assert_eq!(derived.theme_name, crate::ui::DEFAULT_THEME_NAME);
    }
}
