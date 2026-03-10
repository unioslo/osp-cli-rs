use crate::core::fuzzy::{fold_case as unicode_fold_case, search_fuzzy_matcher};
use crate::core::row::Row;
use skim::fuzzy_matcher::FuzzyMatcher;
use std::collections::HashSet;

use crate::dsl::parse::key_spec::ExactMode;
use crate::dsl::parse::path::{PathExpression, Selector, parse_path};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyMatches {
    pub exact: Vec<String>,
    pub partial: Vec<String>,
}

/// Returns matching row keys in input order, preferring exact matches over partial matches.
pub fn match_row_keys<'a>(row: &'a Row, token: &str, exact: ExactMode) -> Vec<&'a str> {
    let matches = compute_key_matches(row, token, exact, false);
    let selected = if !matches.exact.is_empty() {
        matches.exact
    } else {
        matches.partial
    };
    if selected.is_empty() {
        return Vec::new();
    }

    let selected = selected.into_iter().collect::<HashSet<_>>();
    row.keys()
        .filter_map(|key| {
            if selected.contains(key) {
                Some(key.as_str())
            } else {
                None
            }
        })
        .collect()
}

/// Returns exact and partial key matches for `token` without applying preference rules.
pub fn match_row_keys_detailed(row: &Row, token: &str, exact: ExactMode) -> KeyMatches {
    compute_key_matches(row, token, exact, false)
}

/// Returns exact and partial key matches for `token`, allowing opt-in fuzzy
/// fallbacks after literal path/substring matching has failed.
pub fn match_row_keys_detailed_fuzzy(row: &Row, token: &str, exact: ExactMode) -> KeyMatches {
    compute_key_matches(row, token, exact, true)
}

#[cfg(test)]
/// Returns whether `value` contains `query`, recursing into arrays.
pub fn value_contains(value: &serde_json::Value, query: &str, case_sensitive: bool) -> bool {
    match value {
        serde_json::Value::Array(values) => values
            .iter()
            .any(|item| value_contains(item, query, case_sensitive)),
        _ => {
            let rendered = render_value(value);
            if case_sensitive {
                rendered.contains(query)
            } else {
                contains_case_insensitive(&rendered, query)
            }
        }
    }
}

/// Renders a JSON value into the text form used for matching and display.
pub fn render_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(v) => v.to_string(),
        serde_json::Value::Number(v) => v.to_string(),
        serde_json::Value::String(v) => v.clone(),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => value.to_string(),
    }
}

fn expression_segments(expr: &PathExpression, case_sensitive: bool) -> Vec<String> {
    expr.segments
        .iter()
        .filter_map(|segment| segment.name.as_ref())
        .map(|name| {
            if case_sensitive {
                name.clone()
            } else {
                fold_case(name)
            }
        })
        .collect()
}

fn key_segments(key: &str, case_sensitive: bool) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut in_brackets = false;

    for ch in key.chars() {
        match ch {
            '.' if !in_brackets => {
                push_segment(&mut segments, &mut current, case_sensitive);
            }
            '[' => {
                push_segment(&mut segments, &mut current, case_sensitive);
                in_brackets = true;
                current.clear();
            }
            ']' => {
                in_brackets = false;
                current.clear();
            }
            _ => {
                if !in_brackets {
                    current.push(ch);
                }
            }
        }
    }

    push_segment(&mut segments, &mut current, case_sensitive);
    segments
}

fn push_segment(segments: &mut Vec<String>, current: &mut String, case_sensitive: bool) {
    if current.is_empty() {
        return;
    }
    let segment = if case_sensitive {
        current.clone()
    } else {
        fold_case(current)
    };
    segments.push(segment);
    current.clear();
}

fn segments_match(seq: &[String], pattern: &[String], absolute: bool) -> bool {
    if pattern.is_empty() {
        return false;
    }
    if absolute {
        if seq.len() < pattern.len() {
            return false;
        }
        return seq[..pattern.len()] == *pattern;
    }

    // Relative paths use segment-subsequence matching: the pattern must appear
    // in order, but other segments may exist in between.
    let mut pos = 0usize;
    for segment in pattern {
        while pos < seq.len() && &seq[pos] != segment {
            pos += 1;
        }
        if pos == seq.len() {
            return false;
        }
        pos += 1;
    }
    true
}

fn matches_expression_with_selectors(
    key: &str,
    expr: &PathExpression,
    case_sensitive: bool,
) -> bool {
    let Ok(key_expr) = parse_path(key) else {
        return false;
    };
    if expr.absolute {
        if key_expr.segments.len() != expr.segments.len() {
            return false;
        }

        return key_expr.segments.iter().zip(expr.segments.iter()).all(
            |(key_segment, expr_segment)| {
                names_match(&key_segment.name, &expr_segment.name, case_sensitive)
                    && selectors_match(&key_segment.selectors, &expr_segment.selectors)
            },
        );
    }

    let mut position = 0usize;
    for expr_segment in &expr.segments {
        let mut matched = false;
        while position < key_expr.segments.len() {
            let key_segment = &key_expr.segments[position];
            if names_match(&key_segment.name, &expr_segment.name, case_sensitive)
                && selectors_match(&key_segment.selectors, &expr_segment.selectors)
            {
                position += 1;
                matched = true;
                break;
            }
            position += 1;
        }

        if !matched {
            return false;
        }
    }

    true
}

fn names_match(left: &Option<String>, right: &Option<String>, case_sensitive: bool) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => {
            if case_sensitive {
                left == right
            } else {
                eq_case_insensitive(left, right)
            }
        }
        (None, None) => true,
        _ => false,
    }
}

fn selectors_match(keys: &[Selector], exprs: &[Selector]) -> bool {
    if exprs.is_empty() {
        return keys.is_empty();
    }

    let mut key_iter = keys.iter();
    for expr in exprs {
        match expr {
            Selector::Index(target) => match key_iter.next() {
                Some(Selector::Index(actual)) if actual == target => {}
                _ => return false,
            },
            Selector::Fanout => return true,
            Selector::Slice { start, stop, step } => {
                let is_full = start.is_none() && stop.is_none() && step.is_none();
                if is_full {
                    return true;
                }
                return false;
            }
        }
    }

    key_iter.next().is_none()
}

fn compute_key_matches(row: &Row, token: &str, exact: ExactMode, allow_fuzzy: bool) -> KeyMatches {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return KeyMatches::default();
    }

    let expr = parse_path(trimmed)
        .ok()
        .filter(|expr| !expr.segments.is_empty());
    let case_sensitive = matches!(exact, ExactMode::CaseSensitive);
    let allow_partial = matches!(exact, ExactMode::None);
    let expr_has_selectors = expr.as_ref().is_some_and(|expr| {
        expr.segments
            .iter()
            .any(|segment| !segment.selectors.is_empty())
    });
    let expr_segments = expr
        .as_ref()
        .map(|expr| expression_segments(expr, case_sensitive));
    let should_try_plain_label_match = expr.is_none()
        || (!expr.as_ref().is_some_and(|expr| expr.absolute)
            && !expr_has_selectors
            && expr_segments
                .as_ref()
                .map(|segments| segments.len())
                .unwrap_or(0)
                == 1);

    let token_cmp = if case_sensitive {
        trimmed.to_string()
    } else {
        fold_case(trimmed)
    };

    let mut exact_keys = Vec::new();
    let mut partial_keys = Vec::new();

    for key in row.keys() {
        if let Some(expr) = &expr {
            if expr_has_selectors {
                if matches_expression_with_selectors(key, expr, case_sensitive) {
                    exact_keys.push(key.clone());
                    continue;
                }
            } else if let Some(pattern) = &expr_segments {
                let segments = key_segments(key, case_sensitive);
                if segments_match(&segments, pattern, expr.absolute) {
                    exact_keys.push(key.clone());
                    continue;
                }
            }
        }

        if !should_try_plain_label_match {
            continue;
        }

        let segments = key_segments(key, case_sensitive);
        let Some(last_segment) = segments.last() else {
            continue;
        };

        let last_cmp = if case_sensitive {
            last_segment.clone()
        } else {
            fold_case(last_segment)
        };

        let exact_match = match exact {
            ExactMode::CaseSensitive => last_segment == trimmed,
            ExactMode::CaseInsensitive => eq_case_insensitive(last_segment, trimmed),
            ExactMode::None => eq_case_insensitive(last_segment, trimmed),
        };
        if exact_match {
            exact_keys.push(key.clone());
            continue;
        }

        if allow_partial {
            let key_cmp = if case_sensitive {
                key.clone()
            } else {
                fold_case(key)
            };
            if key_cmp.contains(&token_cmp) || last_cmp.contains(&token_cmp) {
                partial_keys.push(key.clone());
                continue;
            }
            if allow_fuzzy
                && (fuzzy_contains_case_insensitive(last_segment, trimmed)
                    || fuzzy_contains_case_insensitive(key, trimmed))
            {
                partial_keys.push(key.clone());
            }
        }
    }

    let mut seen_partial = partial_keys.iter().cloned().collect::<HashSet<_>>();
    for key in &exact_keys {
        if seen_partial.insert(key.clone()) {
            partial_keys.push(key.clone());
        }
    }

    KeyMatches {
        exact: exact_keys,
        partial: partial_keys,
    }
}

/// Compares two strings after Unicode lowercase folding.
pub fn eq_case_insensitive(left: &str, right: &str) -> bool {
    fold_case(left) == fold_case(right)
}

/// Returns whether `haystack` contains `needle` after Unicode lowercase folding.
pub fn contains_case_insensitive(haystack: &str, needle: &str) -> bool {
    fold_case(haystack).contains(&fold_case(needle))
}

/// Lowercases text using Unicode case folding semantics.
pub fn fold_case(text: &str) -> String {
    unicode_fold_case(text)
}

/// Returns whether `candidate` is a close-enough fuzzy match for `query`.
///
/// `%quick` is still a filter, not a ranking system. This helper therefore
/// accepts only clustered fuzzy matches that look typo-like, and leaves loose
/// subsequence hits on the floor.
pub fn fuzzy_contains_case_insensitive(candidate: &str, query: &str) -> bool {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return false;
    }
    if contains_case_insensitive(candidate, trimmed) {
        return true;
    }
    if trimmed.chars().count() < 3 {
        return false;
    }

    let query_lc = fold_case(trimmed);
    fuzzy_variants(candidate).any(|variant| clustered_fuzzy_match(variant, &query_lc))
}

fn fuzzy_variants(candidate: &str) -> impl Iterator<Item = &str> {
    std::iter::once(candidate).chain(wordish_fragments(candidate))
}

fn wordish_fragments(candidate: &str) -> impl Iterator<Item = &str> {
    candidate
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(move |fragment| !fragment.is_empty() && *fragment != candidate)
}

fn clustered_fuzzy_match(candidate: &str, query_lc: &str) -> bool {
    let candidate_lc = fold_case(candidate);
    let Some((_, indices)) = search_fuzzy_matcher().fuzzy_indices(&candidate_lc, query_lc) else {
        return false;
    };
    let Some(first) = indices.first().copied() else {
        return false;
    };
    let Some(last) = indices.last().copied() else {
        return false;
    };

    let query_len = query_lc.chars().count();
    let span = last.saturating_sub(first) + 1;
    let slack = span.saturating_sub(query_len);

    slack <= query_len / 2 + 2
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::dsl::parse::key_spec::ExactMode;

    use super::{
        fuzzy_contains_case_insensitive, match_row_keys, match_row_keys_detailed,
        match_row_keys_detailed_fuzzy, value_contains,
    };

    #[test]
    fn matches_last_segment_case_insensitive() {
        let row = json!({"ldap.uid": "oistes", "mail": "o@uio.no"})
            .as_object()
            .cloned()
            .expect("object");

        let matched = match_row_keys(&row, "UID", ExactMode::CaseInsensitive);
        assert_eq!(matched, vec!["ldap.uid"]);
    }

    #[test]
    fn matches_subsequence_dotted_paths() {
        let row = json!({
            "metadata.asset.id": 42,
            "asset.id": 7,
            "metadata.owner.id": 9
        })
        .as_object()
        .cloned()
        .expect("object");

        let matched = match_row_keys(&row, "asset.id", ExactMode::None);
        assert_eq!(matched, vec!["metadata.asset.id", "asset.id"]);
    }

    #[test]
    fn absolute_paths_require_prefix_match() {
        let row = json!({
            "metadata.asset.id": 42,
            "asset.id": 7
        })
        .as_object()
        .cloned()
        .expect("object");

        let matched = match_row_keys(&row, ".asset.id", ExactMode::None);
        assert_eq!(matched, vec!["asset.id"]);
    }

    #[test]
    fn selector_paths_match_exact_index() {
        let row = json!({
            "items[0].id": 1,
            "items[1].id": 2
        })
        .as_object()
        .cloned()
        .expect("object");

        let matched = match_row_keys(&row, "items[0].id", ExactMode::None);
        assert_eq!(matched, vec!["items[0].id"]);
    }

    #[test]
    fn relative_selector_paths_match_descendant_subsequences() {
        let row = json!({
            "sections[0].entries[0].name": "help",
            "sections[0].entries[1].name": "exit"
        })
        .as_object()
        .cloned()
        .expect("object");

        let matched = match_row_keys(&row, "entries[0].name", ExactMode::None);
        assert_eq!(matched, vec!["sections[0].entries[0].name"]);
    }

    #[test]
    fn detailed_matching_reports_partial_hits_when_exact_match_is_absent() {
        let row = json!({
            "metadata.asset.id": 42,
            "metadata.asset.name": "vm-01"
        })
        .as_object()
        .cloned()
        .expect("object");

        let matches = match_row_keys_detailed(&row, "nam", ExactMode::None);
        assert!(matches.exact.is_empty());
        assert_eq!(matches.partial, vec!["metadata.asset.name".to_string()]);
    }

    #[test]
    fn value_contains_handles_arrays_and_case_sensitivity() {
        let value = json!(["Alpha", {"name": "Bravo"}]);

        assert!(value_contains(&value, "bravo", false));
        assert!(!value_contains(&value, "bravo", true));
        assert!(value_contains(&value, "Alpha", true));
    }

    #[test]
    fn fuzzy_key_matching_accepts_typo_like_last_segment_hits() {
        let row = json!({
            "commands.name": "doctor",
            "commands.short_help": "Inspect runtime health"
        })
        .as_object()
        .cloned()
        .expect("object");

        let matches = match_row_keys_detailed_fuzzy(&row, "naem", ExactMode::None);
        assert_eq!(matches.partial, vec!["commands.name".to_string()]);
    }

    #[test]
    fn fuzzy_text_matching_rejects_loose_subsequence_noise() {
        assert!(fuzzy_contains_case_insensitive("doctor", "docter"));
        assert!(!fuzzy_contains_case_insensitive(
            "subcommands all config last plugins theme",
            "docter"
        ));
    }
}
