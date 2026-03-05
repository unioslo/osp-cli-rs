use anyhow::Result;
use osp_core::{
    output_model::{Group, OutputItems},
    row::Row,
};
use serde_json::Value;

use crate::stages::quick;

pub fn apply(items: OutputItems, spec: &str) -> Result<OutputItems> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Ok(clean_items(items));
    }

    let raw = format!("?{trimmed}");
    let out = match items {
        OutputItems::Rows(rows) => OutputItems::Rows(quick::apply(rows, &raw)?),
        OutputItems::Groups(groups) => OutputItems::Groups(apply_groups_quick(groups, &raw)?),
    };
    Ok(out)
}

fn clean_items(items: OutputItems) -> OutputItems {
    match items {
        OutputItems::Rows(rows) => OutputItems::Rows(clean_rows(rows)),
        OutputItems::Groups(groups) => OutputItems::Groups(
            groups
                .into_iter()
                .map(|group| Group {
                    groups: group.groups,
                    aggregates: group.aggregates,
                    rows: clean_rows(group.rows),
                })
                .collect(),
        ),
    }
}

fn clean_rows(rows: Vec<Row>) -> Vec<Row> {
    rows.into_iter().filter_map(clean_row).collect()
}

fn clean_row(row: Row) -> Option<Row> {
    let cleaned = row
        .into_iter()
        .filter(|(_, value)| !is_empty_value(value))
        .collect::<Row>();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn is_empty_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.is_empty(),
        Value::Array(items) => items.is_empty(),
        _ => false,
    }
}

fn apply_groups_quick(groups: Vec<Group>, raw: &str) -> Result<Vec<Group>> {
    let mut out = Vec::with_capacity(groups.len());
    for group in groups {
        let rows = quick::apply(group.rows, raw)?;
        out.push(Group {
            groups: group.groups,
            aggregates: group.aggregates,
            rows,
        });
    }
    Ok(out)
}
