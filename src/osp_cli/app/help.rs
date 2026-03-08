use std::ffi::OsString;

use crate::osp_config::{ConfigLayer, RuntimeLoadOptions};
use crate::osp_core::output::{ColorMode, RenderMode, UnicodeMode};
use crate::osp_ui::RenderSettings;
use crate::osp_ui::theme::DEFAULT_THEME_NAME;
use clap::Parser;

use crate::osp_cli::cli::Cli;
use crate::osp_cli::theme_loader;
use crate::osp_cli::ui_presentation::{HelpLayout, UiPresentation, help_layout};

use super::{
    RuntimeConfigRequest, build_render_runtime, normalize_profile_override,
    resolve_default_render_width, resolve_known_theme_name, resolve_runtime_config,
};

#[derive(Debug, Clone, Default)]
pub(crate) struct HelpRenderOverrides {
    pub(crate) profile: Option<String>,
    pub(crate) theme: Option<String>,
    pub(crate) presentation: Option<UiPresentation>,
    pub(crate) mode: Option<RenderMode>,
    pub(crate) color: Option<ColorMode>,
    pub(crate) unicode: Option<UnicodeMode>,
    pub(crate) ascii_legacy: bool,
    pub(crate) gammel_og_bitter: bool,
    pub(crate) no_env: bool,
    pub(crate) no_config_file: bool,
}

impl HelpRenderOverrides {
    fn runtime_load_options(&self) -> RuntimeLoadOptions {
        RuntimeLoadOptions {
            include_env: !self.no_env,
            include_config_file: !self.no_config_file,
        }
    }
}

pub(crate) struct HelpRenderContext {
    pub(crate) settings: RenderSettings,
    pub(crate) layout: HelpLayout,
}

pub(crate) fn render_settings_for_help(args: &[OsString]) -> HelpRenderContext {
    let overrides = parse_help_render_overrides(args);
    let profile_override = normalize_profile_override(overrides.profile.clone());
    let help_override_layer = build_help_override_layer(&overrides);
    let config = resolve_runtime_config(
        RuntimeConfigRequest::new(profile_override, Some("cli"))
            .with_runtime_load(overrides.runtime_load_options())
            .with_session_layer(
                (!help_override_layer.entries().is_empty()).then_some(help_override_layer),
            ),
    )
    .ok();
    let default_cli = Cli::try_parse_from(["osp"]).expect("default cli parse should succeed");
    let mut settings = default_cli.render_settings();
    let mut layout = HelpLayout::Full;
    settings.runtime = build_render_runtime(std::env::var("TERM").ok().as_deref());
    if let Some(config) = config.as_ref() {
        let loaded = theme_loader::load_theme_catalog(config);
        default_cli.seed_render_settings_from_config(&mut settings, config);
        layout = help_layout(config);
        settings.width = Some(resolve_default_render_width(config));
        let selected = default_cli.selected_theme_name(config);
        settings.theme_name = resolve_known_theme_name(selected.as_str(), &loaded)
            .unwrap_or_else(|_| DEFAULT_THEME_NAME.to_string());
        settings.theme = loaded
            .resolve(&settings.theme_name)
            .map(|entry| entry.theme.clone());
    }

    HelpRenderContext { settings, layout }
}

fn build_help_override_layer(overrides: &HelpRenderOverrides) -> ConfigLayer {
    let mut layer = ConfigLayer::default();

    if let Some(theme) = overrides.theme.as_deref() {
        layer.set("theme.name", theme.trim());
    }

    if overrides.gammel_og_bitter {
        layer.set("ui.presentation", UiPresentation::Austere.as_config_value());
    } else if let Some(presentation) = overrides.presentation {
        layer.set("ui.presentation", presentation.as_config_value());
    }

    if let Some(mode) = overrides.mode {
        layer.set(
            "ui.mode",
            match mode {
                RenderMode::Auto => "auto",
                RenderMode::Plain => "plain",
                RenderMode::Rich => "rich",
            },
        );
    }

    if let Some(color) = overrides.color {
        layer.set(
            "ui.color.mode",
            match color {
                ColorMode::Auto => "auto",
                ColorMode::Always => "always",
                ColorMode::Never => "never",
            },
        );
    }

    if overrides.ascii_legacy {
        layer.set("ui.unicode.mode", "never");
    } else if let Some(unicode) = overrides.unicode {
        layer.set(
            "ui.unicode.mode",
            match unicode {
                UnicodeMode::Auto => "auto",
                UnicodeMode::Always => "always",
                UnicodeMode::Never => "never",
            },
        );
    }

    layer
}

pub(crate) fn parse_help_render_overrides(args: &[OsString]) -> HelpRenderOverrides {
    let mut out = HelpRenderOverrides::default();
    let mut iter = args
        .iter()
        .skip(1)
        .filter_map(|value| value.to_str())
        .peekable();

    while let Some(token) = iter.next() {
        if let Some(value) = token.strip_prefix("--profile=") {
            if !value.trim().is_empty() {
                out.profile = Some(value.trim().to_string());
            }
            continue;
        }
        if let Some(value) = token.strip_prefix("--theme=") {
            if !value.trim().is_empty() {
                out.theme = Some(value.trim().to_string());
            }
            continue;
        }
        if let Some(value) = token.strip_prefix("--presentation=") {
            out.presentation = UiPresentation::parse(value);
            continue;
        }
        if let Some(value) = token.strip_prefix("--mode=") {
            out.mode = parse_render_mode_arg(value);
            continue;
        }
        if let Some(value) = token.strip_prefix("--color=") {
            out.color = parse_color_mode_arg(value);
            continue;
        }
        if let Some(value) = token.strip_prefix("--unicode=") {
            out.unicode = parse_unicode_mode_arg(value);
            continue;
        }

        match token {
            "--profile" => {
                if let Some(value) = iter.peek().copied()
                    && !value.starts_with('-')
                {
                    out.profile = Some(value.to_string());
                    iter.next();
                }
            }
            "--theme" => {
                if let Some(value) = iter.peek().copied()
                    && !value.starts_with('-')
                {
                    out.theme = Some(value.to_string());
                    iter.next();
                }
            }
            "--presentation" => {
                if let Some(value) = iter.peek().copied()
                    && !value.starts_with('-')
                    && let Some(parsed) = UiPresentation::parse(value)
                {
                    out.presentation = Some(parsed);
                    iter.next();
                }
            }
            "--mode" => {
                if let Some(value) = iter.peek().copied()
                    && !value.starts_with('-')
                    && let Some(parsed) = parse_render_mode_arg(value)
                {
                    out.mode = Some(parsed);
                    iter.next();
                }
            }
            "--color" => {
                if let Some(value) = iter.peek().copied()
                    && !value.starts_with('-')
                    && let Some(parsed) = parse_color_mode_arg(value)
                {
                    out.color = Some(parsed);
                    iter.next();
                }
            }
            "--unicode" => {
                if let Some(value) = iter.peek().copied()
                    && !value.starts_with('-')
                    && let Some(parsed) = parse_unicode_mode_arg(value)
                {
                    out.unicode = Some(parsed);
                    iter.next();
                }
            }
            "--no-env" => out.no_env = true,
            "--no-config" | "--no-config-file" => out.no_config_file = true,
            "--ascii" => out.ascii_legacy = true,
            "--gammel-og-bitter" => out.gammel_og_bitter = true,
            _ => {}
        }
    }

    out
}

fn parse_render_mode_arg(value: &str) -> Option<RenderMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(RenderMode::Auto),
        "plain" => Some(RenderMode::Plain),
        "rich" => Some(RenderMode::Rich),
        _ => None,
    }
}

fn parse_color_mode_arg(value: &str) -> Option<ColorMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(ColorMode::Auto),
        "always" => Some(ColorMode::Always),
        "never" => Some(ColorMode::Never),
        _ => None,
    }
}

fn parse_unicode_mode_arg(value: &str) -> Option<UnicodeMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => Some(UnicodeMode::Auto),
        "always" => Some(UnicodeMode::Always),
        "never" => Some(UnicodeMode::Never),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        parse_color_mode_arg, parse_help_render_overrides, parse_render_mode_arg,
        parse_unicode_mode_arg, render_settings_for_help,
    };
    use crate::osp_cli::ui_presentation::HelpLayout;
    use crate::osp_core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
    use std::ffi::OsString;

    #[test]
    fn render_settings_for_help_honors_austere_override_without_external_config_unit() {
        let context = render_settings_for_help(&[
            OsString::from("osp"),
            OsString::from("--gammel-og-bitter"),
            OsString::from("--no-env"),
            OsString::from("--no-config-file"),
        ]);

        assert_eq!(context.layout, HelpLayout::Minimal);
        assert_eq!(context.settings.mode, RenderMode::Plain);
        assert_eq!(context.settings.color, ColorMode::Never);
        assert_eq!(context.settings.unicode, UnicodeMode::Never);
    }

    #[test]
    fn render_settings_for_help_applies_explicit_overrides_after_preset_unit() {
        let context = render_settings_for_help(&[
            OsString::from("osp"),
            OsString::from("--presentation"),
            OsString::from("compact"),
            OsString::from("--mode"),
            OsString::from("rich"),
            OsString::from("--color"),
            OsString::from("always"),
            OsString::from("--unicode"),
            OsString::from("always"),
            OsString::from("--no-env"),
            OsString::from("--no-config-file"),
        ]);

        assert_eq!(context.layout, HelpLayout::Compact);
        assert_eq!(context.settings.mode, RenderMode::Rich);
        assert_eq!(context.settings.color, ColorMode::Always);
        assert_eq!(context.settings.unicode, UnicodeMode::Always);
        assert_eq!(context.settings.format, OutputFormat::Auto);
    }

    #[test]
    fn parse_help_render_overrides_supports_inline_assignment_forms_unit() {
        let parsed = parse_help_render_overrides(&[
            OsString::from("osp"),
            OsString::from("--profile=prod"),
            OsString::from("--theme=nord"),
            OsString::from("--presentation=compact"),
            OsString::from("--mode=plain"),
            OsString::from("--color=always"),
            OsString::from("--unicode=never"),
        ]);

        assert_eq!(parsed.profile.as_deref(), Some("prod"));
        assert_eq!(parsed.theme.as_deref(), Some("nord"));
        assert_eq!(
            parsed.presentation,
            Some(crate::osp_cli::ui_presentation::UiPresentation::Compact)
        );
        assert_eq!(parsed.mode, Some(RenderMode::Plain));
        assert_eq!(parsed.color, Some(ColorMode::Always));
        assert_eq!(parsed.unicode, Some(UnicodeMode::Never));
    }

    #[test]
    fn parse_help_render_overrides_ignores_invalid_values_without_eating_later_flags_unit() {
        let parsed = parse_help_render_overrides(&[
            OsString::from("osp"),
            OsString::from("--presentation"),
            OsString::from("loud"),
            OsString::from("--mode=LOUD"),
            OsString::from("--color=sideways"),
            OsString::from("--unicode"),
            OsString::from("sometimes"),
            OsString::from("--profile"),
            OsString::from("dev"),
        ]);

        assert_eq!(parsed.presentation, None);
        assert_eq!(parsed.mode, None);
        assert_eq!(parsed.color, None);
        assert_eq!(parsed.unicode, None);
        assert_eq!(parsed.profile.as_deref(), Some("dev"));
    }

    #[test]
    fn help_arg_parsers_accept_case_and_whitespace_unit() {
        assert_eq!(parse_render_mode_arg(" rich "), Some(RenderMode::Rich));
        assert_eq!(parse_color_mode_arg(" WARNING "), None);
        assert_eq!(parse_color_mode_arg(" Always "), Some(ColorMode::Always));
        assert_eq!(parse_unicode_mode_arg(" Never "), Some(UnicodeMode::Never));
        assert_eq!(parse_unicode_mode_arg("maybe"), None);
    }
}
