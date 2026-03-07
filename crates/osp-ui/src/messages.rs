use crate::document::{Block, Document, LineBlock, LinePart};
use crate::inline::render_inline;
use crate::renderer::render_document;
use crate::style::{StyleOverrides, StyleToken, apply_style_with_theme_overrides};
use crate::theme::ThemeDefinition;
use crate::{RenderBackend, ResolvedRenderSettings};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MessageLevel {
    Error,
    Warning,
    Success,
    Info,
    Trace,
}

impl MessageLevel {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SectionFrameStyle {
    None,
    #[default]
    Top,
    Bottom,
    TopBottom,
    Square,
    Round,
}

impl SectionFrameStyle {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "none" | "plain" => Some(Self::None),
            "top" | "rule-top" => Some(Self::Top),
            "bottom" | "rule-bottom" => Some(Self::Bottom),
            "top-bottom" | "both" | "rules" => Some(Self::TopBottom),
            "square" | "box" | "boxed" => Some(Self::Square),
            "round" | "rounded" => Some(Self::Round),
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

#[derive(Debug, Clone, Copy)]
pub struct SectionStyleTokens {
    pub border: StyleToken,
    pub title: StyleToken,
}

#[derive(Clone, Copy)]
pub struct SectionRenderContext<'a> {
    pub color: bool,
    pub theme: &'a ThemeDefinition,
    pub style_overrides: &'a StyleOverrides,
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

    pub fn render_grouped(&self, max_level: MessageLevel) -> String {
        let theme = crate::theme::resolve_theme("plain");
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
            table_overflow: crate::TableOverflow::Clip,
            table_border: crate::TableBorderStyle::Square,
            theme_name: options.theme.id.clone(),
            theme: options.theme.clone(),
            style_overrides: options.style_overrides,
            chrome_frame: options.chrome_frame,
        };
        render_document(&document, resolved)
    }

    fn render_grouped_sections(&self, options: &GroupedRenderOptions<'_>) -> String {
        let mut sections = Vec::new();

        for level in [
            MessageLevel::Error,
            MessageLevel::Warning,
            MessageLevel::Success,
            MessageLevel::Info,
            MessageLevel::Trace,
        ] {
            if level > options.max_level {
                continue;
            }

            let grouped = self
                .entries
                .iter()
                .filter(|entry| entry.level == level)
                .collect::<Vec<&UiMessage>>();
            if grouped.is_empty() {
                continue;
            }

            let body = grouped
                .iter()
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
                SectionStyleTokens {
                    border: level.style_token(),
                    title: level.style_token(),
                },
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

        for level in [
            MessageLevel::Error,
            MessageLevel::Warning,
            MessageLevel::Success,
            MessageLevel::Info,
            MessageLevel::Trace,
        ] {
            if level > max_level {
                continue;
            }

            for entry in self.entries.iter().filter(|entry| entry.level == level) {
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

impl MessageLevel {
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

fn default_message_chrome_frame(layout: MessageLayout) -> SectionFrameStyle {
    match layout {
        MessageLayout::Minimal => SectionFrameStyle::None,
        MessageLayout::Grouped => SectionFrameStyle::TopBottom,
    }
}

pub fn render_section_divider(
    title: &str,
    unicode: bool,
    width: Option<usize>,
    color: bool,
    theme: &ThemeDefinition,
    token: StyleToken,
) -> String {
    render_section_divider_with_overrides(
        title,
        unicode,
        width,
        color,
        theme,
        token,
        &StyleOverrides::default(),
    )
}

pub fn render_section_divider_with_overrides(
    title: &str,
    unicode: bool,
    width: Option<usize>,
    color: bool,
    theme: &ThemeDefinition,
    token: StyleToken,
    style_overrides: &StyleOverrides,
) -> String {
    let fill_char = if unicode { '─' } else { '-' };
    let target_width = width.unwrap_or(12).max(12);
    let title = title.trim();

    let raw = if title.is_empty() {
        fill_char.to_string().repeat(target_width)
    } else {
        let prefix = if unicode {
            format!("─ {title} ")
        } else {
            format!("- {title} ")
        };
        let prefix_width = prefix.chars().count();
        if prefix_width >= target_width {
            prefix
        } else {
            format!(
                "{prefix}{}",
                fill_char.to_string().repeat(target_width - prefix_width)
            )
        }
    };

    if !color {
        return raw;
    }

    if title.is_empty() || token != StyleToken::PanelBorder {
        return apply_style_with_theme_overrides(&raw, token, true, theme, style_overrides);
    }

    let prefix = if unicode { "─ " } else { "- " };
    let title_text = title;
    let prefix_width = prefix.chars().count();
    let title_width = title_text.chars().count();
    let base_width = prefix_width + title_width + 1;
    let fill_len = target_width.saturating_sub(base_width);
    let suffix = if fill_len == 0 {
        " ".to_string()
    } else {
        format!(" {}", fill_char.to_string().repeat(fill_len))
    };

    let styled_prefix = apply_style_with_theme_overrides(
        prefix,
        StyleToken::PanelBorder,
        true,
        theme,
        style_overrides,
    );
    let styled_title = apply_style_with_theme_overrides(
        title_text,
        StyleToken::PanelTitle,
        true,
        theme,
        style_overrides,
    );
    let styled_suffix = apply_style_with_theme_overrides(
        &suffix,
        StyleToken::PanelBorder,
        true,
        theme,
        style_overrides,
    );
    format!("{styled_prefix}{styled_title}{styled_suffix}")
}

pub fn render_section_block_with_overrides(
    title: &str,
    body: &str,
    frame_style: SectionFrameStyle,
    unicode: bool,
    width: Option<usize>,
    render: SectionRenderContext<'_>,
    tokens: SectionStyleTokens,
) -> String {
    match frame_style {
        SectionFrameStyle::None => render_plain_section(title, body, render, tokens.title),
        SectionFrameStyle::Top => {
            render_ruled_section(title, body, true, false, unicode, width, render, tokens)
        }
        SectionFrameStyle::Bottom => {
            render_ruled_section(title, body, false, true, unicode, width, render, tokens)
        }
        SectionFrameStyle::TopBottom => {
            render_ruled_section(title, body, true, true, unicode, width, render, tokens)
        }
        SectionFrameStyle::Square => render_boxed_section(
            title,
            body,
            unicode,
            render,
            tokens,
            BoxFrameChars::square(unicode),
        ),
        SectionFrameStyle::Round => render_boxed_section(
            title,
            body,
            unicode,
            render,
            tokens,
            BoxFrameChars::round(unicode),
        ),
    }
}

fn render_plain_section(
    title: &str,
    body: &str,
    render: SectionRenderContext<'_>,
    title_token: StyleToken,
) -> String {
    let mut out = String::new();
    let title = title.trim();
    let body = body.trim_end_matches('\n');

    if !title.is_empty() {
        out.push_str(&render_section_title_with_overrides(
            title,
            render.color,
            render.theme,
            title_token,
            render.style_overrides,
        ));
        if !body.is_empty() {
            out.push('\n');
        }
    }
    if !body.is_empty() {
        out.push_str(body);
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn render_ruled_section(
    title: &str,
    body: &str,
    top_rule: bool,
    bottom_rule: bool,
    unicode: bool,
    width: Option<usize>,
    render: SectionRenderContext<'_>,
    tokens: SectionStyleTokens,
) -> String {
    let mut out = String::new();
    let body = body.trim_end_matches('\n');
    let title = title.trim();

    if top_rule {
        out.push_str(&render_section_divider_with_overrides(
            title,
            unicode,
            width,
            render.color,
            render.theme,
            tokens.border,
            render.style_overrides,
        ));
    } else if !title.is_empty() {
        out.push_str(&render_section_title_with_overrides(
            title,
            render.color,
            render.theme,
            tokens.title,
            render.style_overrides,
        ));
    }

    if !body.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(body);
    }

    if bottom_rule {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&render_section_divider_with_overrides(
            "",
            unicode,
            width,
            render.color,
            render.theme,
            tokens.border,
            render.style_overrides,
        ));
    }

    out
}

#[derive(Debug, Clone, Copy)]
struct BoxFrameChars {
    top_left: char,
    top_right: char,
    bottom_left: char,
    bottom_right: char,
    horizontal: char,
    vertical: char,
}

impl BoxFrameChars {
    fn square(unicode: bool) -> Self {
        if unicode {
            Self {
                top_left: '┌',
                top_right: '┐',
                bottom_left: '└',
                bottom_right: '┘',
                horizontal: '─',
                vertical: '│',
            }
        } else {
            Self {
                top_left: '+',
                top_right: '+',
                bottom_left: '+',
                bottom_right: '+',
                horizontal: '-',
                vertical: '|',
            }
        }
    }

    fn round(unicode: bool) -> Self {
        if unicode {
            Self {
                top_left: '╭',
                top_right: '╮',
                bottom_left: '╰',
                bottom_right: '╯',
                horizontal: '─',
                vertical: '│',
            }
        } else {
            Self::square(false)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_boxed_section(
    title: &str,
    body: &str,
    _unicode: bool,
    render: SectionRenderContext<'_>,
    tokens: SectionStyleTokens,
    chars: BoxFrameChars,
) -> String {
    let lines = section_body_lines(body);
    let title = title.trim();
    let body_width = lines
        .iter()
        .map(|line| visible_width(line))
        .max()
        .unwrap_or(0);
    let title_width = if title.is_empty() {
        0
    } else {
        title.chars().count() + 2
    };
    let inner_width = body_width.max(title_width).max(8);

    let mut out = String::new();
    out.push_str(&render_box_top(title, inner_width, chars, render, tokens));

    if !lines.is_empty() {
        out.push('\n');
    }

    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(&render_box_body_line(
            line,
            inner_width,
            chars,
            render,
            tokens.border,
        ));
    }

    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(&style_border_segment(
        &format!(
            "{}{}{}",
            chars.bottom_left,
            chars.horizontal.to_string().repeat(inner_width + 2),
            chars.bottom_right
        ),
        render.color,
        render.theme,
        tokens.border,
        render.style_overrides,
    ));
    out
}

fn render_box_top(
    title: &str,
    inner_width: usize,
    chars: BoxFrameChars,
    render: SectionRenderContext<'_>,
    tokens: SectionStyleTokens,
) -> String {
    if title.is_empty() {
        return style_border_segment(
            &format!(
                "{}{}{}",
                chars.top_left,
                chars.horizontal.to_string().repeat(inner_width + 2),
                chars.top_right
            ),
            render.color,
            render.theme,
            tokens.border,
            render.style_overrides,
        );
    }

    let title_width = title.chars().count();
    let remaining = inner_width.saturating_sub(title_width);
    let left = format!("{} ", chars.top_left);
    let right = format!(
        " {}{}",
        chars.horizontal.to_string().repeat(remaining),
        chars.top_right
    );

    format!(
        "{}{}{}",
        style_border_segment(
            &left,
            render.color,
            render.theme,
            tokens.border,
            render.style_overrides,
        ),
        style_title_segment(
            title,
            render.color,
            render.theme,
            tokens.title,
            render.style_overrides,
        ),
        style_border_segment(
            &right,
            render.color,
            render.theme,
            tokens.border,
            render.style_overrides,
        ),
    )
}

fn render_box_body_line(
    line: &str,
    inner_width: usize,
    chars: BoxFrameChars,
    render: SectionRenderContext<'_>,
    border_token: StyleToken,
) -> String {
    let padding = inner_width.saturating_sub(visible_width(line));
    let left = format!("{} ", chars.vertical);
    let right = format!("{} {}", " ".repeat(padding), chars.vertical);
    format!(
        "{}{}{}",
        style_border_segment(
            &left,
            render.color,
            render.theme,
            border_token,
            render.style_overrides,
        ),
        line,
        style_border_segment(
            &right,
            render.color,
            render.theme,
            border_token,
            render.style_overrides,
        ),
    )
}

fn render_section_title_with_overrides(
    title: &str,
    color: bool,
    theme: &ThemeDefinition,
    title_token: StyleToken,
    style_overrides: &StyleOverrides,
) -> String {
    let raw = format!("{title}:");
    style_title_segment(&raw, color, theme, title_token, style_overrides)
}

fn style_border_segment(
    text: &str,
    color: bool,
    theme: &ThemeDefinition,
    token: StyleToken,
    style_overrides: &StyleOverrides,
) -> String {
    if color {
        apply_style_with_theme_overrides(text, token, true, theme, style_overrides)
    } else {
        text.to_string()
    }
}

fn style_title_segment(
    text: &str,
    color: bool,
    theme: &ThemeDefinition,
    token: StyleToken,
    style_overrides: &StyleOverrides,
) -> String {
    if color {
        apply_style_with_theme_overrides(text, token, true, theme, style_overrides)
    } else {
        text.to_string()
    }
}

fn section_body_lines(body: &str) -> Vec<&str> {
    body.trim_end_matches('\n')
        .lines()
        .map(str::trim_end)
        .collect()
}

fn visible_width(text: &str) -> usize {
    let mut width = 0usize;
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\x1b' && matches!(chars.peek(), Some('[')) {
            chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
            continue;
        }
        width += 1;
    }

    width
}

pub fn adjust_verbosity(base: MessageLevel, verbose: u8, quiet: u8) -> MessageLevel {
    let rank = base.as_rank() + verbose as i8 - quiet as i8;
    MessageLevel::from_rank(rank)
}

#[cfg(test)]
mod tests {
    use super::{
        MessageBuffer, MessageLayout, MessageLevel, SectionFrameStyle, SectionRenderContext,
        SectionStyleTokens, adjust_verbosity, render_section_block_with_overrides,
        render_section_divider,
    };
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

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
        let theme = crate::theme::resolve_theme("rose-pine-moon");
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
        let theme = crate::theme::resolve_theme("rose-pine-moon");

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
        let theme = crate::theme::resolve_theme(crate::theme::DEFAULT_THEME_NAME);

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
        let theme = crate::theme::resolve_theme(crate::theme::DEFAULT_THEME_NAME);

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
        let theme = crate::theme::resolve_theme(crate::theme::DEFAULT_THEME_NAME);

        let rendered = messages.render_grouped_with_options(super::GroupedRenderOptions {
            max_level: MessageLevel::Warning,
            color: false,
            unicode: false,
            width: Some(18),
            theme: &theme,
            layout: MessageLayout::Grouped,
            chrome_frame: SectionFrameStyle::TopBottom,
            style_overrides: crate::style::StyleOverrides::default(),
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
    fn section_divider_ignores_columns_env_without_explicit_width() {
        let _guard = env_lock().lock().expect("lock should not be poisoned");
        let original = std::env::var("COLUMNS").ok();
        unsafe {
            std::env::set_var("COLUMNS", "99");
        }

        let divider = render_section_divider(
            "",
            false,
            None,
            false,
            &crate::theme::resolve_theme(crate::theme::DEFAULT_THEME_NAME),
            crate::style::StyleToken::PanelBorder,
        );

        match original {
            Some(value) => unsafe { std::env::set_var("COLUMNS", value) },
            None => unsafe { std::env::remove_var("COLUMNS") },
        }

        assert_eq!(divider.len(), 12);
    }

    #[test]
    fn section_frame_style_parses_expected_names_unit() {
        assert_eq!(
            SectionFrameStyle::parse("top"),
            Some(SectionFrameStyle::Top)
        );
        assert_eq!(
            SectionFrameStyle::parse("top-bottom"),
            Some(SectionFrameStyle::TopBottom)
        );
        assert_eq!(
            SectionFrameStyle::parse("round"),
            Some(SectionFrameStyle::Round)
        );
        assert_eq!(
            SectionFrameStyle::parse("square"),
            Some(SectionFrameStyle::Square)
        );
        assert_eq!(
            SectionFrameStyle::parse("none"),
            Some(SectionFrameStyle::None)
        );
    }

    #[test]
    fn top_bottom_section_frame_wraps_body_with_rules_unit() {
        let theme = crate::theme::resolve_theme(crate::theme::DEFAULT_THEME_NAME);
        let render = SectionRenderContext {
            color: false,
            theme: &theme,
            style_overrides: &crate::style::StyleOverrides::default(),
        };
        let tokens = SectionStyleTokens {
            border: crate::style::StyleToken::PanelBorder,
            title: crate::style::StyleToken::PanelTitle,
        };
        let rendered = render_section_block_with_overrides(
            "Commands",
            "  show\n  delete",
            SectionFrameStyle::TopBottom,
            true,
            Some(18),
            render,
            tokens,
        );

        assert!(rendered.contains("Commands"));
        assert!(rendered.contains("show"));
        assert!(
            rendered
                .lines()
                .last()
                .is_some_and(|line| line.contains('─'))
        );
    }

    #[test]
    fn square_section_frame_boxes_body_unit() {
        let theme = crate::theme::resolve_theme(crate::theme::DEFAULT_THEME_NAME);
        let render = SectionRenderContext {
            color: false,
            theme: &theme,
            style_overrides: &crate::style::StyleOverrides::default(),
        };
        let tokens = SectionStyleTokens {
            border: crate::style::StyleToken::PanelBorder,
            title: crate::style::StyleToken::PanelTitle,
        };
        let rendered = render_section_block_with_overrides(
            "Usage",
            "osp config show",
            SectionFrameStyle::Square,
            true,
            None,
            render,
            tokens,
        );

        assert!(rendered.contains("┌"));
        assert!(rendered.contains("│ osp config show"));
        assert!(rendered.contains("┘"));
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

    #[test]
    fn section_frame_styles_cover_none_bottom_and_round_unit() {
        let theme = crate::theme::resolve_theme(crate::theme::DEFAULT_THEME_NAME);
        let render = SectionRenderContext {
            color: false,
            theme: &theme,
            style_overrides: &crate::style::StyleOverrides::default(),
        };
        let tokens = SectionStyleTokens {
            border: crate::style::StyleToken::PanelBorder,
            title: crate::style::StyleToken::PanelTitle,
        };
        let plain = render_section_block_with_overrides(
            "Note",
            "body",
            SectionFrameStyle::None,
            false,
            Some(16),
            render,
            tokens,
        );
        let bottom = render_section_block_with_overrides(
            "Note",
            "body",
            SectionFrameStyle::Bottom,
            false,
            Some(16),
            render,
            tokens,
        );
        let round = render_section_block_with_overrides(
            "Note",
            "body",
            SectionFrameStyle::Round,
            true,
            Some(16),
            render,
            tokens,
        );

        assert!(plain.contains("Note:"));
        assert!(bottom.lines().last().is_some_and(|line| line.contains('-')));
        assert!(round.contains("╭"));
        assert!(round.contains("╰"));
    }
}
