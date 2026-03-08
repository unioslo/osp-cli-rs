use super::key_spec::{ExactMode, KeySpec};

/// Scope modifiers for quick-search stages.
///
/// Bare quick search is "key or value". `K` and `V` are explicit narrowers,
/// not separate pipeline concepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickScope {
    KeyOrValue,
    KeyOnly,
    ValueOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickSpec {
    pub scope: QuickScope,
    pub key_spec: KeySpec,
    pub key_not_equals: bool,
}

/// Parse the prefix operators used by the quick-search mini-language.
///
/// This parser is intentionally order-sensitive and loops until it reaches the
/// actual token. That keeps odd-but-supported forms like `!?field` readable in
/// one place instead of scattering prefix handling across the matcher code.
pub fn parse_quick_spec(input: &str) -> QuickSpec {
    let mut remaining = input.trim();
    let mut scope: Option<QuickScope> = None;
    let mut negated = false;
    let mut existence = false;
    let mut exact = ExactMode::None;
    let mut strict_ambiguous = false;

    loop {
        if let Some(rest) = remaining.strip_prefix("!=") {
            negated = true;
            exact = ExactMode::CaseInsensitive;
            remaining = rest.trim_start();
            continue;
        }
        if let Some(rest) = remaining.strip_prefix("==") {
            exact = ExactMode::CaseSensitive;
            strict_ambiguous = true;
            remaining = rest.trim_start();
            continue;
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
        if let Some(rest) = remaining.strip_prefix('=') {
            exact = ExactMode::CaseInsensitive;
            remaining = rest.trim_start();
            continue;
        }
        if scope.is_none() {
            let mut chars = remaining.chars();
            let Some(first) = chars.next() else {
                break;
            };
            let scope_candidate = match first {
                'K' | 'k' => Some(QuickScope::KeyOnly),
                'V' | 'v' => Some(QuickScope::ValueOnly),
                _ => None,
            };
            if let Some(scope_value) = scope_candidate {
                scope = Some(scope_value);
                let rest = &remaining[first.len_utf8()..];
                remaining = rest.trim_start();
                continue;
            }
        }
        break;
    }

    let scope = scope.unwrap_or(QuickScope::KeyOrValue);
    let mut key_not_equals = false;
    if matches!(scope, QuickScope::KeyOnly) && negated && exact == ExactMode::CaseInsensitive {
        key_not_equals = true;
        negated = false;
    }

    let key_spec = KeySpec {
        token: remaining.trim().to_string(),
        negated,
        existence,
        exact,
        strict_ambiguous,
    };

    QuickSpec {
        scope,
        key_spec,
        key_not_equals,
    }
}

#[cfg(test)]
mod tests {
    use super::{QuickScope, parse_quick_spec};

    #[test]
    fn parses_key_scope() {
        let parsed = parse_quick_spec("K uid");
        assert_eq!(parsed.scope, QuickScope::KeyOnly);
        assert_eq!(parsed.key_spec.token, "uid");
    }

    #[test]
    fn parses_value_scope() {
        let parsed = parse_quick_spec("V oistes");
        assert_eq!(parsed.scope, QuickScope::ValueOnly);
        assert_eq!(parsed.key_spec.token, "oistes");
    }

    #[test]
    fn parses_key_not_equals_form() {
        let parsed = parse_quick_spec("K !=uid");
        assert!(parsed.key_not_equals);
        assert_eq!(parsed.key_spec.token, "uid");
    }
}
