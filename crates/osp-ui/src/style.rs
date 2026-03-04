use crate::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleToken {
    None,
    PromptText,
    PromptCommand,
    TableHeader,
    MregKey,
    MessageError,
    MessageWarning,
    MessageSuccess,
    MessageInfo,
    MessageTrace,
}

pub fn apply_style(text: &str, token: StyleToken, color: bool, theme_name: &str) -> String {
    if !color || matches!(token, StyleToken::None) {
        return text.to_string();
    }

    let spec = style_spec_for_token(token, theme_name);
    apply_style_spec(text, spec, color)
}

pub fn apply_style_spec(text: &str, spec: &str, color: bool) -> String {
    if !color {
        return text.to_string();
    }
    let Some(prefix) = style_prefix(spec) else {
        return text.to_string();
    };
    format!("{prefix}{text}\x1b[0m")
}

fn style_spec_for_token(token: StyleToken, theme_name: &str) -> &'static str {
    let theme = theme::resolve_theme(theme_name);
    match token {
        StyleToken::None => "",
        StyleToken::PromptText => theme.palette.text,
        StyleToken::PromptCommand => theme.palette.success,
        StyleToken::TableHeader => theme.palette.accent,
        StyleToken::MregKey => theme.palette.accent,
        StyleToken::MessageError => theme.palette.error,
        StyleToken::MessageWarning => theme.palette.warning,
        StyleToken::MessageSuccess => theme.palette.success,
        StyleToken::MessageInfo => theme.palette.info,
        StyleToken::MessageTrace => theme.palette.border,
    }
}

fn style_prefix(spec: &str) -> Option<String> {
    let mut codes: Vec<String> = Vec::new();

    for raw in spec.split_whitespace() {
        let token = raw.trim().to_ascii_lowercase();
        match token.as_str() {
            "bold" => codes.push("1".to_string()),
            "dim" => codes.push("2".to_string()),
            "italic" => codes.push("3".to_string()),
            "underline" => codes.push("4".to_string()),
            "black" => codes.push("30".to_string()),
            "red" => codes.push("31".to_string()),
            "green" => codes.push("32".to_string()),
            "yellow" => codes.push("33".to_string()),
            "blue" => codes.push("34".to_string()),
            "magenta" => codes.push("35".to_string()),
            "cyan" => codes.push("36".to_string()),
            "white" => codes.push("37".to_string()),
            "bright-black" => codes.push("90".to_string()),
            "bright-red" => codes.push("91".to_string()),
            "bright-green" => codes.push("92".to_string()),
            "bright-yellow" => codes.push("93".to_string()),
            "bright-blue" => codes.push("94".to_string()),
            "bright-magenta" => codes.push("95".to_string()),
            "bright-cyan" => codes.push("96".to_string()),
            "bright-white" => codes.push("97".to_string()),
            _ => {
                if let Some((r, g, b)) = parse_hex_rgb(&token) {
                    codes.push(format!("38;2;{r};{g};{b}"));
                }
            }
        }
    }

    if codes.is_empty() {
        None
    } else {
        Some(format!("\x1b[{}m", codes.join(";")))
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
    use super::{StyleToken, apply_style};

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
    fn color_toggle_off_returns_plain_text() {
        let out = apply_style("warn", StyleToken::MessageWarning, false, "nord");
        assert_eq!(out, "warn");
    }
}
