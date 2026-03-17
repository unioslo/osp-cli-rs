use crate::config::{ConfigSource, ConfigValue, ResolvedConfig, Scope};
use crate::core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use crate::core::output_model::{
    OutputItems, OutputResult, RenderRecommendation, output_items_to_rows,
};
use crate::ui::section_chrome::{RuledSectionPolicy, SectionFrameStyle};
use crate::ui::style;
use crate::ui::theme;
use crate::ui::theme::{DEFAULT_THEME_NAME, ThemeDefinition};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GuideDefaultFormat {
    #[default]
    Guide,
    Inherit,
}

impl GuideDefaultFormat {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "guide" => Some(Self::Guide),
            "inherit" | "none" => Some(Self::Inherit),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderBackend {
    Plain,
    Rich,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableBorderStyle {
    None,
    #[default]
    Square,
    Round,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableOverflow {
    None,
    Clip,
    Ellipsis,
    Wrap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HelpLayout {
    #[default]
    Full,
    Compact,
    Minimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiPresentation {
    Expressive,
    Compact,
    Austere,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PresentationEffect {
    pub preset: UiPresentation,
    pub preset_source: ConfigSource,
    pub preset_scope: Scope,
    pub preset_origin: Option<String>,
    pub seeded_value: ConfigValue,
}

impl UiPresentation {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "expressive" => Some(Self::Expressive),
            "compact" => Some(Self::Compact),
            "austere" | "gammel-og-bitter" => Some(Self::Austere),
            _ => None,
        }
    }

    pub fn as_config_value(self) -> &'static str {
        match self {
            Self::Expressive => "expressive",
            Self::Compact => "compact",
            Self::Austere => "austere",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HelpChromeSettings {
    pub table_chrome: HelpTableChrome,
    pub entry_indent: Option<usize>,
    pub entry_gap: Option<usize>,
    pub section_spacing: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HelpTableChrome {
    Inherit,
    #[default]
    None,
    Square,
    Round,
}

impl HelpTableChrome {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "inherit" => Some(Self::Inherit),
            "none" | "plain" => Some(Self::None),
            "square" | "box" | "boxed" => Some(Self::Square),
            "round" | "rounded" => Some(Self::Round),
            _ => None,
        }
    }

    pub fn resolve(self, table_border: TableBorderStyle) -> TableBorderStyle {
        match self {
            Self::Inherit => table_border,
            Self::None => TableBorderStyle::None,
            Self::Square => TableBorderStyle::Square,
            Self::Round => TableBorderStyle::Round,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RenderRuntime {
    pub stdout_is_tty: bool,
    pub terminal: Option<String>,
    pub no_color: bool,
    pub width: Option<usize>,
    pub locale_utf8: Option<bool>,
}

impl RenderRuntime {}

impl RenderRuntime {
    pub fn builder() -> RenderRuntimeBuilder {
        RenderRuntimeBuilder::default()
    }
}

#[derive(Debug, Clone, Default)]
pub struct RenderRuntimeBuilder {
    runtime: RenderRuntime,
}

impl RenderRuntimeBuilder {
    pub fn with_stdout_is_tty(mut self, stdout_is_tty: bool) -> Self {
        self.runtime.stdout_is_tty = stdout_is_tty;
        self
    }

    pub fn with_terminal(mut self, terminal: impl Into<String>) -> Self {
        self.runtime.terminal = Some(terminal.into());
        self
    }

    pub fn with_no_color(mut self, no_color: bool) -> Self {
        self.runtime.no_color = no_color;
        self
    }

    pub fn with_width(mut self, width: usize) -> Self {
        self.runtime.width = Some(width);
        self
    }

    pub fn with_locale_utf8(mut self, locale_utf8: bool) -> Self {
        self.runtime.locale_utf8 = Some(locale_utf8);
        self
    }

    pub fn build(self) -> RenderRuntime {
        self.runtime
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderSettings {
    pub format: OutputFormat,
    pub format_explicit: bool,
    pub mode: RenderMode,
    pub color: ColorMode,
    pub unicode: UnicodeMode,
    pub theme_name: String,
    pub(crate) theme: Option<ThemeDefinition>,
    pub width: Option<usize>,
    pub margin: usize,
    pub indent_size: usize,
    pub short_list_max: usize,
    pub medium_list_max: usize,
    pub grid_padding: usize,
    pub grid_columns: Option<usize>,
    pub column_weight: usize,
    pub table_overflow: TableOverflow,
    pub table_border: TableBorderStyle,
    pub style_overrides: style::StyleOverrides,
    pub help_chrome: HelpChromeSettings,
    pub mreg_stack_min_col_width: usize,
    pub mreg_stack_overflow_ratio: usize,
    pub chrome_frame: SectionFrameStyle,
    pub ruled_section_policy: RuledSectionPolicy,
    pub guide_default_format: GuideDefaultFormat,
    pub runtime: RenderRuntime,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            format: OutputFormat::Auto,
            format_explicit: false,
            mode: RenderMode::Auto,
            color: ColorMode::Auto,
            unicode: UnicodeMode::Auto,
            theme_name: DEFAULT_THEME_NAME.to_string(),
            theme: None,
            width: None,
            margin: 0,
            indent_size: 2,
            short_list_max: 1,
            medium_list_max: 5,
            grid_padding: 4,
            grid_columns: None,
            column_weight: 3,
            table_overflow: TableOverflow::Clip,
            table_border: TableBorderStyle::Square,
            style_overrides: style::StyleOverrides::default(),
            help_chrome: HelpChromeSettings::default(),
            mreg_stack_min_col_width: 10,
            mreg_stack_overflow_ratio: 200,
            chrome_frame: SectionFrameStyle::Top,
            ruled_section_policy: RuledSectionPolicy::Shared,
            guide_default_format: GuideDefaultFormat::Guide,
            runtime: RenderRuntime::default(),
        }
    }
}

impl RenderSettings {
    pub fn builder() -> RenderSettingsBuilder {
        RenderSettingsBuilder::default()
    }

    pub fn test_plain(format: OutputFormat) -> Self {
        RenderSettingsBuilder::plain(format).build()
    }

    pub fn prefers_guide_rendering(&self) -> bool {
        matches!(self.format, OutputFormat::Guide)
            || (!self.format_explicit
                && matches!(self.guide_default_format, GuideDefaultFormat::Guide))
    }
}

impl TableOverflow {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "none" | "visible" => Some(Self::None),
            "clip" | "hidden" | "crop" => Some(Self::Clip),
            "ellipsis" | "truncate" => Some(Self::Ellipsis),
            "wrap" | "wrapped" => Some(Self::Wrap),
            _ => None,
        }
    }
}

impl TableBorderStyle {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "none" | "plain" => Some(Self::None),
            "square" | "box" | "boxed" => Some(Self::Square),
            "round" | "rounded" => Some(Self::Round),
            _ => None,
        }
    }
}

pub fn help_layout_from_config(config: &ResolvedConfig) -> HelpLayout {
    help_layout_from_presentation_name(config.get_string("ui.presentation"))
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

pub(crate) fn apply_render_config_overrides(
    settings: &mut RenderSettings,
    config: &ResolvedConfig,
) {
    if let Some(value) = config.get_string("ui.format")
        && let Some(parsed) = OutputFormat::parse(value)
    {
        settings.format = parsed;
    }

    if let Some(value) = config.get_string("ui.mode")
        && let Some(parsed) = RenderMode::parse(value)
    {
        settings.mode = parsed;
    }

    if let Some(value) = config.get_string("ui.unicode.mode")
        && let Some(parsed) = UnicodeMode::parse(value)
    {
        settings.unicode = parsed;
    }

    if let Some(value) = config.get_string("ui.color.mode")
        && let Some(parsed) = ColorMode::parse(value)
    {
        settings.color = parsed;
    }

    if let Some(value) = config.get_string("ui.chrome.frame")
        && let Some(parsed) = SectionFrameStyle::parse(value)
    {
        settings.chrome_frame = parsed;
    }

    if let Some(value) = config.get_string("ui.chrome.rule_policy")
        && let Some(parsed) = RuledSectionPolicy::parse(value)
    {
        settings.ruled_section_policy = parsed;
    }

    if let Some(value) = config.get_string("ui.guide.default_format")
        && let Some(parsed) = GuideDefaultFormat::parse(value)
    {
        settings.guide_default_format = parsed;
    }

    if settings.width.is_none() {
        match config.get("ui.width").map(ConfigValue::reveal) {
            Some(ConfigValue::Integer(width)) if *width > 0 => {
                settings.width = Some(*width as usize);
            }
            Some(ConfigValue::String(raw)) => {
                if let Ok(width) = raw.trim().parse::<usize>()
                    && width > 0
                {
                    settings.width = Some(width);
                }
            }
            _ => {}
        }
    }

    sync_render_config_overrides(settings, config);
}

fn help_layout_from_presentation_name(value: Option<&str>) -> HelpLayout {
    match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
        Some("compact") => HelpLayout::Compact,
        Some("austere") | Some("gammel-og-bitter") => HelpLayout::Minimal,
        _ => HelpLayout::Full,
    }
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
            UiPresentation::Austere => Some(ConfigValue::from("austere")),
            UiPresentation::Compact => Some(ConfigValue::from("compact")),
            UiPresentation::Expressive => Some(ConfigValue::from("full")),
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

fn sync_render_config_overrides(settings: &mut RenderSettings, config: &ResolvedConfig) {
    if let Some(value) = config_int(config, "ui.margin")
        && value >= 0
    {
        settings.margin = value as usize;
    }

    if let Some(value) = config_int(config, "ui.indent")
        && value > 0
    {
        settings.indent_size = value as usize;
    }

    if let Some(value) = config_int(config, "ui.short_list_max")
        && value > 0
    {
        settings.short_list_max = value as usize;
    }

    if let Some(value) = config_int(config, "ui.medium_list_max")
        && value > 0
    {
        settings.medium_list_max = value as usize;
    }

    if let Some(value) = config_int(config, "ui.grid_padding")
        && value > 0
    {
        settings.grid_padding = value as usize;
    }

    if let Some(value) = config_int(config, "ui.grid_columns") {
        settings.grid_columns = if value > 0 {
            Some(value as usize)
        } else {
            None
        };
    }

    if let Some(value) = config_int(config, "ui.column_weight")
        && value > 0
    {
        settings.column_weight = value as usize;
    }

    if let Some(value) = config_int(config, "ui.mreg.stack_min_col_width")
        && value > 0
    {
        settings.mreg_stack_min_col_width = value as usize;
    }

    if let Some(value) = config_int(config, "ui.mreg.stack_overflow_ratio")
        && value >= 100
    {
        settings.mreg_stack_overflow_ratio = value as usize;
    }

    if let Some(value) = config.get_string("ui.table.overflow")
        && let Some(parsed) = TableOverflow::parse(value)
    {
        settings.table_overflow = parsed;
    }

    if let Some(value) = config.get_string("ui.table.border")
        && let Some(parsed) = TableBorderStyle::parse(value)
    {
        settings.table_border = parsed;
    }

    if let Some(value) = config.get_string("ui.help.table_chrome")
        && let Some(parsed) = HelpTableChrome::parse(value)
    {
        settings.help_chrome.table_chrome = parsed;
    }

    settings.help_chrome.entry_indent = config_usize_override(config, "ui.help.entry_indent");
    settings.help_chrome.entry_gap = config_usize_override(config, "ui.help.entry_gap");
    settings.help_chrome.section_spacing = config_usize_override(config, "ui.help.section_spacing");

    settings.style_overrides = style::StyleOverrides {
        text: config_non_empty_string(config, "color.text"),
        key: config_non_empty_string(config, "color.key"),
        muted: config_non_empty_string(config, "color.text.muted"),
        table_header: config_non_empty_string(config, "color.table.header"),
        mreg_key: config_non_empty_string(config, "color.mreg.key"),
        value: config_non_empty_string(config, "color.value"),
        number: config_non_empty_string(config, "color.value.number"),
        bool_true: config_non_empty_string(config, "color.value.bool_true"),
        bool_false: config_non_empty_string(config, "color.value.bool_false"),
        null_value: config_non_empty_string(config, "color.value.null"),
        ipv4: config_non_empty_string(config, "color.value.ipv4"),
        ipv6: config_non_empty_string(config, "color.value.ipv6"),
        panel_border: config_non_empty_string(config, "color.panel.border")
            .or_else(|| config_non_empty_string(config, "color.border")),
        panel_title: config_non_empty_string(config, "color.panel.title"),
        code: config_non_empty_string(config, "color.code"),
        json_key: config_non_empty_string(config, "color.json.key"),
        message_error: config_non_empty_string(config, "color.message.error"),
        message_warning: config_non_empty_string(config, "color.message.warning"),
        message_success: config_non_empty_string(config, "color.message.success"),
        message_info: config_non_empty_string(config, "color.message.info"),
        message_trace: config_non_empty_string(config, "color.message.trace"),
    };
}

fn config_int(config: &ResolvedConfig, key: &str) -> Option<i64> {
    match config.get(key).map(ConfigValue::reveal) {
        Some(ConfigValue::Integer(value)) => Some(*value),
        Some(ConfigValue::String(raw)) => raw.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn config_non_empty_string(config: &ResolvedConfig, key: &str) -> Option<String> {
    config
        .get_string(key)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn config_usize_override(config: &ResolvedConfig, key: &str) -> Option<usize> {
    match config.get(key).map(ConfigValue::reveal) {
        Some(ConfigValue::Integer(value)) if *value >= 0 => Some(*value as usize),
        Some(ConfigValue::String(raw)) => {
            let trimmed = raw.trim();
            if trimmed.eq_ignore_ascii_case("inherit") || trimmed.is_empty() {
                None
            } else {
                trimmed.parse::<usize>().ok()
            }
        }
        _ => None,
    }
}

#[derive(Debug, Clone, Default)]
pub struct RenderSettingsBuilder {
    settings: RenderSettings,
}

impl RenderSettingsBuilder {
    pub fn plain(format: OutputFormat) -> Self {
        Self {
            settings: RenderSettings {
                format,
                format_explicit: false,
                mode: RenderMode::Plain,
                color: ColorMode::Never,
                unicode: UnicodeMode::Never,
                ..RenderSettings::default()
            },
        }
    }

    pub fn with_format(mut self, format: OutputFormat) -> Self {
        self.settings.format = format;
        self
    }

    pub fn with_format_explicit(mut self, format_explicit: bool) -> Self {
        self.settings.format_explicit = format_explicit;
        self
    }

    pub fn with_mode(mut self, mode: RenderMode) -> Self {
        self.settings.mode = mode;
        self
    }

    pub fn with_color(mut self, color: ColorMode) -> Self {
        self.settings.color = color;
        self
    }

    pub fn with_unicode(mut self, unicode: UnicodeMode) -> Self {
        self.settings.unicode = unicode;
        self
    }

    pub fn with_width(mut self, width: usize) -> Self {
        self.settings.width = Some(width);
        self
    }

    pub fn with_margin(mut self, margin: usize) -> Self {
        self.settings.margin = margin;
        self
    }

    pub fn with_indent_size(mut self, indent_size: usize) -> Self {
        self.settings.indent_size = indent_size;
        self
    }

    pub fn with_table_overflow(mut self, table_overflow: TableOverflow) -> Self {
        self.settings.table_overflow = table_overflow;
        self
    }

    pub fn with_table_border(mut self, table_border: TableBorderStyle) -> Self {
        self.settings.table_border = table_border;
        self
    }

    pub fn with_help_chrome(mut self, help_chrome: HelpChromeSettings) -> Self {
        self.settings.help_chrome = help_chrome;
        self
    }

    pub fn with_theme_name(mut self, theme_name: impl Into<String>) -> Self {
        self.settings.theme_name = theme_name.into();
        self
    }

    pub fn with_style_overrides(mut self, style_overrides: style::StyleOverrides) -> Self {
        self.settings.style_overrides = style_overrides;
        self
    }

    pub fn with_chrome_frame(mut self, chrome_frame: SectionFrameStyle) -> Self {
        self.settings.chrome_frame = chrome_frame;
        self
    }

    pub fn with_ruled_section_policy(mut self, ruled_section_policy: RuledSectionPolicy) -> Self {
        self.settings.ruled_section_policy = ruled_section_policy;
        self
    }

    pub fn with_guide_default_format(mut self, guide_default_format: GuideDefaultFormat) -> Self {
        self.settings.guide_default_format = guide_default_format;
        self
    }

    pub fn with_runtime(mut self, runtime: RenderRuntime) -> Self {
        self.settings.runtime = runtime;
        self
    }

    pub fn build(self) -> RenderSettings {
        self.settings
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderProfile {
    Normal,
    CopySafe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedHelpChromeSettings {
    pub entry_indent: usize,
    pub entry_gap: Option<usize>,
    pub section_spacing: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRenderSettings {
    pub backend: RenderBackend,
    pub color: bool,
    pub unicode: bool,
    pub width: Option<usize>,
    pub margin: usize,
    pub indent_size: usize,
    pub short_list_max: usize,
    pub medium_list_max: usize,
    pub grid_padding: usize,
    pub grid_columns: Option<usize>,
    pub column_weight: usize,
    pub table_overflow: TableOverflow,
    pub table_border: TableBorderStyle,
    pub help_table_border: TableBorderStyle,
    pub theme_name: String,
    pub theme: ThemeDefinition,
    pub style_overrides: style::StyleOverrides,
    pub help_chrome: ResolvedHelpChromeSettings,
    pub chrome_frame: SectionFrameStyle,
    pub guide_default_format: GuideDefaultFormat,
}

impl RenderSettings {
    fn resolve_color_mode(&self) -> bool {
        match self.color {
            ColorMode::Always => true,
            ColorMode::Never => false,
            ColorMode::Auto => !self.runtime.no_color && self.runtime.stdout_is_tty,
        }
    }

    fn resolve_unicode_mode(&self) -> bool {
        match self.unicode {
            UnicodeMode::Always => true,
            UnicodeMode::Never => false,
            UnicodeMode::Auto => {
                if !self.runtime.stdout_is_tty {
                    return false;
                }
                if matches!(self.runtime.terminal.as_deref(), Some("dumb")) {
                    return false;
                }
                match self.runtime.locale_utf8 {
                    Some(true) => true,
                    Some(false) => false,
                    None => true,
                }
            }
        }
    }

    fn resolve_width(&self) -> Option<usize> {
        if let Some(width) = self.width {
            return (width > 0).then_some(width);
        }
        self.runtime.width.filter(|width| *width > 0)
    }

    pub fn resolve_render_settings(&self) -> ResolvedRenderSettings {
        let backend = match self.mode {
            RenderMode::Plain => RenderBackend::Plain,
            RenderMode::Rich => RenderBackend::Rich,
            RenderMode::Auto => {
                if matches!(self.color, ColorMode::Always)
                    || matches!(self.unicode, UnicodeMode::Always)
                {
                    RenderBackend::Rich
                } else if !self.runtime.stdout_is_tty
                    || matches!(self.runtime.terminal.as_deref(), Some("dumb"))
                {
                    RenderBackend::Plain
                } else {
                    RenderBackend::Rich
                }
            }
        };

        let theme = self
            .theme
            .clone()
            .unwrap_or_else(|| theme::resolve_theme(&self.theme_name));
        let theme_name = theme::normalize_theme_name(&theme.id);
        let help_chrome = ResolvedHelpChromeSettings {
            entry_indent: self.help_chrome.entry_indent.unwrap_or(2),
            entry_gap: self.help_chrome.entry_gap,
            section_spacing: self.help_chrome.section_spacing.unwrap_or(1),
        };

        match backend {
            RenderBackend::Plain => ResolvedRenderSettings {
                backend,
                color: false,
                unicode: false,
                width: self.resolve_width(),
                margin: self.margin,
                indent_size: self.indent_size.max(1),
                short_list_max: self.short_list_max.max(1),
                medium_list_max: self.medium_list_max.max(self.short_list_max.max(1) + 1),
                grid_padding: self.grid_padding.max(1),
                grid_columns: self.grid_columns.filter(|value| *value > 0),
                column_weight: self.column_weight.max(1),
                table_overflow: self.table_overflow,
                table_border: self.table_border,
                help_table_border: self.help_chrome.table_chrome.resolve(self.table_border),
                theme_name,
                theme: theme.clone(),
                style_overrides: self.style_overrides.clone(),
                help_chrome,
                chrome_frame: self.chrome_frame,
                guide_default_format: self.guide_default_format,
            },
            RenderBackend::Rich => ResolvedRenderSettings {
                backend,
                color: self.resolve_color_mode(),
                unicode: self.resolve_unicode_mode(),
                width: self.resolve_width(),
                margin: self.margin,
                indent_size: self.indent_size.max(1),
                short_list_max: self.short_list_max.max(1),
                medium_list_max: self.medium_list_max.max(self.short_list_max.max(1) + 1),
                grid_padding: self.grid_padding.max(1),
                grid_columns: self.grid_columns.filter(|value| *value > 0),
                column_weight: self.column_weight.max(1),
                table_overflow: self.table_overflow,
                table_border: self.table_border,
                help_table_border: self.help_chrome.table_chrome.resolve(self.table_border),
                theme_name,
                theme,
                style_overrides: self.style_overrides.clone(),
                help_chrome,
                chrome_frame: self.chrome_frame,
                guide_default_format: self.guide_default_format,
            },
        }
    }

    pub(crate) fn plain_copy_settings(&self) -> Self {
        Self {
            format: self.format,
            format_explicit: self.format_explicit,
            mode: RenderMode::Plain,
            color: ColorMode::Never,
            unicode: UnicodeMode::Never,
            width: self.width,
            margin: self.margin,
            indent_size: self.indent_size,
            short_list_max: self.short_list_max,
            medium_list_max: self.medium_list_max,
            grid_padding: self.grid_padding,
            grid_columns: self.grid_columns,
            column_weight: self.column_weight,
            table_overflow: self.table_overflow,
            table_border: self.table_border,
            help_chrome: self.help_chrome,
            mreg_stack_min_col_width: self.mreg_stack_min_col_width,
            mreg_stack_overflow_ratio: self.mreg_stack_overflow_ratio,
            theme_name: self.theme_name.clone(),
            theme: self.theme.clone(),
            style_overrides: self.style_overrides.clone(),
            chrome_frame: self.chrome_frame,
            ruled_section_policy: self.ruled_section_policy,
            guide_default_format: self.guide_default_format,
            runtime: self.runtime.clone(),
        }
    }

    pub(crate) fn resolve_output_format(&self, output: &OutputResult) -> OutputFormat {
        if self.format_explicit && !matches!(self.format, OutputFormat::Auto) {
            return self.format;
        }

        if crate::guide::GuideView::try_from_output_result(output).is_some()
            && self.prefers_guide_rendering()
        {
            return OutputFormat::Guide;
        }

        if let Some(recommended) = output.meta.render_recommendation {
            return match recommended {
                RenderRecommendation::Format(format) => format,
                RenderRecommendation::Guide => OutputFormat::Guide,
            };
        }

        if !matches!(self.format, OutputFormat::Auto) {
            return self.format;
        }

        if matches!(output.items, OutputItems::Groups(_)) {
            return OutputFormat::Table;
        }

        let rows = output_items_to_rows(&output.items);
        if rows
            .iter()
            .all(|row| row.len() == 1 && row.contains_key("value"))
        {
            OutputFormat::Value
        } else if rows.len() <= 1 {
            OutputFormat::Mreg
        } else {
            OutputFormat::Table
        }
    }
}

pub fn resolve_settings(
    settings: &RenderSettings,
    profile: RenderProfile,
) -> ResolvedRenderSettings {
    if matches!(profile, RenderProfile::CopySafe) {
        settings.plain_copy_settings().resolve_render_settings()
    } else {
        settings.resolve_render_settings()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        HelpLayout, apply_render_config_overrides, config_int, config_non_empty_string,
        config_usize_override, help_layout_from_config,
    };
    use crate::config::{ConfigLayer, ConfigResolver, LoadedLayers, ResolveOptions};
    use crate::core::output::OutputFormat;
    use crate::ui::build_presentation_defaults_layer;
    use crate::ui::section_chrome::{RuledSectionPolicy, SectionFrameStyle};
    use crate::ui::settings::{
        GuideDefaultFormat, RenderSettings, TableBorderStyle, TableOverflow,
    };

    fn resolved_config(entries: &[(&str, &str)]) -> crate::config::ResolvedConfig {
        let mut defaults = ConfigLayer::default();
        for (key, value) in entries {
            defaults.set(*key, *value);
        }
        ConfigResolver::from_loaded_layers(LoadedLayers {
            defaults,
            ..LoadedLayers::default()
        })
        .resolve(ResolveOptions::default())
        .expect("config should resolve")
    }

    fn resolved_config_with_presentation(
        entries: &[(&str, &str)],
    ) -> crate::config::ResolvedConfig {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        for (key, value) in entries {
            defaults.set(*key, *value);
        }
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        let options = ResolveOptions::default().with_terminal("cli");
        let base = resolver
            .resolve(options.clone())
            .expect("base test config should resolve");
        resolver.set_presentation(build_presentation_defaults_layer(&base));
        resolver
            .resolve(options)
            .expect("test config should resolve")
    }

    fn resolved_config_with_session(
        defaults_entries: &[(&str, &str)],
        session_entries: &[(&str, &str)],
    ) -> crate::config::ResolvedConfig {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        for (key, value) in defaults_entries {
            defaults.set(*key, *value);
        }

        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);

        let mut session = ConfigLayer::default();
        for (key, value) in session_entries {
            session.set(*key, *value);
        }
        resolver.set_session(session);

        let options = ResolveOptions::default().with_terminal("cli");
        let base = resolver
            .resolve(options.clone())
            .expect("base test config should resolve");
        resolver.set_presentation(build_presentation_defaults_layer(&base));
        resolver
            .resolve(options)
            .expect("test config should resolve")
    }

    #[test]
    fn help_layout_from_config_owns_presentation_mapping_unit() {
        assert_eq!(
            help_layout_from_config(&resolved_config(&[])),
            HelpLayout::Full
        );
        assert_eq!(
            help_layout_from_config(&resolved_config(&[("ui.presentation", "expressive")])),
            HelpLayout::Full
        );
        assert_eq!(
            help_layout_from_config(&resolved_config(&[("ui.presentation", "compact")])),
            HelpLayout::Compact
        );
        assert_eq!(
            help_layout_from_config(&resolved_config(&[("ui.presentation", "austere")])),
            HelpLayout::Minimal
        );
        assert_eq!(
            help_layout_from_config(&resolved_config(&[("ui.presentation", "gammel-og-bitter")])),
            HelpLayout::Minimal
        );
    }

    #[test]
    fn render_config_helpers_normalize_strings_blanks_and_integers_unit() {
        let config = resolved_config_with_presentation(&[
            ("ui.width", "120"),
            ("color.text", "  "),
            ("ui.margin", "3"),
        ]);

        assert_eq!(config_int(&config, "ui.width"), Some(120));
        assert_eq!(config_int(&config, "ui.margin"), Some(3));
        assert_eq!(config_non_empty_string(&config, "color.text"), None);
    }

    #[test]
    fn render_config_overrides_use_presentation_defaults_and_explicit_overrides_unit() {
        let config = resolved_config_with_session(
            &[("ui.width", "88")],
            &[
                ("ui.chrome.frame", "round"),
                ("ui.chrome.rule_policy", "stacked"),
                ("ui.table.border", "square"),
                ("ui.table.overflow", "wrap"),
            ],
        );
        let mut settings = RenderSettings::test_plain(OutputFormat::Table);

        apply_render_config_overrides(&mut settings, &config);

        assert_eq!(settings.width, Some(88));
        assert_eq!(settings.chrome_frame, SectionFrameStyle::Round);
        assert_eq!(settings.ruled_section_policy, RuledSectionPolicy::Shared);
        assert_eq!(settings.table_border, TableBorderStyle::Square);
        assert_eq!(settings.table_overflow, TableOverflow::Wrap);

        let config = resolved_config_with_presentation(&[("ui.presentation", "expressive")]);
        let mut settings = RenderSettings::test_plain(OutputFormat::Table);

        apply_render_config_overrides(&mut settings, &config);

        assert_eq!(settings.chrome_frame, SectionFrameStyle::TopBottom);
        assert_eq!(settings.table_border, TableBorderStyle::Round);

        let config = resolved_config_with_session(
            &[("ui.presentation", "expressive")],
            &[("ui.chrome.frame", "square"), ("ui.table.border", "none")],
        );
        let mut settings = RenderSettings::test_plain(OutputFormat::Table);

        apply_render_config_overrides(&mut settings, &config);

        assert_eq!(settings.chrome_frame, SectionFrameStyle::Square);
        assert_eq!(settings.table_border, TableBorderStyle::None);

        let config = resolved_config_with_presentation(&[("ui.guide.default_format", "inherit")]);
        let mut settings = RenderSettings::test_plain(OutputFormat::Json);

        apply_render_config_overrides(&mut settings, &config);

        assert_eq!(settings.guide_default_format, GuideDefaultFormat::Inherit);

        let config = resolved_config_with_presentation(&[
            ("ui.help.entry_indent", "4"),
            ("ui.help.entry_gap", "3"),
            ("ui.help.section_spacing", "inherit"),
        ]);
        let mut settings = RenderSettings::test_plain(OutputFormat::Guide);

        apply_render_config_overrides(&mut settings, &config);

        assert_eq!(
            config_usize_override(&config, "ui.help.entry_indent"),
            Some(4)
        );
        assert_eq!(settings.help_chrome.entry_indent, Some(4));
        assert_eq!(settings.help_chrome.entry_gap, Some(3));
        assert_eq!(settings.help_chrome.section_spacing, None);
    }
}
