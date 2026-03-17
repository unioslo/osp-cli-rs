//! Message rendering intent for the canonical UI pipeline.
//!
//! The current surface stays plain and layout-focused, but it already sits on
//! top of the same chrome boundary the terminal emitter uses.

use unicode_width::UnicodeWidthStr;

use crate::config::ResolvedConfig;
use crate::ui::chrome::{FULL_HELP_LAYOUT_CHROME, PLAIN_SECTION_CHROME};
use crate::ui::style::{StyleToken, ThemeStyler};

use super::{MessageBuffer, MessageLayout, MessageLevel, message_layout_from_config};

#[derive(Debug, Clone, PartialEq, Eq)]
struct RenderedSection {
    level: MessageLevel,
    title: String,
    lines: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum MessageFrameStyle {
    None,
    #[default]
    Top,
    Bottom,
    TopBottom,
    Square,
    Round,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum MessageRulePolicy {
    PerSection,
    #[default]
    Shared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct MessageChrome {
    pub frame_style: MessageFrameStyle,
    pub ruled_policy: MessageRulePolicy,
    pub unicode: bool,
    pub width: Option<usize>,
}

/// Options controlling message rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MessageRenderOptions {
    pub max_level: MessageLevel,
    pub layout: MessageLayout,
}

impl MessageRenderOptions {
    /// Creates full render options.
    pub fn full(max_level: MessageLevel) -> Self {
        Self {
            max_level,
            layout: MessageLayout::Grouped,
        }
    }

    /// Creates compact render options.
    #[cfg(test)]
    pub fn compact(max_level: MessageLevel) -> Self {
        Self {
            max_level,
            layout: MessageLayout::Compact,
        }
    }

    /// Creates austere render options.
    #[cfg(test)]
    pub fn austere(max_level: MessageLevel) -> Self {
        Self {
            max_level,
            layout: MessageLayout::Minimal,
        }
    }

    /// Creates plain ungrouped render options.
    #[cfg(test)]
    pub fn plain(max_level: MessageLevel) -> Self {
        Self {
            max_level,
            layout: MessageLayout::Plain,
        }
    }
}

/// Renders messages using the requested layout.
#[cfg(test)]
pub fn render_messages(buffer: &MessageBuffer, options: MessageRenderOptions) -> String {
    render_messages_internal(buffer, options, None, MessageChrome::default())
}

/// Renders messages using the requested layout and semantic styling.
#[cfg(test)]
pub fn render_messages_with_styler(
    buffer: &MessageBuffer,
    options: MessageRenderOptions,
    styler: &ThemeStyler<'_>,
) -> String {
    render_messages_internal(buffer, options, Some(styler), MessageChrome::default())
}

pub(crate) fn render_messages_with_styler_and_chrome(
    buffer: &MessageBuffer,
    options: MessageRenderOptions,
    styler: &ThemeStyler<'_>,
    chrome: MessageChrome,
) -> String {
    render_messages_internal(buffer, options, Some(styler), chrome)
}

/// Renders messages using config-driven layout selection and semantic styling.
pub(crate) fn render_messages_with_styler_from_config(
    buffer: &MessageBuffer,
    config: &ResolvedConfig,
    max_level: MessageLevel,
    styler: &ThemeStyler<'_>,
    chrome: MessageChrome,
) -> String {
    render_messages_internal(
        buffer,
        MessageRenderOptions {
            max_level,
            layout: message_layout_from_config(config),
        },
        Some(styler),
        chrome,
    )
}

fn render_messages_internal(
    buffer: &MessageBuffer,
    options: MessageRenderOptions,
    styler: Option<&ThemeStyler<'_>>,
    chrome: MessageChrome,
) -> String {
    let rendered = match options.layout {
        MessageLayout::Minimal => render_austere(buffer, options.max_level, styler),
        MessageLayout::Plain => render_plain(buffer, options.max_level, styler),
        MessageLayout::Compact => render_compact(buffer, options.max_level, styler),
        MessageLayout::Grouped => render_full(buffer, options.max_level, styler, chrome),
    };

    if rendered.is_empty() || rendered.ends_with('\n') {
        rendered
    } else {
        format!("{rendered}\n")
    }
}

fn render_austere(
    buffer: &MessageBuffer,
    max_level: MessageLevel,
    styler: Option<&ThemeStyler<'_>>,
) -> String {
    let mut lines = Vec::new();
    for level in MessageLevel::ordered().filter(|level| *level <= max_level) {
        for entry in buffer.entries_for_level(level) {
            let prefix = paint(styler, level.as_env_str(), level.style_token());
            let colon = paint(styler, ":", StyleToken::Punctuation);
            lines.push(format!("  {prefix}{colon} {}", entry.text));
        }
    }
    lines.join("\n")
}

fn render_full(
    buffer: &MessageBuffer,
    max_level: MessageLevel,
    styler: Option<&ThemeStyler<'_>>,
    chrome: MessageChrome,
) -> String {
    let sections = sectioned_messages(buffer, max_level);
    if sections.is_empty() {
        return String::new();
    }

    match (chrome.frame_style, chrome.ruled_policy) {
        (MessageFrameStyle::Top | MessageFrameStyle::TopBottom, MessageRulePolicy::Shared) => {
            render_shared_full_sections(&sections, styler, chrome)
        }
        _ => sections
            .iter()
            .map(|section| render_full_section(section, styler, chrome))
            .collect::<Vec<_>>()
            .join("\n\n"),
    }
}

fn render_compact(
    buffer: &MessageBuffer,
    max_level: MessageLevel,
    styler: Option<&ThemeStyler<'_>>,
) -> String {
    sectioned_messages(buffer, max_level)
        .iter()
        .map(|section| render_compact_section(section, styler))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn render_plain(
    buffer: &MessageBuffer,
    max_level: MessageLevel,
    styler: Option<&ThemeStyler<'_>>,
) -> String {
    MessageLevel::ordered()
        .filter(|level| *level <= max_level)
        .flat_map(|level| {
            buffer
                .entries_for_level(level)
                .map(|entry| paint(styler, &format!("  {}", entry.text), level.style_token()))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn sectioned_messages(buffer: &MessageBuffer, max_level: MessageLevel) -> Vec<RenderedSection> {
    let mut sections = Vec::new();
    for level in MessageLevel::ordered().filter(|level| *level <= max_level) {
        let lines = buffer
            .entries_for_level(level)
            .map(|entry| entry.text.clone())
            .collect::<Vec<_>>();
        if lines.is_empty() {
            continue;
        }
        sections.push(RenderedSection {
            level,
            title: level.title().to_string(),
            lines,
        });
    }
    sections
}

fn render_shared_full_sections(
    sections: &[RenderedSection],
    styler: Option<&ThemeStyler<'_>>,
    chrome: MessageChrome,
) -> String {
    let mut lines = Vec::new();
    for section in sections {
        let title =
            FULL_HELP_LAYOUT_CHROME.render_title(&section.title, chrome.width, chrome.unicode);
        lines.push(paint(styler, &title, section.level.style_token()));
        lines.extend(
            section
                .lines
                .iter()
                .map(|line| paint(styler, &format!("  {line}"), StyleToken::Text)),
        );
    }
    if matches!(chrome.frame_style, MessageFrameStyle::TopBottom)
        && let Some(rule) = ruled_line(chrome.width, chrome.unicode)
    {
        lines.push(paint(styler, &rule, StyleToken::Border));
    }
    lines.join("\n")
}

fn render_full_section(
    section: &RenderedSection,
    styler: Option<&ThemeStyler<'_>>,
    chrome: MessageChrome,
) -> String {
    match chrome.frame_style {
        MessageFrameStyle::None => render_compact_section(section, styler),
        MessageFrameStyle::Top => render_framed_section(section, styler, chrome, true, false),
        MessageFrameStyle::Bottom => render_framed_section(section, styler, chrome, false, true),
        MessageFrameStyle::TopBottom => render_framed_section(section, styler, chrome, true, true),
        MessageFrameStyle::Square => {
            render_boxed_section(section, styler, chrome, BoxChars::square(chrome.unicode))
        }
        MessageFrameStyle::Round => {
            render_boxed_section(section, styler, chrome, BoxChars::round(chrome.unicode))
        }
    }
}

fn render_framed_section(
    section: &RenderedSection,
    styler: Option<&ThemeStyler<'_>>,
    chrome: MessageChrome,
    top_rule: bool,
    bottom_rule: bool,
) -> String {
    let mut lines = Vec::new();
    if top_rule {
        let title =
            FULL_HELP_LAYOUT_CHROME.render_title(&section.title, chrome.width, chrome.unicode);
        lines.push(paint(styler, &title, section.level.style_token()));
    } else {
        let title = PLAIN_SECTION_CHROME.render_title(&section.title, None, false);
        lines.push(paint(styler, &title, section.level.style_token()));
    }
    lines.extend(
        section
            .lines
            .iter()
            .map(|line| paint(styler, &format!("  {line}"), StyleToken::Text)),
    );
    if bottom_rule && let Some(rule) = ruled_line(chrome.width, chrome.unicode) {
        lines.push(paint(styler, &rule, StyleToken::Border));
    }
    lines.join("\n")
}

fn render_compact_section(section: &RenderedSection, styler: Option<&ThemeStyler<'_>>) -> String {
    let title = PLAIN_SECTION_CHROME.render_title(&section.title, None, false);
    let mut lines = vec![paint(styler, &title, section.level.style_token())];
    lines.extend(
        section
            .lines
            .iter()
            .map(|line| paint(styler, &format!("  {line}"), StyleToken::Text)),
    );
    lines.join("\n")
}

#[derive(Debug, Clone, Copy)]
struct BoxChars {
    top_left: char,
    top_right: char,
    bottom_left: char,
    bottom_right: char,
    horizontal: char,
    vertical: char,
}

impl BoxChars {
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

fn render_boxed_section(
    section: &RenderedSection,
    styler: Option<&ThemeStyler<'_>>,
    chrome: MessageChrome,
    chars: BoxChars,
) -> String {
    let body_lines = section
        .lines
        .iter()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>();
    let content_width = std::iter::once(section.title.as_str())
        .chain(body_lines.iter().map(String::as_str))
        .map(UnicodeWidthStr::width)
        .max()
        .unwrap_or(0);
    let inner_width = chrome
        .width
        .unwrap_or(content_width + 2)
        .max(content_width + 2);

    let title_width = UnicodeWidthStr::width(section.title.as_str());
    let right_fill = inner_width.saturating_sub(title_width + 2);
    let top = format!(
        "{} {} {}{}",
        chars.top_left,
        section.title,
        chars.horizontal,
        chars
            .horizontal
            .to_string()
            .repeat(right_fill.saturating_sub(1))
    )
    .trim_end_matches(chars.horizontal)
    .to_string()
        + &chars.top_right.to_string();

    let mut lines = vec![paint(styler, &top, section.level.style_token())];
    for line in body_lines {
        let pad = inner_width.saturating_sub(UnicodeWidthStr::width(line.as_str()));
        let body = format!(
            "{}{}{:<pad$}{}",
            chars.vertical,
            line,
            "",
            chars.vertical,
            pad = pad
        );
        lines.push(paint(styler, &body, StyleToken::Text));
    }
    let bottom = format!(
        "{}{}{}",
        chars.bottom_left,
        chars.horizontal.to_string().repeat(inner_width),
        chars.bottom_right
    );
    lines.push(paint(styler, &bottom, StyleToken::Border));
    lines.join("\n")
}

fn ruled_line(width: Option<usize>, unicode: bool) -> Option<String> {
    let fill = if unicode { '─' } else { '-' };
    Some(fill.to_string().repeat(width.unwrap_or(24).max(12)))
}

fn paint(styler: Option<&ThemeStyler<'_>>, text: &str, token: StyleToken) -> String {
    styler
        .map(|styler| styler.paint(text, token))
        .unwrap_or_else(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        MessageChrome, MessageFrameStyle, MessageRenderOptions, MessageRulePolicy, render_messages,
        render_messages_internal, render_messages_with_styler,
    };
    use crate::ui::ThemeStyler;
    use crate::ui::messages::{MessageBuffer, MessageLevel};
    use crate::ui::theme::resolve_theme;

    #[test]
    fn full_render_orders_sections_and_filters_levels() {
        let mut buffer = MessageBuffer::default();
        buffer.error("bad");
        buffer.warning("careful");
        buffer.success("done");
        buffer.info("hint");

        let rendered = render_messages(&buffer, MessageRenderOptions::full(MessageLevel::Success));
        assert!(rendered.contains("Errors"));
        assert!(rendered.contains("\n  bad"));
        assert!(rendered.contains("Warnings"));
        assert!(rendered.contains("\n  careful"));
        assert!(rendered.contains("Success"));
        assert!(rendered.contains("\n  done"));
        assert!(!rendered.contains("Info:"));
    }

    #[test]
    fn austere_render_uses_inline_prefixes() {
        let mut buffer = MessageBuffer::default();
        buffer.error("bad");
        buffer.info("hint");

        let rendered = render_messages(&buffer, MessageRenderOptions::austere(MessageLevel::Info));
        assert!(rendered.contains("  error: bad"));
        assert!(rendered.contains("  info: hint"));
        assert!(!rendered.contains("Errors:"));
    }

    #[test]
    fn compact_render_keeps_titles_without_rule_chrome_unit() {
        let mut buffer = MessageBuffer::default();
        buffer.error("bad");
        buffer.warning("careful");

        let rendered = render_messages(
            &buffer,
            MessageRenderOptions::compact(MessageLevel::Warning),
        );

        assert!(rendered.contains("Errors:"));
        assert!(rendered.contains("\n  bad"));
        assert!(rendered.contains("Warnings:"));
        assert!(!rendered.contains("--------"));
    }

    #[test]
    fn plain_render_emits_bodies_without_titles_unit() {
        let mut buffer = MessageBuffer::default();
        buffer.error("bad");
        buffer.warning("careful");

        let rendered = render_messages(&buffer, MessageRenderOptions::plain(MessageLevel::Warning));

        assert!(!rendered.contains("Errors"));
        assert!(!rendered.contains("Warnings"));
        assert!(rendered.contains("  bad"));
        assert!(rendered.contains("  careful"));
    }

    #[test]
    fn full_render_honors_top_bottom_shared_chrome_unit() {
        let mut buffer = MessageBuffer::default();
        buffer.error("bad");
        buffer.warning("careful");

        let rendered = render_messages_internal(
            &buffer,
            MessageRenderOptions::full(MessageLevel::Warning),
            None,
            MessageChrome {
                frame_style: MessageFrameStyle::TopBottom,
                ruled_policy: MessageRulePolicy::Shared,
                unicode: false,
                width: Some(16),
            },
        );

        assert!(rendered.contains("- Errors "));
        assert!(rendered.contains("- Warnings "));
        assert!(rendered.ends_with("----------------\n"));
    }

    #[test]
    fn styled_full_render_colors_titles_and_body_unit() {
        let mut buffer = MessageBuffer::default();
        buffer.error("bad");
        buffer.warning("careful");

        let theme = resolve_theme("dracula");
        let overrides = crate::ui::StyleOverrides::default();
        let styler = ThemeStyler::new(true, &theme, &overrides);
        let rendered = render_messages_with_styler(
            &buffer,
            MessageRenderOptions::full(MessageLevel::Warning),
            &styler,
        );

        assert!(rendered.contains("\x1b["));
        assert!(rendered.contains("Errors"));
        assert!(rendered.contains("\x1b[38;2;248;248;242m  careful"));
    }

    #[test]
    fn styled_austere_render_colors_prefix_without_recoloring_message_body_unit() {
        let mut buffer = MessageBuffer::default();
        buffer.info("hint");

        let theme = resolve_theme("dracula");
        let overrides = crate::ui::StyleOverrides::default();
        let styler = ThemeStyler::new(true, &theme, &overrides);
        let rendered = render_messages_with_styler(
            &buffer,
            MessageRenderOptions::austere(MessageLevel::Info),
            &styler,
        );

        assert!(rendered.contains("  \x1b[38;2;139;233;253minfo\x1b[0m"));
        assert!(rendered.ends_with(" hint\n"));
    }
}
