use std::ffi::OsString;

use crate::config::{ConfigLayer, RuntimeLoadOptions};
use crate::core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use crate::guide::HelpLevel;
use crate::ui::RenderSettings;
use crate::ui::{HelpLayout, help_layout_from_config};

use crate::ui::UiPresentation;
use crate::ui::theme_loader;

use super::{
    RuntimeConfigRequest, RuntimeContext, TerminalKind, assembly, build_render_runtime,
    normalize_profile_override, resolve_runtime_config,
};

#[derive(Debug, Clone, Default)]
pub(crate) struct HelpRenderOverrides {
    pub(crate) profile: Option<String>,
    pub(crate) theme: Option<String>,
    pub(crate) presentation: Option<UiPresentation>,
    pub(crate) format: Option<OutputFormat>,
    pub(crate) mode: Option<RenderMode>,
    pub(crate) color: Option<ColorMode>,
    pub(crate) unicode: Option<UnicodeMode>,
    pub(crate) ascii_legacy: bool,
    pub(crate) gammel_og_bitter: bool,
    pub(crate) no_env: bool,
    pub(crate) no_config_file: bool,
    pub(crate) defaults_only: bool,
    pub(crate) verbose: u8,
    pub(crate) quiet: u8,
}

impl HelpRenderOverrides {
    fn runtime_load_options(&self) -> RuntimeLoadOptions {
        crate::cli::runtime_load_options_from_flags(
            self.no_env,
            self.no_config_file,
            self.defaults_only,
        )
    }
}

pub(crate) struct HelpRenderContext {
    pub(crate) settings: RenderSettings,
    pub(crate) layout: HelpLayout,
    pub(crate) help_level: HelpLevel,
}

pub(crate) fn help_level(
    config: &crate::config::ResolvedConfig,
    verbose: u8,
    quiet: u8,
) -> HelpLevel {
    match config.get_string("ui.help.level") {
        Some(value) if value != "inherit" => {
            HelpLevel::parse(value).unwrap_or_else(|| derived_help_level(verbose, quiet))
        }
        Some(_) | None => derived_help_level(verbose, quiet),
    }
}

pub(crate) fn derived_help_level(verbose: u8, quiet: u8) -> HelpLevel {
    let rank = (2_i16 + i16::from(verbose) - i16::from(quiet)).clamp(0, 3);
    match rank {
        0 => HelpLevel::None,
        1 => HelpLevel::Tiny,
        2 => HelpLevel::Normal,
        _ => HelpLevel::Verbose,
    }
}

pub(crate) fn render_settings_for_help(
    args: &[OsString],
    product_defaults: &ConfigLayer,
) -> HelpRenderContext {
    let overrides = parse_help_render_overrides(args);
    let profile_override = normalize_profile_override(overrides.profile.clone());
    let help_override_layer = build_help_override_layer(&overrides);
    let config = resolve_runtime_config(
        RuntimeConfigRequest::new(profile_override, Some("cli"))
            .with_runtime_load(overrides.runtime_load_options())
            .with_product_defaults(product_defaults.clone())
            .with_session_layer(
                (!help_override_layer.entries().is_empty()).then_some(help_override_layer),
            ),
    )
    .ok();
    let terminal_env = std::env::var("TERM").ok();
    let runtime_context = RuntimeContext::new(None, TerminalKind::Cli, terminal_env.clone());
    let mut settings = RenderSettings::default();
    let mut layout = HelpLayout::Full;
    let effective_help_level;
    settings.runtime = build_render_runtime(terminal_env.as_deref());
    if let Some(config) = config.as_ref() {
        let loaded = theme_loader::load_theme_catalog(config);
        settings = assembly::derive_render_settings_or_fallback(
            &runtime_context,
            config,
            &loaded,
            assembly::RenderSettingsSeed::DefaultAuto,
            None,
        );
        layout = help_layout_from_config(config);
        effective_help_level = help_level(config, overrides.verbose, overrides.quiet);
    } else {
        effective_help_level = derived_help_level(overrides.verbose, overrides.quiet);
    }
    if let Some(format) = overrides.format {
        settings.format = format;
        settings.format_explicit = true;
    }

    HelpRenderContext {
        settings,
        layout,
        help_level: effective_help_level,
    }
}

fn build_help_override_layer(overrides: &HelpRenderOverrides) -> ConfigLayer {
    let mut layer = ConfigLayer::default();
    crate::cli::append_appearance_overrides(
        &mut layer,
        overrides.theme.as_deref(),
        if overrides.gammel_og_bitter {
            Some(UiPresentation::Austere)
        } else {
            overrides.presentation
        },
    );

    if let Some(mode) = overrides.mode {
        layer.set("ui.mode", mode.as_str());
    }

    if let Some(color) = overrides.color {
        layer.set("ui.color.mode", color.as_str());
    }

    if overrides.ascii_legacy {
        layer.set("ui.unicode.mode", "never");
    } else if let Some(unicode) = overrides.unicode {
        layer.set("ui.unicode.mode", unicode.as_str());
    }

    layer
}

pub(crate) fn parse_help_render_overrides(args: &[OsString]) -> HelpRenderOverrides {
    let mut out = HelpRenderOverrides::default();
    let scanned = crate::cli::invocation::scan_cli_argv(args).ok();
    if let Some(scanned) = scanned.as_ref() {
        apply_invocation_overrides(&mut out, &scanned.invocation);
    }

    let fallback_invocation_parse = scanned.is_none();
    let source = scanned
        .as_ref()
        .map_or(args, |scanned| scanned.argv.as_slice());
    let mut iter = source
        .iter()
        .skip(1)
        .filter_map(|value| value.to_str())
        .peekable();

    while let Some(token) = iter.next() {
        if fallback_invocation_parse {
            match token {
                "--guide" => {
                    out.format = Some(OutputFormat::Guide);
                    continue;
                }
                "--verbose" => {
                    out.verbose = out.verbose.saturating_add(1);
                    continue;
                }
                "--quiet" => {
                    out.quiet = out.quiet.saturating_add(1);
                    continue;
                }
                "--json" => {
                    out.format = Some(OutputFormat::Json);
                    continue;
                }
                "--table" => {
                    out.format = Some(OutputFormat::Table);
                    continue;
                }
                "--mreg" => {
                    out.format = Some(OutputFormat::Mreg);
                    continue;
                }
                "--value" => {
                    out.format = Some(OutputFormat::Value);
                    continue;
                }
                "--md" => {
                    out.format = Some(OutputFormat::Markdown);
                    continue;
                }
                token
                    if token.starts_with('-')
                        && !token.starts_with("--")
                        && token.chars().skip(1).all(|ch| matches!(ch, 'v' | 'q')) =>
                {
                    for ch in token.chars().skip(1) {
                        match ch {
                            'v' => out.verbose = out.verbose.saturating_add(1),
                            'q' => out.quiet = out.quiet.saturating_add(1),
                            _ => {}
                        }
                    }
                    continue;
                }
                _ => {}
            }

            if let Some(value) = token.strip_prefix("--format=") {
                out.format = OutputFormat::parse(value);
                continue;
            }
            if let Some(value) = token.strip_prefix("--mode=") {
                out.mode = RenderMode::parse(value);
                continue;
            }
            if let Some(value) = token.strip_prefix("--color=") {
                out.color = ColorMode::parse(value);
                continue;
            }
            if let Some(value) = token.strip_prefix("--unicode=") {
                out.unicode = UnicodeMode::parse(value);
                continue;
            }
        }

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
            "--format" if fallback_invocation_parse => {
                if let Some(value) = iter.peek().copied()
                    && !value.starts_with('-')
                    && let Some(parsed) = OutputFormat::parse(value)
                {
                    out.format = Some(parsed);
                    iter.next();
                }
            }
            "--mode" if fallback_invocation_parse => {
                if let Some(value) = iter.peek().copied()
                    && !value.starts_with('-')
                    && let Some(parsed) = RenderMode::parse(value)
                {
                    out.mode = Some(parsed);
                    iter.next();
                }
            }
            "--color" if fallback_invocation_parse => {
                if let Some(value) = iter.peek().copied()
                    && !value.starts_with('-')
                    && let Some(parsed) = ColorMode::parse(value)
                {
                    out.color = Some(parsed);
                    iter.next();
                }
            }
            "--unicode" if fallback_invocation_parse => {
                if let Some(value) = iter.peek().copied()
                    && !value.starts_with('-')
                    && let Some(parsed) = UnicodeMode::parse(value)
                {
                    out.unicode = Some(parsed);
                    iter.next();
                }
            }
            "--no-env" => out.no_env = true,
            "--no-config" | "--no-config-file" => out.no_config_file = true,
            "--defaults-only" => out.defaults_only = true,
            "--ascii" => out.ascii_legacy = true,
            "--gammel-og-bitter" => out.gammel_og_bitter = true,
            _ => {}
        }
    }

    out
}

fn apply_invocation_overrides(
    out: &mut HelpRenderOverrides,
    invocation: &crate::cli::invocation::InvocationOptions,
) {
    out.format = invocation.format;
    out.mode = invocation.mode;
    out.color = invocation.color;
    out.unicode = invocation.unicode;
    out.verbose = invocation.verbose;
    out.quiet = invocation.quiet;
}

#[cfg(test)]
mod tests {
    use super::{parse_help_render_overrides, render_settings_for_help};
    use crate::config::ConfigLayer;
    use crate::core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
    use crate::guide::HelpLevel;
    use crate::ui::HelpLayout;
    use std::ffi::OsString;

    fn help_args(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn render_settings_for_help_combines_presentation_format_and_level_overrides_unit() {
        let context = render_settings_for_help(
            &help_args(&["osp", "--gammel-og-bitter", "--no-env", "--no-config-file"]),
            &ConfigLayer::default(),
        );

        assert_eq!(context.layout, HelpLayout::Minimal);
        assert_eq!(context.help_level, HelpLevel::Normal);
        assert_eq!(context.settings.mode, RenderMode::Plain);
        assert_eq!(context.settings.color, ColorMode::Never);
        assert_eq!(context.settings.unicode, UnicodeMode::Never);
        assert_eq!(context.settings.format, OutputFormat::Auto);
        assert!(!context.settings.format_explicit);

        let context = render_settings_for_help(
            &help_args(&[
                "osp",
                "--presentation",
                "compact",
                "--mode",
                "rich",
                "--color",
                "always",
                "--unicode",
                "always",
                "--no-env",
                "--no-config-file",
            ]),
            &ConfigLayer::default(),
        );

        assert_eq!(context.layout, HelpLayout::Compact);
        assert_eq!(context.help_level, HelpLevel::Normal);
        assert_eq!(context.settings.mode, RenderMode::Rich);
        assert_eq!(context.settings.color, ColorMode::Always);
        assert_eq!(context.settings.unicode, UnicodeMode::Always);
        assert_eq!(context.settings.format, OutputFormat::Auto);
        assert!(!context.settings.format_explicit);

        let context = render_settings_for_help(
            &help_args(&["osp", "--json", "--no-env", "--no-config-file"]),
            &ConfigLayer::default(),
        );

        assert_eq!(context.settings.format, OutputFormat::Json);
        assert!(context.settings.format_explicit);
        let context = render_settings_for_help(
            &help_args(&["osp", "--guide", "--no-env", "--no-config-file"]),
            &ConfigLayer::default(),
        );

        assert_eq!(context.settings.format, OutputFormat::Guide);
        assert!(context.settings.format_explicit);

        for (args, expected_level) in [
            (
                &["osp", "-v", "--no-env", "--no-config-file"][..],
                HelpLevel::Verbose,
            ),
            (
                &["osp", "-q", "--no-env", "--no-config-file"][..],
                HelpLevel::Tiny,
            ),
            (
                &["osp", "-qq", "--no-env", "--no-config-file"][..],
                HelpLevel::None,
            ),
        ] {
            let context = render_settings_for_help(&help_args(args), &ConfigLayer::default());
            assert_eq!(context.help_level, expected_level);
        }
    }

    #[test]
    fn parse_help_render_overrides_handle_inline_assignments_invalid_values_and_flags_unit() {
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
            Some(crate::ui::UiPresentation::Compact)
        );
        assert_eq!(parsed.mode, Some(RenderMode::Plain));
        assert_eq!(parsed.color, Some(ColorMode::Always));
        assert_eq!(parsed.unicode, Some(UnicodeMode::Never));

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

        let parsed = parse_help_render_overrides(&help_args(&[
            "osp",
            "--guide",
            "--mode",
            "rich",
            "--color",
            "always",
            "--unicode",
            "never",
            "-vq",
            "--verbose",
            "--quiet",
            "--ascii",
            "--no-env",
            "--no-config",
            "--defaults-only",
        ]));
        assert_eq!(parsed.format, Some(OutputFormat::Guide));
        assert_eq!(parsed.mode, Some(RenderMode::Rich));
        assert_eq!(parsed.color, Some(ColorMode::Always));
        assert_eq!(parsed.verbose, 2);
        assert_eq!(parsed.quiet, 2);
        assert!(parsed.ascii_legacy);
        assert!(parsed.no_env);
        assert!(parsed.no_config_file);
        assert!(parsed.defaults_only);
    }

    #[test]
    fn help_arg_parsers_accept_case_whitespace_and_invalid_values_unit() {
        assert_eq!(RenderMode::parse(" rich "), Some(RenderMode::Rich));
        assert_eq!(RenderMode::parse("LOUD"), None);
        assert_eq!(ColorMode::parse(" WARNING "), None);
        assert_eq!(ColorMode::parse(" Always "), Some(ColorMode::Always));
        assert_eq!(UnicodeMode::parse(" Never "), Some(UnicodeMode::Never));
        assert_eq!(UnicodeMode::parse("maybe"), None);
    }
}
