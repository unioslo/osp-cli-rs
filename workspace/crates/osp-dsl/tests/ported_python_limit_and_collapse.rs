use osp_core::output_model::OutputResult;
use osp_dsl::apply_pipeline;
use serde_json::json;

fn obj(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    value.as_object().cloned().expect("fixture must be object")
}

fn output_rows(output: &OutputResult) -> &[serde_json::Map<String, serde_json::Value>] {
    output.as_rows().expect("expected row output")
}

#[test]
fn limit_positive() {
    let rows = vec![
        obj(json!({"host": "alpha"})),
        obj(json!({"host": "beta"})),
        obj(json!({"host": "gamma"})),
    ];

    let output = apply_pipeline(rows, &["L 1".to_string()]).expect("pipeline should pass");
    assert_eq!(output_rows(&output).len(), 1);
    assert_eq!(
        output_rows(&output)[0].get("host").and_then(|v| v.as_str()),
        Some("alpha")
    );
}

#[test]
fn limit_zero() {
    let rows = vec![obj(json!({"host": "alpha"})), obj(json!({"host": "beta"}))];
    let output = apply_pipeline(rows, &["L 0".to_string()]).expect("pipeline should pass");
    assert!(output_rows(&output).is_empty());
}

#[test]
fn limit_negative_count_takes_tail() {
    let rows = vec![
        obj(json!({"value": 0})),
        obj(json!({"value": 1})),
        obj(json!({"value": 2})),
        obj(json!({"value": 3})),
        obj(json!({"value": 4})),
    ];

    let output = apply_pipeline(rows, &["L -2".to_string()]).expect("pipeline should pass");
    let values = output_rows(&output)
        .iter()
        .filter_map(|row| row.get("value").and_then(|v| v.as_i64()))
        .collect::<Vec<_>>();
    assert_eq!(values, vec![3, 4]);
}

#[test]
fn limit_with_offset() {
    let rows = vec![
        obj(json!({"host": "alpha"})),
        obj(json!({"host": "beta"})),
        obj(json!({"host": "gamma"})),
    ];

    let output = apply_pipeline(rows, &["L 2 1".to_string()]).expect("pipeline should pass");
    assert_eq!(output_rows(&output).len(), 2);
    assert_eq!(
        output_rows(&output)[0].get("host").and_then(|v| v.as_str()),
        Some("beta")
    );
}

#[test]
fn collapse_passes_through_ungrouped_rows() {
    let rows = vec![
        obj(json!({"uid": "oistes"})),
        obj(json!({"uid": "andreasd"})),
    ];
    let output = apply_pipeline(rows.clone(), &["Z".to_string()]).expect("pipeline should pass");
    assert_eq!(output_rows(&output), rows.as_slice());
}
