//! The UI module exists to turn structured output into predictable terminal
//! text, while keeping rendering decisions separate from business logic.
//!
//! The UI stack has three layers:
//!
//! - [`format`] lowers rows and semantic outputs into a structured
//!   [`crate::ui::Document`].
//! - the internal renderer turns that document into terminal text using
//!   resolved width, color, unicode, and theme settings.
//! - inline/theme/style helpers provide smaller reusable building blocks for
//!   prompts, messages, and rich text fragments.
//!
//! Keep the distinction between "document shaping" and "terminal rendering"
//! clear. Most bugs become easier to localize once you know which side of that
//! boundary is wrong.
//!
//! Contract:
//!
//! - UI code may depend on structured output and render settings
//! - it should not own config precedence, command execution, or provider I/O
//! - terminal styling decisions should stay here rather than leaking into the
//!   rest of the app
//!
//! Public API shape:
//!
//! - semantic payloads like [`crate::ui::Document`] stay direct and cheap to
//!   inspect
//! - [`crate::ui::RenderRuntimeBuilder`] and
//!   [`crate::ui::RenderSettingsBuilder`] are the guided construction path for
//!   the heavier rendering configuration surfaces
//! - [`crate::ui::ResolvedRenderSettings`] stays a derived value, not another
//!   mutable configuration object
//! - guided render configuration follows the crate-wide naming rule:
//!   `builder(...)` returns `*Builder`, builder setters use `with_*`, and
//!   `build()` is the terminal step
//! - callers that only need a stable default baseline can use
//!   [`crate::ui::RenderSettings::builder`],
//!   [`crate::ui::RenderRuntime::builder`], or
//!   [`crate::ui::RenderSettings::test_plain`]

pub mod chrome;
/// Clipboard integration helpers for copy-safe output flows.
pub mod clipboard;
mod display;
pub mod document;
pub(crate) mod document_model;
pub(crate) mod format;
/// Lightweight inline-markup parsing and rendering helpers.
pub mod inline;
pub mod interactive;
mod layout;
pub mod messages;
pub(crate) mod presentation;
mod renderer;
mod resolution;
/// Semantic style tokens and explicit style overrides layered over the theme.
pub mod style;
/// Built-in theme definitions and theme lookup helpers.
pub mod theme;
pub(crate) mod theme_loader;
mod width;

use crate::core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use crate::core::output_model::{OutputItems, OutputResult};
use crate::core::row::Row;
use crate::guide::GuideView;

pub use chrome::{
    RuledSectionPolicy, SectionFrameStyle, SectionRenderContext, SectionStyleTokens,
    render_section_block_with_overrides, render_section_divider_with_overrides,
};
pub use clipboard::{ClipboardError, ClipboardService};
pub use document::{
    Block, CodeBlock, Document, JsonBlock, LineBlock, LinePart, MregBlock, MregEntry, MregRow,
    MregValue, PanelBlock, PanelRules, TableAlign, TableBlock, TableStyle, ValueBlock,
};
pub use inline::{line_from_inline, parts_from_inline, render_inline};
pub use interactive::{Interactive, InteractiveResult, InteractiveRuntime, Spinner};
pub use messages::{
    GroupedRenderOptions, MessageBuffer, MessageLayout, MessageLevel, UiMessage, adjust_verbosity,
};
pub(crate) use resolution::ResolvedGuideRenderSettings;
#[cfg(test)]
pub(crate) use resolution::ResolvedHelpChromeSettings;
pub(crate) use resolution::ResolvedRenderPlan;
pub use resolution::ResolvedRenderSettings;
pub use style::{StyleOverrides, StyleToken};
pub use theme::{
    DEFAULT_THEME_NAME, ThemeDefinition, ThemeOverrides, ThemePalette, all_themes,
    available_theme_names, builtin_themes, display_name_from_id, find_builtin_theme, find_theme,
    is_known_theme, normalize_theme_name, resolve_theme,
};

/// Runtime terminal characteristics used when resolving render behavior.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RenderRuntime {
    /// Whether standard output is attached to a TTY.
    pub stdout_is_tty: bool,
    /// Terminal program identifier when known.
    pub terminal: Option<String>,
    /// Whether color should be suppressed regardless of theme.
    pub no_color: bool,
    /// Measured terminal width, when available.
    pub width: Option<usize>,
    /// Whether the locale is known to support UTF-8.
    pub locale_utf8: Option<bool>,
}

impl RenderRuntime {
    /// Starts building runtime terminal facts for render resolution.
    pub fn builder() -> RenderRuntimeBuilder {
        RenderRuntimeBuilder::new()
    }
}

/// Builder for [`RenderRuntime`].
#[derive(Debug, Clone, Default)]
pub struct RenderRuntimeBuilder {
    runtime: RenderRuntime,
}

impl RenderRuntimeBuilder {
    /// Creates a builder seeded with [`RenderRuntime::default`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets whether standard output is attached to a TTY.
    pub fn with_stdout_is_tty(mut self, stdout_is_tty: bool) -> Self {
        self.runtime.stdout_is_tty = stdout_is_tty;
        self
    }

    /// Sets the terminal program identifier.
    pub fn with_terminal(mut self, terminal: impl Into<String>) -> Self {
        self.runtime.terminal = Some(terminal.into());
        self
    }

    /// Sets whether color should be suppressed regardless of theme.
    pub fn with_no_color(mut self, no_color: bool) -> Self {
        self.runtime.no_color = no_color;
        self
    }

    /// Sets the measured terminal width.
    pub fn with_width(mut self, width: usize) -> Self {
        self.runtime.width = Some(width);
        self
    }

    /// Sets whether the locale is known to support UTF-8.
    pub fn with_locale_utf8(mut self, locale_utf8: bool) -> Self {
        self.runtime.locale_utf8 = Some(locale_utf8);
        self
    }

    /// Builds the runtime terminal facts.
    pub fn build(self) -> RenderRuntime {
        self.runtime
    }
}

/// User-configurable settings for rendering CLI output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HelpChromeSettings {
    /// Border style override for help/guide tables.
    pub table_chrome: HelpTableChrome,
    /// Explicit indentation override for help entries.
    pub entry_indent: Option<usize>,
    /// Explicit gap override between help entry columns.
    pub entry_gap: Option<usize>,
    /// Explicit spacing override between help sections.
    pub section_spacing: Option<usize>,
}

/// User-configurable settings for rendering CLI output.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RenderSettings {
    /// Preferred output format.
    pub format: OutputFormat,
    /// Whether `format` was chosen explicitly by the caller.
    pub format_explicit: bool,
    /// Preferred rendering mode.
    pub mode: RenderMode,
    /// Color behavior selection.
    pub color: ColorMode,
    /// Unicode behavior selection.
    pub unicode: UnicodeMode,
    /// Explicit width override for rendering.
    pub width: Option<usize>,
    /// Left margin applied to rendered blocks.
    pub margin: usize,
    /// Indentation width used for nested structures.
    pub indent_size: usize,
    /// Maximum list length rendered in compact form.
    pub short_list_max: usize,
    /// Maximum list length rendered in medium form before expanding further.
    pub medium_list_max: usize,
    /// Horizontal padding between grid columns.
    pub grid_padding: usize,
    /// Explicit grid column count override.
    pub grid_columns: Option<usize>,
    /// Relative weighting for adaptive grid columns.
    pub column_weight: usize,
    /// Overflow policy for table cells.
    pub table_overflow: TableOverflow,
    /// Border style for general table rendering.
    pub table_border: TableBorderStyle,
    /// Help/guide-specific chrome settings.
    pub help_chrome: HelpChromeSettings,
    /// Minimum width before stacked MREG columns are used.
    pub mreg_stack_min_col_width: usize,
    /// Threshold controlling when MREG content overflows into stacked mode.
    pub mreg_stack_overflow_ratio: usize,
    /// Selected theme name.
    pub theme_name: String,
    /// Cached resolved theme derived from `theme_name`.
    ///
    /// This stays crate-internal so external callers cannot create
    /// contradictory `theme_name` / resolved-theme pairs.
    pub(crate) theme: Option<ThemeDefinition>,
    /// Per-token style overrides layered on top of the theme.
    pub style_overrides: StyleOverrides,
    /// Section frame style used for grouped chrome.
    pub chrome_frame: SectionFrameStyle,
    /// Placement policy for ruled section separators across sibling sections.
    pub ruled_section_policy: RuledSectionPolicy,
    /// Fallback behavior for semantic guide output.
    pub guide_default_format: GuideDefaultFormat,
    /// Runtime terminal facts used during auto-resolution.
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
            help_chrome: HelpChromeSettings::default(),
            mreg_stack_min_col_width: 10,
            mreg_stack_overflow_ratio: 200,
            theme_name: crate::ui::theme::DEFAULT_THEME_NAME.to_string(),
            theme: None,
            style_overrides: crate::ui::style::StyleOverrides::default(),
            chrome_frame: SectionFrameStyle::Top,
            ruled_section_policy: RuledSectionPolicy::PerSection,
            guide_default_format: GuideDefaultFormat::Guide,
            runtime: RenderRuntime::default(),
        }
    }
}

/// Builder for [`RenderSettings`].
#[derive(Debug, Clone, Default)]
pub struct RenderSettingsBuilder {
    settings: RenderSettings,
}

impl RenderSettingsBuilder {
    /// Creates a builder seeded with [`RenderSettings::default`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a builder seeded with [`RenderSettings::test_plain`].
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

    /// Sets the preferred output format.
    pub fn with_format(mut self, format: OutputFormat) -> Self {
        self.settings.format = format;
        self
    }

    /// Sets whether the output format was chosen explicitly.
    pub fn with_format_explicit(mut self, format_explicit: bool) -> Self {
        self.settings.format_explicit = format_explicit;
        self
    }

    /// Sets the preferred rendering mode.
    pub fn with_mode(mut self, mode: RenderMode) -> Self {
        self.settings.mode = mode;
        self
    }

    /// Sets color behavior.
    pub fn with_color(mut self, color: ColorMode) -> Self {
        self.settings.color = color;
        self
    }

    /// Sets Unicode behavior.
    pub fn with_unicode(mut self, unicode: UnicodeMode) -> Self {
        self.settings.unicode = unicode;
        self
    }

    /// Sets an explicit width override.
    pub fn with_width(mut self, width: usize) -> Self {
        self.settings.width = Some(width);
        self
    }

    /// Sets the left margin.
    pub fn with_margin(mut self, margin: usize) -> Self {
        self.settings.margin = margin;
        self
    }

    /// Sets the indentation width for nested structures.
    pub fn with_indent_size(mut self, indent_size: usize) -> Self {
        self.settings.indent_size = indent_size;
        self
    }

    /// Sets the overflow policy for table cells.
    pub fn with_table_overflow(mut self, table_overflow: TableOverflow) -> Self {
        self.settings.table_overflow = table_overflow;
        self
    }

    /// Sets the general table border style.
    pub fn with_table_border(mut self, table_border: TableBorderStyle) -> Self {
        self.settings.table_border = table_border;
        self
    }

    /// Sets the grouped help/guide chrome settings.
    pub fn with_help_chrome(mut self, help_chrome: HelpChromeSettings) -> Self {
        self.settings.help_chrome = help_chrome;
        self
    }

    /// Sets the selected theme name.
    pub fn with_theme_name(mut self, theme_name: impl Into<String>) -> Self {
        self.settings.theme_name = theme_name.into();
        self
    }

    /// Sets style overrides layered over the theme.
    pub fn with_style_overrides(mut self, style_overrides: StyleOverrides) -> Self {
        self.settings.style_overrides = style_overrides;
        self
    }

    /// Sets the section frame style.
    pub fn with_chrome_frame(mut self, chrome_frame: SectionFrameStyle) -> Self {
        self.settings.chrome_frame = chrome_frame;
        self
    }

    /// Sets how ruled separators are shared across sibling sections.
    pub fn with_ruled_section_policy(mut self, ruled_section_policy: RuledSectionPolicy) -> Self {
        self.settings.ruled_section_policy = ruled_section_policy;
        self
    }

    /// Sets the default guide-rendering preference.
    pub fn with_guide_default_format(mut self, guide_default_format: GuideDefaultFormat) -> Self {
        self.settings.guide_default_format = guide_default_format;
        self
    }

    /// Sets the runtime terminal facts used during auto-resolution.
    pub fn with_runtime(mut self, runtime: RenderRuntime) -> Self {
        self.settings.runtime = runtime;
        self
    }

    /// Builds the render settings value.
    pub fn build(self) -> RenderSettings {
        self.settings
    }
}

/// Default output format to use when guide rendering is not explicitly requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GuideDefaultFormat {
    /// Prefer semantic guide output when the caller did not request a format.
    #[default]
    Guide,
    /// Inherit the caller-selected format without forcing guide mode.
    Inherit,
}

impl GuideDefaultFormat {
    /// Parses the guide fallback mode used when no explicit output format wins.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::ui::GuideDefaultFormat;
    ///
    /// assert_eq!(GuideDefaultFormat::parse("guide"), Some(GuideDefaultFormat::Guide));
    /// assert_eq!(GuideDefaultFormat::parse("none"), Some(GuideDefaultFormat::Inherit));
    /// assert_eq!(GuideDefaultFormat::parse("wat"), None);
    /// ```
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "guide" => Some(Self::Guide),
            "inherit" | "none" => Some(Self::Inherit),
            _ => None,
        }
    }
}

/// Rendering backend selected for the current output pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderBackend {
    /// Render without terminal-rich features.
    Plain,
    /// Render using ANSI and richer terminal affordances.
    Rich,
}

/// Overflow strategy for table cell content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableOverflow {
    /// Leave overflow management to the terminal.
    None,
    /// Hard-clip overflowing cell content.
    Clip,
    /// Truncate overflowing content with an ellipsis marker.
    Ellipsis,
    /// Wrap overflowing content onto multiple lines.
    Wrap,
}

impl TableOverflow {
    /// Parses the table-cell overflow policy accepted by config and flags.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::ui::TableOverflow;
    ///
    /// assert_eq!(TableOverflow::parse("wrap"), Some(TableOverflow::Wrap));
    /// assert_eq!(TableOverflow::parse("truncate"), Some(TableOverflow::Ellipsis));
    /// assert_eq!(TableOverflow::parse("wat"), None);
    /// ```
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

/// Border style applied to rendered tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableBorderStyle {
    /// Render tables without outer borders.
    None,
    /// Render tables with square box-drawing borders.
    #[default]
    Square,
    /// Render tables with rounded box-drawing borders.
    Round,
}

impl TableBorderStyle {
    /// Parses the table border chrome accepted by config and flags.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::ui::TableBorderStyle;
    ///
    /// assert_eq!(TableBorderStyle::parse("box"), Some(TableBorderStyle::Square));
    /// assert_eq!(TableBorderStyle::parse("rounded"), Some(TableBorderStyle::Round));
    /// assert_eq!(TableBorderStyle::parse("wat"), None);
    /// ```
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "none" | "plain" => Some(Self::None),
            "square" | "box" | "boxed" => Some(Self::Square),
            "round" | "rounded" => Some(Self::Round),
            _ => None,
        }
    }
}

/// Border style override for help tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HelpTableChrome {
    /// Reuse the normal table border style.
    Inherit,
    /// Render help tables without box chrome.
    #[default]
    None,
    /// Render help tables with square box-drawing borders.
    Square,
    /// Render help tables with rounded box-drawing borders.
    Round,
}

impl HelpTableChrome {
    /// Parses the help-table chrome override accepted by config.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::ui::HelpTableChrome;
    ///
    /// assert_eq!(HelpTableChrome::parse("inherit"), Some(HelpTableChrome::Inherit));
    /// assert_eq!(HelpTableChrome::parse("plain"), Some(HelpTableChrome::None));
    /// assert_eq!(HelpTableChrome::parse("wat"), None);
    /// ```
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "inherit" => Some(Self::Inherit),
            "none" | "plain" => Some(Self::None),
            "square" | "box" | "boxed" => Some(Self::Square),
            "round" | "rounded" => Some(Self::Round),
            _ => None,
        }
    }

    /// Resolves the concrete help-table border after applying the override.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::ui::{HelpTableChrome, TableBorderStyle};
    ///
    /// assert_eq!(
    ///     HelpTableChrome::Inherit.resolve(TableBorderStyle::Round),
    ///     TableBorderStyle::Round
    /// );
    /// assert_eq!(
    ///     HelpTableChrome::Square.resolve(TableBorderStyle::None),
    ///     TableBorderStyle::Square
    /// );
    /// ```
    pub fn resolve(self, table_border: TableBorderStyle) -> TableBorderStyle {
        match self {
            Self::Inherit => table_border,
            Self::None => TableBorderStyle::None,
            Self::Square => TableBorderStyle::Square,
            Self::Round => TableBorderStyle::Round,
        }
    }
}

impl RenderSettings {
    /// Starts building render settings from the default UI baseline.
    pub fn builder() -> RenderSettingsBuilder {
        RenderSettingsBuilder::new()
    }

    /// Shared plain-mode baseline for deterministic tests and examples.
    ///
    /// This keeps docs and tests from duplicating a large struct literal every
    /// time they need a stable no-color rendering baseline.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::output::OutputFormat;
    /// use osp_cli::ui::{RenderBackend, RenderSettings};
    ///
    /// let resolved = RenderSettings::test_plain(OutputFormat::Json)
    ///     .resolve_render_settings();
    ///
    /// assert_eq!(resolved.backend, RenderBackend::Plain);
    /// assert!(!resolved.color);
    /// assert!(!resolved.unicode);
    /// ```
    pub fn test_plain(format: OutputFormat) -> Self {
        RenderSettingsBuilder::plain(format).build()
    }

    /// Returns whether guide output should be preferred for the current
    /// settings.
    ///
    /// This only falls back to guide mode when the caller did not explicitly
    /// request another format.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::output::OutputFormat;
    /// use osp_cli::ui::{GuideDefaultFormat, RenderSettings};
    ///
    /// let mut settings = RenderSettings::test_plain(OutputFormat::Auto);
    /// settings.format_explicit = false;
    /// settings.guide_default_format = GuideDefaultFormat::Guide;
    /// assert!(settings.prefers_guide_rendering());
    ///
    /// settings.format_explicit = true;
    /// settings.format = OutputFormat::Json;
    /// assert!(!settings.prefers_guide_rendering());
    /// ```
    pub fn prefers_guide_rendering(&self) -> bool {
        matches!(self.format, OutputFormat::Guide)
            || (!self.format_explicit
                && matches!(self.guide_default_format, GuideDefaultFormat::Guide))
    }
}

/// Renders rows using the configured output format.
pub fn render_rows(rows: &[Row], settings: &RenderSettings) -> String {
    render_output(
        &OutputResult {
            items: OutputItems::Rows(rows.to_vec()),
            document: None,
            meta: Default::default(),
        },
        settings,
    )
}

/// Renders a structured output result using the configured output format.
pub fn render_output(output: &OutputResult, settings: &RenderSettings) -> String {
    let plan = settings.resolve_render_plan(output);
    if matches!(plan.format, OutputFormat::Markdown)
        && let Some(guide) = GuideView::try_from_output_result(output)
    {
        return guide.to_markdown_with_width(plan.render.width);
    }
    let document = format::build_document_from_output_plan(output, &plan);
    renderer::render_document(&document, plan.render)
}

fn render_guide_document(document: &Document, settings: &RenderSettings) -> String {
    let mut rendered = render_document_resolved(document, settings.resolve_render_settings());
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    rendered
}

pub(crate) fn render_guide_view_with_options(
    guide: &GuideView,
    settings: &RenderSettings,
    options: crate::ui::format::help::GuideRenderOptions<'_>,
) -> String {
    if matches!(
        format::resolve_output_format(&guide.to_output_result(), settings),
        OutputFormat::Guide
    ) {
        let document = crate::ui::format::help::build_guide_document_from_view(guide, options);
        return render_guide_document(&document, settings);
    }

    render_output(&guide.to_output_result(), settings)
}

pub(crate) fn render_guide_payload(
    config: &crate::config::ResolvedConfig,
    settings: &RenderSettings,
    guide: &GuideView,
) -> String {
    render_guide_payload_with_layout(
        guide,
        settings,
        crate::ui::presentation::help_layout(config),
    )
}

pub(crate) fn render_guide_payload_with_layout(
    guide: &GuideView,
    settings: &RenderSettings,
    layout: crate::ui::presentation::HelpLayout,
) -> String {
    let guide_settings = settings.resolve_guide_render_settings();
    render_guide_view_with_options(
        guide,
        settings,
        crate::ui::format::help::GuideRenderOptions {
            title_prefix: None,
            layout,
            guide: guide_settings,
            panel_kind: None,
        },
    )
}

pub(crate) fn render_guide_output_with_options(
    output: &OutputResult,
    settings: &RenderSettings,
    options: crate::ui::format::help::GuideRenderOptions<'_>,
) -> String {
    if matches!(
        format::resolve_output_format(output, settings),
        OutputFormat::Guide
    ) && let Some(guide) = GuideView::try_from_output_result(output)
    {
        return render_guide_view_with_options(&guide, settings, options);
    }

    render_output(output, settings)
}

pub(crate) fn guide_render_options<'a>(
    config: &'a crate::config::ResolvedConfig,
    settings: &'a RenderSettings,
) -> crate::ui::format::help::GuideRenderOptions<'a> {
    let guide_settings = settings.resolve_guide_render_settings();
    crate::ui::format::help::GuideRenderOptions {
        title_prefix: None,
        layout: crate::ui::presentation::help_layout(config),
        guide: guide_settings,
        panel_kind: None,
    }
}

pub(crate) fn render_structured_output(
    config: &crate::config::ResolvedConfig,
    settings: &RenderSettings,
    output: &OutputResult,
) -> String {
    if GuideView::try_from_output_result(output).is_some() {
        return render_guide_output_with_options(
            output,
            settings,
            guide_render_options(config, settings),
        );
    }
    render_output(output, settings)
}

/// Renders a document directly with the resolved UI settings.
pub fn render_document(document: &Document, settings: &RenderSettings) -> String {
    let resolved = settings.resolve_render_settings();
    renderer::render_document(document, resolved)
}

pub(crate) fn render_document_resolved(
    document: &Document,
    settings: ResolvedRenderSettings,
) -> String {
    renderer::render_document(document, settings)
}

/// Renders rows in plain copy-safe form.
///
/// Copy helpers intentionally bypass ANSI and rich terminal styling so the
/// clipboard gets stable plain text.
///
/// # Examples
///
/// ```
/// use osp_cli::core::output::OutputFormat;
/// use osp_cli::row;
/// use osp_cli::ui::{RenderSettings, render_rows_for_copy};
///
/// let rendered = render_rows_for_copy(
///     &[row! { "uid" => "alice" }],
///     &RenderSettings::test_plain(OutputFormat::Json),
/// );
///
/// assert!(rendered.contains("\"uid\": \"alice\""));
/// ```
pub fn render_rows_for_copy(rows: &[Row], settings: &RenderSettings) -> String {
    render_output_for_copy(
        &OutputResult {
            items: OutputItems::Rows(rows.to_vec()),
            document: None,
            meta: Default::default(),
        },
        settings,
    )
}

/// Renders an output result in plain copy-safe form.
pub fn render_output_for_copy(output: &OutputResult, settings: &RenderSettings) -> String {
    let copy_settings = settings.plain_copy_settings();
    let plan = copy_settings.resolve_render_plan(output);
    if matches!(plan.format, OutputFormat::Markdown)
        && let Some(guide) = GuideView::try_from_output_result(output)
    {
        return guide.to_markdown_with_width(plan.render.width);
    }
    let document = format::build_document_from_output_plan(output, &plan);
    renderer::render_document(&document, plan.render)
}

/// Renders a document in plain copy-safe form.
pub fn render_document_for_copy(document: &Document, settings: &RenderSettings) -> String {
    let copy_settings = settings.plain_copy_settings();
    let resolved = copy_settings.resolve_render_settings();
    renderer::render_document(document, resolved)
}

/// Copies rendered rows to the configured clipboard service.
pub fn copy_rows_to_clipboard(
    rows: &[Row],
    settings: &RenderSettings,
    clipboard: &clipboard::ClipboardService,
) -> Result<(), clipboard::ClipboardError> {
    copy_output_to_clipboard(
        &OutputResult {
            items: OutputItems::Rows(rows.to_vec()),
            document: None,
            meta: Default::default(),
        },
        settings,
        clipboard,
    )
}

/// Copies rendered output to the configured clipboard service.
pub fn copy_output_to_clipboard(
    output: &OutputResult,
    settings: &RenderSettings,
    clipboard: &clipboard::ClipboardService,
) -> Result<(), clipboard::ClipboardError> {
    let text = render_output_for_copy(output, settings);
    clipboard.copy_text(&text)
}

#[cfg(test)]
mod tests {
    use super::{
        GuideDefaultFormat, HelpChromeSettings, HelpTableChrome, RenderBackend, RenderRuntime,
        RenderSettings, RenderSettingsBuilder, TableBorderStyle, TableOverflow, format,
        render_document, render_document_for_copy, render_output, render_output_for_copy,
        render_rows, render_rows_for_copy,
    };
    use crate::core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
    use crate::core::output_model::OutputResult;
    use crate::core::row::Row;
    use crate::guide::GuideView;
    use crate::ui::document::{Block, MregValue, TableStyle};
    use serde_json::json;

    fn settings(format: OutputFormat) -> RenderSettings {
        RenderSettings {
            mode: RenderMode::Auto,
            ..RenderSettings::test_plain(format)
        }
    }

    #[test]
    fn document_builder_selects_auto_and_explicit_block_shapes_unit() {
        let value_rows = vec![{
            let mut row = Row::new();
            row.insert("value".to_string(), json!("hello"));
            row
        }];
        let document = format::build_document(&value_rows, &settings(OutputFormat::Auto));
        assert!(matches!(document.blocks[0], Block::Value(_)));

        let mreg_rows = vec![{
            let mut row = Row::new();
            row.insert("uid".to_string(), json!("oistes"));
            row
        }];
        let document = format::build_document(&mreg_rows, &settings(OutputFormat::Auto));
        assert!(matches!(document.blocks[0], Block::Mreg(_)));

        let table_rows = vec![
            {
                let mut row = Row::new();
                row.insert("uid".to_string(), json!("one"));
                row
            },
            {
                let mut row = Row::new();
                row.insert("uid".to_string(), json!("two"));
                row
            },
        ];
        let document = format::build_document(&table_rows, &settings(OutputFormat::Auto));
        assert!(matches!(document.blocks[0], Block::Table(_)));

        let rich_rows = vec![{
            let mut row = Row::new();
            row.insert("uid".to_string(), json!("oistes"));
            row.insert("groups".to_string(), json!(["a", "b"]));
            row
        }];
        let document = format::build_document(&rich_rows, &settings(OutputFormat::Mreg));
        let Block::Mreg(block) = &document.blocks[0] else {
            panic!("expected mreg block");
        };
        assert_eq!(block.rows.len(), 1);
        assert!(
            block.rows[0]
                .entries
                .iter()
                .any(|entry| matches!(entry.value, MregValue::Scalar(_)))
        );
        assert!(
            block.rows[0]
                .entries
                .iter()
                .any(|entry| matches!(entry.value, MregValue::VerticalList(_)))
        );

        let markdown_rows = vec![{
            let mut row = Row::new();
            row.insert("uid".to_string(), json!("oistes"));
            row
        }];
        let document = format::build_document(&markdown_rows, &settings(OutputFormat::Markdown));
        let Block::Table(table) = &document.blocks[0] else {
            panic!("expected table block");
        };
        assert_eq!(table.style, TableStyle::Markdown);
    }

    #[test]
    fn semantic_guide_markdown_output_and_copy_remain_section_based_unit() {
        let output =
            GuideView::from_text("Usage: osp history <COMMAND>\n\nCommands:\n  list  Show\n")
                .to_output_result();
        let settings = RenderSettings {
            format: OutputFormat::Markdown,
            format_explicit: true,
            ..settings(OutputFormat::Markdown)
        };

        let rendered = render_output(&output, &settings);
        let copied = render_output_for_copy(&output, &settings);

        for text in [&rendered, &copied] {
            assert!(text.contains("## Usage"));
            assert!(text.contains("## Commands"));
            assert!(text.contains("- `list` Show"));
            assert!(!text.contains("| name"));
        }
        assert!(!copied.contains("\x1b["));
    }

    #[test]
    fn render_builders_and_parse_helpers_cover_configuration_surface_unit() {
        let runtime = RenderRuntime::builder()
            .with_stdout_is_tty(true)
            .with_terminal("xterm-256color")
            .with_no_color(true)
            .with_width(98)
            .with_locale_utf8(false)
            .build();
        assert_eq!(
            runtime,
            RenderRuntime {
                stdout_is_tty: true,
                terminal: Some("xterm-256color".to_string()),
                no_color: true,
                width: Some(98),
                locale_utf8: Some(false),
            }
        );

        let settings = RenderSettings::builder()
            .with_format(OutputFormat::Markdown)
            .with_format_explicit(true)
            .with_mode(RenderMode::Rich)
            .with_color(ColorMode::Always)
            .with_unicode(UnicodeMode::Auto)
            .with_width(98)
            .with_margin(2)
            .with_indent_size(4)
            .with_table_overflow(TableOverflow::Wrap)
            .with_table_border(TableBorderStyle::Round)
            .with_help_chrome(HelpChromeSettings {
                table_chrome: HelpTableChrome::Inherit,
                ..HelpChromeSettings::default()
            })
            .with_theme_name("dracula")
            .with_style_overrides(Default::default())
            .with_chrome_frame(crate::ui::SectionFrameStyle::Round)
            .with_guide_default_format(GuideDefaultFormat::Inherit)
            .with_runtime(runtime.clone())
            .build();
        assert_eq!(settings.format, OutputFormat::Markdown);
        assert!(settings.format_explicit);
        assert_eq!(settings.mode, RenderMode::Rich);
        assert_eq!(settings.color, ColorMode::Always);
        assert_eq!(settings.unicode, UnicodeMode::Auto);
        assert_eq!(settings.width, Some(98));
        assert_eq!(settings.margin, 2);
        assert_eq!(settings.indent_size, 4);
        assert_eq!(settings.table_overflow, TableOverflow::Wrap);
        assert_eq!(settings.table_border, TableBorderStyle::Round);
        assert_eq!(settings.help_chrome.table_chrome, HelpTableChrome::Inherit);
        assert_eq!(settings.theme_name, "dracula");
        assert_eq!(settings.chrome_frame, crate::ui::SectionFrameStyle::Round);
        assert_eq!(settings.guide_default_format, GuideDefaultFormat::Inherit);
        assert_eq!(settings.runtime, runtime);

        let plain = RenderSettingsBuilder::plain(OutputFormat::Json).build();
        assert_eq!(plain.mode, RenderMode::Plain);
        assert_eq!(plain.color, ColorMode::Never);
        assert_eq!(plain.unicode, UnicodeMode::Never);

        assert_eq!(
            GuideDefaultFormat::parse("none"),
            Some(GuideDefaultFormat::Inherit)
        );
        assert_eq!(GuideDefaultFormat::parse("wat"), None);
        assert_eq!(
            HelpTableChrome::parse("round"),
            Some(HelpTableChrome::Round)
        );
        assert_eq!(HelpTableChrome::parse("wat"), None);
        assert_eq!(
            HelpTableChrome::Inherit.resolve(TableBorderStyle::Round),
            TableBorderStyle::Round
        );
        assert_eq!(
            HelpTableChrome::None.resolve(TableBorderStyle::Square),
            TableBorderStyle::None
        );
        assert_eq!(
            HelpTableChrome::Square.resolve(TableBorderStyle::None),
            TableBorderStyle::Square
        );
        assert_eq!(
            TableBorderStyle::parse("none"),
            Some(TableBorderStyle::None)
        );
        assert_eq!(
            TableBorderStyle::parse("box"),
            Some(TableBorderStyle::Square)
        );
        assert_eq!(
            TableBorderStyle::parse("square"),
            Some(TableBorderStyle::Square)
        );
        assert_eq!(
            TableBorderStyle::parse("round"),
            Some(TableBorderStyle::Round)
        );
        assert_eq!(
            TableBorderStyle::parse("rounded"),
            Some(TableBorderStyle::Round)
        );
        assert_eq!(TableBorderStyle::parse("mystery"), None);
        assert_eq!(TableOverflow::parse("visible"), Some(TableOverflow::None));
        assert_eq!(TableOverflow::parse("crop"), Some(TableOverflow::Clip));
        assert_eq!(
            TableOverflow::parse("truncate"),
            Some(TableOverflow::Ellipsis)
        );
        assert_eq!(TableOverflow::parse("wrapped"), Some(TableOverflow::Wrap));
        assert_eq!(TableOverflow::parse("other"), None);
    }

    #[test]
    fn render_resolution_covers_public_helpers_mode_runtime_and_force_rules_unit() {
        let rows = vec![{
            let mut row = Row::new();
            row.insert("uid".to_string(), json!("alice"));
            row
        }];
        let rendered = render_rows(&rows, &settings(OutputFormat::Table));
        assert!(rendered.contains("uid"));
        assert!(rendered.contains("alice"));

        let dumb_terminal = RenderSettings {
            mode: RenderMode::Rich,
            color: ColorMode::Auto,
            unicode: UnicodeMode::Auto,
            width: Some(0),
            grid_columns: Some(0),
            runtime: RenderRuntime {
                stdout_is_tty: true,
                terminal: Some("dumb".to_string()),
                no_color: false,
                width: Some(0),
                locale_utf8: Some(true),
            },
            ..RenderSettings::test_plain(OutputFormat::Table)
        };
        let dumb_resolved = dumb_terminal.resolve_render_settings();
        assert_eq!(dumb_resolved.backend, RenderBackend::Rich);
        assert!(dumb_resolved.color);
        assert!(!dumb_resolved.unicode);
        assert_eq!(dumb_resolved.width, None);
        assert_eq!(dumb_resolved.grid_columns, None);

        let locale_false = RenderSettings {
            mode: RenderMode::Rich,
            color: ColorMode::Auto,
            unicode: UnicodeMode::Auto,
            runtime: RenderRuntime {
                stdout_is_tty: true,
                terminal: Some("xterm-256color".to_string()),
                no_color: false,
                width: Some(72),
                locale_utf8: Some(false),
            },
            ..RenderSettings::test_plain(OutputFormat::Table)
        };
        let locale_resolved = locale_false.resolve_render_settings();
        assert!(locale_resolved.color);
        assert!(!locale_resolved.unicode);
        assert_eq!(locale_resolved.width, Some(72));

        let plain = RenderSettings {
            format: OutputFormat::Table,
            color: ColorMode::Always,
            unicode: UnicodeMode::Always,
            ..RenderSettings::test_plain(OutputFormat::Table)
        };
        let resolved = plain.resolve_render_settings();
        assert_eq!(resolved.backend, RenderBackend::Plain);
        assert!(!resolved.color);
        assert!(!resolved.unicode);

        let rich = RenderSettings {
            format: OutputFormat::Table,
            mode: RenderMode::Rich,
            color: ColorMode::Always,
            unicode: UnicodeMode::Always,
            ..RenderSettings::test_plain(OutputFormat::Table)
        };
        let resolved = rich.resolve_render_settings();
        assert_eq!(resolved.backend, RenderBackend::Rich);
        assert!(resolved.color);
        assert!(resolved.unicode);
        let auto = RenderSettings {
            mode: RenderMode::Auto,
            color: ColorMode::Auto,
            unicode: UnicodeMode::Auto,
            runtime: super::RenderRuntime {
                stdout_is_tty: true,
                terminal: Some("dumb".to_string()),
                no_color: false,
                width: Some(72),
                locale_utf8: Some(false),
            },
            ..RenderSettings::test_plain(OutputFormat::Table)
        };
        let resolved = auto.resolve_render_settings();
        assert_eq!(resolved.backend, RenderBackend::Plain);
        assert!(!resolved.color);
        assert!(!resolved.unicode);
        assert_eq!(resolved.width, Some(72));

        let forced_color = RenderSettings {
            mode: RenderMode::Auto,
            color: ColorMode::Always,
            unicode: UnicodeMode::Auto,
            runtime: super::RenderRuntime {
                stdout_is_tty: false,
                terminal: Some("xterm-256color".to_string()),
                no_color: false,
                width: Some(80),
                locale_utf8: Some(true),
            },
            ..RenderSettings::test_plain(OutputFormat::Table)
        };
        let resolved = forced_color.resolve_render_settings();
        assert_eq!(resolved.backend, RenderBackend::Rich);
        assert!(resolved.color);

        let forced_unicode = RenderSettings {
            mode: RenderMode::Auto,
            color: ColorMode::Auto,
            unicode: UnicodeMode::Always,
            runtime: super::RenderRuntime {
                stdout_is_tty: false,
                terminal: Some("dumb".to_string()),
                no_color: true,
                width: Some(64),
                locale_utf8: Some(false),
            },
            ..RenderSettings::test_plain(OutputFormat::Table)
        };
        let resolved = forced_unicode.resolve_render_settings();
        assert_eq!(resolved.backend, RenderBackend::Rich);
        assert!(!resolved.color);
        assert!(resolved.unicode);

        let guide_settings = RenderSettings {
            help_chrome: HelpChromeSettings {
                table_chrome: HelpTableChrome::Inherit,
                entry_indent: Some(4),
                entry_gap: Some(3),
                section_spacing: Some(0),
            },
            table_border: TableBorderStyle::Round,
            chrome_frame: crate::ui::SectionFrameStyle::TopBottom,
            ..RenderSettings::test_plain(OutputFormat::Guide)
        };
        let guide_resolved = guide_settings.resolve_guide_render_settings();
        assert_eq!(
            guide_resolved.frame_style,
            crate::ui::SectionFrameStyle::TopBottom
        );
        assert_eq!(
            guide_resolved.help_chrome.table_border,
            TableBorderStyle::Round
        );
        assert_eq!(guide_resolved.help_chrome.entry_indent, Some(4));
        assert_eq!(guide_resolved.help_chrome.entry_gap, Some(3));
        assert_eq!(guide_resolved.help_chrome.section_spacing, Some(0));

        let mreg_settings = RenderSettings {
            short_list_max: 0,
            medium_list_max: 0,
            indent_size: 0,
            mreg_stack_min_col_width: 0,
            mreg_stack_overflow_ratio: 10,
            ..RenderSettings::test_plain(OutputFormat::Mreg)
        };
        let mreg_resolved = mreg_settings.resolve_mreg_build_settings();
        assert_eq!(mreg_resolved.short_list_max, 1);
        assert_eq!(mreg_resolved.medium_list_max, 2);
        assert_eq!(mreg_resolved.indent_size, 1);
        assert_eq!(mreg_resolved.stack_min_col_width, 1);
        assert_eq!(mreg_resolved.stack_overflow_ratio, 100);
    }

    #[test]
    fn copy_helpers_force_plain_copy_mode_for_rows_documents_and_json_unit() {
        let table_rows = vec![{
            let mut row = Row::new();
            row.insert("uid".to_string(), json!("oistes"));
            row.insert(
                "description".to_string(),
                json!("very long text that will be shown"),
            );
            row
        }];
        let rich_table = RenderSettings {
            format: OutputFormat::Table,
            mode: RenderMode::Rich,
            color: ColorMode::Always,
            unicode: UnicodeMode::Always,
            ..RenderSettings::test_plain(OutputFormat::Table)
        };
        let table_copy = render_rows_for_copy(&table_rows, &rich_table);
        assert!(!table_copy.contains("\x1b["));
        assert!(!table_copy.contains('┌'));
        assert!(table_copy.contains('+'));

        let value_rows = vec![{
            let mut row = Row::new();
            row.insert("value".to_string(), json!("hello"));
            row
        }];
        let rich_value = RenderSettings {
            mode: RenderMode::Rich,
            color: ColorMode::Always,
            unicode: UnicodeMode::Always,
            ..RenderSettings::test_plain(OutputFormat::Value)
        };
        let value_copy = render_rows_for_copy(&value_rows, &rich_value);
        assert_eq!(value_copy.trim(), "hello");
        assert!(!value_copy.contains("\x1b["));

        let document = crate::ui::Document {
            blocks: vec![Block::Line(crate::ui::LineBlock {
                parts: vec![crate::ui::LinePart {
                    text: "hello".to_string(),
                    token: None,
                }],
            })],
        };
        let rich_document = RenderSettings {
            mode: RenderMode::Rich,
            color: ColorMode::Always,
            unicode: UnicodeMode::Always,
            ..RenderSettings::test_plain(OutputFormat::Table)
        };
        let rich = render_document(&document, &rich_document);
        let copied = render_document_for_copy(&document, &rich_document);

        assert!(rich.contains("hello"));
        assert!(copied.contains("hello"));
        assert!(!copied.contains("\x1b["));

        let json_rows = vec![{
            let mut row = Row::new();
            row.insert("uid".to_string(), json!("alice"));
            row.insert("count".to_string(), json!(2));
            row
        }];
        let json_settings = RenderSettings {
            format: OutputFormat::Json,
            mode: RenderMode::Rich,
            color: ColorMode::Always,
            unicode: UnicodeMode::Always,
            ..RenderSettings::test_plain(OutputFormat::Json)
        };

        let output = OutputResult::from_rows(json_rows);
        let rendered = render_output(&output, &json_settings);
        let copied = render_output_for_copy(&output, &json_settings);

        assert!(rendered.contains("\"uid\""));
        assert!(rendered.contains("\x1b["));
        assert_eq!(
            copied,
            "[\n  {\n    \"uid\": \"alice\",\n    \"count\": 2\n  }\n]\n"
        );
        assert!(!copied.contains("\x1b["));
    }
}
