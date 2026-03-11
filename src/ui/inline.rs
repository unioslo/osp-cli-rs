use crate::ui::document::{LineBlock, LinePart};
use crate::ui::style::{StyleOverrides, StyleToken, apply_style_with_theme_overrides};
use crate::ui::theme::ThemeDefinition;

/// Parses lightweight inline markup into styled line parts.
///
/// Recognizes backtick-delimited code, single-asterisk muted text, and
/// double-asterisk emphasized text. Escaped marker characters are preserved.
///
/// # Examples
///
/// ```
/// use osp_cli::ui::{StyleToken, parts_from_inline};
///
/// let parts = parts_from_inline("Use `uid` and *optional* **flags**");
///
/// assert_eq!(parts[0].text, "Use ");
/// assert_eq!(parts[1].token, Some(StyleToken::Key));
/// assert_eq!(parts[1].text, "uid");
/// assert_eq!(parts[3].token, Some(StyleToken::Muted));
/// assert_eq!(parts[5].token, Some(StyleToken::PanelBorder));
/// ```
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
        if ch == '\\' && i + 1 < chars.len() {
            buf.push(chars[i + 1]);
            i += 2;
            continue;
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

/// Parses inline markup and wraps the result in a single [`LineBlock`].
///
/// This keeps the block model aligned with the richer document renderer while
/// still letting callers use the lightweight inline grammar.
///
/// # Examples
///
/// ```
/// use osp_cli::ui::{StyleToken, line_from_inline};
///
/// let line = line_from_inline("`uid`");
///
/// assert_eq!(line.parts.len(), 1);
/// assert_eq!(line.parts[0].token, Some(StyleToken::Key));
/// assert_eq!(line.parts[0].text, "uid");
/// ```
pub fn line_from_inline(text: &str) -> LineBlock {
    LineBlock {
        parts: parts_from_inline(text),
    }
}

/// Renders lightweight inline markup to a styled string.
///
/// Returns plain text when `color` is `false` so callers can reuse the same
/// content path for terminals, copy buffers, and test assertions.
///
/// # Examples
///
/// ```
/// use osp_cli::ui::{StyleOverrides, render_inline, resolve_theme};
///
/// let rendered = render_inline(
///     "Use `uid`",
///     false,
///     &resolve_theme("dracula"),
///     &StyleOverrides::default(),
/// );
///
/// assert_eq!(rendered, "Use uid");
/// ```
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
                &part.text, token, color, theme, overrides,
            ));
        } else {
            out.push_str(&part.text);
        }
    }
    out
}

#[cfg(test)]
mod tests;
