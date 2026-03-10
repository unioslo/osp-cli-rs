#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExactMode {
    None,
    CaseInsensitive,
    CaseSensitive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySpec {
    pub token: String,
    pub negated: bool,
    pub existence: bool,
    pub exact: ExactMode,
    pub strict_ambiguous: bool,
}

impl KeySpec {
    /// Parses key-spec prefixes such as `!`, `?`, `=` and `==`.
    pub fn parse(input: &str) -> Self {
        let mut remaining = input.trim();
        let mut negated = false;
        let mut existence = false;
        let mut exact = ExactMode::None;
        let mut strict_ambiguous = false;

        loop {
            if remaining.starts_with("!=") {
                break;
            }
            if let Some(rest) = remaining.strip_prefix('!') {
                negated = !negated;
                remaining = rest.trim_start();
                continue;
            }
            if let Some(rest) = remaining.strip_prefix('?') {
                existence = true;
                remaining = rest.trim_start();
                continue;
            }
            if let Some(rest) = remaining.strip_prefix("==") {
                exact = ExactMode::CaseSensitive;
                strict_ambiguous = true;
                remaining = rest.trim_start();
                continue;
            }
            if let Some(rest) = remaining.strip_prefix('=') {
                exact = ExactMode::CaseInsensitive;
                remaining = rest.trim_start();
                continue;
            }
            break;
        }

        Self {
            token: remaining.to_string(),
            negated,
            existence,
            exact,
            strict_ambiguous,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ExactMode, KeySpec};

    #[test]
    fn parses_case_insensitive_exact() {
        let spec = KeySpec::parse("=uid");
        assert_eq!(spec.token, "uid");
        assert_eq!(spec.exact, ExactMode::CaseInsensitive);
        assert!(!spec.strict_ambiguous);
    }

    #[test]
    fn parses_case_sensitive_exact_with_strict() {
        let spec = KeySpec::parse("==uid");
        assert_eq!(spec.token, "uid");
        assert_eq!(spec.exact, ExactMode::CaseSensitive);
        assert!(spec.strict_ambiguous);
    }

    #[test]
    fn parses_negated_existence() {
        let spec = KeySpec::parse("!?uid");
        assert_eq!(spec.token, "uid");
        assert!(spec.negated);
        assert!(spec.existence);
    }

    #[test]
    fn does_not_treat_bang_equal_as_prefix() {
        let spec = KeySpec::parse("!=uid");
        assert_eq!(spec.token, "!=uid");
        assert!(!spec.negated);
    }
}
