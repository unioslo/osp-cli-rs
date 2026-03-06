use osp_core::output_model::OutputResult;
use osp_dsl::apply_pipeline;
use serde_json::{Map, Value, json};

fn obj(value: Value) -> Map<String, Value> {
    value.as_object().cloned().expect("fixture must be object")
}

fn output_rows(output: &OutputResult) -> &[Map<String, Value>] {
    output.as_rows().expect("expected row output")
}

#[test]
fn filter_matches_quoted_value_with_spaces() {
    let rows = vec![
        obj(json!({"name": "foo bar", "id": 1})),
        obj(json!({"name": "baz", "id": 2})),
    ];

    let output = apply_pipeline(rows, &[r#"F name="foo bar""#.to_string()])
        .expect("quoted value filter should work");

    let expected = vec![obj(json!({"name": "foo bar", "id": 1}))];
    assert_eq!(output_rows(&output), expected.as_slice());
}

#[test]
fn filter_matches_literal_pipe_inside_quoted_value() {
    let rows = vec![
        obj(json!({"note": "foo | bar", "id": 1})),
        obj(json!({"note": "plain", "id": 2})),
    ];

    let output = apply_pipeline(rows, &[r#"F note="foo | bar""#.to_string()])
        .expect("quoted pipe filter should work");

    let expected = vec![obj(json!({"note": "foo | bar", "id": 1}))];
    assert_eq!(output_rows(&output), expected.as_slice());
}

#[test]
fn filter_matches_escaped_quotes_inside_value() {
    let rows = vec![
        obj(json!({"note": "say \"hi\"", "id": 1})),
        obj(json!({"note": "say hi", "id": 2})),
    ];

    let output = apply_pipeline(rows, &[r#"F note="say \"hi\"""#.to_string()])
        .expect("escaped quote filter should work");

    let expected = vec![obj(json!({"note": "say \"hi\"", "id": 1}))];
    assert_eq!(output_rows(&output), expected.as_slice());
}

#[test]
fn filter_matches_single_quoted_value_ending_with_backslash() {
    let rows = vec![
        obj(json!({"path": "C:\\Temp\\", "id": 1})),
        obj(json!({"path": "C:\\Temp", "id": 2})),
    ];

    let output = apply_pipeline(rows, &[r#"F path='C:\Temp\'"#.to_string()])
        .expect("trailing backslash filter should work");

    let expected = vec![obj(json!({"path": "C:\\Temp\\", "id": 1}))];
    assert_eq!(output_rows(&output), expected.as_slice());
}
