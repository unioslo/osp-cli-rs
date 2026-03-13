use crate::core::{output_model::Group, row::Row};
use anyhow::{Result, anyhow};
use serde_json::{Map, Value};

use crate::dsl::{
    eval::resolve::{AddressStep, AddressedValue, enumerate_path_matches},
    parse::path::{PathExpression, parse_path},
    verbs::common::map_group_rows,
};

use super::json;

#[derive(Debug, Clone)]
enum UnrollTarget {
    /// Bare relative field names keep the long-standing "find this descendant
    /// array anywhere under the current semantic object" behavior.
    DescendantField(String),
    /// Parsed path expressions give `U` a precise structural surface instead of
    /// piggybacking on `P field[]`.
    Path(PathExpression),
}

/// Compiled `U` plan.
///
/// `U` is intentionally its own verb instead of a disguised projection. The
/// row executor and the semantic executor both need the same contract:
/// duplicate the nearest owning record/object once per array member, replacing
/// the target array with that single member.
#[derive(Debug, Clone)]
pub(crate) struct UnrollPlan {
    target: UnrollTarget,
}

pub(crate) fn compile(spec: &str) -> Result<UnrollPlan> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("U: missing field name to unroll"));
    }

    let path = parse_path(trimmed)?;
    let target = if !path.absolute {
        match path.segments.as_slice() {
            [segment] if segment.selectors.is_empty() => segment
                .name
                .as_ref()
                .map(|name| UnrollTarget::DescendantField(name.clone()))
                .unwrap_or(UnrollTarget::Path(path)),
            _ => UnrollTarget::Path(path),
        }
    } else {
        UnrollTarget::Path(path)
    };

    Ok(UnrollPlan { target })
}

pub(crate) fn apply_with_plan(rows: Vec<Row>, plan: &UnrollPlan) -> Result<Vec<Row>> {
    let mut out = Vec::new();
    for row in rows {
        out.extend(plan.expand_row(&row)?);
    }
    Ok(out)
}

pub(crate) fn apply_groups_with_plan(groups: Vec<Group>, plan: &UnrollPlan) -> Result<Vec<Group>> {
    let mut out = map_group_rows(groups, |rows| apply_with_plan(rows, plan))?;
    out.retain(|group| !group.rows.is_empty() || !group.aggregates.is_empty());
    Ok(out)
}

pub(crate) fn apply_value_with_plan(value: Value, plan: &UnrollPlan) -> Result<Value> {
    Ok(match &plan.target {
        UnrollTarget::DescendantField(field) => unroll_named_descendant_field(value, field),
        UnrollTarget::Path(path) => unroll_path_value(value, path),
    })
}

impl UnrollPlan {
    pub(crate) fn expand_row(&self, row: &Row) -> Result<Vec<Row>> {
        let expanded = apply_value_with_plan(Value::Object(row.clone()), self)?;
        Ok(rows_from_unrolled_value(expanded))
    }
}

fn rows_from_unrolled_value(value: Value) -> Vec<Row> {
    match value {
        Value::Object(map) => vec![map],
        Value::Array(items) => items
            .into_iter()
            .filter_map(|item| item.as_object().cloned())
            .collect(),
        _ => Vec::new(),
    }
}

fn unroll_path_value(value: Value, path: &PathExpression) -> Value {
    let matches = enumerate_path_matches(&value, path)
        .into_iter()
        .filter(|entry| matches!(entry.value, Value::Array(_)))
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return Value::Null;
    }
    unroll_matches(value, &matches)
}

fn unroll_matches(value: Value, matches: &[AddressedValue]) -> Value {
    let addresses = matches
        .iter()
        .map(|entry| entry.address.clone())
        .collect::<Vec<_>>();
    unroll_at_addresses(value, &addresses)
}

fn unroll_at_addresses(value: Value, addresses: &[Vec<AddressStep>]) -> Value {
    match value {
        Value::Object(map) => unroll_object_at_addresses(map, addresses),
        Value::Array(items) => unroll_array_at_addresses(items, addresses),
        other => other,
    }
}

fn unroll_object_at_addresses(map: Map<String, Value>, addresses: &[Vec<AddressStep>]) -> Value {
    if let Some(field) = direct_target_field(addresses) {
        return duplicate_object_over_field(map, &field);
    }

    let mut map = map;
    let Some(key) = first_target_field(addresses) else {
        return Value::Object(map);
    };

    let Some(child) = map.remove(&key) else {
        return Value::Object(map);
    };

    let child_addresses = child_field_addresses(addresses, &key);
    let child_was_object = matches!(child, Value::Object(_));
    let transformed = unroll_at_addresses(child, &child_addresses);

    if child_was_object && matches!(transformed, Value::Array(_)) {
        return duplicate_object_over_value(map, &key, transformed);
    }

    if !json::is_structurally_empty(&transformed) {
        map.insert(key, transformed);
    }
    Value::Object(map)
}

fn duplicate_object_over_field(mut map: Map<String, Value>, field: &str) -> Value {
    let Some(Value::Array(items)) = map.remove(field) else {
        return Value::Null;
    };

    Value::Array(
        items
            .into_iter()
            .filter(|item| !json::is_structurally_empty(item))
            .map(|item| {
                let mut clone = map.clone();
                clone.insert(field.to_string(), item);
                Value::Object(clone)
            })
            .collect(),
    )
}

fn duplicate_object_over_value(map: Map<String, Value>, field: &str, value: Value) -> Value {
    let Value::Array(items) = value else {
        let mut restored = map;
        restored.insert(field.to_string(), value);
        return Value::Object(restored);
    };

    Value::Array(
        items
            .into_iter()
            .filter(|item| !json::is_structurally_empty(item))
            .map(|item| {
                let mut clone = map.clone();
                clone.insert(field.to_string(), item);
                Value::Object(clone)
            })
            .collect(),
    )
}

fn unroll_array_at_addresses(items: Vec<Value>, addresses: &[Vec<AddressStep>]) -> Value {
    let mut out = Vec::new();

    for (index, item) in items.into_iter().enumerate() {
        let child_addresses = child_index_addresses(addresses, index);
        if child_addresses.is_empty() {
            out.push(item);
            continue;
        }

        let transformed = unroll_at_addresses(item, &child_addresses);
        match transformed {
            Value::Array(values) => {
                out.extend(
                    values
                        .into_iter()
                        .filter(|value| !json::is_structurally_empty(value)),
                );
            }
            other if !json::is_structurally_empty(&other) => out.push(other),
            _ => {}
        }
    }

    Value::Array(out)
}

fn direct_target_field(addresses: &[Vec<AddressStep>]) -> Option<String> {
    addresses
        .iter()
        .find_map(|address| match address.as_slice() {
            [AddressStep::Field(field)] => Some(field.clone()),
            _ => None,
        })
}

fn first_target_field(addresses: &[Vec<AddressStep>]) -> Option<String> {
    addresses.iter().find_map(|address| match address.first() {
        Some(AddressStep::Field(field)) => Some(field.clone()),
        _ => None,
    })
}

fn child_field_addresses(addresses: &[Vec<AddressStep>], field: &str) -> Vec<Vec<AddressStep>> {
    addresses
        .iter()
        .filter_map(|address| match address.split_first() {
            Some((AddressStep::Field(name), rest)) if name == field => Some(rest.to_vec()),
            _ => None,
        })
        .collect()
}

fn child_index_addresses(addresses: &[Vec<AddressStep>], index: usize) -> Vec<Vec<AddressStep>> {
    addresses
        .iter()
        .filter_map(|address| match address.split_first() {
            Some((AddressStep::Index(found), rest)) if *found == index => Some(rest.to_vec()),
            _ => None,
        })
        .collect()
}

fn unroll_named_descendant_field(value: Value, field: &str) -> Value {
    match value {
        // When the current object owns the target array, duplicate the object
        // once per member and replace the field with the single unrolled item.
        // This preserves object envelope metadata instead of flattening the
        // array contents into anonymous rows.
        Value::Object(mut map) if matches!(map.get(field), Some(Value::Array(_))) => {
            let Some(existing) = map.remove(field) else {
                return Value::Object(map);
            };
            let Value::Array(items) = existing else {
                map.insert(field.to_string(), existing);
                return Value::Object(map);
            };
            Value::Array(
                items
                    .into_iter()
                    .filter(|item| !json::is_structurally_empty(item))
                    .map(|item| {
                        let mut clone = map.clone();
                        clone.insert(field.to_string(), item);
                        Value::Object(clone)
                    })
                    .collect::<Vec<_>>(),
            )
        }
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, child) in map {
                let transformed = unroll_named_descendant_field(child, field);
                if !json::is_structurally_empty(&transformed) {
                    out.insert(key, transformed);
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .flat_map(|item| match unroll_named_descendant_field(item, field) {
                    Value::Array(values) => values,
                    other if !json::is_structurally_empty(&other) => vec![other],
                    _ => Vec::new(),
                })
                .collect::<Vec<_>>(),
        ),
        scalar => scalar,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{apply_value_with_plan, compile};

    #[test]
    fn unroll_exact_path_duplicates_only_selected_branch_unit() {
        let plan = compile("sections[0].entries").expect("plan");
        let value = json!({
            "sections": [
                {
                    "title": "Commands",
                    "entries": [
                        {"name": "help"},
                        {"name": "exit"}
                    ]
                },
                {
                    "title": "Options",
                    "entries": [
                        {"name": "--json"}
                    ]
                }
            ]
        });

        let unrolled = apply_value_with_plan(value, &plan).expect("unroll should succeed");
        let sections = unrolled["sections"].as_array().expect("sections");
        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0]["title"], json!("Commands"));
        assert_eq!(sections[0]["entries"]["name"], json!("help"));
        assert_eq!(sections[1]["title"], json!("Commands"));
        assert_eq!(sections[1]["entries"]["name"], json!("exit"));
        assert_eq!(sections[2]["title"], json!("Options"));
        assert_eq!(sections[2]["entries"][0]["name"], json!("--json"));
    }

    #[test]
    fn unroll_dotted_row_path_expands_nested_field_unit() {
        let plan = compile("outer.members").expect("plan");
        let value = json!({
            "cn": "grp",
            "outer": {"members": ["a", "b"]}
        });

        let unrolled = apply_value_with_plan(value, &plan).expect("unroll should succeed");
        let rows = unrolled.as_array().expect("array");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["outer"]["members"], json!("a"));
        assert_eq!(rows[1]["outer"]["members"], json!("b"));
    }
}
