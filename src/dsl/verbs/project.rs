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

#[cfg(test)]
use crate::core::output_model::Group;
use crate::core::row::Row;
#[cfg(test)]
use crate::dsl::verbs::common::map_group_rows;
use anyhow::{Result, anyhow};
use serde_json::{Map, Value};
use std::collections::HashMap;

use crate::dsl::{
    eval::{
        flatten::{coalesce_flat_row_with_fill, flatten_row},
        resolve::{compact_sparse_arrays, sparse_hole},
    },
    verbs::common::parse_terms,
};

use super::selector;

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
/// Empty member sets retain their group metadata.
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

#[cfg(test)]
pub(crate) fn apply_groups_with_plan(groups: Vec<Group>, plan: &ProjectPlan) -> Result<Vec<Group>> {
    map_group_rows(groups, |rows| apply_with_plan(rows, plan))
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

    let drops = selector::collect_compiled_matches(&nested, droppers.iter());
    for pattern in keepers {
        if let Some(column) = pattern.collect_dynamic_column(&nested) {
            dynamic_columns.push(DynamicColumn {
                label: column.0,
                source: pattern.token().to_string(),
                values: column
                    .1
                    .into_iter()
                    .filter(|entry| {
                        !drops
                            .iter()
                            .any(|drop| entry.address.starts_with(&drop.address))
                    })
                    .map(|entry| entry.value)
                    .collect(),
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
        for key in pattern.matched_flat_keys(&flattened) {
            static_flat.remove(&key);
        }
    }

    static_flat.retain(|key, _| {
        !drops.iter().any(|entry| {
            key == &entry.flat_key
                || key.starts_with(&format!("{}.", entry.flat_key))
                || key.starts_with(&format!("{}[", entry.flat_key))
        })
    });

    reject_ambiguous_dynamic_columns(&dynamic_columns)?;
    let mut rows = build_rows_from_dynamic(static_flat, dynamic_columns);
    if rows.is_empty() && keepers.is_empty() {
        rows.push(rebuild_row(&Map::new()));
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

fn rebuild_row(flat: &Row) -> Row {
    let mut value = Value::Object(coalesce_flat_row_with_fill(flat, &sparse_hole()));
    compact_sparse_arrays(&mut value);
    value.as_object().cloned().unwrap_or_default()
}

fn build_rows_from_dynamic(static_flat: Row, dynamic_columns: Vec<DynamicColumn>) -> Vec<Row> {
    if dynamic_columns.is_empty() {
        if static_flat.is_empty() {
            return Vec::new();
        }
        return vec![rebuild_row(&static_flat)];
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
            vec![rebuild_row(&static_flat)]
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

        let projected = rebuild_row(&flat);
        if !projected.is_empty() {
            rows.push(projected);
        }
    }

    rows
}

#[cfg(test)]
mod tests;
