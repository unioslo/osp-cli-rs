//! Shared structural helpers for semantic JSON traversal and rebuild.
//!
//! This file is where the document-first path keeps its main rebuild invariant:
//! preserve first, compact later.
//!
//! Example:
//! - `P items[1]` on `{"items": [0, null, 2]}` must preserve the addressed
//!   `null` and return `{"items": [null]}`
//! - sparse rebuild holes are only internal placeholders used while merging
//!   addressed matches and must never leak into user-visible output
//!
//! Selector verbs should reuse these helpers instead of open-coding their own
//! array compaction or envelope-pruning rules.

use anyhow::Result;
use serde_json::{Map, Value};

use crate::core::output_model::{
    Group, OutputItems, output_items_from_value, output_items_to_value,
};
use crate::core::row::Row;
use crate::dsl::eval::resolve::{
    AddressStep, AddressedValue, compact_sparse_arrays, is_sparse_hole, materialize_path_matches,
    sparse_hole,
};

/// Descends into semantic JSON, finds leaf collections, and applies a row/group stage.
///
/// This is the bridge for verbs whose semantics are naturally defined over
/// tabular collections (`Rows` / `Groups`) while the DSL keeps canonical JSON as
/// the payload authority. Structurally empty results are pruned.
///
/// Selector verbs should not add new behavior here. If a verb is selecting or
/// rewriting addressed descendants, route it through `verbs::selector` instead
/// so structural semantics stay shared.
pub(crate) fn traverse_collections<F>(value: Value, stage: F) -> Result<Value>
where
    F: Fn(OutputItems) -> Result<OutputItems> + Copy,
{
    match value {
        Value::Array(items) if is_collection_array(&items) => {
            apply_collection_stage(Value::Array(items), stage)
        }
        Value::Array(items) => Ok(Value::Array(
            items
                .into_iter()
                .filter_map(|item| match traverse_collections(item, stage) {
                    Ok(transformed) if !is_structurally_empty(&transformed) => {
                        Some(Ok(transformed))
                    }
                    Ok(_) => None,
                    Err(err) => Some(Err(err)),
                })
                .collect::<Result<Vec<_>>>()?,
        )),
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, child) in map {
                let transformed = traverse_collections(child, stage)?;
                if !is_structurally_empty(&transformed) {
                    out.insert(key, transformed);
                }
            }
            Ok(Value::Object(out))
        }
        scalar => Ok(scalar),
    }
}

pub(crate) fn apply_collection_stage<F>(value: Value, stage: F) -> Result<Value>
where
    F: Fn(OutputItems) -> Result<OutputItems>,
{
    match value {
        Value::Array(items) => {
            let transformed = stage(output_items_from_value(Value::Array(items)))?;
            Ok(collection_items_to_value(transformed))
        }
        other => Ok(output_items_to_value(&stage(output_items_from_value(
            other,
        ))?)),
    }
}

pub(crate) fn clean_value(value: Value) -> Option<Value> {
    match value {
        Value::Null => None,
        Value::String(text) if text.is_empty() => None,
        Value::Array(items) => {
            let cleaned = items
                .into_iter()
                .filter_map(clean_value)
                .collect::<Vec<_>>();
            if cleaned.is_empty() {
                None
            } else {
                Some(Value::Array(cleaned))
            }
        }
        Value::Object(map) => {
            let cleaned = map
                .into_iter()
                .filter_map(|(key, value)| clean_value(value).map(|value| (key, value)))
                .collect::<Map<_, _>>();
            if cleaned.is_empty() {
                None
            } else {
                Some(Value::Object(cleaned))
            }
        }
        scalar => Some(scalar),
    }
}

pub(crate) fn is_collection_array(items: &[Value]) -> bool {
    !items.is_empty()
        && (items.iter().all(|item| item.is_object()) || items.iter().all(is_group_value))
}

pub(crate) fn is_group_value(value: &Value) -> bool {
    let Value::Object(map) = value else {
        return false;
    };
    map.get("groups").is_some() && map.get("aggregates").is_some() && map.get("rows").is_some()
}

pub(crate) fn is_leaf_record_map(map: &Map<String, Value>) -> bool {
    !map.is_empty() && map.values().all(is_scalar_or_scalar_array)
}

pub(crate) fn is_scalar_or_scalar_array(value: &Value) -> bool {
    match value {
        Value::Array(items) => items.iter().all(is_scalar_like),
        other => is_scalar_like(other),
    }
}

pub(crate) fn is_scalar_like(value: &Value) -> bool {
    matches!(
        value,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
    )
}

pub(crate) fn is_structurally_empty(value: &Value) -> bool {
    match value {
        value if is_sparse_hole(value) => true,
        Value::Null => true,
        Value::Array(items) => items.is_empty(),
        Value::Object(map) => map.is_empty(),
        _ => false,
    }
}

/// Reattaches stable envelope metadata from `original` onto `narrowed`.
///
/// This is used when a stage first narrows a nested row via flat-path logic
/// and then needs to recover the surrounding object shell. Only stable
/// envelope fields such as titles, notes, and scalar-only metadata are
/// reattached, and only when the narrowed object still contains a nested
/// descendant. That avoids turning a scalar-only projection back into the full
/// original object.
pub(crate) fn preserve_envelope_fields(original: Value, narrowed: Value) -> Value {
    match (original, narrowed) {
        (Value::Object(original_map), Value::Object(narrowed_map)) => {
            preserve_object_envelope(original_map, narrowed_map)
        }
        (Value::Array(original_items), Value::Array(narrowed_items)) => {
            preserve_array_envelope(original_items, narrowed_items)
        }
        (_, narrowed) => narrowed,
    }
}

/// Materializes addressed matches after transforming each matched leaf.
///
/// This is the semantic counterpart to row projection. Callers supply the
/// canonical root plus addressed matches, and this helper rebuilds only the
/// matched branches while preserving envelope metadata on the intermediate
/// containers that still survive.
pub(crate) fn materialize_addressed_transform<F>(
    original: &Value,
    matches: &[AddressedValue],
    preserve_terminal_parent_envelope: bool,
    transform: F,
) -> Value
where
    F: Fn(&Value) -> Value + Copy,
{
    let mut projected = Value::Null;

    for entry in matches {
        insert_transformed_match(&mut projected, &entry.address, transform(&entry.value));
        preserve_intermediate_envelope(
            original,
            &mut projected,
            &entry.address,
            preserve_terminal_parent_envelope,
        );
    }

    projected
}

/// Finalizes a structurally rebuilt subtree so selector verbs preserve useful
/// outer metadata while stripping internal sparse-array holes before returning.
///
/// The preserve-then-compact ordering is load-bearing: envelope restoration can
/// reintroduce sparse holes while aligning surviving descendants with their
/// original array positions, so compaction must happen after preservation.
pub(crate) fn finalize_structural_projection(original: &Value, projected: Value) -> Value {
    let mut projected = preserve_envelope_fields(original.clone(), projected);
    compact_sparse_arrays(&mut projected);
    projected
}

/// Rebuilds exactly the addressed matches and restores envelope metadata
/// without compacting sparse array holes yet.
///
/// Semantic `P` needs this intermediate form so mixed keepers/droppers can
/// still address original array positions before the final compact pass.
pub(crate) fn project_addressed_matches_unfinalized(
    original: &Value,
    matches: &[AddressedValue],
) -> Value {
    preserve_envelope_fields(original.clone(), materialize_path_matches(matches))
}

/// Rebuilds exactly the addressed matches and restores the surviving envelope.
pub(crate) fn project_addressed_matches(original: &Value, matches: &[AddressedValue]) -> Value {
    finalize_structural_projection(original, materialize_path_matches(matches))
}

/// Materializes transformed addressed leaves and finalizes the rebuilt tree.
pub(crate) fn transform_addressed_matches<F>(
    original: &Value,
    matches: &[AddressedValue],
    preserve_terminal_parent_envelope: bool,
    transform: F,
) -> Value
where
    F: Fn(&Value) -> Value + Copy,
{
    let projected = materialize_addressed_transform(
        original,
        matches,
        preserve_terminal_parent_envelope,
        transform,
    );
    let mut projected = projected;
    compact_sparse_arrays(&mut projected);
    projected
}

/// Removes addressed descendants while preserving real selected `null` values
/// and pruning only explicit structural deletions afterward.
pub(crate) fn remove_addressed_matches(mut root: Value, matches: &[AddressedValue]) -> Value {
    if matches.is_empty() {
        return root;
    }

    let mut addresses = matches
        .iter()
        .map(|entry| entry.address.clone())
        .collect::<Vec<_>>();
    addresses.sort_by(|left, right| compare_remove_addresses(right, left));

    for address in addresses {
        remove_address(&mut root, &address);
    }

    root
}

pub(crate) fn is_envelope_scalar(value: &Value) -> bool {
    matches!(
        value,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
    )
}

pub(crate) fn is_envelope_field(value: &Value) -> bool {
    is_envelope_scalar(value)
        || matches!(value, Value::Object(map) if is_leaf_record_map(map))
        || matches!(value, Value::Array(items) if items.iter().all(is_scalar_like))
}

fn collection_items_to_value(items: OutputItems) -> Value {
    match items {
        // Collection-aware semantic stages must preserve array shape even when
        // a narrowing transform leaves only one element. Collapsing a
        // singleton collection into an object destroys semantic contracts like
        // `commands: [entry]` and prevents restore into guide/help payloads.
        OutputItems::Rows(rows) => {
            Value::Array(rows.into_iter().map(Value::Object).collect::<Vec<_>>())
        }
        OutputItems::Groups(groups) => {
            Value::Array(groups.into_iter().map(group_to_value).collect::<Vec<_>>())
        }
    }
}

fn group_to_value(group: Group) -> Value {
    let mut item = Row::new();
    item.insert("groups".to_string(), Value::Object(group.groups));
    item.insert("aggregates".to_string(), Value::Object(group.aggregates));
    item.insert(
        "rows".to_string(),
        Value::Array(
            group
                .rows
                .into_iter()
                .map(Value::Object)
                .collect::<Vec<_>>(),
        ),
    );
    Value::Object(item)
}

pub(crate) fn compare_scalar_values(left: &Value, right: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    match (left, right) {
        (Value::Number(left), Value::Number(right)) => {
            let left = left.as_f64().unwrap_or(0.0);
            let right = right.as_f64().unwrap_or(0.0);
            left.partial_cmp(&right).unwrap_or(Ordering::Equal)
        }
        _ => render_value(left).cmp(&render_value(right)),
    }
}

pub(crate) fn render_value(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => value.clone(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}

fn preserve_object_envelope(
    original_map: Map<String, Value>,
    mut narrowed_map: Map<String, Value>,
) -> Value {
    for (key, original_child) in original_map {
        if let Some(narrowed_child) = narrowed_map.remove(&key) {
            let preserved = preserve_envelope_fields(original_child, narrowed_child);
            narrowed_map.insert(key, preserved);
            continue;
        }

        if object_has_nested_descendants(&narrowed_map) && is_envelope_field(&original_child) {
            narrowed_map.insert(key, original_child);
        }
    }

    Value::Object(narrowed_map)
}

fn preserve_array_envelope(original_items: Vec<Value>, narrowed_items: Vec<Value>) -> Value {
    let width = original_items.len().max(narrowed_items.len());
    let mut out = Vec::with_capacity(width);

    for index in 0..width {
        let original = original_items.get(index).cloned().unwrap_or(Value::Null);
        match narrowed_items.get(index) {
            None => out.push(sparse_hole()),
            Some(narrowed) if is_sparse_hole(narrowed) => out.push(sparse_hole()),
            Some(narrowed) => {
                out.push(preserve_envelope_fields(original, narrowed.clone()));
            }
        }
    }

    Value::Array(out)
}

fn insert_transformed_match(target: &mut Value, address: &[AddressStep], value: Value) {
    let Some((step, rest)) = address.split_first() else {
        *target = value;
        return;
    };

    match step {
        AddressStep::Field(name) => {
            if is_sparse_hole(target) || !matches!(target, Value::Object(_)) {
                *target = Value::Object(Map::new());
            }
            let Value::Object(map) = target else {
                return;
            };
            insert_transformed_match(map.entry(name.clone()).or_insert(Value::Null), rest, value);
        }
        AddressStep::Index(index) => {
            if is_sparse_hole(target) || !matches!(target, Value::Array(_)) {
                *target = Value::Array(Vec::new());
            }
            let Value::Array(items) = target else {
                return;
            };
            if items.len() <= *index {
                items.resize(index + 1, sparse_hole());
            }
            insert_transformed_match(&mut items[*index], rest, value);
        }
    }
}

fn preserve_intermediate_envelope(
    original: &Value,
    projected: &mut Value,
    address: &[AddressStep],
    preserve_terminal_parent_envelope: bool,
) {
    let Some((step, rest)) = address.split_first() else {
        return;
    };

    let (next_original, next_projected) = match step {
        AddressStep::Field(name) => {
            let (Value::Object(original_map), Value::Object(projected_map)) = (original, projected)
            else {
                return;
            };
            let Some(next_original) = original_map.get(name) else {
                return;
            };
            let Some(next_projected) = projected_map.get_mut(name) else {
                return;
            };
            (next_original, next_projected)
        }
        AddressStep::Index(index) => {
            let (Value::Array(original_items), Value::Array(projected_items)) =
                (original, projected)
            else {
                return;
            };
            let Some(next_original) = original_items.get(*index) else {
                return;
            };
            let Some(next_projected) = projected_items.get_mut(*index) else {
                return;
            };
            (next_original, next_projected)
        }
    };

    if rest.is_empty() {
        return;
    }

    if rest.len() == 1 && !preserve_terminal_parent_envelope {
        preserve_intermediate_envelope(
            next_original,
            next_projected,
            rest,
            preserve_terminal_parent_envelope,
        );
        return;
    }

    if let Value::Object(original_map) = next_original
        && let Value::Object(projected_map) = next_projected
    {
        for (key, value) in original_map {
            if !projected_map.contains_key(key) && is_envelope_field(value) {
                projected_map.insert(key.clone(), value.clone());
            }
        }
    }

    preserve_intermediate_envelope(
        next_original,
        next_projected,
        rest,
        preserve_terminal_parent_envelope,
    );
}

fn object_has_nested_descendants(map: &Map<String, Value>) -> bool {
    map.values().any(value_has_nested_descendants)
}

fn value_has_nested_descendants(value: &Value) -> bool {
    match value {
        Value::Object(map) => {
            !map.is_empty()
                && (!is_leaf_record_map(map) || map.values().any(value_has_nested_descendants))
        }
        Value::Array(items) => items.iter().any(|item| !is_structurally_empty(item)),
        _ => false,
    }
}

fn remove_address(target: &mut Value, address: &[AddressStep]) -> bool {
    let Some((step, rest)) = address.split_first() else {
        return true;
    };

    let mut changed = false;
    match step {
        AddressStep::Field(name) => {
            if let Value::Object(map) = target
                && let Some(child) = map.get_mut(name)
            {
                let remove_child = remove_address(child, rest);
                changed = true;
                if remove_child {
                    map.remove(name);
                }
            }
        }
        AddressStep::Index(index) => {
            if let Value::Array(items) = target
                && let Some(child) = items.get_mut(*index)
            {
                let remove_child = remove_address(child, rest);
                changed = true;
                if remove_child {
                    items.remove(*index);
                }
            }
        }
    }

    changed && is_structurally_empty(target)
}

fn compare_remove_addresses(left: &[AddressStep], right: &[AddressStep]) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    for (left_step, right_step) in left.iter().zip(right.iter()) {
        let ordering = match (left_step, right_step) {
            (AddressStep::Field(left), AddressStep::Field(right)) => left.cmp(right),
            (AddressStep::Index(left), AddressStep::Index(right)) => left.cmp(right),
            (AddressStep::Field(_), AddressStep::Index(_)) => Ordering::Less,
            (AddressStep::Index(_), AddressStep::Field(_)) => Ordering::Greater,
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }

    left.len().cmp(&right.len())
}
