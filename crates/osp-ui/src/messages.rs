use std::fmt::Write;

use crate::style::{StyleToken, apply_style};

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

    fn style_token(self) -> StyleToken {
        match self {
            MessageLevel::Error => StyleToken::MessageError,
            MessageLevel::Warning => StyleToken::MessageWarning,
            MessageLevel::Success => StyleToken::MessageSuccess,
            MessageLevel::Info => StyleToken::MessageInfo,
            MessageLevel::Trace => StyleToken::MessageTrace,
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
        let mut out = String::new();

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

            let mut wrote_header = false;
            for entry in self.entries.iter().filter(|entry| entry.level == level) {
                if !wrote_header {
                    let _ = writeln!(out, "{}:", level.title());
                    wrote_header = true;
                }
                let _ = writeln!(out, "- {}", entry.text);
            }

            if wrote_header {
                out.push('\n');
            }
        }

        out
    }

    pub fn render_grouped_styled(
        &self,
        max_level: MessageLevel,
        color: bool,
        unicode: bool,
        width: Option<usize>,
        theme_name: &str,
        boxed: bool,
    ) -> String {
        let mut out = String::new();

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

            let mut wrote_header = false;
            for entry in self.entries.iter().filter(|entry| entry.level == level) {
                if !wrote_header {
                    let header = render_section_divider(
                        level.title(),
                        unicode,
                        width,
                        color,
                        theme_name,
                        level.style_token(),
                    );
                    let _ = writeln!(out, "{header}");
                    wrote_header = true;
                }
                let _ = writeln!(out, "- {}", entry.text);
            }

            if wrote_header {
                if boxed {
                    let footer = render_section_divider(
                        "",
                        unicode,
                        width,
                        color,
                        theme_name,
                        level.style_token(),
                    );
                    let _ = writeln!(out, "{footer}");
                }
                out.push('\n');
            }
        }

        out
    }
}

pub fn render_section_divider(
    title: &str,
    unicode: bool,
    width: Option<usize>,
    color: bool,
    theme_name: &str,
    token: StyleToken,
) -> String {
    let fill_char = if unicode { '─' } else { '-' };
    let target_width = width.unwrap_or(72).max(12);
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

    if color {
        apply_style(&raw, token, true, theme_name)
    } else {
        raw
    }
}

pub fn adjust_verbosity(base: MessageLevel, verbose: u8, quiet: u8) -> MessageLevel {
    let rank = base.as_rank() + verbose as i8 - quiet as i8;
    MessageLevel::from_rank(rank)
}

#[cfg(test)]
mod tests {
    use super::{MessageBuffer, MessageLevel, adjust_verbosity};

    #[test]
    fn default_success_hides_info_and_debug() {
        let mut messages = MessageBuffer::default();
        messages.error("bad");
        messages.warning("careful");
        messages.success("done");
        messages.info("hint");
        messages.trace("trace");

        let rendered = messages.render_grouped(MessageLevel::Success);
        assert!(rendered.contains("Errors:"));
        assert!(rendered.contains("Warnings:"));
        assert!(rendered.contains("Success:"));
        assert!(!rendered.contains("Info:"));
        assert!(!rendered.contains("Trace:"));
    }

    #[test]
    fn styled_render_uses_boxed_headers() {
        let mut messages = MessageBuffer::default();
        messages.error("bad");
        let rendered = messages.render_grouped_styled(
            MessageLevel::Error,
            false,
            true,
            Some(24),
            "rose-pine-moon",
            true,
        );
        assert!(rendered.contains("─ Errors "));
        assert!(
            rendered
                .lines()
                .any(|line| line.trim() == "────────────────────────")
        );
    }

    #[test]
    fn styled_render_color_toggle_controls_ansi() {
        let mut messages = MessageBuffer::default();
        messages.warning("careful");

        let plain = messages.render_grouped_styled(
            MessageLevel::Warning,
            false,
            false,
            Some(28),
            "rose-pine-moon",
            true,
        );
        let colored = messages.render_grouped_styled(
            MessageLevel::Warning,
            true,
            false,
            Some(28),
            "rose-pine-moon",
            true,
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
