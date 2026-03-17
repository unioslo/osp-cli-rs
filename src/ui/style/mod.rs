use nu_ansi_term::{Color, Style};

use crate::ui::theme::ThemeDefinition;

/// Semantic style tokens used across the UI pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleToken {
    None,
    Trace,
    Info,
    Warning,
    Error,
    Success,
    Border,
    PanelBorder,
    PanelTitle,
    Key,
    TableHeader,
    MregKey,
    JsonKey,
    Text,
    Muted,
    TextMuted,
    PromptText,
    PromptCommand,
    Value,
    Number,
    ValueNumber,
    BoolTrue,
    BoolFalse,
    Null,
    Ipv4,
    Ipv6,
    Code,
    MessageError,
    MessageWarning,
    MessageSuccess,
    MessageInfo,
    MessageTrace,
    Punctuation,
}

/// Per-token style overrides layered over the resolved theme palette.
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

/// Small styling facade used by emitters and message/chrome helpers.
#[derive(Debug, Clone, Copy)]
pub struct ThemeStyler<'a> {
    enabled: bool,
    theme: &'a ThemeDefinition,
    overrides: &'a StyleOverrides,
}

impl<'a> ThemeStyler<'a> {
    pub fn new(enabled: bool, theme: &'a ThemeDefinition, overrides: &'a StyleOverrides) -> Self {
        Self {
            enabled,
            theme,
            overrides,
        }
    }

    pub fn paint(&self, text: &str, token: StyleToken) -> String {
        apply_style_spec(
            text,
            style_spec(self.theme, self.overrides, token),
            self.enabled,
        )
    }

    pub fn paint_value(&self, text: &str) -> String {
        self.paint(text, value_style_token(text))
    }
}

#[cfg(test)]
pub fn apply_style(text: &str, token: StyleToken, color: bool, theme_name: &str) -> String {
    let theme = crate::ui::theme::resolve_theme(theme_name);
    apply_style_with_theme(text, token, color, &theme)
}

#[cfg(test)]
pub fn apply_style_with_overrides(
    text: &str,
    token: StyleToken,
    color: bool,
    theme_name: &str,
    overrides: &StyleOverrides,
) -> String {
    let theme = crate::ui::theme::resolve_theme(theme_name);
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
    if !color || text.is_empty() || matches!(token, StyleToken::None) {
        return text.to_string();
    }

    apply_style_spec(text, style_spec(theme, overrides, token), color)
}

/// Returns the style specification for a semantic token under the selected theme.
pub fn style_spec<'a>(
    theme: &'a ThemeDefinition,
    overrides: &'a StyleOverrides,
    token: StyleToken,
) -> &'a str {
    if let Some(spec) = override_spec(overrides, token) {
        return spec;
    }

    match token {
        StyleToken::None => "",
        StyleToken::Trace
        | StyleToken::Border
        | StyleToken::PanelBorder
        | StyleToken::Ipv4
        | StyleToken::Ipv6
        | StyleToken::MessageTrace => theme.palette.border.as_str(),
        StyleToken::Muted | StyleToken::TextMuted | StyleToken::Null | StyleToken::Punctuation => {
            theme.palette.muted.as_str()
        }
        StyleToken::Info | StyleToken::MessageInfo => theme.palette.info.as_str(),
        StyleToken::Warning | StyleToken::MessageWarning => theme.palette.warning.as_str(),
        StyleToken::Error | StyleToken::MessageError | StyleToken::BoolFalse => {
            theme.palette.error.as_str()
        }
        StyleToken::Success
        | StyleToken::BoolTrue
        | StyleToken::PromptCommand
        | StyleToken::MessageSuccess => theme.palette.success.as_str(),
        StyleToken::PanelTitle => theme.palette.title.as_str(),
        StyleToken::Key | StyleToken::TableHeader | StyleToken::MregKey | StyleToken::JsonKey => {
            theme.palette.accent.as_str()
        }
        StyleToken::Text | StyleToken::PromptText | StyleToken::Code | StyleToken::Value => {
            theme.palette.text.as_str()
        }
        StyleToken::Number | StyleToken::ValueNumber => theme.value_number_spec(),
    }
}

fn override_spec<'a>(overrides: &'a StyleOverrides, token: StyleToken) -> Option<&'a str> {
    match token {
        StyleToken::None | StyleToken::PromptCommand => None,
        StyleToken::Trace | StyleToken::MessageTrace => overrides
            .message_trace
            .as_deref()
            .or(overrides.panel_border.as_deref()),
        StyleToken::Info | StyleToken::MessageInfo => overrides.message_info.as_deref(),
        StyleToken::Warning | StyleToken::MessageWarning => overrides.message_warning.as_deref(),
        StyleToken::Error | StyleToken::MessageError => overrides
            .message_error
            .as_deref()
            .or(overrides.panel_border.as_deref()),
        StyleToken::Success | StyleToken::MessageSuccess => overrides.message_success.as_deref(),
        StyleToken::Border | StyleToken::PanelBorder => overrides.panel_border.as_deref(),
        StyleToken::PanelTitle => overrides.panel_title.as_deref(),
        StyleToken::Key => overrides.key.as_deref(),
        StyleToken::TableHeader => overrides
            .table_header
            .as_deref()
            .or(overrides.key.as_deref()),
        StyleToken::MregKey => overrides.mreg_key.as_deref().or(overrides.key.as_deref()),
        StyleToken::JsonKey => overrides.json_key.as_deref().or(overrides.key.as_deref()),
        StyleToken::Text => overrides.text.as_deref(),
        StyleToken::Muted | StyleToken::TextMuted | StyleToken::Punctuation => {
            overrides.muted.as_deref()
        }
        StyleToken::Code => overrides.code.as_deref().or(overrides.text.as_deref()),
        StyleToken::Value | StyleToken::PromptText => {
            overrides.value.as_deref().or(overrides.text.as_deref())
        }
        StyleToken::Number | StyleToken::ValueNumber => overrides.number.as_deref(),
        StyleToken::BoolTrue => overrides
            .bool_true
            .as_deref()
            .or(overrides.message_success.as_deref()),
        StyleToken::BoolFalse => overrides
            .bool_false
            .as_deref()
            .or(overrides.message_error.as_deref()),
        StyleToken::Null => overrides
            .null_value
            .as_deref()
            .or(overrides.muted.as_deref()),
        StyleToken::Ipv4 => overrides
            .ipv4
            .as_deref()
            .or(overrides.panel_border.as_deref()),
        StyleToken::Ipv6 => overrides
            .ipv6
            .as_deref()
            .or(overrides.panel_border.as_deref()),
    }
}

pub fn value_style_token(value: &str) -> StyleToken {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return StyleToken::Value;
    }

    match trimmed.to_ascii_lowercase().as_str() {
        "true" => StyleToken::BoolTrue,
        "false" => StyleToken::BoolFalse,
        "null" | "none" | "nil" | "n/a" => StyleToken::Null,
        _ if trimmed.parse::<f64>().is_ok() => StyleToken::ValueNumber,
        _ => StyleToken::Value,
    }
}

pub fn apply_style_spec(text: &str, spec: &str, enabled: bool) -> String {
    if !enabled || text.is_empty() {
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

/// Validates that a style specification uses syntax the renderer understands.
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

fn is_style_modifier(token: &str) -> bool {
    matches!(token, "bold" | "dim" | "dimmed" | "italic" | "underline")
}

fn apply_style_token(style: Style, token: &str) -> Option<Style> {
    match token {
        "bold" => Some(style.bold()),
        "dim" | "dimmed" => Some(style.dimmed()),
        "italic" => Some(style.italic()),
        "underline" => Some(style.underline()),
        _ => parse_color_token(token).map(|color| style.fg(color)),
    }
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
    match value.as_bytes() {
        [b'#', r, g, b] => Some((
            expand_hex_nibble(*r)?,
            expand_hex_nibble(*g)?,
            expand_hex_nibble(*b)?,
        )),
        [b'#', r1, r2, g1, g2, b1, b2] => Some((
            parse_hex_pair(*r1, *r2)?,
            parse_hex_pair(*g1, *g2)?,
            parse_hex_pair(*b1, *b2)?,
        )),
        _ => None,
    }
}

fn expand_hex_nibble(value: u8) -> Option<u8> {
    let nibble = parse_hex_digit(value)?;
    Some((nibble << 4) | nibble)
}

fn parse_hex_pair(high: u8, low: u8) -> Option<u8> {
    Some((parse_hex_digit(high)? << 4) | parse_hex_digit(low)?)
}

fn parse_hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use crate::ui::theme::resolve_theme;

    use super::{
        StyleOverrides, StyleToken, ThemeStyler, apply_style_spec, is_valid_style_spec, style_spec,
        value_style_token,
    };

    #[test]
    fn style_tokens_follow_palette_defaults_and_overrides_unit() {
        let rose = resolve_theme("rose-pine-moon");
        let overrides = StyleOverrides::default();
        assert_eq!(
            style_spec(&rose, &overrides, StyleToken::TextMuted),
            rose.palette.muted
        );
        assert_eq!(
            style_spec(&rose, &overrides, StyleToken::PanelTitle),
            rose.palette.title
        );

        let overridden = StyleOverrides {
            muted: Some("yellow".to_string()),
            panel_title: Some("bold blue".to_string()),
            ..StyleOverrides::default()
        };
        assert_eq!(
            style_spec(&rose, &overridden, StyleToken::TextMuted),
            "yellow"
        );
        assert_eq!(
            style_spec(&rose, &overridden, StyleToken::PanelTitle),
            "bold blue"
        );
    }

    #[test]
    fn value_tokens_cover_booleans_null_numbers_and_text_unit() {
        assert_eq!(value_style_token("true"), StyleToken::BoolTrue);
        assert_eq!(value_style_token("false"), StyleToken::BoolFalse);
        assert_eq!(value_style_token("null"), StyleToken::Null);
        assert_eq!(value_style_token("19.2"), StyleToken::ValueNumber);
        assert_eq!(value_style_token("hello"), StyleToken::Value);
    }

    #[test]
    fn style_helpers_cover_plain_and_colored_paths_unit() {
        let rose = resolve_theme("rose-pine-moon");
        let overrides = StyleOverrides::default();
        let styler = ThemeStyler::new(true, &rose, &overrides);
        let painted = styler.paint("Errors", StyleToken::MessageError);
        assert!(painted.contains("\u{1b}["));
        assert_eq!(apply_style_spec("x", "wat", true), "x");
        assert!(is_valid_style_spec("bold #abcdef"));
        assert!(!is_valid_style_spec("wat ???"));
    }
}
