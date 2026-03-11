//! Buffered user-facing messages and their rendering helpers.
//!
//! This module exists so command execution can collect messages without
//! deciding immediately how they should be shown. Callers push semantic levels
//! into a [`MessageBuffer`], and the UI later renders that buffer in minimal or
//! grouped form using the active theme and terminal settings.
//!
//! Contract:
//!
//! - this module owns message grouping and severity presentation
//! - it should not own command execution or logging backends
//! - callers should treat `MessageLevel` as user-facing severity, not as a
//!   tracing subsystem

use crate::ui::document::{Block, Document, LineBlock, LinePart};
use crate::ui::inline::render_inline;
use crate::ui::renderer::render_document;
use crate::ui::style::{StyleOverrides, StyleToken};
use crate::ui::theme::ThemeDefinition;
use crate::ui::{RenderBackend, ResolvedRenderSettings};

use crate::ui::chrome::{
    SectionFrameStyle, SectionRenderContext, SectionStyleTokens,
    render_section_block_with_overrides,
};

const ORDERED_MESSAGE_LEVELS: [MessageLevel; 5] = [
    MessageLevel::Error,
    MessageLevel::Warning,
    MessageLevel::Success,
    MessageLevel::Info,
    MessageLevel::Trace,
];

/// Severity level for buffered UI messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MessageLevel {
    /// Error output that should remain visible at every verbosity.
    Error,
    /// Warning output for degraded or surprising behavior.
    Warning,
    /// Success output for completed operations.
    Success,
    /// Informational output for normal command progress.
    Info,
    /// Trace or debug-style output.
    Trace,
}

impl MessageLevel {
    /// Parses the message-level spellings accepted by config and environment
    /// inputs.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::ui::MessageLevel;
    ///
    /// assert_eq!(MessageLevel::parse("warn"), Some(MessageLevel::Warning));
    /// assert_eq!(MessageLevel::parse(" INFO "), Some(MessageLevel::Info));
    /// assert_eq!(MessageLevel::parse("wat"), None);
    /// ```
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "error" => Some(MessageLevel::Error),
            "warning" | "warn" => Some(MessageLevel::Warning),
            "success" => Some(MessageLevel::Success),
            "info" => Some(MessageLevel::Info),
            "trace" => Some(MessageLevel::Trace),
            _ => None,
        }
    }

    fn ordered() -> impl Iterator<Item = Self> {
        ORDERED_MESSAGE_LEVELS.into_iter()
    }

    /// Returns the section title used for grouped rendering.
    pub fn title(self) -> &'static str {
        match self {
            MessageLevel::Error => "Errors",
            MessageLevel::Warning => "Warnings",
            MessageLevel::Success => "Success",
            MessageLevel::Info => "Info",
            MessageLevel::Trace => "Trace",
        }
    }

    fn as_rank(self) -> i8 {
        match self {
            MessageLevel::Error => 0,
            MessageLevel::Warning => 1,
            MessageLevel::Success => 2,
            MessageLevel::Info => 3,
            MessageLevel::Trace => 4,
        }
    }

    /// Returns the lowercase identifier used in environment-style output.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::ui::MessageLevel;
    ///
    /// assert_eq!(MessageLevel::Warning.as_env_str(), "warning");
    /// ```
    pub fn as_env_str(self) -> &'static str {
        match self {
            MessageLevel::Error => "error",
            MessageLevel::Warning => "warning",
            MessageLevel::Success => "success",
            MessageLevel::Info => "info",
            MessageLevel::Trace => "trace",
        }
    }

    fn from_rank(rank: i8) -> Self {
        match rank {
            i8::MIN..=0 => MessageLevel::Error,
            1 => MessageLevel::Warning,
            2 => MessageLevel::Success,
            3 => MessageLevel::Info,
            _ => MessageLevel::Trace,
        }
    }

    fn style_token(self) -> StyleToken {
        match self {
            MessageLevel::Error => StyleToken::MessageError,
            MessageLevel::Warning => StyleToken::MessageWarning,
            MessageLevel::Success => StyleToken::MessageSuccess,
            MessageLevel::Info => StyleToken::MessageInfo,
            MessageLevel::Trace => StyleToken::MessageTrace,
        }
    }
}

/// Layout style used when rendering buffered messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageLayout {
    /// Render each message inline without grouped section chrome.
    Minimal,
    /// Group messages by severity and render section chrome.
    Grouped,
}

impl MessageLayout {
    /// Parses the message layout spellings accepted by configuration.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::ui::MessageLayout;
    ///
    /// assert_eq!(MessageLayout::parse("minimal"), Some(MessageLayout::Minimal));
    /// assert_eq!(MessageLayout::parse("GROUPED"), Some(MessageLayout::Grouped));
    /// assert_eq!(MessageLayout::parse("dense"), None);
    /// ```
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "minimal" => Some(Self::Minimal),
            "grouped" => Some(Self::Grouped),
            _ => None,
        }
    }
}

/// A single UI message with its associated severity.
#[derive(Debug, Clone)]
pub struct UiMessage {
    /// Severity assigned to the message.
    pub level: MessageLevel,
    /// Renderable message text.
    pub text: String,
}

/// In-memory buffer for messages collected during command execution.
#[derive(Debug, Clone, Default)]
pub struct MessageBuffer {
    entries: Vec<UiMessage>,
}

/// Options for rendering grouped message output.
#[derive(Debug, Clone)]
pub struct GroupedRenderOptions<'a> {
    /// Highest message level that should be included in the output.
    pub max_level: MessageLevel,
    /// Whether ANSI color output is enabled.
    pub color: bool,
    /// Whether Unicode box-drawing and symbols are enabled.
    pub unicode: bool,
    /// Optional output width constraint.
    pub width: Option<usize>,
    /// Active theme used for semantic styling.
    pub theme: &'a ThemeDefinition,
    /// Message layout mode.
    pub layout: MessageLayout,
    /// Frame style used for grouped section chrome.
    pub chrome_frame: SectionFrameStyle,
    /// Explicit semantic style overrides layered above the theme.
    pub style_overrides: StyleOverrides,
}

impl MessageBuffer {
    /// Appends a message to the buffer.
    pub fn push<T: Into<String>>(&mut self, level: MessageLevel, text: T) {
        self.entries.push(UiMessage {
            level,
            text: text.into(),
        });
    }

    /// Appends an error message to the buffer.
    pub fn error<T: Into<String>>(&mut self, text: T) {
        self.push(MessageLevel::Error, text);
    }

    /// Appends a warning message to the buffer.
    pub fn warning<T: Into<String>>(&mut self, text: T) {
        self.push(MessageLevel::Warning, text);
    }

    /// Appends a success message to the buffer.
    pub fn success<T: Into<String>>(&mut self, text: T) {
        self.push(MessageLevel::Success, text);
    }

    /// Appends an informational message to the buffer.
    pub fn info<T: Into<String>>(&mut self, text: T) {
        self.push(MessageLevel::Info, text);
    }

    /// Appends a trace message to the buffer.
    pub fn trace<T: Into<String>>(&mut self, text: T) {
        self.push(MessageLevel::Trace, text);
    }

    /// Returns `true` when the buffer contains no messages.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn entries_for_level(&self, level: MessageLevel) -> impl Iterator<Item = &UiMessage> {
        self.entries
            .iter()
            .filter(move |entry| entry.level == level)
    }

    /// Renders messages with the default plain grouped layout.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::ui::{MessageBuffer, MessageLevel};
    ///
    /// let mut messages = MessageBuffer::default();
    /// messages.error("bad");
    /// messages.success("done");
    ///
    /// let rendered = messages.render_grouped(MessageLevel::Success);
    ///
    /// assert!(rendered.contains("Errors"));
    /// assert!(rendered.contains("- bad"));
    /// assert!(rendered.contains("Success"));
    /// ```
    pub fn render_grouped(&self, max_level: MessageLevel) -> String {
        let theme = crate::ui::theme::resolve_theme("plain");
        self.render_grouped_styled(
            max_level,
            false,
            false,
            None,
            &theme,
            MessageLayout::Grouped,
        )
    }

    /// Renders messages with explicit theme and layout settings.
    pub fn render_grouped_styled(
        &self,
        max_level: MessageLevel,
        color: bool,
        unicode: bool,
        width: Option<usize>,
        theme: &ThemeDefinition,
        layout: MessageLayout,
    ) -> String {
        self.render_grouped_with_options(GroupedRenderOptions {
            max_level,
            color,
            unicode,
            width,
            theme,
            layout,
            chrome_frame: default_message_chrome_frame(layout),
            style_overrides: StyleOverrides::default(),
        })
    }

    /// Renders messages using a preassembled options struct.
    pub fn render_grouped_with_options(&self, options: GroupedRenderOptions<'_>) -> String {
        if matches!(options.layout, MessageLayout::Grouped) {
            return self.render_grouped_sections(&options);
        }

        let document = self.build_minimal_document(options.max_level);
        if document.blocks.is_empty() {
            return String::new();
        }

        let resolved = ResolvedRenderSettings {
            backend: if options.color || options.unicode {
                RenderBackend::Rich
            } else {
                RenderBackend::Plain
            },
            color: options.color,
            unicode: options.unicode,
            width: options.width,
            margin: 0,
            indent_size: 2,
            short_list_max: 1,
            medium_list_max: 5,
            grid_padding: 4,
            grid_columns: None,
            column_weight: 3,
            table_overflow: crate::ui::TableOverflow::Clip,
            table_border: crate::ui::TableBorderStyle::Square,
            help_table_border: crate::ui::TableBorderStyle::None,
            theme_name: options.theme.id.clone(),
            theme: options.theme.clone(),
            style_overrides: options.style_overrides,
            chrome_frame: options.chrome_frame,
        };
        render_document(&document, resolved)
    }

    fn render_grouped_sections(&self, options: &GroupedRenderOptions<'_>) -> String {
        let mut sections = Vec::new();

        for level in MessageLevel::ordered().filter(|level| *level <= options.max_level) {
            let mut entries = self.entries_for_level(level).peekable();
            if entries.peek().is_none() {
                continue;
            }

            let body = entries
                .map(|entry| {
                    render_inline(
                        &format!("- {}", entry.text),
                        options.color,
                        options.theme,
                        &options.style_overrides,
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");

            sections.push(render_section_block_with_overrides(
                level.title(),
                &body,
                options.chrome_frame,
                options.unicode,
                options.width,
                SectionRenderContext {
                    color: options.color,
                    theme: options.theme,
                    style_overrides: &options.style_overrides,
                },
                SectionStyleTokens::same(level.style_token()),
            ));
        }

        if sections.is_empty() {
            return String::new();
        }

        let mut out = sections.join("\n\n");
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out
    }

    fn build_minimal_document(&self, max_level: MessageLevel) -> Document {
        let mut blocks = Vec::new();

        for level in MessageLevel::ordered().filter(|level| *level <= max_level) {
            for entry in self.entries_for_level(level) {
                blocks.push(Block::Line(LineBlock {
                    parts: vec![
                        LinePart {
                            text: format!("{}: ", level.as_env_str()),
                            token: Some(level.style_token()),
                        },
                        LinePart {
                            text: entry.text.clone(),
                            token: None,
                        },
                    ],
                }));
            }
        }

        Document { blocks }
    }
}

fn default_message_chrome_frame(layout: MessageLayout) -> SectionFrameStyle {
    match layout {
        MessageLayout::Minimal => SectionFrameStyle::None,
        MessageLayout::Grouped => SectionFrameStyle::TopBottom,
    }
}

/// Adjusts a base verbosity level using `-v` and `-q` style counts.
///
/// # Examples
///
/// ```
/// use osp_cli::ui::{MessageLevel, adjust_verbosity};
///
/// assert_eq!(adjust_verbosity(MessageLevel::Success, 1, 0), MessageLevel::Info);
/// assert_eq!(adjust_verbosity(MessageLevel::Success, 0, 9), MessageLevel::Error);
/// ```
pub fn adjust_verbosity(base: MessageLevel, verbose: u8, quiet: u8) -> MessageLevel {
    let rank = base.as_rank() + verbose as i8 - quiet as i8;
    MessageLevel::from_rank(rank)
}

#[cfg(test)]
mod tests {
    use super::{
        GroupedRenderOptions, MessageBuffer, MessageLayout, MessageLevel, adjust_verbosity,
    };
    use crate::ui::chrome::SectionFrameStyle;

    #[test]
    fn grouped_render_variants_cover_visibility_headers_and_color_toggles_unit() {
        let mut messages = MessageBuffer::default();
        messages.error("bad");
        messages.warning("careful");
        messages.success("done");
        messages.info("hint");
        messages.trace("trace");

        let grouped = messages.render_grouped(MessageLevel::Success);
        assert!(grouped.contains("Errors"));
        assert!(grouped.contains("Warnings"));
        assert!(grouped.contains("Success"));
        assert!(!grouped.contains("Info"));
        assert!(!grouped.contains("Trace"));

        let theme = crate::ui::theme::resolve_theme("rose-pine-moon");
        let boxed = messages.render_grouped_styled(
            MessageLevel::Error,
            false,
            true,
            Some(24),
            &theme,
            MessageLayout::Grouped,
        );
        assert!(boxed.contains("─ Errors "));
        assert!(
            boxed
                .lines()
                .any(|line| line.trim().chars().all(|ch| ch == '─'))
        );

        let plain = messages.render_grouped_styled(
            MessageLevel::Warning,
            false,
            false,
            Some(28),
            &theme,
            MessageLayout::Grouped,
        );
        let colored = messages.render_grouped_styled(
            MessageLevel::Warning,
            true,
            false,
            Some(28),
            &theme,
            MessageLayout::Grouped,
        );
        assert!(!plain.contains("\x1b["));
        assert!(colored.contains("\x1b["));
    }

    #[test]
    fn minimal_render_flattens_with_prefixes_and_stable_plain_output_unit() {
        let mut messages = MessageBuffer::default();
        messages.error("bad");
        messages.warning("careful");
        messages.info("hint");
        let theme = crate::ui::theme::resolve_theme(crate::ui::theme::DEFAULT_THEME_NAME);

        let rendered = messages.render_grouped_styled(
            MessageLevel::Info,
            false,
            false,
            Some(28),
            &theme,
            MessageLayout::Minimal,
        );

        assert!(rendered.contains("error: bad"));
        assert!(rendered.contains("warning: careful"));
        assert!(rendered.contains("info: hint"));
        assert!(!rendered.contains("Errors"));
        assert!(!rendered.contains("- bad"));

        let narrow = messages.render_grouped_styled(
            MessageLevel::Info,
            false,
            false,
            Some(18),
            &theme,
            MessageLayout::Minimal,
        );
        assert_eq!(narrow, "error: bad\nwarning: careful\ninfo: hint\n");
    }

    #[test]
    fn grouped_render_matches_ascii_rule_snapshot_unit() {
        let mut messages = MessageBuffer::default();
        messages.error("bad");
        messages.warning("careful");
        let theme = crate::ui::theme::resolve_theme(crate::ui::theme::DEFAULT_THEME_NAME);

        let rendered = messages.render_grouped_with_options(GroupedRenderOptions {
            max_level: MessageLevel::Warning,
            color: false,
            unicode: false,
            width: Some(18),
            theme: &theme,
            layout: MessageLayout::Grouped,
            chrome_frame: SectionFrameStyle::TopBottom,
            style_overrides: crate::ui::style::StyleOverrides::default(),
        });

        assert_eq!(
            rendered,
            "- Errors ---------\n- bad\n------------------\n\n- Warnings -------\n- careful\n------------------\n"
        );
    }

    #[test]
    fn message_helper_paths_cover_verbosity_levels_layout_and_buffer_basics_unit() {
        assert_eq!(
            adjust_verbosity(MessageLevel::Success, 1, 0),
            MessageLevel::Info
        );
        assert_eq!(
            adjust_verbosity(MessageLevel::Success, 2, 0),
            MessageLevel::Trace
        );
        assert_eq!(
            adjust_verbosity(MessageLevel::Success, 0, 9),
            MessageLevel::Error
        );

        assert_eq!(MessageLevel::Error.title(), "Errors");
        assert_eq!(MessageLevel::Success.as_env_str(), "success");
        assert_eq!(MessageLevel::from_rank(-1), MessageLevel::Error);
        assert_eq!(MessageLevel::from_rank(1), MessageLevel::Warning);
        assert_eq!(MessageLevel::from_rank(9), MessageLevel::Trace);

        assert_eq!(
            MessageLayout::parse("grouped"),
            Some(MessageLayout::Grouped)
        );
        assert_eq!(
            MessageLayout::parse("minimal"),
            Some(MessageLayout::Minimal)
        );
        assert_eq!(MessageLayout::parse("dense"), None);

        let mut messages = MessageBuffer::default();
        assert!(messages.is_empty());
        messages.error("bad");
        messages.success("ok");
        messages.trace("trace");
        assert!(!messages.is_empty());
        assert!(
            messages
                .render_grouped(MessageLevel::Success)
                .contains("Success")
        );
    }
}
