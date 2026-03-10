use std::collections::HashSet;

use crate::core::row::Row;
use serde_json::Value;

use crate::dsl::{
    eval::{
        flatten::{coalesce_flat_row, flatten_row},
        matchers::match_row_keys,
    },
    parse::{
        key_spec::ExactMode,
        path::{PathExpression, Selector, parse_path, requires_materialization},
    },
};

/// Resolves all values addressed by `token` from `row`.
///
/// Direct path traversal is attempted first. If that yields no result for a
/// relative path, the resolver falls back to flattened-key matching.
pub fn resolve_values(row: &Row, token: &str, exact: ExactMode) -> Vec<Value> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    if let Ok(path) = parse_path(trimmed) {
        let nested = Value::Object(row.clone());
        // Fast path for nested JSON traversal. Absolute paths trust the path
        // result even when empty; relative paths fall through to flat-key
        // matching when the nested traversal found nothing.
        let direct = evaluate_path(&nested, &path);
        if path.absolute || !direct.is_empty() {
            return dedup_values(direct);
        }
    }

    // Flat-key fallback for dotted keys that already exist as flattened labels.
    let flattened = flatten_row(row);
    let matched = match_row_keys(&flattened, trimmed, exact);
    let values = matched
        .iter()
        .filter_map(|key| flattened.get(*key).cloned())
        .collect::<Vec<_>>();

    dedup_values(values)
}

/// Returns whether any resolved value for `token` is truthy.
pub fn resolve_values_truthy(row: &Row, token: &str, exact: ExactMode) -> bool {
    resolve_values(row, token, exact).iter().any(is_truthy)
}

/// Returns the first resolved scalar value for `token`, flattening one array level.
pub fn resolve_first_value(row: &Row, token: &str, exact: ExactMode) -> Option<Value> {
    let value = resolve_values(row, token, exact).into_iter().next()?;
    match value {
        Value::Array(values) => values.into_iter().next(),
        scalar => Some(scalar),
    }
}

/// Evaluates a parsed path against `root` and returns all matched values.
pub fn evaluate_path(root: &Value, path: &PathExpression) -> Vec<Value> {
    let mut current: Vec<Value> = vec![root.clone()];

    for segment in &path.segments {
        let mut next = Vec::new();

        for node in current {
            let mut values = Vec::new();
            if let Some(name) = &segment.name {
                if let Value::Object(map) = node
                    && let Some(value) = map.get(name)
                {
                    values.push(value.clone());
                }
            } else {
                values.push(node);
            }

            for selector in &segment.selectors {
                values = apply_selector(values, selector);
                if values.is_empty() {
                    break;
                }
            }

            next.extend(values);
        }

        current = next;
        if current.is_empty() {
            break;
        }
    }

    current
}

/// Evaluates `path` and returns matched values together with their flattened keys.
pub fn enumerate_path_values(root: &Value, path: &PathExpression) -> Vec<(String, Value)> {
    if path.segments.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    traverse_path(root, path, 0, String::new(), &mut out);
    out
}

/// Resolves flattened key/value pairs for `token`.
///
/// The boolean return value indicates whether resolving the token required
/// materializing nested structure rather than simple flat-key matching.
pub fn resolve_pairs(flat_row: &Row, token: &str) -> (Vec<(String, Value)>, bool) {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return (Vec::new(), false);
    }

    let expr = parse_path(trimmed).ok();
    if let Some(expr) = expr
        && !expr.segments.is_empty()
    {
        let materialized = requires_materialization(&expr);
        let nested = Value::Object(coalesce_flat_row(flat_row));
        let pairs = enumerate_path_values(&nested, &expr);
        if materialized || !pairs.is_empty() {
            return (pairs, materialized);
        }
    }

    let matched = match_row_keys(flat_row, trimmed, ExactMode::None);
    if !matched.is_empty() {
        let pairs = matched
            .into_iter()
            .filter_map(|key| {
                flat_row
                    .get(key)
                    .cloned()
                    .map(|value| (key.to_string(), value))
            })
            .collect::<Vec<_>>();
        return (pairs, false);
    }

    // Nothing matched at all: materialize the whole flat row so downstream
    // quick-stage rendering can still project something useful.
    let pairs = flat_row
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<Vec<_>>();
    (pairs, true)
}

fn apply_selector(values: Vec<Value>, selector: &Selector) -> Vec<Value> {
    match selector {
        Selector::Fanout => values
            .into_iter()
            .flat_map(|value| match value {
                Value::Array(items) => items,
                _ => Vec::new(),
            })
            .collect(),
        Selector::Index(index) => values
            .into_iter()
            .filter_map(|value| match value {
                Value::Array(items) => {
                    let len = items.len() as i64;
                    let idx = if *index < 0 { len + index } else { *index };
                    if idx >= 0 {
                        items.get(idx as usize).cloned()
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .collect(),
        Selector::Slice { start, stop, step } => values
            .into_iter()
            .flat_map(|value| match value {
                Value::Array(items) => slice_indices(items.len() as i64, *start, *stop, *step)
                    .into_iter()
                    .filter_map(|index| items.get(index as usize).cloned())
                    .collect(),
                _ => Vec::new(),
            })
            .collect(),
    }
}

fn dedup_values(values: Vec<Value>) -> Vec<Value> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();

    for value in values {
        let Ok(key) = serde_json::to_string(&value) else {
            continue;
        };
        if seen.insert(key) {
            out.push(value);
        }
    }

    out
}

fn traverse_path(
    root: &Value,
    path: &PathExpression,
    segment_index: usize,
    flat_key: String,
    out: &mut Vec<(String, Value)>,
) {
    if segment_index == path.segments.len() {
        out.push((flat_key, root.clone()));
        return;
    }

    let segment = &path.segments[segment_index];
    let mut current: Vec<(Value, String)> = vec![(root.clone(), flat_key)];

    if let Some(name) = &segment.name {
        let mut next = Vec::new();
        for (value, key) in current {
            if let Value::Object(map) = value
                && let Some(child) = map.get(name)
            {
                next.push((child.clone(), append_name(&key, name)));
            }
        }
        current = next;
    }

    for selector in &segment.selectors {
        current = apply_selector_pairs(current, selector);
        if current.is_empty() {
            return;
        }
    }

    for (value, key) in current {
        traverse_path(&value, path, segment_index + 1, key, out);
    }
}

fn apply_selector_pairs(values: Vec<(Value, String)>, selector: &Selector) -> Vec<(Value, String)> {
    let mut out = Vec::new();
    for (value, key) in values {
        let items = match value {
            Value::Array(items) => items,
            _ => Vec::new(),
        };
        let len = items.len() as i64;
        match selector {
            Selector::Fanout => {
                for (index, item) in items.into_iter().enumerate() {
                    out.push((item, append_index(&key, index)));
                }
            }
            Selector::Index(index) => {
                let mut idx = *index;
                if idx < 0 {
                    idx += len;
                }
                if idx >= 0
                    && let Some(item) = items.get(idx as usize)
                {
                    out.push((item.clone(), append_index(&key, idx as usize)));
                }
            }
            Selector::Slice { start, stop, step } => {
                let indices = slice_indices(len, *start, *stop, *step);
                for idx in indices {
                    if let Some(item) = items.get(idx as usize) {
                        out.push((item.clone(), append_index(&key, idx as usize)));
                    }
                }
            }
        }
    }
    out
}

fn slice_indices(len: i64, start: Option<i64>, stop: Option<i64>, step: Option<i64>) -> Vec<i64> {
    let step = step.unwrap_or(1);
    if step == 0 {
        return Vec::new();
    }

    let mut out = Vec::new();
    if step > 0 {
        let mut index = start.unwrap_or(0);
        if index < 0 {
            index += len;
        }
        index = index.clamp(0, len);

        let mut stop_index = stop.unwrap_or(len);
        if stop_index < 0 {
            stop_index += len;
        }
        stop_index = stop_index.clamp(0, len);

        while index < stop_index {
            if index >= 0 {
                out.push(index);
            }
            index += step;
        }
    } else {
        let mut index = start.unwrap_or(len - 1);
        if index < 0 {
            index += len;
        }
        index = index.clamp(-1, len - 1);

        let stop_index = match stop {
            Some(stop_value) => {
                let mut normalized = stop_value;
                if normalized < 0 {
                    normalized += len;
                }
                normalized.clamp(-1, len - 1)
            }
            None => -1,
        };

        while index > stop_index {
            if index >= 0 {
                out.push(index);
            }
            index += step;
        }
    }

    out
}

fn append_name(base: &str, name: &str) -> String {
    if base.is_empty() {
        name.to_string()
    } else {
        format!("{base}.{name}")
    }
}

fn append_index(base: &str, index: usize) -> String {
    if base.is_empty() {
        format!("[{index}]")
    } else {
        format!("{base}[{index}]")
    }
}

/// Applies the DSL's truthiness rules to a JSON value.
pub fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|value| value != 0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(values) => !values.is_empty(),
        Value::Object(map) => !map.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::dsl::parse::{key_spec::ExactMode, path::parse_path};

    use super::{
        enumerate_path_values, evaluate_path, is_truthy, resolve_first_value, resolve_pairs,
        resolve_values, slice_indices,
    };

    #[test]
    fn resolve_values_prefers_direct_path_then_fuzzy_fallback() {
        let row = json!({"metadata": {"asset": {"id": 42}}, "id": 7})
            .as_object()
            .cloned()
            .expect("object");

        let values = resolve_values(&row, "asset.id", ExactMode::None);
        assert_eq!(values, vec![json!(42)]);

        let values = resolve_values(&row, ".asset.id", ExactMode::None);
        assert!(values.is_empty());
    }

    #[test]
    fn evaluate_path_handles_fanout_and_slice() {
        let root = json!({"items": [{"id": 1}, {"id": 2}, {"id": 3}]});

        let path = parse_path("items[].id").expect("path should parse");
        assert_eq!(
            evaluate_path(&root, &path),
            vec![json!(1), json!(2), json!(3)]
        );

        let path = parse_path("items[:2].id").expect("path should parse");
        assert_eq!(evaluate_path(&root, &path), vec![json!(1), json!(2)]);

        let path = parse_path("items[::-1].id").expect("path should parse");
        assert_eq!(
            evaluate_path(&root, &path),
            vec![json!(3), json!(2), json!(1)]
        );
    }

    #[test]
    fn slice_indices_handles_forward_and_reverse_ranges_consistently() {
        assert_eq!(slice_indices(5, Some(1), Some(4), Some(1)), vec![1, 2, 3]);
        assert_eq!(slice_indices(5, None, None, Some(-1)), vec![4, 3, 2, 1, 0]);
        assert_eq!(slice_indices(5, Some(-3), None, Some(1)), vec![2, 3, 4]);
        assert_eq!(slice_indices(0, None, None, Some(-1)), Vec::<i64>::new());
        assert_eq!(slice_indices(5, None, None, Some(0)), Vec::<i64>::new());
    }

    #[test]
    fn truthy_rules_match_dsl_expectations() {
        assert!(!is_truthy(&json!(null)));
        assert!(!is_truthy(&json!("")));
        assert!(!is_truthy(&json!([])));
        assert!(is_truthy(&json!("x")));
        assert!(is_truthy(&json!([1])));
    }

    #[test]
    fn resolve_first_value_unwraps_arrays_and_dedups_results() {
        let row = json!({
            "items": [{"id": 7}, {"id": 7}],
            "dup": [1, 1]
        })
        .as_object()
        .cloned()
        .expect("object");

        assert_eq!(
            resolve_first_value(&row, "items[].id", ExactMode::None),
            Some(json!(7))
        );
        assert_eq!(
            resolve_values(&row, "dup[]", ExactMode::None),
            vec![json!(1)]
        );
    }

    #[test]
    fn resolve_pairs_handles_path_flat_fallback_and_full_materialization() {
        let flat = json!({
            "items[0].id": 1,
            "items[1].id": 2,
            "flat.value": "x"
        })
        .as_object()
        .cloned()
        .expect("object");

        let (path_pairs, materialized) = resolve_pairs(&flat, "items[].id");
        assert!(!materialized);
        assert_eq!(
            path_pairs,
            vec![
                ("items[0].id".to_string(), json!(1)),
                ("items[1].id".to_string(), json!(2))
            ]
        );

        let (materialized_pairs, materialized) = resolve_pairs(&flat, "items[-1].id");
        assert!(materialized);
        assert_eq!(
            materialized_pairs,
            vec![("items[1].id".to_string(), json!(2))]
        );

        let (flat_pairs, materialized) = resolve_pairs(&flat, "flat.value");
        assert!(!materialized);
        assert_eq!(flat_pairs, vec![("flat.value".to_string(), json!("x"))]);

        let (fallback_pairs, materialized) = resolve_pairs(&flat, "missing");
        assert!(materialized);
        assert_eq!(fallback_pairs.len(), 3);
    }

    #[test]
    fn enumerate_paths_and_selectors_cover_negative_indexes() {
        let root = json!({"items": [{"id": 1}, {"id": 2}, {"id": 3}]});
        let path = parse_path("items[-1].id").expect("path should parse");
        assert_eq!(evaluate_path(&root, &path), vec![json!(3)]);

        let enumerated = enumerate_path_values(
            &root,
            &parse_path("items[:2].id").expect("path should parse"),
        );
        assert_eq!(
            enumerated,
            vec![
                ("items[0].id".to_string(), json!(1)),
                ("items[1].id".to_string(), json!(2))
            ]
        );
    }
}
