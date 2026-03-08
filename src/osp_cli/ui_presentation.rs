use crate::osp_config::{ConfigSource, ConfigValue, ResolvedConfig, Scope};
use crate::osp_core::output::{ColorMode, RenderMode, UnicodeMode};
use crate::osp_ui::chrome::SectionFrameStyle;
use crate::osp_ui::messages::{MessageLayout, MessageLevel};
use crate::osp_ui::{RenderSettings, TableBorderStyle};

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

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PresentationEffect {
    pub(crate) preset: UiPresentation,
    pub(crate) preset_source: ConfigSource,
    pub(crate) preset_scope: Scope,
    pub(crate) preset_origin: Option<String>,
    pub(crate) effective_value: ConfigValue,
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

impl HelpLayout {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "full" => Some(Self::Full),
            "compact" => Some(Self::Compact),
            "minimal" => Some(Self::Minimal),
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

pub(crate) fn apply_presentation_to_render_settings(
    settings: &mut RenderSettings,
    config: &ResolvedConfig,
) {
    let presentation = resolve_ui_presentation(config);
    apply_presentation_preset(
        settings,
        presentation,
        is_builtin_default(config, "ui.mode"),
        is_builtin_default(config, "ui.unicode.mode"),
        is_builtin_default(config, "ui.color.mode"),
        is_builtin_default(config, "ui.chrome.frame"),
        is_builtin_default(config, "ui.table.border"),
    );
}

pub(crate) fn apply_presentation_preset(
    settings: &mut RenderSettings,
    presentation: UiPresentation,
    mode_is_default: bool,
    unicode_is_default: bool,
    color_is_default: bool,
    chrome_frame_is_default: bool,
    table_border_is_default: bool,
) {
    match presentation {
        UiPresentation::Austere => {
            if mode_is_default {
                settings.mode = RenderMode::Plain;
            }
            if unicode_is_default {
                settings.unicode = UnicodeMode::Never;
            }
            if color_is_default {
                settings.color = ColorMode::Never;
            }
        }
        UiPresentation::Compact => {
            if unicode_is_default {
                settings.unicode = UnicodeMode::Never;
            }
        }
        UiPresentation::Expressive => {}
    }

    if chrome_frame_is_default {
        settings.chrome_frame = match presentation {
            UiPresentation::Expressive => SectionFrameStyle::TopBottom,
            UiPresentation::Compact => SectionFrameStyle::Top,
            UiPresentation::Austere => SectionFrameStyle::None,
        };
    }

    if table_border_is_default {
        settings.table_border = match presentation {
            UiPresentation::Expressive => TableBorderStyle::Round,
            UiPresentation::Compact => TableBorderStyle::Square,
            UiPresentation::Austere => TableBorderStyle::Square,
        };
    }
}

pub(crate) fn effective_message_layout(config: &ResolvedConfig) -> MessageLayout {
    if !is_builtin_default(config, "ui.messages.layout") {
        return config
            .get_string("ui.messages.layout")
            .and_then(MessageLayout::parse)
            .unwrap_or(MessageLayout::Grouped);
    }

    match resolve_ui_presentation(config) {
        UiPresentation::Expressive | UiPresentation::Compact => MessageLayout::Grouped,
        UiPresentation::Austere => MessageLayout::Minimal,
    }
}

pub(crate) fn effective_repl_simple_prompt(config: &ResolvedConfig) -> bool {
    if !is_builtin_default(config, "repl.simple_prompt") {
        return config.get_bool("repl.simple_prompt").unwrap_or(false);
    }

    match resolve_ui_presentation(config) {
        UiPresentation::Expressive => false,
        UiPresentation::Compact | UiPresentation::Austere => true,
    }
}

#[cfg(test)]
pub(crate) fn effective_section_frame(config: &ResolvedConfig) -> SectionFrameStyle {
    if !is_builtin_default(config, "ui.chrome.frame") {
        return config
            .get_string("ui.chrome.frame")
            .and_then(SectionFrameStyle::parse)
            .unwrap_or(SectionFrameStyle::Top);
    }

    match resolve_ui_presentation(config) {
        UiPresentation::Expressive => SectionFrameStyle::TopBottom,
        UiPresentation::Compact => SectionFrameStyle::Top,
        UiPresentation::Austere => SectionFrameStyle::None,
    }
}

pub(crate) fn effective_repl_intro_style_for_verbosity(
    config: &ResolvedConfig,
    verbosity: MessageLevel,
) -> ReplIntroStyle {
    adjust_repl_intro_style_for_verbosity(effective_repl_intro_style(config), verbosity)
}

pub(crate) fn repl_intro_includes_overview(
    config: &ResolvedConfig,
    verbosity: MessageLevel,
) -> bool {
    matches!(
        effective_repl_intro_style_for_verbosity(config, verbosity),
        ReplIntroStyle::Compact | ReplIntroStyle::Full
    )
}

pub(crate) fn effective_repl_input_mode(config: &ResolvedConfig) -> ReplInputMode {
    config
        .get_string("repl.input_mode")
        .and_then(ReplInputMode::parse)
        .unwrap_or(ReplInputMode::Auto)
}

pub(crate) fn effective_help_layout(config: &ResolvedConfig) -> HelpLayout {
    if !is_builtin_default(config, "ui.help.layout") {
        return config
            .get_string("ui.help.layout")
            .and_then(HelpLayout::parse)
            .unwrap_or(HelpLayout::Full);
    }

    match resolve_ui_presentation(config) {
        UiPresentation::Expressive => HelpLayout::Full,
        UiPresentation::Compact => HelpLayout::Compact,
        UiPresentation::Austere => HelpLayout::Minimal,
    }
}

pub(crate) fn effective_repl_intro_style(config: &ResolvedConfig) -> ReplIntroStyle {
    if !is_builtin_default(config, "repl.intro") {
        return config
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
            .unwrap_or(ReplIntroStyle::Full);
    }

    match resolve_ui_presentation(config) {
        UiPresentation::Austere => ReplIntroStyle::Minimal,
        UiPresentation::Compact => ReplIntroStyle::Compact,
        UiPresentation::Expressive => ReplIntroStyle::Full,
    }
}

fn adjust_repl_intro_style_for_verbosity(
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

pub(crate) fn explain_presentation_effect(
    config: &ResolvedConfig,
    key: &str,
) -> Option<PresentationEffect> {
    if !is_builtin_default(config, key) {
        return None;
    }

    let preset_entry = config.get_value_entry("ui.presentation")?;
    let preset = config
        .get_string("ui.presentation")
        .and_then(UiPresentation::parse)?;
    let effective_value = presentation_seeded_value(preset, key)?;
    let raw_value = config.get(key)?;
    if raw_value.reveal() == effective_value.reveal() {
        return None;
    }

    Some(PresentationEffect {
        preset,
        preset_source: preset_entry.source,
        preset_scope: preset_entry.scope.clone(),
        preset_origin: preset_entry.origin.clone(),
        effective_value,
    })
}

pub(crate) fn is_builtin_default(config: &ResolvedConfig, key: &str) -> bool {
    config
        .get_value_entry(key)
        .map(|entry| matches!(entry.source, ConfigSource::BuiltinDefaults))
        .unwrap_or(true)
}

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
        "ui.help.layout" => match presentation {
            UiPresentation::Expressive => Some(ConfigValue::from("full")),
            UiPresentation::Compact => Some(ConfigValue::from("compact")),
            UiPresentation::Austere => Some(ConfigValue::from("minimal")),
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
    use crate::osp_config::{ConfigLayer, ConfigResolver, ResolveOptions};

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

        resolver
            .resolve(ResolveOptions::default().with_terminal("repl"))
            .expect("config should resolve")
    }

    #[test]
    fn parses_legacy_gammel_og_bitter_alias_unit() {
        assert_eq!(
            UiPresentation::parse("gammel-og-bitter"),
            Some(UiPresentation::Austere)
        );
    }

    #[test]
    fn austere_presentation_seeds_plain_render_defaults_unit() {
        let config = resolved_config(&[("ui.presentation", "austere")], &[], &[]);
        let mut settings = RenderSettings::test_plain(crate::osp_core::output::OutputFormat::Auto);
        settings.mode = RenderMode::Auto;
        settings.unicode = UnicodeMode::Auto;

        apply_presentation_to_render_settings(&mut settings, &config);

        assert_eq!(settings.mode, RenderMode::Plain);
        assert_eq!(settings.unicode, UnicodeMode::Never);
        assert_eq!(settings.color, ColorMode::Never);
        assert_eq!(settings.chrome_frame, SectionFrameStyle::None);
        assert_eq!(settings.table_border, TableBorderStyle::Square);
    }

    #[test]
    fn explicit_render_mode_beats_austere_preset_unit() {
        let config = resolved_config(
            &[("ui.presentation", "austere")],
            &[
                ("ui.mode", "rich"),
                ("ui.unicode.mode", "always"),
                ("ui.color.mode", "always"),
                ("ui.chrome.frame", "top"),
                ("ui.table.border", "round"),
            ],
            &[],
        );
        let mut settings = RenderSettings::test_plain(crate::osp_core::output::OutputFormat::Auto);
        settings.mode = RenderMode::Auto;
        settings.unicode = UnicodeMode::Auto;
        settings.color = ColorMode::Auto;

        apply_presentation_to_render_settings(&mut settings, &config);

        assert_eq!(settings.mode, RenderMode::Auto);
        assert_eq!(settings.unicode, UnicodeMode::Auto);
        assert_eq!(settings.color, ColorMode::Auto);
        assert_eq!(settings.chrome_frame, SectionFrameStyle::Top);
        assert_eq!(settings.table_border, TableBorderStyle::Square);
    }

    #[test]
    fn expressive_presentation_prefers_stronger_chrome_by_default_unit() {
        let config = resolved_config(&[("ui.presentation", "expressive")], &[], &[]);
        assert_eq!(
            effective_section_frame(&config),
            SectionFrameStyle::TopBottom
        );
        let mut settings = RenderSettings::test_plain(crate::osp_core::output::OutputFormat::Auto);
        apply_presentation_to_render_settings(&mut settings, &config);
        assert_eq!(settings.table_border, TableBorderStyle::Round);
    }

    #[test]
    fn compact_presentation_prefers_simple_prompt_and_grouped_messages_unit() {
        let config = resolved_config(&[("ui.presentation", "compact")], &[], &[]);
        let mut settings = RenderSettings::test_plain(crate::osp_core::output::OutputFormat::Auto);
        settings.unicode = UnicodeMode::Auto;

        apply_presentation_to_render_settings(&mut settings, &config);

        assert!(effective_repl_simple_prompt(&config));
        assert_eq!(effective_message_layout(&config), MessageLayout::Grouped);
        assert_eq!(settings.unicode, UnicodeMode::Never);
        assert_eq!(settings.table_border, TableBorderStyle::Square);
    }

    #[test]
    fn austere_presentation_prefers_minimal_messages_unit() {
        let config = resolved_config(&[("ui.presentation", "austere")], &[], &[]);

        assert_eq!(effective_message_layout(&config), MessageLayout::Minimal);
    }

    #[test]
    fn explicit_prompt_and_message_settings_beat_presentation_unit() {
        let config = resolved_config(
            &[("ui.presentation", "compact")],
            &[
                ("repl.simple_prompt", "false"),
                ("ui.messages.layout", "minimal"),
            ],
            &[],
        );

        assert!(!effective_repl_simple_prompt(&config));
        assert_eq!(effective_message_layout(&config), MessageLayout::Minimal);
    }

    #[test]
    fn austere_presentation_keeps_intro_by_default_unit() {
        let config = resolved_config(&[("ui.presentation", "austere")], &[], &[]);
        assert_eq!(effective_repl_intro_style(&config), ReplIntroStyle::Minimal);
    }

    #[test]
    fn explicit_intro_beats_austere_presentation_unit() {
        let config = resolved_config(
            &[("ui.presentation", "austere")],
            &[],
            &[("repl.intro", "full")],
        );
        assert_eq!(effective_repl_intro_style(&config), ReplIntroStyle::Full);
    }

    #[test]
    fn explicit_intro_style_none_disables_intro_unit() {
        let config = resolved_config(&[], &[("repl.intro", "none")], &[]);
        assert_eq!(effective_repl_intro_style(&config), ReplIntroStyle::None);
    }

    #[test]
    fn explicit_repl_input_mode_is_resolved_unit() {
        let config = resolved_config(&[], &[], &[]);
        assert_eq!(effective_repl_input_mode(&config), ReplInputMode::Auto);

        let config = resolved_config(&[], &[("repl.input_mode", "basic")], &[]);
        assert_eq!(effective_repl_input_mode(&config), ReplInputMode::Basic);
    }

    #[test]
    fn compact_presentation_prefers_compact_help_layout_unit() {
        let config = resolved_config(&[("ui.presentation", "compact")], &[], &[]);
        assert_eq!(effective_help_layout(&config), HelpLayout::Compact);
    }

    #[test]
    fn austere_presentation_prefers_minimal_help_layout_unit() {
        let config = resolved_config(&[("ui.presentation", "austere")], &[], &[]);
        assert_eq!(effective_help_layout(&config), HelpLayout::Minimal);
    }

    #[test]
    fn compact_presentation_prefers_minimal_intro_style_unit() {
        let config = resolved_config(&[("ui.presentation", "compact")], &[], &[]);
        assert_eq!(effective_repl_intro_style(&config), ReplIntroStyle::Compact);
        assert!(repl_intro_includes_overview(&config, MessageLevel::Success));
    }

    #[test]
    fn austere_presentation_hides_intro_overview_unit() {
        let config = resolved_config(&[("ui.presentation", "austere")], &[], &[]);
        assert!(!repl_intro_includes_overview(
            &config,
            MessageLevel::Success
        ));
        assert!(repl_intro_includes_overview(&config, MessageLevel::Info));
    }

    #[test]
    fn intro_verbosity_adjustment_bumps_and_suppresses_levels_unit() {
        let austere = resolved_config(&[("ui.presentation", "austere")], &[], &[]);
        assert_eq!(
            effective_repl_intro_style_for_verbosity(&austere, MessageLevel::Success),
            ReplIntroStyle::Minimal
        );
        assert_eq!(
            effective_repl_intro_style_for_verbosity(&austere, MessageLevel::Info),
            ReplIntroStyle::Compact
        );
        assert_eq!(
            effective_repl_intro_style_for_verbosity(&austere, MessageLevel::Trace),
            ReplIntroStyle::Full
        );
        assert_eq!(
            effective_repl_intro_style_for_verbosity(&austere, MessageLevel::Warning),
            ReplIntroStyle::None
        );
    }

    #[test]
    fn explicit_help_layout_beats_presentation_unit() {
        let config = resolved_config(
            &[("ui.presentation", "austere")],
            &[("ui.help.layout", "full")],
            &[],
        );
        assert_eq!(effective_help_layout(&config), HelpLayout::Full);
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
    fn explain_presentation_effect_reports_effective_help_layout_unit() {
        let config = resolved_config(
            &[("ui.presentation", "austere"), ("ui.help.layout", "full")],
            &[],
            &[],
        );
        let effect = explain_presentation_effect(&config, "ui.help.layout")
            .expect("austere should seed minimal help layout");

        assert_eq!(effect.preset, UiPresentation::Austere);
        assert_eq!(effect.preset_source, ConfigSource::BuiltinDefaults);
        assert_eq!(effect.preset_scope, Scope::global());
        assert_eq!(effect.effective_value, ConfigValue::from("minimal"));
    }

    #[test]
    fn explain_presentation_effect_hides_noop_seed_values_unit() {
        let config = resolved_config(&[("ui.presentation", "compact")], &[], &[]);
        assert_eq!(
            explain_presentation_effect(&config, "ui.table.border"),
            None
        );
    }

    #[test]
    fn explain_presentation_effect_respects_explicit_key_override_unit() {
        let config = resolved_config(
            &[("ui.presentation", "austere")],
            &[("ui.help.layout", "compact")],
            &[],
        );
        assert_eq!(explain_presentation_effect(&config, "ui.help.layout"), None);
    }
}
