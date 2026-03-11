use serde_json::json;

use super::{SortCast, apply_value_with_plan, apply_with_plan, compile};
use crate::core::{
    output_model::{Group, OutputItems},
    row::Row,
};

fn row(value: serde_json::Value) -> Row {
    value
        .as_object()
        .cloned()
        .expect("fixture should be an object")
}

#[test]
fn compile_parses_descending_keys_and_casts_unit() {
    let plan = compile("!score AS num host AS ip name AS str").expect("sort should compile");

    assert_eq!(plan.keys.len(), 3);
    assert!(plan.keys[0].descending);
    assert_eq!(plan.keys[0].key_spec.token, "score");
    assert_eq!(plan.keys[0].cast, SortCast::Num);
    assert_eq!(plan.keys[1].cast, SortCast::Ip);
    assert_eq!(plan.keys[2].cast, SortCast::Str);
}

#[test]
fn compile_rejects_empty_specs_and_unknown_casts_unit() {
    assert!(compile("").is_err());
    let err = compile("score AS bogus").expect_err("unknown cast should fail");
    assert!(err.to_string().contains("unsupported cast"));
}

#[test]
fn apply_with_plan_sorts_rows_and_groups_using_requested_casts_unit() {
    let rows = vec![
        row(json!({"score": "10", "host": "10.0.0.2", "name": "beta"})),
        row(json!({"score": 2, "host": "10.0.0.10", "name": "Alpha"})),
        row(json!({"name": "omega"})),
    ];

    let sorted = apply_with_plan(
        OutputItems::Rows(rows),
        &compile("score AS num host AS ip").unwrap(),
    )
    .expect("row sort should work");
    let OutputItems::Rows(sorted_rows) = sorted else {
        panic!("expected row output");
    };
    assert_eq!(
        sorted_rows
            .iter()
            .map(|row| row
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or(""))
            .collect::<Vec<_>>(),
        vec!["Alpha", "beta", "omega"]
    );

    let grouped = apply_with_plan(
        OutputItems::Groups(vec![
            Group {
                groups: row(json!({"team": "beta"})),
                aggregates: row(json!({"count": 1})),
                rows: Vec::new(),
            },
            Group {
                groups: row(json!({"team": "alpha"})),
                aggregates: row(json!({"count": 2})),
                rows: Vec::new(),
            },
        ]),
        &compile("count AS num team AS str").unwrap(),
    )
    .expect("group sort should work");
    let OutputItems::Groups(groups) = grouped else {
        panic!("expected grouped output");
    };
    assert_eq!(
        groups
            .iter()
            .map(|group| group
                .groups
                .get("team")
                .and_then(|value| value.as_str())
                .unwrap())
            .collect::<Vec<_>>(),
        vec!["beta", "alpha"]
    );
}

#[test]
fn apply_value_with_plan_sorts_scalars_and_nested_collection_rows_unit() {
    let sorted_scalars =
        apply_value_with_plan(json!(["b", "a", "c"]), &compile("ignored").unwrap())
            .expect("scalar arrays sort lexically");
    assert_eq!(sorted_scalars, json!(["a", "b", "c"]));

    let nested = json!({
        "hosts": [
            {"host": "10.0.0.10"},
            {"host": "10.0.0.2"}
        ]
    });
    let sorted_nested = apply_value_with_plan(nested, &compile("host AS ip").unwrap())
        .expect("nested collection sort should work");
    assert_eq!(
        sorted_nested,
        json!({
            "hosts": [
                {"host": "10.0.0.2"},
                {"host": "10.0.0.10"}
            ]
        })
    );
}
