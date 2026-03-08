use nu_ansi_term::{Color, Style};

use crate::theme::{self, ThemeDefinition};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StyleOverrides {
    pub text: Option<String>,
    pub key: Option<String>,
    pub muted: Option<String>,
    pub table_header: Option<String>,
    pub mreg_key: Option<String>,
    pub value: Option<String>,
    pub number: Option<String>,
    pub bool_true: Option<String>,
    pub bool_false: Option<String>,
    pub null_value: Option<String>,
    pub ipv4: Option<String>,
    pub ipv6: Option<String>,
    pub panel_border: Option<String>,
    pub panel_title: Option<String>,
    pub code: Option<String>,
    pub json_key: Option<String>,
    pub message_error: Option<String>,
    pub message_warning: Option<String>,
    pub message_success: Option<String>,
    pub message_info: Option<String>,
    pub message_trace: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleToken {
    None,
    Key,
    Muted,
    PromptText,
    PromptCommand,
    TableHeader,
    MregKey,
    JsonKey,
    Code,
    PanelBorder,
    PanelTitle,
    Value,
    Number,
    BoolTrue,
    BoolFalse,
    Null,
    Ipv4,
    Ipv6,
    MessageError,
    MessageWarning,
    MessageSuccess,
    MessageInfo,
    MessageTrace,
}

pub fn apply_style(text: &str, token: StyleToken, color: bool, theme_name: &str) -> String {
    apply_style_with_overrides(text, token, color, theme_name, &StyleOverrides::default())
}

pub fn apply_style_with_overrides(
    text: &str,
    token: StyleToken,
    color: bool,
    theme_name: &str,
    overrides: &StyleOverrides,
) -> String {
    let theme = theme::resolve_theme(theme_name);
    apply_style_with_theme_overrides(text, token, color, &theme, overrides)
}

pub fn apply_style_with_theme(
    text: &str,
    token: StyleToken,
    color: bool,
    theme: &ThemeDefinition,
) -> String {
    apply_style_with_theme_overrides(text, token, color, theme, &StyleOverrides::default())
}

pub fn apply_style_with_theme_overrides(
    text: &str,
    token: StyleToken,
    color: bool,
    theme: &ThemeDefinition,
    overrides: &StyleOverrides,
) -> String {
    if !color || matches!(token, StyleToken::None) {
        return text.to_string();
    }

    apply_style_spec(text, resolve_style_spec(token, theme, overrides), color)
}

pub fn apply_style_spec(text: &str, spec: &str, color: bool) -> String {
    if !color {
        return text.to_string();
    }
    let Some(style) = parse_style_spec(spec) else {
        return text.to_string();
    };
    let prefix = style.prefix().to_string();
    if prefix.is_empty() {
        return text.to_string();
    }
    format!("{prefix}{text}{}", style.suffix())
}

pub fn is_valid_style_spec(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return true;
    }

    trimmed.split_whitespace().all(|raw| {
        let token = raw.trim().to_ascii_lowercase();
        !token.is_empty() && (is_style_modifier(&token) || parse_color_token(&token).is_some())
    })
}

fn resolve_style_spec<'a>(
    token: StyleToken,
    theme: &'a ThemeDefinition,
    overrides: &'a StyleOverrides,
) -> &'a str {
    overrides
        .spec_for(token)
        .unwrap_or_else(|| token.theme_spec(theme))
}

impl StyleOverrides {
    fn spec_for(&self, token: StyleToken) -> Option<&str> {
        match token {
            StyleToken::None | StyleToken::PromptText | StyleToken::PromptCommand => None,
            StyleToken::Key => self.key.as_deref(),
            StyleToken::Muted => self.muted.as_deref(),
            StyleToken::TableHeader => self.table_header.as_deref().or(self.key.as_deref()),
            StyleToken::MregKey => self.mreg_key.as_deref().or(self.key.as_deref()),
            StyleToken::JsonKey => self.json_key.as_deref().or(self.key.as_deref()),
            StyleToken::Code => self.code.as_deref().or(self.text.as_deref()),
            StyleToken::PanelBorder => self.panel_border.as_deref(),
            StyleToken::PanelTitle => self.panel_title.as_deref(),
            StyleToken::Value => self.value.as_deref().or(self.text.as_deref()),
            StyleToken::Number => self.number.as_deref(),
            StyleToken::BoolTrue => self.bool_true.as_deref(),
            StyleToken::BoolFalse => self.bool_false.as_deref(),
            StyleToken::Null => self.null_value.as_deref(),
            StyleToken::Ipv4 => self.ipv4.as_deref(),
            StyleToken::Ipv6 => self.ipv6.as_deref(),
            StyleToken::MessageError => self.message_error.as_deref(),
            StyleToken::MessageWarning => self.message_warning.as_deref(),
            StyleToken::MessageSuccess => self.message_success.as_deref(),
            StyleToken::MessageInfo => self.message_info.as_deref(),
            StyleToken::MessageTrace => self.message_trace.as_deref(),
        }
    }
}

impl StyleToken {
    fn theme_spec<'a>(self, theme: &'a ThemeDefinition) -> &'a str {
        match self {
            StyleToken::None => "",
            StyleToken::Key
            | StyleToken::TableHeader
            | StyleToken::MregKey
            | StyleToken::JsonKey => &theme.palette.accent,
            StyleToken::Muted | StyleToken::Null => &theme.palette.muted,
            StyleToken::PromptText | StyleToken::Code | StyleToken::Value => &theme.palette.text,
            StyleToken::PromptCommand | StyleToken::BoolTrue | StyleToken::MessageSuccess => {
                &theme.palette.success
            }
            StyleToken::PanelBorder
            | StyleToken::Ipv4
            | StyleToken::Ipv6
            | StyleToken::MessageTrace => &theme.palette.border,
            StyleToken::PanelTitle => &theme.palette.title,
            StyleToken::Number => theme.value_number_spec(),
            StyleToken::BoolFalse | StyleToken::MessageError => &theme.palette.error,
            StyleToken::MessageWarning => &theme.palette.warning,
            StyleToken::MessageInfo => &theme.palette.info,
        }
    }
}

fn parse_style_spec(spec: &str) -> Option<Style> {
    let mut style = Style::new();
    let mut changed = false;

    for raw in spec.split_whitespace() {
        let token = raw.trim().to_ascii_lowercase();
        if token.is_empty() {
            continue;
        }

        if let Some(updated) = apply_style_token(style, &token) {
            style = updated;
            changed = true;
        }
    }

    changed.then_some(style)
}

fn apply_style_token(style: Style, token: &str) -> Option<Style> {
    match token {
        "bold" => Some(style.bold()),
        "dim" => Some(style.dimmed()),
        "italic" => Some(style.italic()),
        "underline" => Some(style.underline()),
        _ => parse_color_token(token).map(|color| style.fg(color)),
    }
}

fn is_style_modifier(token: &str) -> bool {
    matches!(token, "bold" | "dim" | "italic" | "underline")
}

fn parse_color_token(token: &str) -> Option<Color> {
    match token {
        "black" => Some(Color::Black),
        "red" => Some(Color::Red),
        "green" => Some(Color::Green),
        "yellow" => Some(Color::Yellow),
        "blue" => Some(Color::Blue),
        "purple" | "magenta" => Some(Color::Purple),
        "cyan" => Some(Color::Cyan),
        "white" => Some(Color::White),
        "bright-black" => Some(Color::DarkGray),
        "bright-red" => Some(Color::LightRed),
        "bright-green" => Some(Color::LightGreen),
        "bright-yellow" => Some(Color::LightYellow),
        "bright-blue" => Some(Color::LightBlue),
        "bright-purple" | "bright-magenta" => Some(Color::LightPurple),
        "bright-cyan" => Some(Color::LightCyan),
        "bright-white" => Some(Color::LightGray),
        _ => parse_hex_rgb(token).map(|(r, g, b)| Color::Rgb(r, g, b)),
    }
}

fn parse_hex_rgb(value: &str) -> Option<(u8, u8, u8)> {
    if !value.starts_with('#') || value.len() != 7 {
        return None;
    }
    let r = u8::from_str_radix(&value[1..3], 16).ok()?;
    let g = u8::from_str_radix(&value[3..5], 16).ok()?;
    let b = u8::from_str_radix(&value[5..7], 16).ok()?;
    Some((r, g, b))
}

#[cfg(test)]
mod tests {
    use crate::theme;

    use super::{
        StyleOverrides, StyleToken, apply_style, apply_style_spec, apply_style_with_overrides,
        apply_style_with_theme, apply_style_with_theme_overrides,
    };

    #[test]
    fn plain_theme_disables_styling_even_with_color_enabled() {
        let out = apply_style("hello", StyleToken::MessageInfo, true, "plain");
        assert_eq!(out, "hello");
    }

    #[test]
    fn dracula_error_uses_bold_truecolor_escape() {
        let out = apply_style("oops", StyleToken::MessageError, true, "dracula");
        assert!(out.starts_with("\x1b[1;38;2;255;85;85m"));
        assert!(out.ends_with("\x1b[0m"));
    }

    #[test]
    fn nord_and_dracula_produce_different_info_colors() {
        let nord = apply_style("info", StyleToken::MessageInfo, true, "nord");
        let dracula = apply_style("info", StyleToken::MessageInfo, true, "dracula");
        assert_ne!(nord, dracula);
    }

    #[test]
    fn dracula_number_uses_theme_override_color() {
        let out = apply_style("42", StyleToken::Number, true, "dracula");
        assert!(out.starts_with("\x1b[38;2;255;121;198m"));
    }

    #[test]
    fn color_toggle_off_returns_plain_text() {
        let out = apply_style("warn", StyleToken::MessageWarning, false, "nord");
        assert_eq!(out, "warn");
    }

    #[test]
    fn explicit_override_takes_precedence_over_theme_token() {
        let out = apply_style_with_overrides(
            "head",
            StyleToken::TableHeader,
            true,
            "nord",
            &StyleOverrides {
                table_header: Some("#ff0000".to_string()),
                ..Default::default()
            },
        );
        assert!(out.starts_with("\x1b[38;2;255;0;0m"));
    }

    #[test]
    fn generic_text_override_reaches_value_and_code_tokens_unit() {
        let overrides = StyleOverrides {
            text: Some("#112233".to_string()),
            ..Default::default()
        };
        let value =
            apply_style_with_overrides("hello", StyleToken::Value, true, "nord", &overrides);
        let code =
            apply_style_with_overrides("let x = 1;", StyleToken::Code, true, "nord", &overrides);

        assert!(value.starts_with("\x1b[38;2;17;34;51m"));
        assert!(code.starts_with("\x1b[38;2;17;34;51m"));
    }

    #[test]
    fn generic_key_override_reaches_key_like_tokens_unit() {
        let overrides = StyleOverrides {
            key: Some("#abcdef".to_string()),
            ..Default::default()
        };
        let table =
            apply_style_with_overrides("host", StyleToken::TableHeader, true, "nord", &overrides);
        let json =
            apply_style_with_overrides("\"uid\"", StyleToken::JsonKey, true, "nord", &overrides);

        assert!(table.starts_with("\x1b[38;2;171;205;239m"));
        assert!(json.starts_with("\x1b[38;2;171;205;239m"));
    }

    #[test]
    fn message_override_reaches_message_tokens_unit() {
        let overrides = StyleOverrides {
            message_warning: Some("#ffaa00".to_string()),
            ..Default::default()
        };
        let out = apply_style_with_overrides(
            "careful",
            StyleToken::MessageWarning,
            true,
            "nord",
            &overrides,
        );
        assert!(out.starts_with("\x1b[38;2;255;170;0m"));
    }

    #[test]
    fn none_token_and_invalid_specs_fall_back_to_plain_text_unit() {
        assert_eq!(
            apply_style("plain", StyleToken::None, true, "nord"),
            "plain"
        );
        assert_eq!(apply_style_spec("plain", "mystery-token", true), "plain");
        assert_eq!(
            apply_style_spec("plain", "bold #zzzzzz", true),
            "\x1b[1mplain\x1b[0m"
        );
    }

    #[test]
    fn theme_and_override_helpers_cover_prompt_panel_and_ip_tokens_unit() {
        let theme = theme::resolve_theme("nord");

        let prompt = apply_style_with_theme("osp", StyleToken::PromptCommand, true, &theme);
        let ipv6 = apply_style_with_theme("::1", StyleToken::Ipv6, true, &theme);
        assert_ne!(prompt, "osp");
        assert_ne!(ipv6, "::1");

        let overrides = StyleOverrides {
            panel_border: Some("underline".to_string()),
            panel_title: Some("#445566".to_string()),
            ipv4: Some("bright-green".to_string()),
            bool_false: Some("red".to_string()),
            null_value: Some("dim".to_string()),
            ..Default::default()
        };

        assert!(
            apply_style_with_theme_overrides(
                "border",
                StyleToken::PanelBorder,
                true,
                &theme,
                &overrides
            )
            .starts_with("\x1b[4m")
        );
        assert!(
            apply_style_with_theme_overrides(
                "title",
                StyleToken::PanelTitle,
                true,
                &theme,
                &overrides
            )
            .starts_with("\x1b[38;2;68;85;102m")
        );
        assert!(
            apply_style_with_theme_overrides(
                "127.0.0.1",
                StyleToken::Ipv4,
                true,
                &theme,
                &overrides
            )
            .starts_with("\x1b[92m")
        );
        assert!(
            apply_style_with_theme_overrides(
                "false",
                StyleToken::BoolFalse,
                true,
                &theme,
                &overrides
            )
            .starts_with("\x1b[31m")
        );
        assert!(
            apply_style_with_theme_overrides("null", StyleToken::Null, true, &theme, &overrides)
                .starts_with("\x1b[2m")
        );
    }

    #[test]
    fn prompt_text_and_trace_tokens_cover_theme_defaults_unit() {
        let theme = theme::resolve_theme("dracula");
        let prompt = apply_style_with_theme("osp>", StyleToken::PromptText, true, &theme);
        let trace = apply_style_with_theme("trace", StyleToken::MessageTrace, true, &theme);

        assert_ne!(prompt, "osp>");
        assert_ne!(trace, "trace");
    }
}
