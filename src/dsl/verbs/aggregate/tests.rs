use serde_json::json;

use super::{
    AggregateFn, apply_value_with_plan, apply_with_plan, compile, count_macro, count_macro_value,
};
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
fn compile_parses_functions_columns_and_aliases_unit() {
    let parenthesized = compile("avg(score) AS mean").expect("aggregate should compile");
    assert_eq!(parenthesized.spec.function, AggregateFn::Avg);
    assert_eq!(parenthesized.spec.column_raw.as_deref(), Some("score"));
    assert_eq!(parenthesized.spec.alias, "mean");

    let bare = compile("sum score").expect("aggregate should compile");
    assert_eq!(bare.spec.function, AggregateFn::Sum);
    assert_eq!(bare.spec.column_raw.as_deref(), Some("score"));
    assert_eq!(bare.spec.alias, "score");
}

#[test]
fn compile_rejects_empty_and_malformed_function_calls_unit() {
    assert!(compile("").is_err());
    let err = compile("avg(score").expect_err("malformed aggregate should fail");
    assert!(err.to_string().contains("malformed function call"));
}

#[test]
fn compile_uses_default_aliases_for_parenthesized_and_default_forms_unit() {
    let count = compile("count").expect("count should compile");
    assert_eq!(count.spec.function, AggregateFn::Count);
    assert_eq!(count.spec.alias, "count");

    let max = compile("max(score)").expect("parenthesized aggregate should compile");
    assert_eq!(max.spec.function, AggregateFn::Max);
    assert_eq!(max.spec.alias, "max(score)");
}

#[test]
fn apply_with_plan_aggregates_rows_and_grouped_rows_unit() {
    let rows = vec![
        row(json!({"score": 2, "present": true, "tags": ["a", "b"]})),
        row(json!({"score": "4.5", "present": false, "tags": ["c"]})),
        row(json!({"score": null, "present": true, "tags": []})),
    ];

    let summed = apply_with_plan(
        OutputItems::Rows(rows.clone()),
        &compile("sum score").unwrap(),
    )
    .expect("sum should work");
    let OutputItems::Rows(summed_rows) = summed else {
        panic!("expected row output");
    };
    assert_eq!(summed_rows, vec![row(json!({"score": 6.5}))]);

    let counted = apply_with_plan(
        OutputItems::Rows(rows.clone()),
        &compile("count ?present AS matched").unwrap(),
    )
    .expect("existence count should work");
    let OutputItems::Rows(counted_rows) = counted else {
        panic!("expected row output");
    };
    assert_eq!(counted_rows, vec![row(json!({"matched": 3}))]);

    let grouped = apply_with_plan(
        OutputItems::Groups(vec![Group {
            groups: row(json!({"team": "ops"})),
            aggregates: row(json!({})),
            rows,
        }]),
        &compile("max score AS top_score").unwrap(),
    )
    .expect("group aggregate should work");
    let OutputItems::Groups(groups) = grouped else {
        panic!("expected grouped output");
    };
    assert_eq!(groups[0].aggregates.get("top_score"), Some(&json!("4.5")));
}

#[test]
fn count_macros_cover_rows_groups_and_semantic_values_unit() {
    let rows = vec![row(json!({"uid": "alice"})), row(json!({"uid": "bob"}))];
    let counted = count_macro(OutputItems::Rows(rows), "").expect("count macro should work");
    let OutputItems::Rows(counted_rows) = counted else {
        panic!("expected row output");
    };
    assert_eq!(counted_rows, vec![row(json!({"count": 2}))]);

    let grouped = count_macro(
        OutputItems::Groups(vec![Group {
            groups: row(json!({"team": "ops"})),
            aggregates: row(json!({})),
            rows: vec![row(json!({"uid": "alice"}))],
        }]),
        "",
    )
    .expect("group count macro should work");
    let OutputItems::Rows(group_rows) = grouped else {
        panic!("expected row output");
    };
    assert_eq!(group_rows, vec![row(json!({"team": "ops", "count": 1}))]);

    let count_value = count_macro_value(json!([{"uid": "alice"}, {"uid": "bob"}]), "")
        .expect("semantic count should work");
    assert_eq!(count_value, json!([{ "count": 2 }]));

    let err = count_macro(OutputItems::Rows(vec![]), "extra").expect_err("C takes no args");
    assert!(err.to_string().contains("C takes no arguments"));
}

#[test]
fn apply_value_with_plan_traverses_nested_collections_unit() {
    let value = json!({
        "teams": [
            {"score": 2},
            {"score": "3.5"},
            {"score": null}
        ],
        "name": "ops"
    });

    let aggregated = apply_value_with_plan(value, &compile("avg(score) AS average").unwrap())
        .expect("aggregate value traversal should work");
    assert_eq!(
        aggregated,
        json!({
            "teams": [{ "average": 2.75 }],
            "name": "ops"
        })
    );
}

#[test]
fn aggregates_cover_empty_numeric_sets_and_ordered_extrema_unit() {
    let empty_avg = apply_with_plan(
        OutputItems::Rows(vec![row(json!({"score": null}))]),
        &compile("avg score AS average").unwrap(),
    )
    .expect("empty numeric set should still aggregate");
    let OutputItems::Rows(avg_rows) = empty_avg else {
        panic!("expected row output");
    };
    assert_eq!(avg_rows, vec![row(json!({"average": 0.0}))]);

    let extrema = apply_with_plan(
        OutputItems::Rows(vec![
            row(json!({"score": 2, "name": "beta"})),
            row(json!({"score": 1, "name": "alpha"})),
        ]),
        &compile("min name AS first").unwrap(),
    )
    .expect("min should work");
    let OutputItems::Rows(min_rows) = extrema else {
        panic!("expected row output");
    };
    assert_eq!(min_rows, vec![row(json!({"first": "alpha"}))]);
}
