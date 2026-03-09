#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuoteStyle {
    Single,
    Double,
}

pub fn quote_for_shell(value: &str, style: QuoteStyle) -> String {
    match style {
        QuoteStyle::Single => quote_single(value),
        QuoteStyle::Double => quote_double(value),
    }
}

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
