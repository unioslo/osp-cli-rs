/// Quoting style to use when formatting a shell argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteStyle {
    /// Wraps the value in single quotes and escapes embedded single quotes.
    Single,
    /// Wraps the value in double quotes and escapes shell-sensitive characters.
    Double,
}

/// Quotes `value` for shell reuse using the requested quoting style.
///
/// Use [`QuoteStyle::Single`] when you want the most literal shell-safe output,
/// and [`QuoteStyle::Double`] when interpolation-style shell syntax should stay
/// visually recognizable to the user.
///
/// # Examples
///
/// ```
/// use osp_cli::core::shell_words::{QuoteStyle, quote_for_shell};
///
/// assert_eq!(quote_for_shell("O'Brien", QuoteStyle::Single), "'O'\\''Brien'");
/// assert_eq!(quote_for_shell("hello world", QuoteStyle::Double), "\"hello world\"");
/// ```
pub fn quote_for_shell(value: &str, style: QuoteStyle) -> String {
    match style {
        QuoteStyle::Single => quote_single(value),
        QuoteStyle::Double => quote_double(value),
    }
}

/// Escapes shell-sensitive characters without adding surrounding quotes.
///
/// This is useful for tab-completion and history displays where adding full
/// quotes would be noisier than backslash-escaping.
///
/// # Examples
///
/// ```
/// use osp_cli::core::shell_words::escape_for_shell;
///
/// assert_eq!(
///     escape_for_shell("team docs/file name.txt"),
///     "team\\ docs/file\\ name.txt"
/// );
/// ```
pub fn escape_for_shell(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if is_unquoted_safe(ch) {
            out.push(ch);
        } else {
            out.push('\\');
            out.push(ch);
        }
    }
    out
}

fn quote_single(value: &str) -> String {
    let mut out = String::from("'");
    for ch in value.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

fn quote_double(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' | '\\' | '$' | '`' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn is_unquoted_safe(ch: char) -> bool {
    ch.is_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-' | '~' | ':' | '+')
}

#[cfg(test)]
mod tests {
    use super::{QuoteStyle, escape_for_shell, quote_for_shell};

    #[test]
    fn escape_for_shell_backslashes_unsafe_characters() {
        assert_eq!(
            escape_for_shell("team docs/file name.txt"),
            "team\\ docs/file\\ name.txt"
        );
        assert_eq!(escape_for_shell("rød"), "rød");
    }

    #[test]
    fn quote_for_shell_handles_single_quotes() {
        assert_eq!(
            quote_for_shell("O'Brien", QuoteStyle::Single),
            "'O'\\''Brien'"
        );
    }

    #[test]
    fn quote_for_shell_handles_double_quotes() {
        assert_eq!(
            quote_for_shell("a\"b$`c\\d", QuoteStyle::Double),
            "\"a\\\"b\\$\\`c\\\\d\""
        );
    }
}
