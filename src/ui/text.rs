use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub(crate) fn display_width(raw: &str) -> usize {
    let mut width = 0;
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && matches!(chars.peek(), Some('[')) {
            chars.next();
            for next in chars.by_ref() {
                if ('@'..='~').contains(&next) {
                    break;
                }
            }
            continue;
        }
        width += UnicodeWidthChar::width(ch).unwrap_or(0);
    }
    width
}

pub(crate) fn visible_inline_text(value: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = value.chars().collect();
    let mut i = 0usize;

    while i < chars.len() {
        let ch = chars[i];
        if ch == '\\' && i + 1 < chars.len() {
            out.push(chars[i + 1]);
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
                if chars[end..end + fence]
                    .iter()
                    .all(|candidate| *candidate == '`')
                {
                    out.extend(chars[i + fence..end].iter());
                    i = end + fence;
                    break;
                }
                end += 1;
            }
            if i != end + fence {
                out.push(ch);
                i += 1;
            }
            continue;
        }

        if ch == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
            let mut end = i + 2;
            while end + 1 < chars.len() {
                if chars[end] == '*' && chars[end + 1] == '*' {
                    out.extend(chars[i + 2..end].iter());
                    i = end + 2;
                    break;
                }
                end += 1;
            }
            if i != end + 2 {
                out.push(ch);
                i += 1;
            }
            continue;
        }

        if ch == '*' {
            let mut end = i + 1;
            while end < chars.len() {
                if chars[end] == '*' {
                    out.extend(chars[i + 1..end].iter());
                    i = end + 1;
                    break;
                }
                end += 1;
            }
            if i != end + 1 {
                out.push(ch);
                i += 1;
            }
            continue;
        }

        out.push(ch);
        i += 1;
    }

    out
}

pub(crate) fn crop_display_width(raw: &str, width: usize) -> String {
    let mut rendered = String::new();
    let mut rendered_width = 0;
    for ch in raw.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if rendered_width + ch_width > width {
            break;
        }
        rendered.push(ch);
        rendered_width += ch_width;
    }
    rendered
}

pub(crate) fn wrap_display_width(raw: &str, width: usize) -> Vec<String> {
    if raw.is_empty() || width == 0 {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    let mut current = String::new();
    for word in raw.split_whitespace() {
        let word_parts = if UnicodeWidthStr::width(word) > width {
            split_display_width(word, width)
        } else {
            vec![word.to_string()]
        };
        for part in word_parts {
            let separator_width = usize::from(!current.is_empty());
            if UnicodeWidthStr::width(current.as_str())
                + separator_width
                + UnicodeWidthStr::width(part.as_str())
                > width
                && !current.is_empty()
            {
                lines.push(std::mem::take(&mut current));
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(&part);
            if UnicodeWidthStr::width(current.as_str()) == width {
                lines.push(std::mem::take(&mut current));
            }
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn split_display_width(raw: &str, width: usize) -> Vec<String> {
    let mut parts = Vec::new();
    let mut remaining = raw;
    while !remaining.is_empty() {
        let part = crop_display_width(remaining, width);
        if part.is_empty() {
            break;
        }
        let bytes = part.len();
        parts.push(part);
        remaining = &remaining[bytes..];
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::{crop_display_width, display_width, visible_inline_text, wrap_display_width};
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn visible_inline_text_unwraps_escaped_and_balanced_markup_unit() {
        assert_eq!(
            visible_inline_text(r"\*literal\* `code` **bold** *italics* ``two words``"),
            "*literal* code bold italics two words"
        );
    }

    #[test]
    fn visible_inline_text_preserves_unbalanced_markup_unit() {
        assert_eq!(visible_inline_text("keep `broken"), "keep `broken");
        assert_eq!(visible_inline_text("keep **broken"), "keep **broken");
        assert_eq!(visible_inline_text("keep *broken"), "keep *broken");
    }

    #[test]
    fn display_width_helpers_preserve_unicode_and_bound_lines_unit() {
        assert_eq!(display_width("\x1b[31m界\x1b[0m"), 2);
        assert_eq!(crop_display_width("ab🙂cd", 4), "ab🙂");
        let lines = wrap_display_width("alpha βeta 🙂🙂 omega", 7);

        assert_eq!(lines.join(" "), "alpha βeta 🙂🙂 omega");
        assert!(
            lines
                .iter()
                .all(|line| UnicodeWidthStr::width(line.as_str()) <= 7)
        );
    }
}
