use osp_core::output_model::OutputItems;
use osp_dsl::{apply_pipeline, execute_pipeline};
use serde_json::{Map, Value, json};

fn obj(value: Value) -> Map<String, Value> {
    value.as_object().cloned().expect("fixture must be object")
}

#[test]
fn filter_grouped_data_keeps_groups_with_matching_rows() {
    let grouped = execute_pipeline(
        vec![
            obj(json!({"dept": "sales", "active": true})),
            obj(json!({"dept": "sales", "active": false})),
            obj(json!({"dept": "eng", "active": true})),
        ],
        &["G dept".to_string(), "F ?active".to_string()],
    )
    .expect("group filter should run");

    match grouped.items {
        OutputItems::Groups(groups) => assert_eq!(groups.len(), 2),
        OutputItems::Rows(_) => panic!("expected grouped output"),
    }
}

#[test]
fn filter_list_contains_and_negated_contains_follow_python_contract() {
    let rows = vec![
        obj(json!({"name": "a", "tags": ["a", "b"]})),
        obj(json!({"name": "b", "tags": ["c", "d"]})),
    ];

    let output =
        apply_pipeline(rows.clone(), &["F tags=b".to_string()]).expect("list filter should work");
    assert_eq!(output, vec![obj(json!({"name": "a", "tags": ["a", "b"]}))]);

    let output =
        apply_pipeline(rows, &["F !tags=b".to_string()]).expect("negated list filter should work");
    assert_eq!(output, vec![obj(json!({"name": "b", "tags": ["c", "d"]}))]);
}

#[test]
fn filter_existence_treats_null_empty_string_and_empty_list_as_falsy() {
    let rows = vec![
        obj(json!({"name": "nullish", "val": null, "tags": []})),
        obj(json!({"name": "empty", "val": "", "tags": []})),
        obj(json!({"name": "present", "val": "x", "tags": ["prod"]})),
    ];

    let output =
        apply_pipeline(rows.clone(), &["F ?val".to_string()]).expect("value existence should work");
    assert_eq!(
        output,
        vec![obj(
            json!({"name": "present", "val": "x", "tags": ["prod"]})
        )]
    );

    let output =
        apply_pipeline(rows, &["F ?tags".to_string()]).expect("list existence should work");
    assert_eq!(
        output,
        vec![obj(
            json!({"name": "present", "val": "x", "tags": ["prod"]})
        )]
    );
}

#[test]
fn filter_datetime_comparison_handles_mixed_tz_naive_and_aware_values() {
    let rows = vec![
        obj(json!({"ts": "2026-02-13T20:00:00+00:00"})),
        obj(json!({"ts": "2026-02-12T08:00:00+00:00"})),
    ];

    let output = apply_pipeline(rows, &["F ts>2026-02-13 00:00:00".to_string()])
        .expect("datetime filter should parse");

    assert_eq!(output.len(), 1);
    assert_eq!(
        output[0].get("ts"),
        Some(&json!("2026-02-13T20:00:00+00:00"))
    );
}

#[test]
fn filter_boolean_string_matches_boolean_value() {
    let rows = vec![
        obj(json!({"name": "on", "active": true})),
        obj(json!({"name": "off", "active": false})),
    ];

    let output =
        apply_pipeline(rows, &["F active=true".to_string()]).expect("boolean filter should work");

    assert_eq!(output, vec![obj(json!({"name": "on", "active": true}))]);
}

#[test]
fn filter_numeric_equality_matches_numbers_and_numeric_strings() {
    let rows = vec![
        obj(json!({"id": 1, "name": "int"})),
        obj(json!({"id": "1", "name": "string"})),
        obj(json!({"id": 2, "name": "other"})),
    ];

    let output =
        apply_pipeline(rows, &["F id=1".to_string()]).expect("numeric equality should work");

    assert_eq!(
        output,
        vec![
            obj(json!({"id": 1, "name": "int"})),
            obj(json!({"id": "1", "name": "string"})),
        ]
    );
}
