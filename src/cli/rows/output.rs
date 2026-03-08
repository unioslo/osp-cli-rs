use crate::core::output_model::{
    ColumnAlignment, Group, OutputItems, OutputMeta, OutputResult,
    compute_key_index as core_compute_key_index,
};
use crate::core::plugin::{ColumnAlignmentV1, ResponseMetaV1};
use crate::core::row::Row;

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
            column_align: meta
                .map(|value| {
                    value
                        .column_align
                        .iter()
                        .copied()
                        .map(column_alignment_from_plugin)
                        .collect()
                })
                .unwrap_or_default(),
            wants_copy: false,
            grouped: false,
        },
    }
}

fn column_alignment_from_plugin(value: ColumnAlignmentV1) -> ColumnAlignment {
    match value {
        ColumnAlignmentV1::Default => ColumnAlignment::Default,
        ColumnAlignmentV1::Left => ColumnAlignment::Left,
        ColumnAlignmentV1::Center => ColumnAlignment::Center,
        ColumnAlignmentV1::Right => ColumnAlignment::Right,
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
    core_compute_key_index(rows)
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

#[cfg(test)]
mod tests {
    use super::{output_to_rows, plugin_data_to_output_result, rows_to_output_result};
    use crate::core::output_model::{
        ColumnAlignment, Group, OutputItems, OutputMeta, OutputResult,
    };
    use crate::core::plugin::{ColumnAlignmentV1, ResponseMetaV1};
    use serde_json::Value;
    use serde_json::json;

    #[test]
    fn plugin_meta_preserves_column_alignment_unit() {
        let output = plugin_data_to_output_result(
            json!([{ "name": "alice", "count": 2 }]),
            Some(&ResponseMetaV1 {
                format_hint: Some("table".to_string()),
                columns: Some(vec!["name".to_string(), "count".to_string()]),
                column_align: vec![ColumnAlignmentV1::Left, ColumnAlignmentV1::Right],
            }),
        );

        assert_eq!(
            output.meta.key_index,
            vec!["name".to_string(), "count".to_string()]
        );
        assert_eq!(
            output.meta.column_align,
            vec![ColumnAlignment::Left, ColumnAlignment::Right]
        );
    }

    #[test]
    fn plugin_meta_maps_all_alignment_variants_unit() {
        let output = plugin_data_to_output_result(
            json!([{ "name": "alice", "count": 2, "status": "ok", "notes": "ready" }]),
            Some(&ResponseMetaV1 {
                format_hint: Some("table".to_string()),
                columns: Some(vec![
                    "name".to_string(),
                    "count".to_string(),
                    "status".to_string(),
                    "notes".to_string(),
                ]),
                column_align: vec![
                    ColumnAlignmentV1::Default,
                    ColumnAlignmentV1::Left,
                    ColumnAlignmentV1::Center,
                    ColumnAlignmentV1::Right,
                ],
            }),
        );

        assert_eq!(
            output.meta.column_align,
            vec![
                ColumnAlignment::Default,
                ColumnAlignment::Left,
                ColumnAlignment::Center,
                ColumnAlignment::Right,
            ]
        );
    }

    #[test]
    fn rows_round_trip_through_output_result_unit() {
        let rows = vec![
            crate::row! { "uid" => "alice", "count" => 2 },
            crate::row! { "uid" => "bob", "count" => 3 },
        ];

        let output = rows_to_output_result(rows.clone());

        assert_eq!(output_to_rows(&output), rows);
        assert_eq!(
            output.meta.key_index,
            vec!["uid".to_string(), "count".to_string()]
        );
    }

    #[test]
    fn grouped_output_flattens_group_headers_and_rows_unit() {
        let output = OutputResult {
            items: OutputItems::Groups(vec![
                Group {
                    groups: crate::row! { "team" => "ops" },
                    aggregates: crate::row! { "count" => 2 },
                    rows: vec![
                        crate::row! { "user" => "alice" },
                        crate::row! { "user" => "bob" },
                    ],
                },
                Group {
                    groups: crate::row! { "team" => "infra" },
                    aggregates: crate::row! { "count" => 0 },
                    rows: Vec::new(),
                },
            ]),
            meta: OutputMeta {
                key_index: vec!["team".to_string(), "count".to_string(), "user".to_string()],
                column_align: Vec::new(),
                wants_copy: false,
                grouped: true,
            },
        };

        let rows = output_to_rows(&output);

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0]["team"], Value::String("ops".to_string()));
        assert_eq!(rows[0]["count"], Value::from(2));
        assert_eq!(rows[0]["user"], Value::String("alice".to_string()));
        assert_eq!(rows[1]["user"], Value::String("bob".to_string()));
        assert_eq!(rows[2]["team"], Value::String("infra".to_string()));
        assert_eq!(rows[2]["count"], Value::from(0));
        assert_eq!(rows[2].get("user"), None);
    }

    #[test]
    fn plugin_data_scalar_and_object_shapes_are_normalized_unit() {
        let scalar = plugin_data_to_output_result(json!("hello"), None);
        let object = plugin_data_to_output_result(json!({ "uid": "alice", "count": 2 }), None);

        let scalar_rows = output_to_rows(&scalar);
        let object_rows = output_to_rows(&object);

        assert_eq!(scalar_rows, vec![crate::row! { "value" => "hello" }]);
        assert_eq!(
            object_rows,
            vec![crate::row! { "uid" => "alice", "count" => 2 }]
        );
    }
}
