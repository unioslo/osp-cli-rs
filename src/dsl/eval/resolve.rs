//! Addressed resolution is the semantic contract behind path-shaped selectors.
//!
//! Keep this split stable:
//! - bare `name` means permissive descendant matching
//! - dotted/indexed syntax like `metadata.owner` or `items[0].name` means
//!   strict path traversal
//!
//! Example: on `{"a.b": 1, "a": {"b": 2}}`, selector `a.b` means the nested
//! path `{"a": {"b": 2}}`. It must not silently fall back to the literal
//! dotted key.

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
        path::{
            PathExpression, Selector, is_structural_path_token, parse_path,
            requires_materialization,
        },
    },
};

const SPARSE_HOLE_SENTINEL: &str = "\u{0}__osp_sparse_hole__";

/// One concrete step in an addressed JSON path match.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AddressStep {
    Field(String),
    Index(usize),
}

/// A path match with both logical address and rendered flat-key label.
#[derive(Debug, Clone, PartialEq)]
pub struct AddressedValue {
    pub address: Vec<AddressStep>,
    pub flat_key: String,
    pub value: Value,
}

/// Resolves all values addressed by `token` from `row`.
///
/// Structural path tokens use direct path traversal only. Bare tokens still
/// fall back to flattened-key matching so permissive descendant semantics
/// remain available for simple names.
pub fn resolve_values(row: &Row, token: &str, exact: ExactMode) -> Vec<Value> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    if let Ok(path) = parse_path(trimmed) {
        let nested = Value::Object(row.clone());
        let direct = evaluate_path(&nested, &path);
        if is_structural_path_token(trimmed, &path) || !direct.is_empty() {
            return dedup_values(direct);
        }
    }

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
    enumerate_path_matches(root, path)
        .into_iter()
        .map(|entry| (entry.flat_key, entry.value))
        .collect()
}

/// Resolves addressed path matches using direct path traversal only.
pub fn resolve_path_matches(root: &Value, token: &str, exact: ExactMode) -> Vec<AddressedValue> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    if let Ok(path) = parse_path(trimmed) {
        let _ = exact;
        return dedup_addressed_values(enumerate_path_matches(root, &path));
    }

    Vec::new()
}

/// Resolves addressed matches for permissive descendant selectors.
///
/// Bare tokens still get one exact direct-path pass first so top-level fields
/// like `usage` keep their simple behavior. If that misses, the resolver falls
/// back to flattened descendant matching over addressed leaf values.
pub fn resolve_descendant_matches(
    root: &Value,
    token: &str,
    exact: ExactMode,
) -> Vec<AddressedValue> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    if let Ok(path) = parse_path(trimmed) {
        let direct = enumerate_path_matches(root, &path);
        if !direct.is_empty() {
            return dedup_addressed_values(direct);
        }
    }

    let addressed = enumerate_flattened_addressed_values(root);
    let flattened = addressed
        .iter()
        .map(|entry| (entry.flat_key.clone(), entry.value.clone()))
        .collect::<Row>();
    let matched = match_row_keys(&flattened, trimmed, exact);
    dedup_addressed_values(
        matched
            .into_iter()
            .flat_map(|key| {
                addressed
                    .iter()
                    .filter(move |entry| entry.flat_key == key)
                    .cloned()
            })
            .collect(),
    )
}

/// Evaluates `path` and returns addressed matches that can rebuild structure.
pub fn enumerate_path_matches(root: &Value, path: &PathExpression) -> Vec<AddressedValue> {
    if path.segments.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    traverse_path(root, path, 0, String::new(), Vec::new(), &mut out);
    out
}

/// Rebuilds a minimal JSON subtree from addressed path matches.
///
/// Array holes introduced by index-based selection are represented with an
/// internal sparse-hole sentinel so callers can preserve real selected `null`
/// values. Call [`compact_sparse_arrays`] after any intermediate envelope
/// restoration/merge work to strip those holes from the final payload.
pub fn materialize_path_matches(matches: &[AddressedValue]) -> Value {
    let mut out = Value::Null;
    for entry in matches {
        insert_addressed_value(&mut out, &entry.address, entry.value.clone());
    }
    out
}

/// Returns whether `path` selects exact addressed descendants only.
///
/// This is the safe subset for structure-native transforms that can rebuild a
/// precise subtree from matched addresses without reinterpreting fanout or
/// slice semantics.
#[cfg(test)]
pub fn is_exact_address_path(path: &PathExpression) -> bool {
    let has_selectors = path
        .segments
        .iter()
        .any(|segment| !segment.selectors.is_empty());
    !path.segments.is_empty()
        && (path.absolute || has_selectors)
        && path.segments.iter().all(|segment| {
            segment
                .selectors
                .iter()
                .all(|selector| matches!(selector, Selector::Index(index) if *index >= 0))
        })
}

pub(crate) fn is_sparse_hole(value: &Value) -> bool {
    matches!(
        value,
        Value::Object(map)
            if map.len() == 1
                && map.get(SPARSE_HOLE_SENTINEL) == Some(&Value::Bool(true))
    )
}

pub(crate) fn sparse_hole() -> Value {
    let mut marker = serde_json::Map::new();
    marker.insert(SPARSE_HOLE_SENTINEL.to_string(), Value::Bool(true));
    Value::Object(marker)
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
        if is_structural_path_token(trimmed, &expr) || materialized || !pairs.is_empty() {
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

fn dedup_addressed_values(values: Vec<AddressedValue>) -> Vec<AddressedValue> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();

    for value in values {
        if seen.insert(address_key(&value.address)) {
            out.push(value);
        }
    }

    out
}

fn address_key(address: &[AddressStep]) -> Vec<AddressStep> {
    address.to_vec()
}

fn traverse_path(
    root: &Value,
    path: &PathExpression,
    segment_index: usize,
    flat_key: String,
    address: Vec<AddressStep>,
    out: &mut Vec<AddressedValue>,
) {
    if segment_index == path.segments.len() {
        out.push(AddressedValue {
            address,
            flat_key,
            value: root.clone(),
        });
        return;
    }

    let segment = &path.segments[segment_index];
    let mut current: Vec<(Value, String, Vec<AddressStep>)> =
        vec![(root.clone(), flat_key, address)];

    if let Some(name) = &segment.name {
        let mut next = Vec::new();
        for (value, key, address) in current {
            if let Value::Object(map) = value
                && let Some(child) = map.get(name)
            {
                let mut next_address = address;
                next_address.push(AddressStep::Field(name.clone()));
                next.push((child.clone(), append_name(&key, name), next_address));
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

    for (value, key, address) in current {
        traverse_path(&value, path, segment_index + 1, key, address, out);
    }
}

fn apply_selector_pairs(
    values: Vec<(Value, String, Vec<AddressStep>)>,
    selector: &Selector,
) -> Vec<(Value, String, Vec<AddressStep>)> {
    let mut out = Vec::new();
    for (value, key, address) in values {
        let items = match value {
            Value::Array(items) => items,
            _ => Vec::new(),
        };
        let len = items.len() as i64;
        match selector {
            Selector::Fanout => {
                for (index, item) in items.into_iter().enumerate() {
                    let mut next_address = address.clone();
                    next_address.push(AddressStep::Index(index));
                    out.push((item, append_index(&key, index), next_address));
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
                    let mut next_address = address.clone();
                    next_address.push(AddressStep::Index(idx as usize));
                    out.push((item.clone(), append_index(&key, idx as usize), next_address));
                }
            }
            Selector::Slice { start, stop, step } => {
                let indices = slice_indices(len, *start, *stop, *step);
                for idx in indices {
                    if let Some(item) = items.get(idx as usize) {
                        let mut next_address = address.clone();
                        next_address.push(AddressStep::Index(idx as usize));
                        out.push((item.clone(), append_index(&key, idx as usize), next_address));
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

fn enumerate_flattened_addressed_values(root: &Value) -> Vec<AddressedValue> {
    let mut out = Vec::new();
    match root {
        Value::Object(map) => {
            for (key, value) in map {
                let mut address = vec![AddressStep::Field(key.clone())];
                flatten_addressed_value(Some(key.as_str()), value, &mut address, &mut out);
            }
        }
        Value::Array(items) => {
            for (index, value) in items.iter().enumerate() {
                let mut address = vec![AddressStep::Index(index)];
                let flat_key = format!("[{index}]");
                flatten_addressed_value(Some(flat_key.as_str()), value, &mut address, &mut out);
            }
        }
        _ => {}
    }
    out
}

fn flatten_addressed_value(
    prefix: Option<&str>,
    value: &Value,
    address: &mut Vec<AddressStep>,
    out: &mut Vec<AddressedValue>,
) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let next_prefix = match prefix {
                    Some(parent) => append_name(parent, key),
                    None => key.clone(),
                };
                address.push(AddressStep::Field(key.clone()));
                flatten_addressed_value(Some(next_prefix.as_str()), child, address, out);
                address.pop();
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                let next_prefix = match prefix {
                    Some(parent) => append_index(parent, index),
                    None => format!("[{index}]"),
                };
                address.push(AddressStep::Index(index));
                flatten_addressed_value(Some(next_prefix.as_str()), child, address, out);
                address.pop();
            }
        }
        _ => {
            if let Some(flat_key) = prefix {
                out.push(AddressedValue {
                    address: address.clone(),
                    flat_key: flat_key.to_string(),
                    value: value.clone(),
                });
            }
        }
    }
}

fn insert_addressed_value(target: &mut Value, address: &[AddressStep], value: Value) {
    if address.is_empty() {
        *target = value;
        return;
    }

    match &address[0] {
        AddressStep::Field(name) => {
            if is_sparse_hole(target) || !matches!(target, Value::Object(_)) {
                *target = Value::Object(serde_json::Map::new());
            }
            let Value::Object(map) = target else {
                unreachable!("object ensured above")
            };
            let entry = map.entry(name.clone()).or_insert(Value::Null);
            insert_addressed_value(entry, &address[1..], value);
        }
        AddressStep::Index(index) => {
            if is_sparse_hole(target) || !matches!(target, Value::Array(_)) {
                *target = Value::Array(Vec::new());
            }
            let Value::Array(items) = target else {
                unreachable!("array ensured above")
            };
            if items.len() <= *index {
                items.resize(index + 1, sparse_hole());
            }
            insert_addressed_value(&mut items[*index], &address[1..], value);
        }
    }
}

/// Removes internal sparse-hole sentinels from arrays rebuilt from addresses.
pub fn compact_sparse_arrays(value: &mut Value) {
    match value {
        Value::Array(items) => {
            for item in items.iter_mut() {
                compact_sparse_arrays(item);
            }
            items.retain(|item| !is_sparse_hole(item));
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                compact_sparse_arrays(item);
            }
        }
        _ => {}
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
mod tests;
