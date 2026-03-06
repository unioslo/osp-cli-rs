use crate::document::{Block, Document, LineBlock, LinePart};
use crate::format::message::{
    MessageContent, MessageFormatter, MessageKind, MessageOptions, MessageRules,
};
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

    fn as_kind(self) -> MessageKind {
        match self {
            MessageLevel::Error => MessageKind::Error,
            MessageLevel::Warning => MessageKind::Warning,
            MessageLevel::Success => MessageKind::Success,
            MessageLevel::Info => MessageKind::Info,
            MessageLevel::Trace => MessageKind::Trace,
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
pub enum MessageRenderFormat {
    Groups,
    Rules,
}

impl MessageRenderFormat {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "groups" | "grouped" | "plain" => Some(Self::Groups),
            "rules" | "panel" | "boxes" | "boxed" => Some(Self::Rules),
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
            MessageRenderFormat::Rules,
        )
    }

    pub fn render_grouped_styled(
        &self,
        max_level: MessageLevel,
        color: bool,
        unicode: bool,
        width: Option<usize>,
        theme: &ThemeDefinition,
        format: MessageRenderFormat,
    ) -> String {
        self.render_grouped_styled_with_overrides(
            max_level,
            color,
            unicode,
            width,
            theme,
            format,
            &StyleOverrides::default(),
        )
    }

    pub fn render_grouped_styled_with_overrides(
        &self,
        max_level: MessageLevel,
        color: bool,
        unicode: bool,
        width: Option<usize>,
        theme: &ThemeDefinition,
        format: MessageRenderFormat,
        style_overrides: &StyleOverrides,
    ) -> String {
        let document = self.build_grouped_document(max_level, format);
        if document.blocks.is_empty() {
            return String::new();
        }

        let resolved = ResolvedRenderSettings {
            backend: if color || unicode {
                RenderBackend::Rich
            } else {
                RenderBackend::Plain
            },
            color,
            unicode,
            width,
            margin: 0,
            indent_size: 2,
            short_list_max: 1,
            medium_list_max: 5,
            grid_padding: 4,
            grid_columns: None,
            column_weight: 3,
            table_overflow: crate::TableOverflow::Clip,
            theme_name: theme.id.clone(),
            theme: theme.clone(),
            style_overrides: style_overrides.clone(),
        };
        render_document(&document, resolved)
    }

    fn build_grouped_document(
        &self,
        max_level: MessageLevel,
        format: MessageRenderFormat,
    ) -> Document {
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
                    Block::Line(LineBlock {
                        parts: vec![LinePart {
                            text: format!("- {}", entry.text),
                            token: None,
                        }],
                    })
                })
                .collect::<Vec<Block>>();

            if matches!(format, MessageRenderFormat::Rules) {
                let rendered = MessageFormatter::build(
                    MessageContent::Document(Document { blocks: body }),
                    MessageOptions {
                        rules: MessageRules::Both,
                        kind: level.as_kind(),
                        title: Some(level.title().to_string()),
                    },
                );
                blocks.extend(rendered.blocks);
            } else {
                blocks.push(Block::Line(LineBlock {
                    parts: vec![LinePart {
                        text: format!("{}:", level.title()),
                        token: None,
                    }],
                }));
                blocks.extend(body);
            }

            blocks.push(Block::Line(LineBlock {
                parts: vec![LinePart {
                    text: String::new(),
                    token: None,
                }],
            }));
        }

        Document { blocks }
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
    let target_width = width
        .or_else(|| {
            std::env::var("COLUMNS")
                .ok()
                .and_then(|value| value.parse::<usize>().ok())
                .filter(|value| *value > 0)
        })
        .unwrap_or(12)
        .max(12);
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

pub fn adjust_verbosity(base: MessageLevel, verbose: u8, quiet: u8) -> MessageLevel {
    let rank = base.as_rank() + verbose as i8 - quiet as i8;
    MessageLevel::from_rank(rank)
}

#[cfg(test)]
mod tests {
    use super::{MessageBuffer, MessageLevel, MessageRenderFormat, adjust_verbosity};

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
            MessageRenderFormat::Rules,
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
            MessageRenderFormat::Rules,
        );
        let colored = messages.render_grouped_styled(
            MessageLevel::Warning,
            true,
            false,
            Some(28),
            &theme,
            MessageRenderFormat::Rules,
        );
        assert!(!plain.contains("\x1b["));
        assert!(colored.contains("\x1b["));
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
}
