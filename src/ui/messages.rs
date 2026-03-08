use crate::ui::document::{Block, Document, LineBlock, LinePart};
use crate::ui::inline::render_inline;
use crate::ui::renderer::render_document;
use crate::ui::style::{StyleOverrides, StyleToken};
use crate::ui::theme::ThemeDefinition;
use crate::ui::{RenderBackend, ResolvedRenderSettings};

pub use crate::ui::chrome::{
    SectionFrameStyle, SectionRenderContext, SectionStyleTokens,
    render_section_block_with_overrides,
    render_section_block_with_overrides as render_section_block, render_section_divider,
    render_section_divider_with_overrides,
};

const ORDERED_MESSAGE_LEVELS: [MessageLevel; 5] = [
    MessageLevel::Error,
    MessageLevel::Warning,
    MessageLevel::Success,
    MessageLevel::Info,
    MessageLevel::Trace,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MessageLevel {
    Error,
    Warning,
    Success,
    Info,
    Trace,
}

impl MessageLevel {
    fn ordered() -> impl Iterator<Item = Self> {
        ORDERED_MESSAGE_LEVELS.into_iter()
    }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageLayout {
    Minimal,
    Grouped,
}

impl MessageLayout {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "minimal" => Some(Self::Minimal),
            "grouped" => Some(Self::Grouped),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct UiMessage {
    pub level: MessageLevel,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct MessageBuffer {
    entries: Vec<UiMessage>,
}

#[derive(Debug, Clone)]
pub struct GroupedRenderOptions<'a> {
    pub max_level: MessageLevel,
    pub color: bool,
    pub unicode: bool,
    pub width: Option<usize>,
    pub theme: &'a ThemeDefinition,
    pub layout: MessageLayout,
    pub chrome_frame: SectionFrameStyle,
    pub style_overrides: StyleOverrides,
}

impl MessageBuffer {
    pub fn push<T: Into<String>>(&mut self, level: MessageLevel, text: T) {
        self.entries.push(UiMessage {
            level,
            text: text.into(),
        });
    }

    pub fn error<T: Into<String>>(&mut self, text: T) {
        self.push(MessageLevel::Error, text);
    }

    pub fn warning<T: Into<String>>(&mut self, text: T) {
        self.push(MessageLevel::Warning, text);
    }

    pub fn success<T: Into<String>>(&mut self, text: T) {
        self.push(MessageLevel::Success, text);
    }

    pub fn info<T: Into<String>>(&mut self, text: T) {
        self.push(MessageLevel::Info, text);
    }

    pub fn trace<T: Into<String>>(&mut self, text: T) {
        self.push(MessageLevel::Trace, text);
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn entries_for_level(&self, level: MessageLevel) -> impl Iterator<Item = &UiMessage> {
        self.entries
            .iter()
            .filter(move |entry| entry.level == level)
    }

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
    fn default_success_hides_info_and_debug() {
        let mut messages = MessageBuffer::default();
        messages.error("bad");
        messages.warning("careful");
        messages.success("done");
        messages.info("hint");
        messages.trace("trace");

        let rendered = messages.render_grouped(MessageLevel::Success);
        assert!(rendered.contains("Errors"));
        assert!(rendered.contains("Warnings"));
        assert!(rendered.contains("Success"));
        assert!(!rendered.contains("Info"));
        assert!(!rendered.contains("Trace"));
    }

    #[test]
    fn styled_render_uses_boxed_headers() {
        let mut messages = MessageBuffer::default();
        messages.error("bad");
        let theme = crate::ui::theme::resolve_theme("rose-pine-moon");
        let rendered = messages.render_grouped_styled(
            MessageLevel::Error,
            false,
            true,
            Some(24),
            &theme,
            MessageLayout::Grouped,
        );
        assert!(rendered.contains("─ Errors "));
        assert!(
            rendered
                .lines()
                .any(|line| line.trim().chars().all(|ch| ch == '─'))
        );
    }

    #[test]
    fn styled_render_color_toggle_controls_ansi() {
        let mut messages = MessageBuffer::default();
        messages.warning("careful");
        let theme = crate::ui::theme::resolve_theme("rose-pine-moon");

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
    fn minimal_render_flattens_messages_with_level_prefixes_unit() {
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
    }

    #[test]
    fn minimal_render_matches_plain_snapshot_unit() {
        let mut messages = MessageBuffer::default();
        messages.error("bad");
        messages.warning("careful");
        messages.info("hint");
        let theme = crate::ui::theme::resolve_theme(crate::ui::theme::DEFAULT_THEME_NAME);

        let rendered = messages.render_grouped_styled(
            MessageLevel::Info,
            false,
            false,
            Some(18),
            &theme,
            MessageLayout::Minimal,
        );

        assert_eq!(rendered, "error: bad\nwarning: careful\ninfo: hint\n");
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
    fn verbosity_adjustment_clamps() {
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
    }

    #[test]
    fn message_level_helpers_cover_titles_env_and_rank_unit() {
        assert_eq!(MessageLevel::Error.title(), "Errors");
        assert_eq!(MessageLevel::Success.as_env_str(), "success");
        assert_eq!(MessageLevel::from_rank(-1), MessageLevel::Error);
        assert_eq!(MessageLevel::from_rank(1), MessageLevel::Warning);
        assert_eq!(MessageLevel::from_rank(9), MessageLevel::Trace);
    }

    #[test]
    fn message_layout_parser_and_buffer_helpers_cover_basic_paths_unit() {
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
