use crate::config::{ConfigSource, ConfigValue, ResolvedConfig, Scope};
#[cfg(test)]
use crate::core::output::{ColorMode, RenderMode, UnicodeMode};
#[cfg(test)]
use crate::ui::chrome::SectionFrameStyle;
use crate::ui::messages::{MessageLayout, MessageLevel};
#[cfg(test)]
use crate::ui::{RenderSettings, TableBorderStyle};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UiPresentation {
    Expressive,
    Compact,
    Austere,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReplIntroStyle {
    None,
    Minimal,
    Compact,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReplInputMode {
    Auto,
    Interactive,
    Basic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HelpLayout {
    Full,
    Compact,
    Minimal,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum HelpLevel {
    None,
    Tiny,
    #[default]
    Normal,
    Verbose,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PresentationEffect {
    pub(crate) preset: UiPresentation,
    pub(crate) preset_source: ConfigSource,
    pub(crate) preset_scope: Scope,
    pub(crate) preset_origin: Option<String>,
    pub(crate) seeded_value: ConfigValue,
}

impl ReplIntroStyle {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "none" | "off" => Some(Self::None),
            "minimal" | "austere" => Some(Self::Minimal),
            "compact" => Some(Self::Compact),
            "full" => Some(Self::Full),
            _ => None,
        }
    }
}

impl HelpLevel {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "none" | "off" => Some(Self::None),
            "tiny" => Some(Self::Tiny),
            "normal" => Some(Self::Normal),
            "verbose" => Some(Self::Verbose),
            _ => None,
        }
    }
}

impl ReplInputMode {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "interactive" | "full" => Some(Self::Interactive),
            "basic" | "plain" => Some(Self::Basic),
            _ => None,
        }
    }
}

impl UiPresentation {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "expressive" => Some(Self::Expressive),
            "compact" => Some(Self::Compact),
            "austere" | "gammel-og-bitter" => Some(Self::Austere),
            _ => None,
        }
    }

    pub(crate) fn as_config_value(self) -> &'static str {
        match self {
            Self::Expressive => "expressive",
            Self::Compact => "compact",
            Self::Austere => "austere",
        }
    }
}

pub(crate) fn resolve_ui_presentation(config: &ResolvedConfig) -> UiPresentation {
    config
        .get_string("ui.presentation")
        .and_then(UiPresentation::parse)
        .unwrap_or(UiPresentation::Expressive)
}

pub(crate) fn build_presentation_defaults_layer(
    config: &ResolvedConfig,
) -> crate::config::ConfigLayer {
    let mut layer = crate::config::ConfigLayer::default();
    let presentation = resolve_ui_presentation(config);
    for key in PRESENTATION_KEYS {
        // Presentation only seeds canonical keys that are still on builtin defaults. Once a
        // concrete layer chose a value for the key, that concrete value wins and presentation no
        // longer participates for that setting.
        if config
            .get_value_entry(key)
            .map(|entry| matches!(entry.source, ConfigSource::BuiltinDefaults))
            .unwrap_or(true)
            && let Some(value) = presentation_seeded_value(presentation, key)
        {
            layer.set(*key, value);
        }
    }
    layer
}

pub(crate) fn message_layout(config: &ResolvedConfig) -> MessageLayout {
    config
        .get_string("ui.messages.layout")
        .and_then(MessageLayout::parse)
        .unwrap_or(MessageLayout::Grouped)
}

#[cfg(test)]
pub(crate) fn section_frame_style(config: &ResolvedConfig) -> SectionFrameStyle {
    config
        .get_string("ui.chrome.frame")
        .and_then(SectionFrameStyle::parse)
        .unwrap_or(SectionFrameStyle::Top)
}

pub(crate) fn repl_simple_prompt(config: &ResolvedConfig) -> bool {
    config.get_bool("repl.simple_prompt").unwrap_or(false)
}

pub(crate) fn intro_style(config: &ResolvedConfig) -> ReplIntroStyle {
    config
        .get_string("repl.intro")
        .and_then(ReplIntroStyle::parse)
        .or_else(|| {
            config.get_bool("repl.intro").map(|enabled| {
                if enabled {
                    ReplIntroStyle::Full
                } else {
                    ReplIntroStyle::None
                }
            })
        })
        .unwrap_or(ReplIntroStyle::Full)
}

pub(crate) fn intro_style_with_verbosity(
    style: ReplIntroStyle,
    verbosity: MessageLevel,
) -> ReplIntroStyle {
    let mut rank = match style {
        ReplIntroStyle::None => 0_i8,
        ReplIntroStyle::Minimal => 1,
        ReplIntroStyle::Compact => 2,
        ReplIntroStyle::Full => 3,
    };
    let delta = match verbosity {
        MessageLevel::Error | MessageLevel::Warning => -3,
        MessageLevel::Success => 0,
        MessageLevel::Info => 1,
        MessageLevel::Trace => 2,
    };
    rank = (rank + delta).clamp(0, 3);
    match rank {
        0 => ReplIntroStyle::None,
        1 => ReplIntroStyle::Minimal,
        2 => ReplIntroStyle::Compact,
        _ => ReplIntroStyle::Full,
    }
}

#[cfg(test)]
pub(crate) fn repl_intro_includes_overview(style: ReplIntroStyle) -> bool {
    matches!(style, ReplIntroStyle::Compact | ReplIntroStyle::Full)
}

pub(crate) fn repl_input_mode(config: &ResolvedConfig) -> ReplInputMode {
    config
        .get_string("repl.input_mode")
        .and_then(ReplInputMode::parse)
        .unwrap_or(ReplInputMode::Auto)
}

pub(crate) fn help_layout(config: &ResolvedConfig) -> HelpLayout {
    match resolve_ui_presentation(config) {
        UiPresentation::Expressive => HelpLayout::Full,
        UiPresentation::Compact => HelpLayout::Compact,
        UiPresentation::Austere => HelpLayout::Minimal,
    }
}

pub(crate) fn help_level(config: &ResolvedConfig, verbose: u8, quiet: u8) -> HelpLevel {
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

pub(crate) fn explain_presentation_effect(
    config: &ResolvedConfig,
    key: &str,
) -> Option<PresentationEffect> {
    let seeded_entry = config.get_value_entry(key)?;
    if !matches!(seeded_entry.source, ConfigSource::PresentationDefaults) {
        return None;
    }

    let preset_entry = config.get_value_entry("ui.presentation")?;
    let preset = config
        .get_string("ui.presentation")
        .and_then(UiPresentation::parse)?;
    let seeded_value = presentation_seeded_value(preset, key)?;

    Some(PresentationEffect {
        preset,
        preset_source: preset_entry.source,
        preset_scope: preset_entry.scope.clone(),
        preset_origin: preset_entry.origin.clone(),
        seeded_value,
    })
}

const PRESENTATION_KEYS: &[&str] = &[
    "ui.mode",
    "ui.unicode.mode",
    "ui.color.mode",
    "ui.chrome.frame",
    "ui.table.border",
    "ui.messages.layout",
    "repl.simple_prompt",
    "repl.intro",
];

fn presentation_seeded_value(presentation: UiPresentation, key: &str) -> Option<ConfigValue> {
    match key {
        "ui.mode" => match presentation {
            UiPresentation::Austere => Some(ConfigValue::from("plain")),
            UiPresentation::Compact | UiPresentation::Expressive => None,
        },
        "ui.unicode.mode" => match presentation {
            UiPresentation::Compact | UiPresentation::Austere => Some(ConfigValue::from("never")),
            UiPresentation::Expressive => None,
        },
        "ui.color.mode" => match presentation {
            UiPresentation::Austere => Some(ConfigValue::from("never")),
            UiPresentation::Compact | UiPresentation::Expressive => None,
        },
        "ui.chrome.frame" => match presentation {
            UiPresentation::Expressive => Some(ConfigValue::from("top-bottom")),
            UiPresentation::Compact => Some(ConfigValue::from("top")),
            UiPresentation::Austere => Some(ConfigValue::from("none")),
        },
        "ui.table.border" => match presentation {
            UiPresentation::Expressive => Some(ConfigValue::from("round")),
            UiPresentation::Compact | UiPresentation::Austere => Some(ConfigValue::from("square")),
        },
        "ui.messages.layout" => match presentation {
            UiPresentation::Austere => Some(ConfigValue::from("minimal")),
            UiPresentation::Compact | UiPresentation::Expressive => {
                Some(ConfigValue::from("grouped"))
            }
        },
        "repl.simple_prompt" => match presentation {
            UiPresentation::Expressive => Some(ConfigValue::Bool(false)),
            UiPresentation::Compact | UiPresentation::Austere => Some(ConfigValue::Bool(true)),
        },
        "repl.intro" => match presentation {
            UiPresentation::Austere => Some(ConfigValue::from("minimal")),
            UiPresentation::Compact => Some(ConfigValue::from("compact")),
            UiPresentation::Expressive => Some(ConfigValue::from("full")),
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions};

    fn resolved_config(
        defaults: &[(&str, &str)],
        file: &[(&str, &str)],
        session: &[(&str, &str)],
    ) -> ResolvedConfig {
        let mut resolver = ConfigResolver::default();
        let mut defaults_layer = ConfigLayer::default();
        defaults_layer.set("profile.default", "default");
        for (key, value) in defaults {
            defaults_layer.set(*key, *value);
        }
        resolver.set_defaults(defaults_layer);

        let mut file_layer = ConfigLayer::default();
        for (key, value) in file {
            file_layer.set(*key, *value);
        }
        resolver.set_file(file_layer);

        let mut session_layer = ConfigLayer::default();
        for (key, value) in session {
            session_layer.set(*key, *value);
        }
        resolver.set_session(session_layer);

        let base = resolver
            .resolve(ResolveOptions::default().with_terminal("repl"))
            .expect("base config should resolve");
        resolver.set_presentation(build_presentation_defaults_layer(&base));

        resolver
            .resolve(ResolveOptions::default().with_terminal("repl"))
            .expect("config should resolve")
    }

    fn auto_settings_from_config(config: &ResolvedConfig) -> RenderSettings {
        let mut settings = RenderSettings::test_plain(crate::core::output::OutputFormat::Auto);
        settings.mode = RenderMode::Auto;
        settings.unicode = UnicodeMode::Auto;
        settings.color = ColorMode::Auto;
        crate::cli::apply_render_settings_from_config(&mut settings, config);
        settings
    }

    #[test]
    fn parses_legacy_gammel_og_bitter_alias_unit() {
        assert_eq!(
            UiPresentation::parse("gammel-og-bitter"),
            Some(UiPresentation::Austere)
        );
    }

    #[test]
    fn presentation_presets_and_overrides_shape_runtime_defaults_unit() {
        struct Case<'a> {
            label: &'a str,
            defaults: &'a [(&'a str, &'a str)],
            file: &'a [(&'a str, &'a str)],
            session: &'a [(&'a str, &'a str)],
            mode: RenderMode,
            unicode: UnicodeMode,
            color: ColorMode,
            frame: SectionFrameStyle,
            border: TableBorderStyle,
            messages: MessageLayout,
            simple_prompt: bool,
            intro: ReplIntroStyle,
            help: HelpLayout,
        }

        let cases = [
            Case {
                label: "expressive defaults",
                defaults: &[("ui.presentation", "expressive")],
                file: &[],
                session: &[],
                mode: RenderMode::Auto,
                unicode: UnicodeMode::Auto,
                color: ColorMode::Auto,
                frame: SectionFrameStyle::TopBottom,
                border: TableBorderStyle::Round,
                messages: MessageLayout::Grouped,
                simple_prompt: false,
                intro: ReplIntroStyle::Full,
                help: HelpLayout::Full,
            },
            Case {
                label: "compact defaults",
                defaults: &[("ui.presentation", "compact")],
                file: &[],
                session: &[],
                mode: RenderMode::Auto,
                unicode: UnicodeMode::Never,
                color: ColorMode::Auto,
                frame: SectionFrameStyle::Top,
                border: TableBorderStyle::Square,
                messages: MessageLayout::Grouped,
                simple_prompt: true,
                intro: ReplIntroStyle::Compact,
                help: HelpLayout::Compact,
            },
            Case {
                label: "austere defaults",
                defaults: &[("ui.presentation", "austere")],
                file: &[],
                session: &[],
                mode: RenderMode::Plain,
                unicode: UnicodeMode::Never,
                color: ColorMode::Never,
                frame: SectionFrameStyle::None,
                border: TableBorderStyle::Square,
                messages: MessageLayout::Minimal,
                simple_prompt: true,
                intro: ReplIntroStyle::Minimal,
                help: HelpLayout::Minimal,
            },
            Case {
                label: "compact prompt/message overrides",
                defaults: &[("ui.presentation", "compact")],
                file: &[
                    ("repl.simple_prompt", "false"),
                    ("ui.messages.layout", "minimal"),
                ],
                session: &[],
                mode: RenderMode::Auto,
                unicode: UnicodeMode::Never,
                color: ColorMode::Auto,
                frame: SectionFrameStyle::Top,
                border: TableBorderStyle::Square,
                messages: MessageLayout::Minimal,
                simple_prompt: false,
                intro: ReplIntroStyle::Compact,
                help: HelpLayout::Compact,
            },
            Case {
                label: "austere explicit render overrides",
                defaults: &[("ui.presentation", "austere")],
                file: &[
                    ("ui.mode", "rich"),
                    ("ui.unicode.mode", "always"),
                    ("ui.color.mode", "always"),
                    ("ui.chrome.frame", "top"),
                    ("ui.table.border", "round"),
                ],
                session: &[],
                mode: RenderMode::Rich,
                unicode: UnicodeMode::Always,
                color: ColorMode::Always,
                frame: SectionFrameStyle::Top,
                border: TableBorderStyle::Round,
                messages: MessageLayout::Minimal,
                simple_prompt: true,
                intro: ReplIntroStyle::Minimal,
                help: HelpLayout::Minimal,
            },
            Case {
                label: "austere explicit intro override",
                defaults: &[("ui.presentation", "austere")],
                file: &[],
                session: &[("repl.intro", "full")],
                mode: RenderMode::Plain,
                unicode: UnicodeMode::Never,
                color: ColorMode::Never,
                frame: SectionFrameStyle::None,
                border: TableBorderStyle::Square,
                messages: MessageLayout::Minimal,
                simple_prompt: true,
                intro: ReplIntroStyle::Full,
                help: HelpLayout::Minimal,
            },
        ];

        for case in cases {
            let config = resolved_config(case.defaults, case.file, case.session);
            let settings = auto_settings_from_config(&config);

            assert_eq!(settings.mode, case.mode, "{}", case.label);
            assert_eq!(settings.unicode, case.unicode, "{}", case.label);
            assert_eq!(settings.color, case.color, "{}", case.label);
            assert_eq!(settings.chrome_frame, case.frame, "{}", case.label);
            assert_eq!(section_frame_style(&config), case.frame, "{}", case.label);
            assert_eq!(settings.table_border, case.border, "{}", case.label);
            assert_eq!(message_layout(&config), case.messages, "{}", case.label);
            assert_eq!(
                repl_simple_prompt(&config),
                case.simple_prompt,
                "{}",
                case.label
            );
            assert_eq!(intro_style(&config), case.intro, "{}", case.label);
            assert_eq!(help_layout(&config), case.help, "{}", case.label);
        }
    }

    #[test]
    fn explicit_intro_and_input_mode_overrides_are_respected_unit() {
        let config = resolved_config(&[], &[("repl.intro", "none")], &[]);
        assert_eq!(intro_style(&config), ReplIntroStyle::None);

        let config = resolved_config(&[], &[], &[]);
        assert_eq!(repl_input_mode(&config), ReplInputMode::Auto);

        let config = resolved_config(&[], &[("repl.input_mode", "basic")], &[]);
        assert_eq!(repl_input_mode(&config), ReplInputMode::Basic);
    }

    #[test]
    fn intro_verbosity_adjustment_and_overview_visibility_cover_compact_and_austere_unit() {
        let compact = resolved_config(&[("ui.presentation", "compact")], &[], &[]);
        let compact_intro = intro_style(&compact);
        assert_eq!(compact_intro, ReplIntroStyle::Compact);
        assert!(repl_intro_includes_overview(intro_style_with_verbosity(
            compact_intro,
            MessageLevel::Success,
        )));

        let austere = resolved_config(&[("ui.presentation", "austere")], &[], &[]);
        let austere_intro = intro_style(&austere);
        assert_eq!(austere_intro, ReplIntroStyle::Minimal);
        for (verbosity, expected_style, expected_overview) in [
            (MessageLevel::Success, ReplIntroStyle::Minimal, false),
            (MessageLevel::Info, ReplIntroStyle::Compact, true),
            (MessageLevel::Trace, ReplIntroStyle::Full, true),
            (MessageLevel::Warning, ReplIntroStyle::None, false),
        ] {
            let adjusted = intro_style_with_verbosity(austere_intro, verbosity);
            assert_eq!(adjusted, expected_style);
            assert_eq!(repl_intro_includes_overview(adjusted), expected_overview);
        }
    }

    #[test]
    fn config_value_name_stays_canonical_unit() {
        assert_eq!(UiPresentation::Austere.as_config_value(), "austere");
        assert_eq!(
            ConfigValue::from("austere"),
            ConfigValue::String("austere".to_string())
        );
    }

    #[test]
    fn explain_presentation_effect_reports_seeded_chrome_frame_unit() {
        let config = resolved_config(&[("ui.presentation", "austere")], &[], &[]);
        let effect = explain_presentation_effect(&config, "ui.chrome.frame")
            .expect("austere should seed chrome frame");

        assert_eq!(effect.preset, UiPresentation::Austere);
        assert_eq!(effect.preset_source, ConfigSource::BuiltinDefaults);
        assert_eq!(effect.preset_scope, Scope::global());
        assert_eq!(effect.seeded_value, ConfigValue::from("none"));
    }

    #[test]
    fn explain_presentation_effect_reports_seeded_table_border_unit() {
        let config = resolved_config(&[("ui.presentation", "compact")], &[], &[]);
        assert!(explain_presentation_effect(&config, "ui.table.border").is_some());
    }

    #[test]
    fn explain_presentation_effect_respects_explicit_key_override_unit() {
        let config = resolved_config(
            &[("ui.presentation", "austere")],
            &[("ui.chrome.frame", "top-bottom")],
            &[],
        );
        assert_eq!(
            explain_presentation_effect(&config, "ui.chrome.frame"),
            None
        );
    }

    #[test]
    fn help_level_derivation_and_explicit_override_unit() {
        for (verbose, quiet, expected) in [
            (0, 0, HelpLevel::Normal),
            (1, 0, HelpLevel::Verbose),
            (2, 0, HelpLevel::Verbose),
            (0, 1, HelpLevel::Tiny),
            (0, 2, HelpLevel::None),
            (1, 1, HelpLevel::Normal),
        ] {
            assert_eq!(derived_help_level(verbose, quiet), expected);
        }

        let config = resolved_config(
            &[("ui.presentation", "expressive")],
            &[("ui.help.level", "tiny")],
            &[],
        );

        assert_eq!(help_level(&config, 1, 0), HelpLevel::Tiny);
    }
}
