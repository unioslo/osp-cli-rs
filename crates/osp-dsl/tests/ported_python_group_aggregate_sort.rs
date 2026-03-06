use osp_core::output_model::{OutputItems, OutputResult};
use osp_dsl::{apply_pipeline, execute_pipeline};
use serde_json::json;

fn obj(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    value.as_object().cloned().expect("fixture must be object")
}

fn output_rows(output: &OutputResult) -> &[serde_json::Map<String, serde_json::Value>] {
    output.as_rows().expect("expected row output")
}

#[test]
fn group_by_scalar_key() {
    let rows = vec![
        obj(json!({"dept": "USIT", "host": "alpha"})),
        obj(json!({"dept": "USIT", "host": "beta"})),
        obj(json!({"dept": null, "host": "standalone"})),
    ];

    let output = execute_pipeline(rows, &["G dept".to_string()]).expect("pipeline should pass");
    assert!(output.meta.grouped);

    match output.items {
        OutputItems::Groups(groups) => {
            assert_eq!(groups.len(), 2);
            let usit = groups
                .iter()
                .find(|group| group.groups.get("dept") == Some(&json!("USIT")))
                .expect("USIT group should exist");
            assert_eq!(usit.rows.len(), 2);
        }
        OutputItems::Rows(_) => panic!("expected grouped output"),
    }
}

#[test]
fn group_by_list_value_fanout() {
    let rows = vec![
        obj(json!({"host": "alpha", "netgroups": ["ansatt", "ucore"]})),
        obj(json!({"host": "beta", "netgroups": ["ansatt"]})),
        obj(json!({"host": "gamma", "netgroups": []})),
    ];

    let output =
        execute_pipeline(rows, &["G netgroups[]".to_string()]).expect("pipeline should pass");
    match output.items {
        OutputItems::Groups(groups) => {
            let labels = groups
                .iter()
                .map(|group| {
                    group
                        .groups
                        .get("netgroups")
                        .cloned()
                        .unwrap_or(json!(null))
                })
                .collect::<Vec<_>>();
            assert!(labels.contains(&json!("ansatt")));
            assert!(labels.contains(&json!("ucore")));
            assert!(labels.contains(&json!(null)));
        }
        OutputItems::Rows(_) => panic!("expected grouped output"),
    }
}

#[test]
fn aggregate_global_count_and_sum() {
    let rows = vec![
        obj(json!({"numbers": ["1", "10"]})),
        obj(json!({"numbers": ["2", "20"]})),
        obj(json!({"numbers": []})),
    ];

    let output = apply_pipeline(rows.clone(), &["A count total_hosts".to_string()])
        .expect("pipeline should pass");
    let expected = vec![obj(json!({"total_hosts": 3}))];
    assert_eq!(output_rows(&output), expected.as_slice());

    let output = apply_pipeline(rows, &["A sum(numbers[]) total_numbers".to_string()])
        .expect("pipeline should pass");
    assert_eq!(
        output_rows(&output)[0]
            .get("total_numbers")
            .and_then(|value| value.as_f64()),
        Some(33.0)
    );
}

#[test]
fn aggregate_avg_and_grouped_sum_then_collapse() {
    let rows = vec![
        obj(json!({"dept": "sales", "amount": 100})),
        obj(json!({"dept": "sales", "amount": 200})),
        obj(json!({"dept": "eng", "amount": 50})),
    ];

    let output = apply_pipeline(rows.clone(), &["A avg(amount) avg_amount".to_string()])
        .expect("pipeline should pass");
    assert_eq!(
        output_rows(&output)[0]
            .get("avg_amount")
            .and_then(|value| value.as_f64()),
        Some(350.0 / 3.0)
    );

    let output = apply_pipeline(
        rows,
        &[
            "G dept".to_string(),
            "A sum(amount) total".to_string(),
            "Z".to_string(),
            "S dept".to_string(),
        ],
    )
    .expect("pipeline should pass");

    assert_eq!(output_rows(&output).len(), 2);
    assert_eq!(
        output_rows(&output)[0]
            .get("dept")
            .and_then(|value| value.as_str()),
        Some("eng")
    );
    assert_eq!(
        output_rows(&output)[1]
            .get("dept")
            .and_then(|value| value.as_str()),
        Some("sales")
    );
}

#[test]
fn count_macro_matches_python_contract() {
    let rows = vec![
        obj(json!({"id": 1})),
        obj(json!({"id": 2})),
        obj(json!({"id": 3})),
    ];

    let output = apply_pipeline(rows, &["C".to_string()]).expect("pipeline should pass");
    let expected = vec![obj(json!({"count": 3}))];
    assert_eq!(output_rows(&output), expected.as_slice());
}

#[test]
fn sort_numeric_and_descending() {
    let rows = vec![
        obj(json!({"vlan": "300"})),
        obj(json!({"vlan": "75"})),
        obj(json!({"vlan": "100"})),
    ];

    let asc = apply_pipeline(rows.clone(), &["S vlan".to_string()]).expect("pipeline should pass");
    let asc_values = output_rows(&asc)
        .iter()
        .filter_map(|row| row.get("vlan").and_then(|value| value.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(asc_values, vec!["75", "100", "300"]);

    let desc = apply_pipeline(rows, &["S !vlan".to_string()]).expect("pipeline should pass");
    let desc_values = output_rows(&desc)
        .iter()
        .filter_map(|row| row.get("vlan").and_then(|value| value.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(desc_values, vec!["300", "100", "75"]);
}
