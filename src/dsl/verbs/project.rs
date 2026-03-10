//! Projection keeps the selector surface while letting users narrow structure.
//!
//! The important rules here are:
//! - keepers and droppers resolve against original addresses, not already
//!   compacted output
//! - structural rebuild happens before generic compaction
//! - row fanout labels must be unambiguous
//!
//! Example:
//! - `P sections[1].entries[0].name !sections[0]` should treat the dropper
//!   against the original tree, not delete the only surviving rebuilt branch
//! - row projection `P users[].name groups[].name` should fail loudly because
//!   both fanouts want the same dynamic `name` label

use crate::core::{output_model::Group, row::Row};
use anyhow::{Result, anyhow};
use serde_json::{Map, Value};
use std::collections::HashMap;

use crate::dsl::{
    eval::{
        flatten::{coalesce_flat_row, flatten_row},
        resolve::{compact_sparse_arrays, is_sparse_hole, sparse_hole},
    },
    verbs::common::{map_group_rows, parse_terms},
};

use super::{json, selector};

#[derive(Debug, Clone)]
pub(crate) struct ProjectPlan {
    keepers: Vec<selector::CompiledSelector>,
    droppers: Vec<selector::CompiledSelector>,
}

impl ProjectPlan {
    pub(crate) fn project_row(&self, row: &Row) -> Result<Vec<Row>> {
        project_single_row(row, &self.keepers, &self.droppers)
    }
}

pub(crate) fn compile(spec: &str) -> Result<ProjectPlan> {
    let (keepers, droppers) = parse_patterns(spec)?;
    if keepers.is_empty() && droppers.is_empty() {
        return Err(anyhow!("P requires one or more keys"));
    }

    Ok(ProjectPlan { keepers, droppers })
}

#[cfg(test)]
/// Projects flat rows according to the keep/drop patterns in `spec`.
///
/// Fanout selectors may expand one input row into multiple output rows.
pub fn apply(rows: Vec<Row>, spec: &str) -> Result<Vec<Row>> {
    let plan = compile(spec)?;
    apply_with_plan(rows, &plan)
}

#[cfg(test)]
/// Projects the rows inside each group while preserving group metadata.
///
/// Groups with no remaining rows and no aggregates are dropped.
pub fn apply_groups(groups: Vec<Group>, spec: &str) -> Result<Vec<Group>> {
    let plan = compile(spec)?;
    apply_groups_with_plan(groups, &plan)
}

pub(crate) fn apply_with_plan(rows: Vec<Row>, plan: &ProjectPlan) -> Result<Vec<Row>> {
    let mut out = Vec::new();
    for row in rows {
        out.extend(plan.project_row(&row)?);
    }
    Ok(out)
}

pub(crate) fn apply_groups_with_plan(groups: Vec<Group>, plan: &ProjectPlan) -> Result<Vec<Group>> {
    let mut out = map_group_rows(groups, |rows| {
        let mut projected_rows = Vec::new();
        for row in &rows {
            projected_rows.extend(plan.project_row(row)?);
        }
        Ok(projected_rows)
    })?;
    out.retain(|group| !group.rows.is_empty() || !group.aggregates.is_empty());
    Ok(out)
}

pub(crate) fn apply_value_with_plan(value: Value, plan: &ProjectPlan) -> Result<Value> {
    if let Some(projected) = try_apply_addressed_projection(&value, plan)? {
        return Ok(projected);
    }

    selector::project_descendants(
        value,
        |rows| project_rows_to_value_with_plan(rows, plan),
        |value| project_collection_value_with_plan(value, plan),
    )
}

fn project_rows_to_value_with_plan(rows: Vec<Row>, plan: &ProjectPlan) -> Result<Value> {
    let projected = apply_with_plan(rows, plan)?;
    Ok(match projected.as_slice() {
        [] => Value::Null,
        [row] => Value::Object(row.clone()),
        _ => Value::Array(projected.into_iter().map(Value::Object).collect()),
    })
}

fn project_collection_value_with_plan(value: Value, plan: &ProjectPlan) -> Result<Value> {
    json::apply_collection_stage(value, |items| match items {
        crate::core::output_model::OutputItems::Rows(rows) => Ok(
            crate::core::output_model::OutputItems::Rows(apply_with_plan(rows, plan)?),
        ),
        crate::core::output_model::OutputItems::Groups(groups) => Ok(
            crate::core::output_model::OutputItems::Groups(apply_groups_with_plan(groups, plan)?),
        ),
    })
}

fn try_apply_addressed_projection(root: &Value, plan: &ProjectPlan) -> Result<Option<Value>> {
    // Structural selectors own the original-address rebuild path. Generic
    // keepers/droppers still apply afterward, but only against the rebuilt
    // survivor tree so we do not re-resolve by compacted array position.
    let (structural_keepers, other_keepers): (Vec<_>, Vec<_>) = plan
        .keepers
        .iter()
        .cloned()
        .partition(selector::CompiledSelector::is_structural);
    let (structural_droppers, other_droppers): (Vec<_>, Vec<_>) = plan
        .droppers
        .iter()
        .cloned()
        .partition(selector::CompiledSelector::is_structural);

    if structural_keepers.is_empty() && structural_droppers.is_empty() {
        return Ok(None);
    }

    let uses_sparse_structural_projection = !structural_keepers.is_empty();
    let mut projected = if structural_keepers.is_empty() {
        if other_keepers.is_empty() {
            root.clone()
        } else {
            apply_value_with_plan(
                root.clone(),
                &ProjectPlan {
                    keepers: other_keepers.clone(),
                    droppers: Vec::new(),
                },
            )?
        }
    } else {
        let mut projected = project_structural_paths_unfinalized(root, &structural_keepers);
        if !other_keepers.is_empty() {
            apply_generic_keepers_within_survivors(root, &mut projected, &other_keepers);
        }
        projected
    };

    if !structural_droppers.is_empty() {
        projected = drop_structural_paths(projected, &structural_droppers);
    }

    if !other_droppers.is_empty() {
        if uses_sparse_structural_projection {
            apply_generic_droppers_within_survivors(root, &mut projected, &other_droppers);
        } else {
            projected = apply_value_with_plan(
                projected,
                &ProjectPlan {
                    keepers: Vec::new(),
                    droppers: other_droppers,
                },
            )?;
        }
    }

    if uses_sparse_structural_projection {
        compact_sparse_arrays(&mut projected);
    }

    Ok(Some(projected))
}

fn project_structural_paths_unfinalized(
    root: &Value,
    keepers: &[selector::CompiledSelector],
) -> Value {
    selector::project_compiled_unfinalized(root, keepers.iter())
}

fn drop_structural_paths(root: Value, droppers: &[selector::CompiledSelector]) -> Value {
    selector::remove_compiled(root, droppers.iter())
}

fn apply_generic_keepers_within_survivors(
    original: &Value,
    projected: &mut Value,
    keepers: &[selector::CompiledSelector],
) {
    match (original, projected) {
        (Value::Object(original_map), Value::Object(projected_map)) => {
            for key in matched_direct_keys(original_map, keepers) {
                if let Some(value) = original_map.get(&key)
                    && !projected_map.contains_key(&key)
                {
                    projected_map.insert(key, value.clone());
                }
            }

            let keys = projected_map.keys().cloned().collect::<Vec<_>>();
            for key in keys {
                let Some(original_child) = original_map.get(&key) else {
                    continue;
                };
                let Some(projected_child) = projected_map.get_mut(&key) else {
                    continue;
                };
                apply_generic_keepers_within_survivors(original_child, projected_child, keepers);
            }
        }
        (Value::Array(original_items), Value::Array(projected_items)) => {
            for (index, projected_item) in projected_items.iter_mut().enumerate() {
                if is_sparse_hole(projected_item) {
                    continue;
                }
                let Some(original_item) = original_items.get(index) else {
                    continue;
                };
                apply_generic_keepers_within_survivors(original_item, projected_item, keepers);
            }
        }
        _ => {}
    }
}

fn apply_generic_droppers_within_survivors(
    original: &Value,
    projected: &mut Value,
    droppers: &[selector::CompiledSelector],
) {
    match (original, projected) {
        (Value::Object(original_map), Value::Object(projected_map)) => {
            for key in matched_direct_keys(original_map, droppers) {
                projected_map.remove(&key);
            }

            let keys = projected_map.keys().cloned().collect::<Vec<_>>();
            for key in keys {
                let Some(original_child) = original_map.get(&key) else {
                    continue;
                };
                let Some(projected_child) = projected_map.get_mut(&key) else {
                    continue;
                };
                apply_generic_droppers_within_survivors(original_child, projected_child, droppers);
                if json::is_structurally_empty(projected_child) {
                    projected_map.remove(&key);
                }
            }
        }
        (Value::Array(original_items), Value::Array(projected_items)) => {
            for (index, projected_item) in projected_items.iter_mut().enumerate() {
                if is_sparse_hole(projected_item) {
                    continue;
                }
                let Some(original_item) = original_items.get(index) else {
                    continue;
                };
                apply_generic_droppers_within_survivors(original_item, projected_item, droppers);
                if json::is_structurally_empty(projected_item) {
                    *projected_item = sparse_hole();
                }
            }
        }
        _ => {}
    }
}

fn matched_direct_keys(direct_row: &Row, patterns: &[selector::CompiledSelector]) -> Vec<String> {
    let mut matched = Vec::new();
    for pattern in patterns {
        for key in pattern.matched_flat_keys(direct_row) {
            if !matched.contains(&key) {
                matched.push(key);
            }
        }
    }
    matched
}

fn parse_patterns(
    spec: &str,
) -> Result<(
    Vec<selector::CompiledSelector>,
    Vec<selector::CompiledSelector>,
)> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let mut keepers = Vec::new();
    let mut droppers = Vec::new();
    for text in parse_terms(trimmed)? {
        let drop = text.starts_with('!');
        let pattern = selector::CompiledSelector::parse(&text);

        if drop {
            droppers.push(pattern);
        } else {
            keepers.push(pattern);
        }
    }

    Ok((keepers, droppers))
}

fn project_single_row(
    row: &Row,
    keepers: &[selector::CompiledSelector],
    droppers: &[selector::CompiledSelector],
) -> Result<Vec<Row>> {
    let flattened = flatten_row(row);
    let nested = Value::Object(row.clone());

    let mut static_flat = if keepers.is_empty() {
        flattened.clone()
    } else {
        Map::new()
    };
    let mut dynamic_columns: Vec<DynamicColumn> = Vec::new();

    for pattern in keepers {
        if let Some(column) = pattern.collect_dynamic_column(&nested) {
            dynamic_columns.push(DynamicColumn {
                label: column.0,
                source: pattern.token().to_string(),
                values: column.1,
            });
            continue;
        }

        for key in pattern.matched_flat_keys(&flattened) {
            if let Some(value) = flattened.get(&key) {
                static_flat.insert(key, value.clone());
            }
        }
    }

    for pattern in droppers {
        dynamic_columns.retain(|column| !pattern.matches_dynamic_label(&column.label));

        for key in pattern.matched_flat_keys(&flattened) {
            static_flat.remove(&key);
        }
    }

    reject_ambiguous_dynamic_columns(&dynamic_columns)?;
    let mut rows = build_rows_from_dynamic(static_flat, dynamic_columns);
    if rows.is_empty() && keepers.is_empty() {
        rows.push(coalesce_flat_row(&Map::new()));
    }
    Ok(rows)
}

#[derive(Debug, Clone)]
struct DynamicColumn {
    label: String,
    source: String,
    values: Vec<Value>,
}

fn reject_ambiguous_dynamic_columns(columns: &[DynamicColumn]) -> Result<()> {
    let mut grouped: HashMap<&str, Vec<&str>> = HashMap::new();
    for column in columns {
        grouped
            .entry(column.label.as_str())
            .or_default()
            .push(column.source.as_str());
    }

    let Some((label, selectors)) = grouped
        .into_iter()
        .find(|(_, selectors)| selectors.len() > 1)
    else {
        return Ok(());
    };

    Err(anyhow!(
        "ambiguous dynamic projection label `{label}` from selectors: {}",
        selectors.join(", ")
    ))
}

fn build_rows_from_dynamic(
    static_flat: Row,
    dynamic_columns: Vec<DynamicColumn>,
) -> Vec<Row> {
    if dynamic_columns.is_empty() {
        if static_flat.is_empty() {
            return Vec::new();
        }
        return vec![coalesce_flat_row(&static_flat)];
    }

    let row_count = dynamic_columns
        .iter()
        .map(|column| column.values.len())
        .max()
        .unwrap_or(0);
    if row_count == 0 {
        return if static_flat.is_empty() {
            Vec::new()
        } else {
            vec![coalesce_flat_row(&static_flat)]
        };
    }

    let mut rows = Vec::new();
    for index in 0..row_count {
        let mut flat = static_flat.clone();
        for column in &dynamic_columns {
            if let Some(value) = column.values.get(index) {
                match value {
                    Value::Object(map) => {
                        for (key, nested_value) in map {
                            flat.insert(key.clone(), nested_value.clone());
                        }
                    }
                    scalar => {
                        flat.insert(column.label.clone(), scalar.clone());
                    }
                }
            } else {
                flat.insert(column.label.clone(), Value::Null);
            }
        }

        let projected = coalesce_flat_row(&flat);
        if !projected.is_empty() {
            rows.push(projected);
        }
    }

    rows
}

#[cfg(test)]
mod tests {
    use crate::core::output_model::Group;
    use serde_json::json;

    use super::{apply, apply_groups, compile};

    #[test]
    fn keeps_requested_columns() {
        let rows = vec![
            json!({"uid": "oistes", "cn": "Oistein", "mail": "o@uio.no"})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let projected = apply(rows, "uid cn").expect("project should work");
        assert!(projected[0].contains_key("uid"));
        assert!(projected[0].contains_key("cn"));
        assert!(!projected[0].contains_key("mail"));
    }

    #[test]
    fn drops_column_with_prefix() {
        let rows = vec![
            json!({"uid": "oistes", "status": "active"})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let projected = apply(rows, "!status").expect("project should work");
        assert!(projected[0].contains_key("uid"));
        assert!(!projected[0].contains_key("status"));
    }

    #[test]
    fn supports_selector_fanout() {
        let rows = vec![
            json!({
                "interfaces": [
                    {"mac": "aa:bb"},
                    {"mac": "cc:dd"}
                ]
            })
            .as_object()
            .cloned()
            .expect("object"),
        ];

        let projected = apply(rows, "interfaces[].mac").expect("project should work");
        assert_eq!(projected.len(), 2);
        assert_eq!(projected[0].get("mac"), Some(&json!("aa:bb")));
        assert_eq!(projected[1].get("mac"), Some(&json!("cc:dd")));
    }

    #[test]
    fn quoted_commas_remain_single_projection_terms() {
        let rows = vec![
            json!({"display,name": "Alice", "display": "wrong", "name": "wrong"})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let projected = apply(rows, "\"display,name\"").expect("project should work");
        assert_eq!(
            projected,
            vec![
                json!({"display,name": "Alice"})
                    .as_object()
                    .cloned()
                    .expect("object")
            ]
        );
    }

    #[test]
    fn keeps_all_exact_nested_matches() {
        let rows = vec![
            json!({
                "id": 55753,
                "txts": {"id": 27994},
                "ipaddresses": [{"id": 57171}, {"id": 57172}],
                "metadata": {"asset": {"id": 42}}
            })
            .as_object()
            .cloned()
            .expect("object"),
        ];

        let projected = apply(rows, "id").expect("project should work");
        assert_eq!(
            projected,
            vec![
                json!({
                    "id": 55753,
                    "txts": {"id": 27994},
                    "ipaddresses": [{"id": 57171}, {"id": 57172}],
                    "metadata": {"asset": {"id": 42}}
                })
                .as_object()
                .cloned()
                .expect("object")
            ]
        );
    }

    #[test]
    fn absolute_path_projection_keeps_only_exact_nested_key() {
        let rows = vec![
            json!({"id": 1, "nested": {"id": 2}, "other": {"id": 3}})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let projected = apply(rows, ".nested.id").expect("project should work");
        assert_eq!(
            projected,
            vec![
                json!({"nested": {"id": 2}})
                    .as_object()
                    .cloned()
                    .expect("object")
            ]
        );
    }

    #[test]
    fn relative_path_projection_requires_exact_flat_path_match() {
        let rows = vec![
            json!({"metadata": {"asset": {"id": 42}}})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let projected = apply(rows, "asset.id").expect("project should work");
        assert!(projected.is_empty());
    }

    #[test]
    fn apply_groups_keeps_aggregate_only_groups_even_when_rows_drop_out() {
        let groups = vec![Group {
            groups: json!({"dept": "eng"}).as_object().cloned().expect("object"),
            aggregates: json!({"count": 2}).as_object().cloned().expect("object"),
            rows: vec![
                json!({"uid": "alice"})
                    .as_object()
                    .cloned()
                    .expect("object"),
                json!({"uid": "bob"}).as_object().cloned().expect("object"),
            ],
        }];

        let projected = apply_groups(groups, "missing").expect("group project should work");
        assert_eq!(projected.len(), 1);
        assert!(projected[0].rows.is_empty());
        assert_eq!(projected[0].aggregates.get("count"), Some(&json!(2)));
    }

    #[test]
    fn empty_project_spec_is_rejected() {
        let err = apply(
            vec![
                json!({"uid": "alice"})
                    .as_object()
                    .cloned()
                    .expect("object"),
            ],
            "   ",
        )
        .expect_err("empty spec should fail");

        assert!(err.to_string().contains("requires one or more keys"));
    }

    #[test]
    fn dropping_dynamic_projection_label_removes_fanout_column() {
        let rows = vec![
            json!({
                "uid": "alice",
                "interfaces": [{"mac": "aa:bb"}, {"mac": "cc:dd"}]
            })
            .as_object()
            .cloned()
            .expect("object"),
        ];

        let projected = apply(rows, "uid interfaces[].mac !mac").expect("project should work");
        assert_eq!(
            projected,
            vec![
                json!({"uid": "alice"})
                    .as_object()
                    .cloned()
                    .expect("object")
            ]
        );
    }

    #[test]
    fn rejects_ambiguous_dynamic_projection_labels() {
        let rows = vec![
            json!({
                "users": [{"name": "alice"}],
                "groups": [{"name": "ops"}]
            })
            .as_object()
            .cloned()
            .expect("object"),
        ];

        let err = apply(rows, "users[].name groups[].name").expect_err("project should fail");
        assert!(err.to_string().contains("ambiguous dynamic projection label `name`"));
        assert!(err.to_string().contains("users[].name"));
        assert!(err.to_string().contains("groups[].name"));
    }

    #[test]
    fn compile_treats_prefixed_path_droppers_as_structural_selectors() {
        let plan = compile("!sections[0].entries[0]").expect("project should compile");

        assert_eq!(plan.droppers.len(), 1);
        assert!(plan.droppers[0].is_structural());
    }
}
