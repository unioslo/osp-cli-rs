use crate::document::{LineBlock, LinePart};
use crate::style::{StyleOverrides, StyleToken, apply_style_with_theme_overrides};
use crate::theme::ThemeDefinition;

pub fn parts_from_inline(text: &str) -> Vec<LinePart> {
    let mut parts: Vec<LinePart> = Vec::new();
    let mut buf = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0usize;

    let flush = |parts: &mut Vec<LinePart>, buf: &mut String| {
        if !buf.is_empty() {
            parts.push(LinePart {
                text: buf.clone(),
                token: None,
            });
            buf.clear();
        }
    };

    while i < chars.len() {
        let ch = chars[i];
        if ch == '\\' {
            if i + 1 < chars.len() {
                buf.push(chars[i + 1]);
                i += 2;
                continue;
            }
        }

        if ch == '`' {
            let fence = if i + 1 < chars.len() && chars[i + 1] == '`' {
                2
            } else {
                1
            };
            let mut end = i + fence;
            while end + fence - 1 < chars.len() {
                if chars[end..end + fence].iter().all(|c| *c == '`') {
                    flush(&mut parts, &mut buf);
                    let content: String = chars[i + fence..end].iter().collect();
                    parts.push(LinePart {
                        text: content,
                        token: Some(StyleToken::Key),
                    });
                    i = end + fence;
                    break;
                }
                end += 1;
            }
            if end + fence - 1 < chars.len() {
                continue;
            }
        }

        if ch == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
            let mut end = i + 2;
            while end + 1 < chars.len() {
                if chars[end] == '*' && chars[end + 1] == '*' {
                    flush(&mut parts, &mut buf);
                    let content: String = chars[i + 2..end].iter().collect();
                    parts.push(LinePart {
                        text: content,
                        token: Some(StyleToken::PanelBorder),
                    });
                    i = end + 2;
                    break;
                }
                end += 1;
            }
            if end + 1 < chars.len() {
                continue;
            }
        }

        if ch == '*' {
            let mut end = i + 1;
            while end < chars.len() {
                if chars[end] == '*' {
                    flush(&mut parts, &mut buf);
                    let content: String = chars[i + 1..end].iter().collect();
                    parts.push(LinePart {
                        text: content,
                        token: Some(StyleToken::Muted),
                    });
                    i = end + 1;
                    break;
                }
                end += 1;
            }
            if end < chars.len() {
                continue;
            }
        }

        buf.push(ch);
        i += 1;
    }

    flush(&mut parts, &mut buf);
    parts
}

pub fn line_from_inline(text: &str) -> LineBlock {
    LineBlock {
        parts: parts_from_inline(text),
    }
}

pub fn render_inline(
    text: &str,
    color: bool,
    theme: &ThemeDefinition,
    overrides: &StyleOverrides,
) -> String {
    let mut out = String::new();
    for part in parts_from_inline(text) {
        if let Some(token) = part.token {
            out.push_str(&apply_style_with_theme_overrides(
                &part.text,
                token,
                color,
                theme,
                overrides,
            ));
        } else {
            out.push_str(&part.text);
        }
    }
    out
}
