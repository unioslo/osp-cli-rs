//! Render-setting types and config-derived UI resolution.
//!
//! This module exists so callers can keep a stable separation between:
//!
//! - user/config-facing render preferences such as format, color, and chrome
//! - runtime facts such as tty state and terminal width
//! - the final resolved settings used by planning and emitters
//!
//! Public callers usually construct or tweak [`RenderSettings`] and let this
//! module derive the richer [`ResolvedRenderSettings`] view consumed by the
//! rest of the UI pipeline.

use crate::config::{ConfigSource, ConfigValue, DEFAULT_UI_WIDTH_MAX, ResolvedConfig, Scope};
use crate::core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use crate::core::output_model::{OutputItems, OutputResult, RenderRecommendation};
use crate::ui::section_chrome::{RuledSectionPolicy, SectionFrameStyle};
use crate::ui::style;
use crate::ui::theme;
use crate::ui::theme::{DEFAULT_THEME_NAME, ThemeDefinition};

/// Concrete renderer family chosen after settings resolution.
///
/// `Plain` keeps output copy-safe and decoration-light. `Rich` enables the
/// styled terminal renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderBackend {
    Plain,
    Rich,
}

/// Border style used for table-like layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableBorderStyle {
    None,
    #[default]
    Square,
    Round,
}

/// Overflow policy for wide table cells.
///
/// This is intentionally presentation-only. It should not change the logical
/// row model, only how wide content is displayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableOverflow {
    None,
    Clip,
    Ellipsis,
    Wrap,
}

/// How automatically selected tables handle columns that cannot fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableTooWide {
    #[default]
    Mreg,
    Hide,
}

impl TableTooWide {
    /// All variants in canonical configuration order.
    pub const ALL: [Self; 2] = [Self::Mreg, Self::Hide];

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "mreg" => Some(Self::Mreg),
            "hide" => Some(Self::Hide),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mreg => "mreg",
            Self::Hide => "hide",
        }
    }
}

/// Where the effective output format was selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FormatProvenance {
    Explicit,
    Configured,
    Recommended,
    Inferred,
}

/// Layout density for guide/help rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HelpLayout {
    #[default]
    Full,
    Compact,
    Minimal,
}

/// Named presentation preset used to seed several UI defaults at once.
///
/// Presets are a convenience layer on top of the regular UI/config keys, not
/// a second rendering pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiPresentation {
    Expressive,
    Compact,
    Austere,
}

/// Provenance for a chosen [`UiPresentation`] preset.
///
/// This is used by explain/introspection surfaces so callers can see which
/// value seeded presentation defaults before later overrides were applied.
#[derive(Debug, Clone, PartialEq)]
pub struct PresentationEffect {
    pub preset: UiPresentation,
    pub preset_source: ConfigSource,
    pub preset_scope: Scope,
    pub preset_origin: Option<String>,
    pub seeded_value: ConfigValue,
}

impl UiPresentation {
    /// All variants in canonical configuration order.
    pub const ALL: [Self; 3] = [Self::Expressive, Self::Compact, Self::Austere];

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "expressive" => Some(Self::Expressive),
            "compact" => Some(Self::Compact),
            "austere" | "gammel-og-bitter" => Some(Self::Austere),
            _ => None,
        }
    }

    pub fn as_config_value(self) -> &'static str {
        self.as_str()
    }

    /// Returns the canonical configuration spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Expressive => "expressive",
            Self::Compact => "compact",
            Self::Austere => "austere",
        }
    }
}

/// Border style override for one family of tables (help tables, nested tables).
///
/// `Inherit` follows `ui.table.border`; the other variants force a specific
/// border for that family only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableBorderOverride {
    Inherit,
    #[default]
    None,
    Square,
    Round,
}

impl TableBorderOverride {
    /// All variants in canonical configuration order.
    pub const ALL: [Self; 4] = [Self::Inherit, Self::None, Self::Square, Self::Round];

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

    /// Returns the canonical configuration spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inherit => "inherit",
            Self::None => "none",
            Self::Square => "square",
            Self::Round => "round",
        }
    }
}

/// Runtime facts that influence auto render decisions.
///
/// `RenderSettings` describes caller preference, while `RenderRuntime`
/// describes the ambient terminal/process conditions those preferences are
/// resolved against.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RenderRuntime {
    /// Where the reader types suggested commands.
    pub command_scope: crate::core::CommandScope,
    /// Whether the visible stdout path is a tty.
    pub stdout_is_tty: bool,
    /// Raw terminal kind such as `xterm-256color`, if known.
    pub terminal: Option<String>,
    /// Whether the runtime has already forced color off.
    pub no_color: bool,
    /// Detected or caller-supplied terminal width hint.
    pub width: Option<usize>,
    /// Whether locale detection suggests UTF-8 output is safe.
    pub locale_utf8: Option<bool>,
}

/// Caller-facing render preferences for the canonical UI pipeline.
///
/// This is the main "what do I want?" settings surface. Auto decisions still
/// happen later by combining these preferences with [`RenderRuntime`] and a
/// [`RenderTarget`].
///
/// Rule of thumb:
///
/// - `format`, `mode`, `color`, and `unicode` describe the top-level output
///   intent
/// - `format_explicit` says whether the caller forced the format and therefore
///   wants recommendation/auto inference to stop
/// - `runtime` carries the ambient facts used to resolve the auto knobs
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderSettings {
    /// Execution policy used to expose usable command prefixes in help.
    pub command_prefixes: crate::command_words::CommandPrefixes,
    pub format: OutputFormat,
    /// Canonical invocation path, without positional arguments.
    pub(crate) command_path: Vec<String>,
    /// Exact defaults from the resolved config snapshot; selection stays local.
    pub(crate) command_formats: std::collections::BTreeMap<String, OutputFormat>,
    pub format_explicit: bool,
    /// Render semantic documents as guides unless an explicit format is selected.
    pub guide_default_format: bool,
    /// Show screen table fit summaries and recovery actions.
    pub table_hints: bool,
    /// Command detail level; positive values retain empty human fields.
    pub verbosity: u8,
    pub mode: RenderMode,
    pub color: ColorMode,
    pub unicode: UnicodeMode,
    pub theme_name: String,
    pub(crate) theme: Option<ThemeDefinition>,
    pub width: Option<usize>,
    /// Maximum width for rendered output; zero leaves the measured width uncapped.
    pub width_max: usize,
    pub margin: usize,
    pub table_overflow: TableOverflow,
    pub table_too_wide: TableTooWide,
    /// Blank lines between cards; `None` preserves each style's default.
    pub mreg_gap: Option<usize>,
    /// Minimum scalar-derived card width, including indentation.
    pub mreg_min_width: usize,
    /// Maximum card width, also limited by the terminal.
    pub mreg_max_width: usize,
    /// Maximum aligned label column, including separators.
    pub mreg_label_max: usize,
    pub table_border: TableBorderStyle,
    /// Border for tables nested inside a record, such as `addresses (2):`.
    pub nested_table_border: TableBorderOverride,
    pub style_overrides: style::StyleOverrides,
    pub chrome_frame: SectionFrameStyle,
    pub ruled_section_policy: RuledSectionPolicy,
    pub runtime: RenderRuntime,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            command_prefixes: Default::default(),
            format: OutputFormat::Auto,
            command_path: Vec::new(),
            command_formats: Default::default(),
            format_explicit: false,
            guide_default_format: true,
            table_hints: true,
            verbosity: 0,
            mode: RenderMode::Auto,
            color: ColorMode::Auto,
            unicode: UnicodeMode::Auto,
            theme_name: DEFAULT_THEME_NAME.to_string(),
            theme: None,
            width: None,
            width_max: usize::try_from(DEFAULT_UI_WIDTH_MAX).unwrap_or(0),
            margin: 0,
            table_overflow: TableOverflow::Ellipsis,
            table_too_wide: TableTooWide::Mreg,
            mreg_gap: None,
            mreg_min_width: 40,
            mreg_max_width: 80,
            mreg_label_max: 18,
            table_border: TableBorderStyle::Square,
            nested_table_border: TableBorderOverride::None,
            style_overrides: style::StyleOverrides::default(),
            chrome_frame: SectionFrameStyle::Top,
            ruled_section_policy: RuledSectionPolicy::Shared,
            runtime: RenderRuntime::default(),
        }
    }
}

impl RenderSettings {
    /// Applies resolved configuration while retaining the caller's terminal facts.
    /// This uses the host's ordinary presentation preferences and overrides.
    pub fn with_config(mut self, config: &ResolvedConfig) -> Self {
        apply_render_config_overrides(&mut self, config);
        self
    }

    pub fn test_plain(format: OutputFormat) -> Self {
        Self {
            format,
            mode: RenderMode::Plain,
            color: ColorMode::Never,
            unicode: UnicodeMode::Never,
            ..Self::default()
        }
    }
}

impl TableOverflow {
    /// All variants in canonical configuration order.
    pub const ALL: [Self; 4] = [Self::None, Self::Clip, Self::Ellipsis, Self::Wrap];

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "none" | "visible" => Some(Self::None),
            "clip" | "hidden" | "crop" => Some(Self::Clip),
            "ellipsis" | "truncate" => Some(Self::Ellipsis),
            "wrap" | "wrapped" => Some(Self::Wrap),
            _ => None,
        }
    }

    /// Returns the canonical configuration spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Clip => "clip",
            Self::Ellipsis => "ellipsis",
            Self::Wrap => "wrap",
        }
    }
}

impl TableBorderStyle {
    /// All variants in canonical configuration order.
    pub const ALL: [Self; 3] = [Self::None, Self::Square, Self::Round];

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "none" | "plain" => Some(Self::None),
            "square" | "box" | "boxed" => Some(Self::Square),
            "round" | "rounded" => Some(Self::Round),
            _ => None,
        }
    }

    /// Returns the canonical configuration spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Square => "square",
            Self::Round => "round",
        }
    }
}

pub fn help_layout_from_config(config: &ResolvedConfig) -> HelpLayout {
    match resolve_ui_presentation(config) {
        UiPresentation::Expressive => HelpLayout::Full,
        UiPresentation::Compact => HelpLayout::Compact,
        UiPresentation::Austere => HelpLayout::Minimal,
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

    settings.command_formats = config
        .values()
        .keys()
        .filter_map(|key| {
            let path = key.strip_prefix("ui.command_formats.")?;
            let format = config.get_string(key).and_then(OutputFormat::parse)?;
            Some((path.to_owned(), format))
        })
        .collect();

    settings.table_hints = config
        .get_string("ui.help.level")
        .and_then(crate::core::HelpLevel::parse)
        != Some(crate::core::HelpLevel::None);

    if let Some(value) = config.get_string("ui.guide.default_format") {
        settings.guide_default_format = value == "guide";
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

    if settings.runtime.stdout_is_tty
        && settings.width.is_none()
        && config
            .get_value_entry("ui.width")
            .is_some_and(|entry| entry.source != ConfigSource::BuiltinDefaults)
    {
        settings.width = config_usize_override(config, "ui.width").filter(|width| *width > 0);
    }

    if let Some(width_max) = config_int(config, "ui.width-max") {
        settings.width_max = usize::try_from(width_max).unwrap_or(0);
    }

    sync_render_config_overrides(settings, config);
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
    settings.command_prefixes = crate::command_words::CommandPrefixes::from_config(config);
    if let Some(value) = config_int(config, "ui.margin")
        && value >= 0
    {
        settings.margin = value as usize;
    }

    if let Some(value) = config.get_string("ui.table.overflow")
        && let Some(parsed) = TableOverflow::parse(value)
    {
        settings.table_overflow = parsed;
    }

    if let Some(value) = config.get_string("ui.table.too_wide")
        && let Some(parsed) = TableTooWide::parse(value)
    {
        settings.table_too_wide = parsed;
    }

    for (key, setting) in [
        ("ui.mreg.min_width", &mut settings.mreg_min_width),
        ("ui.mreg.max_width", &mut settings.mreg_max_width),
        ("ui.mreg.label_max", &mut settings.mreg_label_max),
    ] {
        if let Some(value) = config_int(config, key).and_then(|value| usize::try_from(value).ok())
            && value > 0
        {
            *setting = value;
        }
    }

    if let Some(value) = config.get_string("ui.mreg.gap") {
        settings.mreg_gap = value.parse().ok();
    }

    if let Some(value) = config.get_string("ui.table.border")
        && let Some(parsed) = TableBorderStyle::parse(value)
    {
        settings.table_border = parsed;
    }

    if let Some(value) = config.get_string("ui.table.nested_border")
        && let Some(parsed) = TableBorderOverride::parse(value)
    {
        settings.nested_table_border = parsed;
    }
}

fn config_int(config: &ResolvedConfig, key: &str) -> Option<i64> {
    match config.get(key).map(ConfigValue::reveal) {
        Some(ConfigValue::Integer(value)) => Some(*value),
        Some(ConfigValue::String(raw)) => raw.trim().parse::<i64>().ok(),
        _ => None,
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderTarget {
    ReplScreen,
    OneShotScreen,
    Pager,
    Editor,
    Clipboard,
    Pipe,
}

impl RenderTarget {
    /// Selects the visible target from the caller's runtime facts.
    pub fn screen(settings: &RenderSettings) -> Self {
        if !settings.runtime.stdout_is_tty {
            Self::Pipe
        } else if matches!(
            settings.runtime.command_scope,
            crate::core::CommandScope::OneShot
        ) {
            Self::OneShotScreen
        } else {
            Self::ReplScreen
        }
    }

    pub(crate) fn is_screen(self) -> bool {
        matches!(self, Self::ReplScreen | Self::OneShotScreen)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRenderSettings {
    /// Whether help may emphasize executable command abbreviations.
    pub command_prefixes_enabled: bool,
    /// Literal-only commands in the current help document.
    pub document_literal_commands: Vec<String>,
    pub display_limits: bool,
    pub table_hints: bool,
    /// Command detail level; positive values retain empty human fields.
    pub verbosity: u8,
    pub backend: RenderBackend,
    pub color: bool,
    pub unicode: bool,
    /// Screen width, including any fallback; `None` means an unlimited target.
    pub width: Option<usize>,
    pub margin: usize,
    pub table_overflow: TableOverflow,
    pub table_too_wide: TableTooWide,
    /// Blank lines between cards; `None` preserves each style's default.
    pub mreg_gap: Option<usize>,
    /// Minimum scalar-derived card width, including indentation.
    pub mreg_min_width: usize,
    /// Maximum card width, also limited by the terminal.
    pub mreg_max_width: usize,
    /// Maximum aligned label column, including separators.
    pub mreg_label_max: usize,
    pub table_border: TableBorderStyle,
    pub nested_table_border: TableBorderStyle,
    pub theme_name: String,
    pub theme: ThemeDefinition,
    pub style_overrides: style::StyleOverrides,
    pub chrome_frame: SectionFrameStyle,
    pub ruled_section_policy: RuledSectionPolicy,
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
                self.runtime.locale_utf8.unwrap_or(true)
            }
        }
    }

    fn resolve_width(&self) -> Option<usize> {
        let configured = self.width.filter(|width| *width > 0);
        let measured = self
            .runtime
            .width
            .filter(|width| self.runtime.stdout_is_tty && *width > 0);
        let width = match (configured, measured) {
            (Some(configured), Some(measured)) => configured.min(measured),
            (Some(configured), None) => configured,
            (None, Some(measured)) => measured,
            // Nothing measured (piped or redirected output): natural width, so
            // `| head` and files keep whole values instead of a 72-column fit.
            (None, None) => return None,
        };

        Some(if self.width_max == 0 {
            width
        } else {
            width.min(self.width_max)
        })
    }

    fn resolve_preferences(&self) -> ResolvedRenderSettings {
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

        ResolvedRenderSettings {
            command_prefixes_enabled: self.command_prefixes.enabled(!matches!(
                self.runtime.command_scope,
                crate::core::CommandScope::OneShot
            )),
            document_literal_commands: Vec::new(),
            display_limits: false,
            table_hints: self.table_hints,
            verbosity: self.verbosity,
            backend,
            color: backend == RenderBackend::Rich && self.resolve_color_mode(),
            unicode: backend == RenderBackend::Rich && self.resolve_unicode_mode(),
            width: self.resolve_width(),
            margin: self.margin,
            table_overflow: self.table_overflow,
            table_too_wide: self.table_too_wide,
            mreg_gap: self.mreg_gap,
            mreg_min_width: self.mreg_min_width,
            mreg_max_width: self.mreg_max_width,
            mreg_label_max: self.mreg_label_max,
            table_border: self.table_border,
            nested_table_border: self.nested_table_border.resolve(self.table_border),
            theme_name,
            theme,
            style_overrides: self.style_overrides.clone(),
            chrome_frame: self.chrome_frame,
            ruled_section_policy: self.ruled_section_policy,
        }
    }

    pub(crate) fn resolve_render_settings(&self) -> ResolvedRenderSettings {
        resolve_settings(self, RenderTarget::screen(self))
    }

    pub(crate) fn plain_copy_settings(&self) -> Self {
        settings_for_target(self, RenderTarget::Clipboard)
    }

    /// Selects invocation/config preference before result-shape inference.
    /// The second value distinguishes an exact command default for documents.
    fn format_preference(&self) -> (OutputFormat, bool) {
        if !self.format_explicit
            && let Some(format) = self.command_formats.get(&self.command_path.join("."))
        {
            return (*format, true);
        }
        (self.format, false)
    }

    /// Shared preference for output-policy checks that happen before rendering.
    pub(crate) fn requested_format(&self) -> OutputFormat {
        self.format_preference().0
    }

    pub(crate) fn resolve_output_format(
        &self,
        output: &OutputResult,
    ) -> (OutputFormat, FormatProvenance) {
        use FormatProvenance::{Configured, Explicit, Inferred, Recommended};
        let (format, command_default) = self.format_preference();
        if self.format_explicit && format != OutputFormat::Auto {
            return (format, Explicit);
        }
        // Documents keep their layout unless an invocation requests a format.
        if output.meta.document_rows {
            return if (self.guide_default_format && !command_default)
                || format == OutputFormat::Auto
            {
                (OutputFormat::Auto, Inferred)
            } else {
                (format, Configured)
            };
        }
        if format != OutputFormat::Auto {
            return (format, Configured);
        }
        let inferred = match &output.items {
            OutputItems::Groups(_) => OutputFormat::Table,
            OutputItems::Rows(rows)
                if rows
                    .iter()
                    .all(|row| row.len() == 1 && row.contains_key("value")) =>
            {
                OutputFormat::Value
            }
            OutputItems::Rows(rows) if rows.len() <= 1 => OutputFormat::Mreg,
            _ => OutputFormat::Table,
        };
        let recommendation = match output.meta.render_recommendation {
            Some(RenderRecommendation::Format(format)) if format != OutputFormat::Auto => {
                Some(format)
            }
            _ => None,
        };
        if let Some(format) = recommendation {
            return (format, Recommended);
        }
        (inferred, Inferred)
    }
}

/// Maps destination policy to render preferences in one place.
fn settings_for_target(settings: &RenderSettings, target: RenderTarget) -> RenderSettings {
    let target = if target.is_screen() && !settings.runtime.stdout_is_tty {
        RenderTarget::Pipe
    } else {
        target
    };
    let mut effective = settings.clone();
    if target != RenderTarget::ReplScreen {
        effective.margin = 0;
    }
    if !target.is_screen() {
        effective.width = None;
        effective.width_max = 0;
        effective.runtime.width = None;
        effective.table_overflow = TableOverflow::None;
        effective.table_too_wide = TableTooWide::Hide;
    } else if effective.runtime.stdout_is_tty && effective.resolve_width().is_none() {
        // The only width fallback: an unmeasured screen, never an unlimited target.
        effective.width = Some(100);
    }
    if target == RenderTarget::Pipe {
        effective.runtime.stdout_is_tty = false;
    }
    if matches!(target, RenderTarget::Editor | RenderTarget::Clipboard) {
        effective.color = ColorMode::Never;
        effective.runtime.no_color = true;
    }
    if target == RenderTarget::Clipboard {
        effective.mode = RenderMode::Plain;
        effective.unicode = UnicodeMode::Never;
    }
    effective
}

pub fn resolve_settings(settings: &RenderSettings, target: RenderTarget) -> ResolvedRenderSettings {
    let mut resolved = settings_for_target(settings, target).resolve_preferences();
    resolved.display_limits = target.is_screen() && settings.runtime.stdout_is_tty;
    resolved
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::{
        HelpLayout, RenderBackend, RenderRuntime, RenderTarget, TableBorderOverride,
        TableBorderStyle, TableOverflow, UiPresentation, apply_render_config_overrides, config_int,
        explain_presentation_effect, help_layout_from_config,
    };
    use crate::config::{
        ConfigLayer, ConfigResolver, ConfigSource, ConfigValue, LoadedLayers, ResolveOptions,
    };
    use crate::core::document::Document;
    use crate::core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
    use crate::core::output_model::{
        Group, OutputItems, OutputMeta, OutputResult, RenderRecommendation,
    };
    use crate::row;
    use crate::ui::build_presentation_defaults_layer;
    use crate::ui::section_chrome::{RuledSectionPolicy, SectionFrameStyle};
    use crate::ui::settings::RenderSettings;

    fn resolved_config(entries: &[(&str, &str)]) -> crate::config::ResolvedConfig {
        let mut defaults = ConfigLayer::default();
        for (key, value) in entries {
            defaults.set(*key, *value);
        }
        ConfigResolver::from_loaded_layers_with_schema(
            LoadedLayers {
                defaults,
                ..LoadedLayers::default()
            },
            crate::config::ConfigSchema::default(),
        )
        .resolve(ResolveOptions::default())
        .expect("config should resolve")
    }

    fn resolved_config_with_presentation(
        entries: &[(&str, &str)],
    ) -> crate::config::ResolvedConfig {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        defaults.set("theme.path", Vec::<String>::new());
        for (key, value) in entries {
            defaults.set(*key, *value);
        }
        let mut layers = crate::config::LoadedLayers {
            defaults,
            ..crate::config::LoadedLayers::default()
        };
        let options = ResolveOptions::default().with_terminal("cli");
        let base = ConfigResolver::from_loaded_layers_with_schema(
            layers.clone(),
            crate::config::ConfigSchema::default(),
        )
        .resolve(options.clone())
        .expect("base test config should resolve");
        layers.presentation = build_presentation_defaults_layer(&base);
        ConfigResolver::from_loaded_layers_with_schema(
            layers.clone(),
            crate::config::ConfigSchema::default(),
        )
        .resolve(options)
        .expect("test config should resolve")
    }

    fn resolved_config_with_session(
        defaults_entries: &[(&str, &str)],
        session_entries: &[(&str, &str)],
    ) -> crate::config::ResolvedConfig {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        defaults.set("theme.path", Vec::<String>::new());
        for (key, value) in defaults_entries {
            defaults.set(*key, *value);
        }

        let mut layers = crate::config::LoadedLayers {
            defaults,
            ..crate::config::LoadedLayers::default()
        };

        let mut session = ConfigLayer::default();
        for (key, value) in session_entries {
            session.set(*key, *value);
        }
        layers.session = session;

        let options = ResolveOptions::default().with_terminal("cli");
        let base = ConfigResolver::from_loaded_layers_with_schema(
            layers.clone(),
            crate::config::ConfigSchema::default(),
        )
        .resolve(options.clone())
        .expect("base test config should resolve");
        layers.presentation = build_presentation_defaults_layer(&base);
        ConfigResolver::from_loaded_layers_with_schema(
            layers.clone(),
            crate::config::ConfigSchema::default(),
        )
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
        let config = resolved_config_with_presentation(&[("ui.width", "120"), ("ui.margin", "3")]);

        assert_eq!(config_int(&config, "ui.width"), Some(120));
        assert_eq!(config_int(&config, "ui.margin"), Some(3));
    }

    #[test]
    fn render_config_overrides_use_presentation_defaults_and_explicit_overrides_unit() {
        let config = resolved_config_with_session(
            &[],
            &[
                ("ui.width", "88"),
                ("ui.chrome.frame", "round"),
                ("ui.chrome.rule_policy", "stacked"),
                ("ui.table.border", "square"),
                ("ui.table.overflow", "wrap"),
            ],
        );
        let mut settings = RenderSettings::test_plain(OutputFormat::Table);
        settings.runtime.stdout_is_tty = true;

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
    }

    #[test]
    fn render_settings_resolution_and_output_format_ownership_unit() {
        let defaults = RenderSettings::default();
        assert_eq!(defaults.format, OutputFormat::Auto);
        assert_eq!(defaults.theme_name, super::DEFAULT_THEME_NAME);
        assert_eq!(defaults.table_border, TableBorderStyle::Square);

        let plain_settings = RenderSettings {
            runtime: RenderRuntime {
                stdout_is_tty: false,
                width: Some(72),
                ..RenderRuntime::default()
            },
            ..RenderSettings::default()
        };
        let plain = plain_settings.resolve_render_settings();
        assert_eq!(plain.backend, RenderBackend::Plain);
        assert!(!plain.color);
        assert!(!plain.unicode);
        assert_eq!(plain.width, None);

        let rich_settings = RenderSettings {
            mode: RenderMode::Auto,
            color: ColorMode::Always,
            unicode: UnicodeMode::Always,
            runtime: RenderRuntime {
                stdout_is_tty: true,
                terminal: Some("xterm-256color".into()),
                locale_utf8: Some(true),
                width: Some(88),
                ..RenderRuntime::default()
            },
            ..RenderSettings::default()
        };
        let rich = rich_settings.resolve_render_settings();
        assert_eq!(rich.backend, RenderBackend::Rich);
        assert!(rich.color);
        assert!(rich.unicode);
        assert_eq!(rich.width, Some(88));

        let copy = rich_settings.plain_copy_settings();
        let resolved_copy = super::resolve_settings(&rich_settings, RenderTarget::Clipboard);
        assert_eq!(copy.mode, RenderMode::Plain);
        assert_eq!(copy.color, ColorMode::Never);
        assert_eq!(copy.unicode, UnicodeMode::Never);
        assert_eq!(resolved_copy.backend, RenderBackend::Plain);
        assert!(!resolved_copy.color);
        assert!(!resolved_copy.unicode);

        let guide_output =
            Document::from_help_text("Usage: osp history <COMMAND>\n").to_output_result();
        assert_eq!(
            plain_settings.resolve_output_format(&guide_output).0,
            OutputFormat::Auto
        );

        let mut explicit_json = RenderSettings::test_plain(OutputFormat::Json);
        explicit_json.format_explicit = true;
        assert_eq!(
            explicit_json.resolve_output_format(&guide_output).0,
            OutputFormat::Json
        );

        let mut recommended = OutputResult::from_rows(vec![row! { "uid" => "alice" }]);
        recommended.meta.render_recommendation =
            Some(RenderRecommendation::Format(OutputFormat::Markdown));
        assert_eq!(
            plain_settings.resolve_output_format(&recommended).0,
            OutputFormat::Markdown
        );

        let grouped = OutputResult {
            items: OutputItems::Groups(vec![Group {
                groups: row! { "team" => "prod" },
                aggregates: row! { "count" => 2 },
                rows: vec![row! { "uid" => "alice" }, row! { "uid" => "bob" }],
            }]),
            document: None,
            meta: OutputMeta {
                title_field: None,
                display_notes: Vec::new(),
                context: Default::default(),
                key_index: vec!["team".to_string(), "count".to_string(), "uid".to_string()],
                unix_timestamp_columns: Vec::new(),
                display_rules: Vec::new(),
                display_columns: None,
                display_column_labels: None,
                column_align: Vec::new(),
                wants_copy: false,
                wants_page: false,
                wants_editor: false,
                grouped: true,
                document_rows: false,
                document_row_roles: Vec::new(),
                document_entry_gap: None,
                document_literal_commands: Vec::new(),
                document_command_prefixes: false,
                document_long_help: false,
                render_recommendation: None,
                display_limit: None,
            },
        };
        assert_eq!(
            plain_settings.resolve_output_format(&grouped).0,
            OutputFormat::Table
        );

        let value_output = OutputResult::from_rows(vec![row! { "value" => "hello" }]);
        assert_eq!(
            plain_settings.resolve_output_format(&value_output).0,
            OutputFormat::Value
        );

        let mreg_output = OutputResult::from_rows(vec![row! { "uid" => "alice" }]);
        assert_eq!(
            plain_settings.resolve_output_format(&mreg_output).0,
            OutputFormat::Mreg
        );
    }

    #[test]
    fn parser_aliases_and_presentation_effects_remain_canonical_unit() {
        assert_eq!(
            TableBorderOverride::parse("boxed"),
            Some(TableBorderOverride::Square)
        );
        assert_eq!(
            TableBorderOverride::parse("rounded"),
            Some(TableBorderOverride::Round)
        );
        assert_eq!(
            TableBorderOverride::Square.resolve(TableBorderStyle::None),
            TableBorderStyle::Square
        );
        assert_eq!(TableOverflow::parse("hidden"), Some(TableOverflow::Clip));
        assert_eq!(
            TableBorderStyle::parse("boxed"),
            Some(TableBorderStyle::Square)
        );
        assert_eq!(
            UiPresentation::parse("gammel-og-bitter"),
            Some(UiPresentation::Austere)
        );
        assert_eq!(UiPresentation::Compact.as_config_value(), "compact");

        let config = resolved_config_with_presentation(&[("ui.presentation", "compact")]);
        let effect = explain_presentation_effect(&config, "ui.messages.layout")
            .expect("presentation default should be explainable");
        assert_eq!(effect.preset, UiPresentation::Compact);
        assert_eq!(effect.seeded_value, ConfigValue::from("compact"));
        assert_eq!(effect.preset_source, ConfigSource::BuiltinDefaults);
    }
}
