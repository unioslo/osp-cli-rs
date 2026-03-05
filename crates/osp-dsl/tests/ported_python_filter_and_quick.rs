use osp_dsl::apply_pipeline;
use serde_json::json;

fn obj(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    value.as_object().cloned().expect("fixture must be object")
}

#[test]
fn filter_no_matches() {
    let rows = vec![obj(json!({"name": "a"})), obj(json!({"name": "b"}))];
    let output =
        apply_pipeline(rows, &["F name=nonexistent".to_string()]).expect("pipeline should pass");
    assert!(output.is_empty());
}

#[test]
fn filter_contains_match() {
    let rows = vec![
        obj(json!({"status": "active"})),
        obj(json!({"status": "inactive"})),
        obj(json!({"status": "pending"})),
    ];
    let output =
        apply_pipeline(rows, &["F status active".to_string()]).expect("pipeline should pass");

    assert_eq!(output.len(), 2);
    let statuses = output
        .iter()
        .filter_map(|row| row.get("status").and_then(|value| value.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(statuses, vec!["active", "inactive"]);
}

#[test]
fn filter_case_sensitive_strict() {
    let rows = vec![obj(json!({"name": "ALPHA"})), obj(json!({"name": "alpha"}))];
    let output =
        apply_pipeline(rows, &["F name == ==alpha".to_string()]).expect("pipeline should pass");

    assert_eq!(output.len(), 1);
    assert_eq!(
        output[0].get("name").and_then(|value| value.as_str()),
        Some("alpha")
    );
}

#[test]
fn filter_missing_key_negated_matches() {
    let rows = vec![
        obj(json!({"name": "a", "val": 1})),
        obj(json!({"name": "b"})),
    ];

    let output = apply_pipeline(rows, &["F !val=1".to_string()]).expect("pipeline should pass");
    assert_eq!(output.len(), 1);
    assert_eq!(
        output[0].get("name").and_then(|value| value.as_str()),
        Some("b")
    );
}

#[test]
fn filter_not_equals_operator() {
    let rows = vec![
        obj(json!({"status": "active"})),
        obj(json!({"status": "inactive"})),
        obj(json!({"status": "pending"})),
    ];

    let output =
        apply_pipeline(rows, &["F status != active".to_string()]).expect("pipeline should pass");
    let statuses = output
        .iter()
        .filter_map(|row| row.get("status").and_then(|value| value.as_str()))
        .collect::<Vec<_>>();

    assert_eq!(statuses, vec!["inactive", "pending"]);
}

#[test]
fn filter_boolean_values() {
    let rows = vec![obj(json!({"active": true})), obj(json!({"active": false}))];
    let output =
        apply_pipeline(rows, &["F active=true".to_string()]).expect("pipeline should pass");
    assert_eq!(output.len(), 1);
    assert_eq!(
        output[0].get("active").and_then(|value| value.as_bool()),
        Some(true)
    );
}

#[test]
fn filter_list_contains_and_negated_contains() {
    let rows = vec![
        obj(json!({"tags": ["a", "b"]})),
        obj(json!({"tags": ["c", "d"]})),
    ];

    let output =
        apply_pipeline(rows.clone(), &["F tags=b".to_string()]).expect("pipeline should pass");
    assert_eq!(output.len(), 1);

    let output = apply_pipeline(rows, &["F !tags=b".to_string()]).expect("pipeline should pass");
    assert_eq!(output.len(), 1);
    assert_eq!(
        output[0]
            .get("tags")
            .and_then(|value| value.as_array())
            .map(|arr| arr.len()),
        Some(2)
    );
}

#[test]
fn filter_inline_numeric_tokens() {
    let rows = vec![
        obj(json!({"vlan": 100})),
        obj(json!({"vlan": 303})),
        obj(json!({"vlan": 70})),
    ];

    let output =
        apply_pipeline(rows.clone(), &["F vlan>75".to_string()]).expect("pipeline should pass");
    assert_eq!(output.len(), 2);

    let output =
        apply_pipeline(rows.clone(), &["F vlan>=303".to_string()]).expect("pipeline should pass");
    assert_eq!(output.len(), 1);

    let output = apply_pipeline(rows, &["F vlan==303".to_string()]).expect("pipeline should pass");
    assert_eq!(output.len(), 1);
}

#[test]
fn quick_stage_matches_default_scope() {
    let rows = vec![
        obj(json!({"uid": "oistes"})),
        obj(json!({"uid": "andreasd"})),
    ];

    let output = apply_pipeline(rows, &["oist".to_string()]).expect("pipeline should pass");
    assert_eq!(output.len(), 1);
    assert_eq!(
        output[0].get("uid").and_then(|value| value.as_str()),
        Some("oistes")
    );
}

#[test]
fn quick_stage_key_scope() {
    let rows = vec![obj(json!({"uid": "oistes"})), obj(json!({"cn": "Andreas"}))];

    let output = apply_pipeline(rows, &["K uid".to_string()]).expect("pipeline should pass");
    assert_eq!(output.len(), 1);
    assert!(output[0].contains_key("uid"));
}
