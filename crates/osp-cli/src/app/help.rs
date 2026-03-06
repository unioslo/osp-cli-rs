use std::ffi::OsString;

use clap::Parser;
use osp_config::RuntimeLoadOptions;
use osp_core::output::{ColorMode, RenderMode, UnicodeMode};
use osp_ui::RenderSettings;
use osp_ui::theme::{DEFAULT_THEME_NAME, normalize_theme_name};

use crate::cli::Cli;
use crate::theme_loader;

use super::{
    RuntimeConfigRequest, build_render_runtime, normalize_profile_override,
    resolve_default_render_width, resolve_known_theme_name, resolve_runtime_config,
};

#[derive(Debug, Clone, Default)]
pub(crate) struct HelpRenderOverrides {
    pub(crate) profile: Option<String>,
    pub(crate) theme: Option<String>,
    pub(crate) mode: Option<RenderMode>,
    pub(crate) color: Option<ColorMode>,
    pub(crate) unicode: Option<UnicodeMode>,
    pub(crate) ascii_legacy: bool,
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

pub(crate) fn render_settings_for_help(args: &[OsString]) -> RenderSettings {
    let overrides = parse_help_render_overrides(args);
    let profile_override = normalize_profile_override(overrides.profile.clone());
    let config = resolve_runtime_config(
        RuntimeConfigRequest::new(profile_override, Some("cli"))
            .with_runtime_load(overrides.runtime_load_options()),
    )
    .ok();
    let mut catalog: Option<theme_loader::ThemeCatalog> = None;

    let default_cli = Cli::try_parse_from(["osp"]).expect("default cli parse should succeed");
    let mut settings = default_cli.render_settings();
    settings.runtime = build_render_runtime(std::env::var("TERM").ok().as_deref());
    if let Some(config) = config.as_ref() {
        let loaded = theme_loader::load_theme_catalog(config);
        default_cli.seed_render_settings_from_config(&mut settings, config);
        settings.width = Some(resolve_default_render_width(config));
        let selected = default_cli.selected_theme_name(config);
        settings.theme_name = resolve_known_theme_name(selected.as_str(), &loaded)
            .unwrap_or_else(|_| DEFAULT_THEME_NAME.to_string());
        settings.theme = loaded
            .resolve(&settings.theme_name)
            .map(|entry| entry.theme.clone());
        catalog = Some(loaded);
    }

    if let Some(mode) = overrides.mode {
        settings.mode = mode;
    }
    if let Some(color) = overrides.color {
        settings.color = color;
    }
    if let Some(unicode) = overrides.unicode {
        settings.unicode = unicode;
    }
    if overrides.ascii_legacy {
        settings.unicode = UnicodeMode::Never;
    }
    if let Some(theme) = overrides.theme.as_deref() {
        settings.theme_name = if let Some(catalog) = catalog.as_ref() {
            resolve_known_theme_name(theme, catalog)
                .unwrap_or_else(|_| DEFAULT_THEME_NAME.to_string())
        } else {
            normalize_theme_name(theme)
        };
    }
    settings.theme = if let Some(catalog) = catalog.as_ref() {
        catalog
            .resolve(&settings.theme_name)
            .map(|entry| entry.theme.clone())
    } else {
        Some(osp_ui::theme::resolve_theme(&settings.theme_name))
    };

    settings
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
