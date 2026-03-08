use osp_core::row::Row;
use std::collections::HashSet;

use crate::parse::key_spec::ExactMode;
use crate::parse::path::{PathExpression, Selector, parse_path};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyMatches {
    pub exact: Vec<String>,
    pub partial: Vec<String>,
}

pub fn match_row_keys<'a>(row: &'a Row, token: &str, exact: ExactMode) -> Vec<&'a str> {
    let matches = compute_key_matches(row, token, exact);
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

pub fn match_row_keys_detailed(row: &Row, token: &str, exact: ExactMode) -> KeyMatches {
    compute_key_matches(row, token, exact)
}

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
                rendered
                    .to_ascii_lowercase()
                    .contains(&query.to_ascii_lowercase())
            }
        }
    }
}

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
                name.to_ascii_lowercase()
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
        current.to_ascii_lowercase()
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
    if key_expr.segments.len() != expr.segments.len() {
        return false;
    }

    for (key_segment, expr_segment) in key_expr.segments.iter().zip(expr.segments.iter()) {
        if !names_match(&key_segment.name, &expr_segment.name, case_sensitive) {
            return false;
        }
        if !selectors_match(&key_segment.selectors, &expr_segment.selectors) {
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
                left.eq_ignore_ascii_case(right)
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

fn compute_key_matches(row: &Row, token: &str, exact: ExactMode) -> KeyMatches {
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
        trimmed.to_ascii_lowercase()
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
            last_segment.to_ascii_lowercase()
        };

        let exact_match = match exact {
            ExactMode::CaseSensitive => last_segment == trimmed,
            ExactMode::CaseInsensitive => last_segment.eq_ignore_ascii_case(trimmed),
            ExactMode::None => last_segment.eq_ignore_ascii_case(trimmed),
        };
        if exact_match {
            exact_keys.push(key.clone());
            continue;
        }

        if allow_partial {
            let key_cmp = if case_sensitive {
                key.clone()
            } else {
                key.to_ascii_lowercase()
            };
            if key_cmp.contains(&token_cmp) || last_cmp.contains(&token_cmp) {
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::parse::key_spec::ExactMode;

    use super::{match_row_keys, match_row_keys_detailed, value_contains};

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
}
