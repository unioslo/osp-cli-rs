//! Buffered user-facing messages and their canonical rendering helpers.

mod render;
#[cfg(test)]
mod sink;

use super::text::visible_inline_text;
use crate::config::ResolvedConfig;
use crate::ui::section_chrome::RuledSectionPolicy;
use crate::ui::section_chrome::SectionFrameStyle;
use crate::ui::settings::{RenderProfile, RenderSettings, resolve_settings};
use crate::ui::style::{StyleOverrides, StyleToken, ThemeStyler};
use crate::ui::theme::ThemeDefinition;

pub(crate) use render::render_messages_with_styler_and_chrome;
pub(crate) use render::{
    MessageChrome, MessageFrameStyle, MessageRenderOptions, MessageRulePolicy,
    render_messages_with_styler_from_config,
};

/// Severity level for buffered UI messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MessageLevel {
    Error,
    Warning,
    Success,
    Info,
    Trace,
}

impl MessageLevel {
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

    pub(crate) fn ordered() -> impl Iterator<Item = Self> {
        [
            MessageLevel::Error,
            MessageLevel::Warning,
            MessageLevel::Success,
            MessageLevel::Info,
            MessageLevel::Trace,
        ]
        .into_iter()
    }

    pub(crate) fn style_token(self) -> StyleToken {
        match self {
            MessageLevel::Error => StyleToken::Error,
            MessageLevel::Warning => StyleToken::Warning,
            MessageLevel::Success => StyleToken::Success,
            MessageLevel::Info => StyleToken::Info,
            MessageLevel::Trace => StyleToken::Trace,
        }
    }
}

/// Layout style used when rendering buffered messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageLayout {
    Minimal,
    Plain,
    Compact,
    Grouped,
}

impl MessageLayout {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "minimal" | "austere" => Some(Self::Minimal),
            "plain" | "none" => Some(Self::Plain),
            "compact" => Some(Self::Compact),
            "grouped" | "full" => Some(Self::Grouped),
            _ => None,
        }
    }
}

/// A single UI message with its associated severity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiMessage {
    pub level: MessageLevel,
    pub text: String,
}

/// In-memory buffer for messages collected during command execution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MessageBuffer {
    entries: Vec<UiMessage>,
}

/// Options for rendering grouped message output.
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
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push<T: Into<String>>(&mut self, level: MessageLevel, text: T) {
        self.push_message(UiMessage::new(level, text));
    }

    pub(crate) fn push_message(&mut self, message: UiMessage) {
        self.entries.push(message);
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

    pub fn entries(&self) -> &[UiMessage] {
        &self.entries
    }

    pub(crate) fn entries_for_level(
        &self,
        level: MessageLevel,
    ) -> impl Iterator<Item = &UiMessage> {
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
        if self.is_empty() {
            return String::new();
        }

        let styler = ThemeStyler::new(options.color, options.theme, &options.style_overrides);
        render::render_messages_with_styler_and_chrome(
            self,
            render::MessageRenderOptions {
                max_level: options.max_level,
                layout: options.layout,
            },
            &styler,
            render::MessageChrome {
                frame_style: message_frame_style(options.chrome_frame),
                ruled_policy: render::MessageRulePolicy::PerSection,
                unicode: options.unicode,
                width: options.width,
            },
        )
    }
}

pub(crate) fn message_layout_from_config(config: &ResolvedConfig) -> MessageLayout {
    config
        .get_string("ui.messages.layout")
        .and_then(MessageLayout::parse)
        .unwrap_or(MessageLayout::Grouped)
}

pub(crate) fn render_messages_from_settings(
    config: &ResolvedConfig,
    settings: &RenderSettings,
    messages: &MessageBuffer,
    verbosity: MessageLevel,
) -> String {
    let resolved = resolve_settings(settings, RenderProfile::Normal);
    let styler = ThemeStyler::new(resolved.color, &resolved.theme, &resolved.style_overrides);
    render_messages_with_styler_from_config(
        messages,
        config,
        verbosity,
        &styler,
        MessageChrome {
            frame_style: match settings.chrome_frame {
                SectionFrameStyle::None => MessageFrameStyle::None,
                SectionFrameStyle::Top => MessageFrameStyle::Top,
                SectionFrameStyle::Bottom => MessageFrameStyle::Bottom,
                SectionFrameStyle::TopBottom => MessageFrameStyle::TopBottom,
                SectionFrameStyle::Square => MessageFrameStyle::Square,
                SectionFrameStyle::Round => MessageFrameStyle::Round,
            },
            ruled_policy: match settings.ruled_section_policy {
                RuledSectionPolicy::PerSection => MessageRulePolicy::PerSection,
                RuledSectionPolicy::Shared => MessageRulePolicy::Shared,
            },
            unicode: resolved.unicode,
            width: resolved.width,
        },
    )
}

pub(crate) fn render_messages_without_config(
    settings: &RenderSettings,
    messages: &MessageBuffer,
    verbosity: MessageLevel,
) -> String {
    let resolved = resolve_settings(settings, RenderProfile::Normal);
    let styler = ThemeStyler::new(resolved.color, &resolved.theme, &resolved.style_overrides);
    render_messages_with_styler_and_chrome(
        &visible_message_buffer(messages),
        MessageRenderOptions::full(verbosity),
        &styler,
        MessageChrome {
            frame_style: MessageFrameStyle::TopBottom,
            ruled_policy: MessageRulePolicy::Shared,
            unicode: resolved.unicode,
            width: resolved.width.or(Some(12)),
        },
    )
}

fn visible_message_buffer(messages: &MessageBuffer) -> MessageBuffer {
    let mut out = MessageBuffer::default();
    for entry in messages.entries() {
        out.push(entry.level, visible_inline_text(&entry.text));
    }
    out
}

fn default_message_chrome_frame(layout: MessageLayout) -> SectionFrameStyle {
    match layout {
        MessageLayout::Minimal | MessageLayout::Plain | MessageLayout::Compact => {
            SectionFrameStyle::None
        }
        MessageLayout::Grouped => SectionFrameStyle::TopBottom,
    }
}

fn message_frame_style(frame: SectionFrameStyle) -> render::MessageFrameStyle {
    match frame {
        SectionFrameStyle::None => render::MessageFrameStyle::None,
        SectionFrameStyle::Top => render::MessageFrameStyle::Top,
        SectionFrameStyle::Bottom => render::MessageFrameStyle::Bottom,
        SectionFrameStyle::TopBottom => render::MessageFrameStyle::TopBottom,
        SectionFrameStyle::Square => render::MessageFrameStyle::Square,
        SectionFrameStyle::Round => render::MessageFrameStyle::Round,
    }
}

pub fn adjust_verbosity(base: MessageLevel, verbose: u8, quiet: u8) -> MessageLevel {
    let rank = base.as_rank() + verbose as i8 - quiet as i8;
    MessageLevel::from_rank(rank)
}

impl UiMessage {
    pub(crate) fn new(level: MessageLevel, text: impl Into<String>) -> Self {
        Self {
            level,
            text: text.into(),
        }
    }
}
