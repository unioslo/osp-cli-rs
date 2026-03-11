use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use super::{
    JqError, apply_value_with_expr, apply_with_expr, compile, json_to_rows, run_jq_with_program,
    value_to_group,
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

fn write_script(name: &str, body: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("osp-cli-{name}-{unique}.sh"));
    fs::write(&path, body).expect("script should write");
    let mut perms = fs::metadata(&path).expect("script metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).expect("script perms");
    path
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
        json_to_rows(json!(["alice", {"uid": "bob"}])),
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
fn run_jq_with_program_covers_missing_failure_invalid_and_empty_output_unit() {
    assert!(matches!(
        run_jq_with_program("/definitely/missing/jq", ".", &json!(null)),
        Err(JqError::ExecutableNotFound { .. })
    ));

    let failing = write_script("jq-fail", "#!/bin/sh\nprintf 'boom\\n' >&2\nexit 7\n");
    let err = run_jq_with_program(failing.to_str().unwrap(), ".", &json!(null))
        .expect_err("failing program should error");
    assert!(matches!(
        err,
        JqError::FailedWithStderr { status_code: 7, .. }
    ));

    let silent = write_script("jq-silent", "#!/bin/sh\nexit 9\n");
    let err = run_jq_with_program(silent.to_str().unwrap(), ".", &json!(null))
        .expect_err("silent failure should error");
    assert!(matches!(
        err,
        JqError::FailedWithoutStderr { status_code: 9 }
    ));

    let invalid = write_script("jq-invalid", "#!/bin/sh\nprintf 'not-json'\n");
    assert!(matches!(
        run_jq_with_program(invalid.to_str().unwrap(), ".", &json!(null)),
        Err(JqError::InvalidJsonOutput { .. })
    ));

    let empty = write_script("jq-empty", "#!/bin/sh\nexit 0\n");
    assert_eq!(
        run_jq_with_program(empty.to_str().unwrap(), ".", &json!(null)).unwrap(),
        None
    );
}

#[test]
fn apply_with_expr_uses_real_jq_for_rows_groups_and_values_unit() {
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
fn apply_with_expr_returns_empty_rows_when_jq_emits_no_output_unit() {
    let rows = apply_with_expr(OutputItems::Rows(Vec::new()), ".").expect("empty rows should work");
    let OutputItems::Rows(rendered_rows) = rows else {
        panic!("expected row output");
    };
    assert!(rendered_rows.is_empty());
}
