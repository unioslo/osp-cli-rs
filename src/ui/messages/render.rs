//! Message rendering intent for the canonical UI pipeline.
//!
//! The current surface stays plain and layout-focused, but it already sits on
//! top of the same chrome boundary the terminal emitter uses.

use crate::config::ResolvedConfig;
use crate::ui::chrome::PLAIN_SECTION_CHROME;
use crate::ui::section_chrome::{
    RuledSectionPolicy, SectionFrameStyle, SectionRenderContext, SectionStyleTokens,
    render_section_block_with_overrides, render_section_divider_with_overrides,
};
use crate::ui::style::{StyleToken, ThemeStyler};
use crate::ui::text::wrap_display_width;

use super::{MessageBuffer, MessageLayout, MessageLevel, message_layout_from_config};

#[derive(Debug, Clone, PartialEq, Eq)]
struct RenderedSection {
    level: MessageLevel,
    title: String,
    lines: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MessageChrome {
    pub frame_style: SectionFrameStyle,
    pub ruled_policy: RuledSectionPolicy,
    pub unicode: bool,
    pub width: Option<usize>,
}

impl Default for MessageChrome {
    fn default() -> Self {
        Self {
            frame_style: SectionFrameStyle::Top,
            ruled_policy: RuledSectionPolicy::Shared,
            unicode: false,
            width: None,
        }
    }
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
    render_messages_unstyled_with_chrome(buffer, options, MessageChrome::default())
}

/// Renders messages using the requested layout and semantic styling.
#[cfg(test)]
pub fn render_messages_with_styler(
    buffer: &MessageBuffer,
    options: MessageRenderOptions,
    styler: &ThemeStyler<'_>,
) -> String {
    render_messages_internal(buffer, options, styler, MessageChrome::default())
}

pub(crate) fn render_messages_with_styler_and_chrome(
    buffer: &MessageBuffer,
    options: MessageRenderOptions,
    styler: &ThemeStyler<'_>,
    chrome: MessageChrome,
) -> String {
    render_messages_internal(buffer, options, styler, chrome)
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
        styler,
        chrome,
    )
}

fn render_messages_internal(
    buffer: &MessageBuffer,
    options: MessageRenderOptions,
    styler: &ThemeStyler<'_>,
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

#[cfg(test)]
fn render_messages_unstyled_with_chrome(
    buffer: &MessageBuffer,
    options: MessageRenderOptions,
    chrome: MessageChrome,
) -> String {
    let theme = crate::ui::theme::resolve_theme("plain");
    let overrides = crate::ui::style::StyleOverrides::default();
    let styler = ThemeStyler::new(false, &theme, &overrides);
    render_messages_internal(buffer, options, &styler, chrome)
}

fn render_austere(
    buffer: &MessageBuffer,
    max_level: MessageLevel,
    styler: &ThemeStyler<'_>,
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
    styler: &ThemeStyler<'_>,
    chrome: MessageChrome,
) -> String {
    let sections = sectioned_messages(buffer, max_level);
    if sections.is_empty() {
        return String::new();
    }

    match (chrome.frame_style, chrome.ruled_policy) {
        (SectionFrameStyle::Top | SectionFrameStyle::TopBottom, RuledSectionPolicy::Shared) => {
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
    styler: &ThemeStyler<'_>,
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
    styler: &ThemeStyler<'_>,
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
    styler: &ThemeStyler<'_>,
    chrome: MessageChrome,
) -> String {
    let render = section_render_context(styler);
    let width = chrome.width.or(Some(24));
    let mut lines = Vec::new();
    for section in sections {
        lines.push(render_section_divider_with_overrides(
            &section.title,
            chrome.unicode,
            width,
            render,
            SectionStyleTokens::same(section.level.style_token()),
        ));
        lines.extend(message_body_lines(section, chrome.width, styler));
    }
    if matches!(chrome.frame_style, SectionFrameStyle::TopBottom) {
        let token = sections
            .last()
            .map(|section| section.level.style_token())
            .unwrap_or(StyleToken::Border);
        lines.push(render_section_divider_with_overrides(
            "",
            chrome.unicode,
            width,
            render,
            SectionStyleTokens::same(token),
        ));
    }
    lines.join("\n")
}

fn render_full_section(
    section: &RenderedSection,
    styler: &ThemeStyler<'_>,
    chrome: MessageChrome,
) -> String {
    let body = message_body_lines(section, chrome.width, styler).join("\n");
    render_section_block_with_overrides(
        &section.title,
        &body,
        chrome.frame_style,
        chrome.unicode,
        chrome.width.or(Some(24)),
        section_render_context(styler),
        SectionStyleTokens::same(section.level.style_token()),
    )
}

fn section_render_context<'a>(styler: &ThemeStyler<'a>) -> SectionRenderContext<'a> {
    SectionRenderContext {
        color: styler.enabled,
        theme: styler.theme,
        style_overrides: styler.overrides,
    }
}

fn render_compact_section(section: &RenderedSection, styler: &ThemeStyler<'_>) -> String {
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

fn message_body_lines(
    section: &RenderedSection,
    width: Option<usize>,
    styler: &ThemeStyler<'_>,
) -> Vec<String> {
    // Bootstrap errors can only supply the 12-column chrome fallback, which is
    // not a trustworthy terminal width. Wrapping at that hint makes paths and
    // diagnostic identifiers unreadable; wait for a useful width measurement.
    let content_width = width
        .filter(|width| *width >= 40)
        .map(|width| width.saturating_sub(2));
    section
        .lines
        .iter()
        .flat_map(|line| {
            line.split('\n').flat_map(|line| match content_width {
                Some(width) => wrap_display_width(line, width),
                None => vec![line.to_string()],
            })
        })
        .map(|line| paint(styler, &format!("  {line}"), StyleToken::Text))
        .collect()
}

fn paint(styler: &ThemeStyler<'_>, text: &str, token: StyleToken) -> String {
    styler.paint(text, token)
}

#[cfg(test)]
mod tests {
    use super::{
        MessageChrome, MessageRenderOptions, render_messages, render_messages_internal,
        render_messages_unstyled_with_chrome, render_messages_with_styler,
        render_messages_with_styler_from_config,
    };
    use crate::config::{ConfigLayer, ConfigResolver, LoadedLayers, ResolveOptions};
    use crate::ui::ThemeStyler;
    use crate::ui::messages::{MessageBuffer, MessageLevel};
    use crate::ui::section_chrome::{RuledSectionPolicy, SectionFrameStyle};
    use crate::ui::theme::resolve_theme;
    use unicode_width::UnicodeWidthStr;

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

        let rendered = render_messages_unstyled_with_chrome(
            &buffer,
            MessageRenderOptions::full(MessageLevel::Warning),
            MessageChrome {
                frame_style: SectionFrameStyle::TopBottom,
                ruled_policy: RuledSectionPolicy::Shared,
                unicode: false,
                width: Some(16),
            },
        );

        assert!(rendered.contains("- Errors "));
        assert!(rendered.contains("- Warnings "));
        assert!(rendered.ends_with("----------------\n"));
    }

    #[test]
    fn full_render_wraps_long_messages_within_chrome_width_unit() {
        let mut buffer = MessageBuffer::default();
        buffer.error(
            "capabilities_request_failed: Failed to fetch capabilities for provider 'vmware'.",
        );

        let rendered = render_messages_unstyled_with_chrome(
            &buffer,
            MessageRenderOptions::full(MessageLevel::Error),
            MessageChrome {
                frame_style: SectionFrameStyle::TopBottom,
                ruled_policy: RuledSectionPolicy::Shared,
                unicode: true,
                width: Some(40),
            },
        );

        assert!(
            rendered.contains("\n  capabilities_request_failed: Failed to\n"),
            "{rendered}"
        );
        assert!(
            rendered.contains("\n  fetch capabilities for provider\n  'vmware'.\n"),
            "{rendered}"
        );
        assert!(
            rendered
                .lines()
                .all(|line| UnicodeWidthStr::width(line) <= 40)
        );
    }

    #[test]
    fn styled_top_bottom_message_bottom_rule_uses_message_level_color_unit() {
        let mut buffer = MessageBuffer::default();
        buffer.success("done");

        let theme = resolve_theme("dracula");
        let overrides = crate::ui::StyleOverrides::default();
        let styler = ThemeStyler::new(true, &theme, &overrides);
        let rendered = render_messages_internal(
            &buffer,
            MessageRenderOptions::full(MessageLevel::Success),
            &styler,
            MessageChrome {
                frame_style: SectionFrameStyle::TopBottom,
                ruled_policy: RuledSectionPolicy::PerSection,
                unicode: false,
                width: Some(12),
            },
        );
        let bottom = rendered
            .lines()
            .last()
            .expect("rendered message should have a closing rule");

        assert_eq!(bottom, "\x1b[38;2;80;250;123m------------\x1b[0m");
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

    #[test]
    fn config_driven_message_rendering_uses_layout_from_config_unit() {
        let mut buffer = MessageBuffer::default();
        buffer.error("bad");
        let theme = resolve_theme("dracula");
        let overrides = crate::ui::StyleOverrides::default();
        let styler = ThemeStyler::new(true, &theme, &overrides);
        let config = resolved_config(&[("ui.messages.layout", "compact")]);

        let rendered = render_messages_with_styler_from_config(
            &buffer,
            &config,
            MessageLevel::Error,
            &styler,
            MessageChrome::default(),
        );

        assert!(rendered.contains("Errors:"));
        assert!(rendered.contains("bad"));
        assert!(!rendered.contains("--------"));
    }

    #[test]
    fn full_render_supports_bottom_and_round_frames_unit() {
        let mut buffer = MessageBuffer::default();
        buffer.error("bad");

        let bottom = render_messages_unstyled_with_chrome(
            &buffer,
            MessageRenderOptions::full(MessageLevel::Error),
            MessageChrome {
                frame_style: SectionFrameStyle::Bottom,
                ruled_policy: RuledSectionPolicy::PerSection,
                unicode: false,
                width: Some(12),
            },
        );
        assert!(bottom.starts_with("Errors:"));
        assert!(bottom.contains("\n------------"));

        let round = render_messages_unstyled_with_chrome(
            &buffer,
            MessageRenderOptions::full(MessageLevel::Error),
            MessageChrome {
                frame_style: SectionFrameStyle::Round,
                ruled_policy: RuledSectionPolicy::PerSection,
                unicode: true,
                width: Some(14),
            },
        );
        assert!(round.contains('╭'));
        assert!(round.contains('│'));
        assert!(round.contains('╯'));
    }
}
