//! Small adapters between row-oriented helpers and [`OutputResult`].
//!
//! These helpers keep command/builtin code from reimplementing the repetitive
//! "rows in, structured output out" and "plugin response metadata into output
//! metadata" conversions.

use crate::core::output_model::{
    ColumnAlignment, OutputDocument, OutputDocumentKind, OutputMeta, OutputResult,
    compute_key_index as core_compute_key_index, output_items_from_value, output_items_to_rows,
};
use crate::core::plugin::{ColumnAlignmentV1, ResponseMetaV1};
use crate::core::row::Row;
use serde_json::{Map, Value};

pub(crate) fn rows_to_output_result(rows: Vec<Row>) -> OutputResult {
    OutputResult::from_rows(rows)
}

pub(crate) fn output_to_rows(output: &OutputResult) -> Vec<Row> {
    output_items_to_rows(&output.items)
}

pub(crate) fn plugin_data_to_output_result(
    data: serde_json::Value,
    meta: Option<&ResponseMetaV1>,
) -> OutputResult {
    let document = meta
        .is_some_and(|meta| meta.preserve_json_document || meta.row_path.is_some())
        .then(|| OutputDocument::new(OutputDocumentKind::Json, data.clone()));
    let row_data = meta
        .and_then(|meta| meta.row_path.as_deref())
        .and_then(|row_path| data.get(row_path))
        .cloned()
        .unwrap_or(data);
    let row_data = match meta.filter(|meta| {
        meta.row_path.is_some()
            || (meta.preserve_json_document
                && meta
                    .columns
                    .as_ref()
                    .is_some_and(|columns| !columns.is_empty())
                && row_data.is_object())
    }) {
        Some(meta) => project_display_value(row_data, meta),
        None => row_data,
    };
    let items = output_items_from_value(row_data);
    let rows = output_items_to_rows(&items);
    let key_index = meta
        .and_then(|value| {
            (!value.column_labels.is_empty())
                .then(|| value.column_labels.clone())
                .or_else(|| value.columns.clone())
        })
        .filter(|columns| !columns.is_empty())
        .unwrap_or_else(|| compute_key_index(&rows));
    OutputResult {
        items,
        document,
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
            render_recommendation: None,
        },
    }
}

fn project_display_value(data: Value, meta: &ResponseMetaV1) -> Value {
    let Some(columns) = meta.columns.as_ref().filter(|columns| !columns.is_empty()) else {
        return data;
    };
    let labels = if meta.column_labels.is_empty() {
        columns
    } else {
        &meta.column_labels
    };
    let project = |item: &Value, keep_missing: bool| {
        let mut row = Map::new();
        for (path, label) in columns.iter().zip(labels) {
            match value_at_path(item, path) {
                Some(value) if keep_missing || !value.is_null() => {
                    row.insert(label.clone(), value.clone());
                }
                None if keep_missing => {
                    row.insert(label.clone(), Value::Null);
                }
                Some(_) | None => {}
            }
        }
        Value::Object(row)
    };
    match data {
        Value::Array(items) => Value::Array(items.iter().map(|item| project(item, true)).collect()),
        Value::Object(map) => project(&Value::Object(map), false),
        other => other,
    }
}

fn value_at_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.')
        .filter(|segment| !segment.is_empty())
        .try_fold(value, |current, segment| current.get(segment))
}

fn column_alignment_from_plugin(value: ColumnAlignmentV1) -> ColumnAlignment {
    match value {
        ColumnAlignmentV1::Default => ColumnAlignment::Default,
        ColumnAlignmentV1::Left => ColumnAlignment::Left,
        ColumnAlignmentV1::Center => ColumnAlignment::Center,
        ColumnAlignmentV1::Right => ColumnAlignment::Right,
    }
}

fn compute_key_index(rows: &[Row]) -> Vec<String> {
    core_compute_key_index(rows)
}

#[cfg(test)]
mod tests {
    use super::{output_to_rows, plugin_data_to_output_result, rows_to_output_result};
    use crate::core::output::OutputFormat;
    use crate::core::output_model::{
        ColumnAlignment, Group, OutputItems, OutputMeta, OutputResult,
    };
    use crate::core::plugin::{ColumnAlignmentV1, ResponseMetaV1};
    use crate::dsl::apply_output_pipeline;
    use crate::ui::{RenderSettings, render_output};
    use serde_json::Value;
    use serde_json::json;

    #[test]
    fn plugin_meta_and_data_shapes_preserve_alignment_and_normalize_rows_unit() {
        let output = plugin_data_to_output_result(
            json!([{ "name": "alice", "count": 2 }]),
            Some(&ResponseMetaV1 {
                format_hint: Some("table".to_string()),
                columns: Some(vec!["name".to_string(), "count".to_string()]),
                column_align: vec![ColumnAlignmentV1::Left, ColumnAlignmentV1::Right],
                column_labels: Vec::new(),
                row_path: None,
                preserve_json_document: false,
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
                column_labels: Vec::new(),
                row_path: None,
                preserve_json_document: false,
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

        let scalar = plugin_data_to_output_result(json!("hello"), None);
        let object = plugin_data_to_output_result(json!({ "uid": "alice", "count": 2 }), None);
        let scalar_array = plugin_data_to_output_result(json!(["alice", "bob"]), None);
        let empty_array = plugin_data_to_output_result(json!([]), None);

        let scalar_rows = output_to_rows(&scalar);
        let object_rows = output_to_rows(&object);
        let scalar_array_rows = output_to_rows(&scalar_array);
        let empty_array_rows = output_to_rows(&empty_array);

        assert_eq!(scalar_rows, vec![crate::row! { "value" => "hello" }]);
        assert_eq!(
            object_rows,
            vec![crate::row! { "uid" => "alice", "count" => 2 }]
        );
        assert_eq!(
            scalar_array_rows,
            vec![
                crate::row! { "value" => "alice" },
                crate::row! { "value" => "bob" }
            ]
        );
        assert!(empty_array_rows.is_empty());
    }

    #[test]
    fn plugin_document_metadata_preserves_the_canonical_json_value_unit() {
        let data = json!({
            "mreg": {"name": "db01.uio.no"},
            "ldap": {"host": "db01.uio.no"}
        });
        let output = plugin_data_to_output_result(
            data.clone(),
            Some(&ResponseMetaV1 {
                preserve_json_document: true,
                ..ResponseMetaV1::default()
            }),
        );

        assert_eq!(
            output.document.as_ref().map(|document| &document.value),
            Some(&data)
        );
    }

    #[test]
    fn plugin_row_path_renders_rows_but_keeps_canonical_json_unit() {
        let data = json!({
            "items": [
                {"name": "db01.uio.no", "provider": "vmware"},
                {"name": "api01.uio.no", "provider": "nrec"}
            ],
            "page": {"next_cursor": "cursor-2"},
            "targets": ["vmware:local", "nrec:local"]
        });
        let output = plugin_data_to_output_result(
            data.clone(),
            Some(&ResponseMetaV1 {
                columns: Some(vec!["name".to_string(), "provider".to_string()]),
                row_path: Some("items".to_string()),
                ..ResponseMetaV1::default()
            }),
        );

        let table = render_output(&output, &RenderSettings::test_plain(OutputFormat::Table));
        assert!(table.contains("db01.uio.no"));
        assert!(table.contains("api01.uio.no"));
        assert!(!table.contains("next_cursor"));
        assert!(!table.contains("targets"));

        let mut json_settings = RenderSettings::test_plain(OutputFormat::Json);
        json_settings.format_explicit = true;
        let rendered_json = render_output(&output, &json_settings);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&rendered_json)
                .expect("rendered output should be JSON"),
            data
        );
    }

    #[test]
    fn plugin_row_path_projects_nested_columns_with_display_labels_unit() {
        let output = plugin_data_to_output_result(
            json!({
                "items": [{
                    "name": "db01.uio.no",
                    "provider": {"name": "vmware"},
                    "compute": {"display": "4 CPU / 8 GiB"}
                }]
            }),
            Some(&ResponseMetaV1 {
                columns: Some(vec![
                    "name".to_string(),
                    "provider.name".to_string(),
                    "compute.display".to_string(),
                ]),
                column_labels: vec![
                    "NAME".to_string(),
                    "PROVIDER".to_string(),
                    "COMPUTE".to_string(),
                ],
                row_path: Some("items".to_string()),
                ..ResponseMetaV1::default()
            }),
        );

        assert_eq!(
            output_to_rows(&output),
            vec![crate::row! {
                "NAME" => "db01.uio.no",
                "PROVIDER" => "vmware",
                "COMPUTE" => "4 CPU / 8 GiB"
            }]
        );
    }

    #[test]
    fn plugin_document_projects_a_curated_record_but_keeps_canonical_json_unit() {
        let data = json!({
            "task_id": 1855,
            "action": "reboot",
            "target": {"display": "db02.uio.no"},
            "status": {"name": "waiting_approval", "code": 301},
            "request": {"force": false}
        });
        let output = plugin_data_to_output_result(
            data.clone(),
            Some(&ResponseMetaV1 {
                columns: Some(vec![
                    "task_id".to_string(),
                    "action".to_string(),
                    "target.display".to_string(),
                    "status.name".to_string(),
                    "approval.progress".to_string(),
                ]),
                column_labels: vec![
                    "Task".to_string(),
                    "Action".to_string(),
                    "Target".to_string(),
                    "Status".to_string(),
                    "Approval".to_string(),
                ],
                preserve_json_document: true,
                ..ResponseMetaV1::default()
            }),
        );

        assert_eq!(
            output_to_rows(&output),
            vec![crate::row! {
                "Task" => 1855,
                "Action" => "reboot",
                "Target" => "db02.uio.no",
                "Status" => "waiting_approval"
            }]
        );
        assert_eq!(
            output.document.as_ref().map(|document| &document.value),
            Some(&data)
        );
        let rendered = render_output(&output, &RenderSettings::test_plain(OutputFormat::Mreg));
        assert!(rendered.contains("Task:   1855"));
        assert!(rendered.contains("Target: db02.uio.no"));
        assert!(!rendered.contains("Approval"));
        assert!(!rendered.contains("force"));
    }

    #[test]
    fn plugin_row_path_dsl_starts_from_the_canonical_document_unit() {
        let output = plugin_data_to_output_result(
            json!({
                "items": [{"name": "db01.uio.no"}],
                "page": {"next_cursor": "cursor-2"}
            }),
            Some(&ResponseMetaV1 {
                row_path: Some("items".to_string()),
                ..ResponseMetaV1::default()
            }),
        );

        let projected = apply_output_pipeline(output, &["P page.next_cursor".to_string()])
            .expect("pipeline should see canonical page metadata");

        assert_eq!(
            projected
                .document
                .as_ref()
                .map(|document| document.value.clone()),
            Some(json!({"page": {"next_cursor": "cursor-2"}}))
        );
    }

    #[test]
    fn output_row_helpers_round_trip_and_flatten_groups_unit() {
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
            document: None,
            meta: OutputMeta {
                key_index: vec!["team".to_string(), "count".to_string(), "user".to_string()],
                column_align: Vec::new(),
                wants_copy: false,
                grouped: true,
                render_recommendation: None,
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
}
