use osp_core::output_model::{Group, OutputItems, OutputMeta, OutputResult};
use osp_core::plugin::ResponseMetaV1;
use osp_core::row::Row;
use std::collections::HashSet;

pub(crate) fn rows_to_output_result(rows: Vec<Row>) -> OutputResult {
    OutputResult::from_rows(rows)
}

pub(crate) fn output_to_rows(output: &OutputResult) -> Vec<Row> {
    match &output.items {
        OutputItems::Rows(rows) => rows.clone(),
        OutputItems::Groups(groups) => {
            let mut out = Vec::new();
            for group in groups {
                if group.rows.is_empty() {
                    out.push(merge_group_header_row(group));
                    continue;
                }
                for row in &group.rows {
                    out.push(merge_group_row(group, row));
                }
            }
            out
        }
    }
}

pub(crate) fn plugin_data_to_output_result(
    data: serde_json::Value,
    meta: Option<&ResponseMetaV1>,
) -> OutputResult {
    let rows = response_to_rows(data);
    let key_index = meta
        .and_then(|value| value.columns.clone())
        .filter(|columns| !columns.is_empty())
        .unwrap_or_else(|| compute_key_index(&rows));
    OutputResult {
        items: OutputItems::Rows(rows),
        meta: OutputMeta {
            key_index,
            wants_copy: false,
            grouped: false,
        },
    }
}

fn response_to_rows(data: serde_json::Value) -> Vec<Row> {
    match data {
        serde_json::Value::Array(items)
            if items
                .iter()
                .all(|item| matches!(item, serde_json::Value::Object(_))) =>
        {
            items
                .into_iter()
                .filter_map(|item| item.as_object().cloned())
                .collect::<Vec<Row>>()
        }
        serde_json::Value::Object(map) => vec![map],
        scalar => vec![crate::row! { "value" => scalar }],
    }
}

fn compute_key_index(rows: &[Row]) -> Vec<String> {
    let mut keys = Vec::new();
    let mut seen = HashSet::new();
    for row in rows {
        for key in row.keys() {
            if seen.insert(key.clone()) {
                keys.push(key.clone());
            }
        }
    }
    keys
}

fn merge_group_header_row(group: &Group) -> Row {
    let mut row = group.groups.clone();
    for (key, value) in &group.aggregates {
        row.insert(key.clone(), value.clone());
    }
    row
}

fn merge_group_row(group: &Group, row: &Row) -> Row {
    let mut merged = group.groups.clone();
    for (key, value) in &group.aggregates {
        merged.insert(key.clone(), value.clone());
    }
    for (key, value) in row {
        merged.insert(key.clone(), value.clone());
    }
    merged
}
