use serde_json::json;

use super::{
    JqError, apply_value_with_expr, apply_with_expr, compile, compile_program, run_jaq,
    value_to_group,
};
use crate::core::{
    output_model::{Group, OutputItems, rows_from_value},
    row::Row,
};

fn row(value: serde_json::Value) -> Row {
    value
        .as_object()
        .cloned()
        .expect("fixture should be an object")
}

#[test]
fn compile_rejects_empty_expressions_and_normalizes_quotes_unit() {
    assert!(matches!(compile("   "), Err(JqError::MissingExpression)));
    assert_eq!(compile("' .[] '").unwrap(), " .[] ");
    assert_eq!(compile("| map(.uid)").unwrap(), "map(.uid)");
}

#[test]
fn json_helpers_wrap_scalars_and_restore_group_fallback_metadata_unit() {
    assert_eq!(
        rows_from_value(json!(["alice", {"uid": "bob"}])),
        vec![row(json!({"value": "alice"})), row(json!({"uid": "bob"}))]
    );

    let fallback = Group {
        groups: row(json!({"team": "ops"})),
        aggregates: row(json!({"count": 2})),
        rows: vec![row(json!({"uid": "alice"}))],
    };
    let rebuilt = value_to_group(
        &json!({
            "rows": [{"uid": "bob"}]
        }),
        &fallback,
    )
    .expect("rows should rebuild a group");
    assert_eq!(rebuilt.groups, fallback.groups);
    assert_eq!(rebuilt.aggregates, fallback.aggregates);
    assert_eq!(rebuilt.rows, vec![row(json!({"uid": "bob"}))]);
    assert!(value_to_group(&json!({"uid": "alice"}), &fallback).is_none());
}

#[test]
fn run_jaq_reports_compile_eval_non_json_and_empty_output_unit() {
    assert!(matches!(
        compile_program(".["),
        Err(JqError::CompileFailed { .. })
    ));

    let failing = compile_program("error(\"boom\")").expect("program should compile");
    assert!(matches!(
        run_jaq(&failing, &json!(null)),
        Err(JqError::EvaluationFailed { .. })
    ));

    let non_json = compile_program("{(1): 2}").expect("program should compile");
    assert!(matches!(
        run_jaq(&non_json, &json!(null)),
        Err(JqError::InvalidJsonOutput { .. })
    ));

    let empty = compile_program("empty").expect("program should compile");
    assert_eq!(run_jaq(&empty, &json!(null)).unwrap(), None);
}

#[test]
fn run_jaq_collects_multiple_outputs_into_one_json_array_unit() {
    let program = compile_program(".[]").expect("program should compile");
    assert_eq!(
        run_jaq(&program, &json!(["alice", "bob"])).unwrap(),
        Some(json!(["alice", "bob"]))
    );
}

#[test]
fn apply_with_expr_uses_real_jaq_for_rows_groups_and_values_unit() {
    let rows = apply_with_expr(
        OutputItems::Rows(vec![
            row(json!({"uid": "alice", "team": "ops"})),
            row(json!({"uid": "bob", "team": "eng"})),
        ]),
        "map({uid})",
    )
    .expect("row jq should work");
    let OutputItems::Rows(row_items) = rows else {
        panic!("expected row output");
    };
    assert_eq!(
        row_items,
        vec![row(json!({"uid": "alice"})), row(json!({"uid": "bob"}))]
    );

    let groups = apply_with_expr(
        OutputItems::Groups(vec![Group {
            groups: row(json!({"team": "ops"})),
            aggregates: row(json!({"count": 2})),
            rows: vec![row(json!({"uid": "alice"})), row(json!({"uid": "bob"}))],
        }]),
        "{rows: (.rows | map({uid}))}",
    )
    .expect("group jq should work");
    let OutputItems::Groups(group_items) = groups else {
        panic!("expected grouped output");
    };
    assert_eq!(group_items[0].groups, row(json!({"team": "ops"})));
    assert_eq!(group_items[0].aggregates, row(json!({"count": 2})));
    assert_eq!(
        group_items[0].rows,
        vec![row(json!({"uid": "alice"})), row(json!({"uid": "bob"}))]
    );

    let value = apply_value_with_expr(json!([{"uid": "alice"}, {"uid": "bob"}]), "map(.uid)")
        .expect("value jq should work");
    assert_eq!(value, json!(["alice", "bob"]));
}

#[test]
fn apply_with_expr_returns_empty_rows_when_jaq_emits_no_output_unit() {
    let rows = apply_with_expr(OutputItems::Rows(Vec::new()), ".").expect("empty rows should work");
    let OutputItems::Rows(rendered_rows) = rows else {
        panic!("expected row output");
    };
    assert!(rendered_rows.is_empty());
}
