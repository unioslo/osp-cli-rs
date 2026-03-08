use std::sync::{Mutex, OnceLock};
#[cfg(unix)]
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::core::output_model::{Group, OutputItems};
use serde_json::json;

use super::{
    apply, apply_rows, group_to_value, json_to_rows, normalize_expression, run_jq,
    run_jq_with_program, value_to_group,
};

fn row(value: serde_json::Value) -> crate::core::row::Row {
    value
        .as_object()
        .cloned()
        .expect("fixture should be an object")
}

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[test]
fn normalize_expression_trims_quotes_and_leading_pipe() {
    assert_eq!(
        normalize_expression(" '| .rows' ").expect("quoted expression should normalize"),
        ".rows"
    );
    assert_eq!(
        normalize_expression("| map(.uid)").expect("leading pipe should normalize"),
        "map(.uid)"
    );
    assert!(normalize_expression("   ").is_err());
    assert!(normalize_expression("'   '").is_err());
}

#[test]
fn json_helpers_round_trip_groups_and_scalar_rows() {
    let group = Group {
        groups: row(json!({"team": "ops"})),
        aggregates: row(json!({"count": 2})),
        rows: vec![row(json!({"uid": "oistes"}))],
    };
    let payload = group_to_value(&group);
    let restored = value_to_group(
        &json!({
            "groups": {"team": "eng"},
            "aggregates": {"count": 9},
            "rows": [{"uid": "andreasd"}]
        }),
        &group,
    )
    .expect("group payload should restore");
    assert_eq!(
        restored.groups.get("team").and_then(|value| value.as_str()),
        Some("eng")
    );
    assert_eq!(
        restored
            .aggregates
            .get("count")
            .and_then(|value| value.as_i64()),
        Some(9)
    );
    assert_eq!(
        restored.rows[0].get("uid").and_then(|value| value.as_str()),
        Some("andreasd")
    );

    let rows = json_to_rows(json!([{"uid": "oistes"}, 7]));
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].get("uid").and_then(|value| value.as_str()),
        Some("oistes")
    );
    assert_eq!(
        rows[1].get("value").and_then(|value| value.as_i64()),
        Some(7)
    );

    let fallback = value_to_group(&payload, &group).expect("original payload should round trip");
    assert_eq!(fallback.rows.len(), 1);
}

#[test]
fn run_jq_and_apply_cover_row_and_group_paths() {
    let _guard = env_lock().lock().expect("env lock should not be poisoned");
    let rows = vec![
        row(json!({"uid": "oistes"})),
        row(json!({"uid": "andreasd"})),
    ];
    let filtered = match apply_rows(rows.clone(), "map(select(.uid == \"oistes\"))") {
        Ok(filtered) => filtered,
        Err(err)
            if matches!(
                err.downcast_ref::<super::JqError>(),
                Some(super::JqError::ExecutableNotFound { .. })
            ) =>
        {
            return;
        }
        Err(err) => panic!("jq row filter should succeed: {err}"),
    };
    assert_eq!(filtered.len(), 1);
    assert_eq!(
        filtered[0].get("uid").and_then(|value| value.as_str()),
        Some("oistes")
    );

    let payload = json!([{"uid": "oistes"}]);
    let direct = match run_jq("map(.uid)", &payload) {
        Ok(direct) => direct,
        Err(super::JqError::ExecutableNotFound { .. }) => return,
        Err(err) => panic!("jq should run: {err}"),
    };
    assert_eq!(direct, Some(json!(["oistes"])));

    let none = match run_jq("empty", &payload) {
        Ok(none) => none,
        Err(super::JqError::ExecutableNotFound { .. }) => return,
        Err(err) => panic!("jq empty output should be allowed: {err}"),
    };
    assert!(none.is_none());

    let groups = OutputItems::Groups(vec![Group {
        groups: row(json!({"team": "ops"})),
        aggregates: row(json!({"count": 1})),
        rows: vec![row(json!({"uid": "oistes"}))],
    }]);
    let replaced = match apply(
        groups,
        "{groups:{team:\"eng\"}, aggregates:{count:9}, rows:[{uid:\"andreasd\"}]}",
    ) {
        Ok(replaced) => replaced,
        Err(err)
            if matches!(
                err.downcast_ref::<super::JqError>(),
                Some(super::JqError::ExecutableNotFound { .. })
            ) =>
        {
            return;
        }
        Err(err) => panic!("group replacement should succeed: {err}"),
    };
    let OutputItems::Groups(groups) = replaced else {
        panic!("expected grouped output");
    };
    assert_eq!(
        groups[0]
            .groups
            .get("team")
            .and_then(|value| value.as_str()),
        Some("eng")
    );
    assert_eq!(
        groups[0].rows[0]
            .get("uid")
            .and_then(|value| value.as_str()),
        Some("andreasd")
    );
}

#[test]
fn run_jq_reports_missing_binary_and_invalid_output() {
    let _guard = env_lock().lock().expect("env lock should not be poisoned");
    let missing = run_jq_with_program(
        "jq-this-command-should-not-exist",
        ".",
        &json!({"uid": "oistes"}),
    )
    .expect_err("missing jq binary should fail");
    assert!(matches!(missing, super::JqError::ExecutableNotFound { .. }));

    let invalid = match run_jq(".[]", &json!([1, 2])) {
        Err(err) => err,
        Ok(_) => panic!("non-json jq output should fail"),
    };
    if matches!(invalid, super::JqError::ExecutableNotFound { .. }) {
        return;
    }
    assert!(matches!(invalid, super::JqError::InvalidJsonOutput { .. }));
}

#[test]
fn normalize_expression_reports_typed_missing_expression_unit() {
    assert!(matches!(
        normalize_expression("   ").unwrap_err(),
        super::JqError::MissingExpression
    ));
    assert!(matches!(
        normalize_expression("'   '").unwrap_err(),
        super::JqError::MissingExpression
    ));
}

#[test]
fn jq_helpers_cover_empty_rows_and_group_fallbacks_unit() {
    let rows = apply_rows(Vec::new(), ".").expect("empty rows should succeed");
    assert!(rows.is_empty());

    let fallback = Group {
        groups: row(json!({"team": "ops"})),
        aggregates: row(json!({"count": 1})),
        rows: vec![row(json!({"uid": "oistes"}))],
    };
    let restored = value_to_group(&json!({"rows": [{"uid": "andreasd"}]}), &fallback)
        .expect("rows-only payload should still become a group");
    assert_eq!(
        restored.groups.get("team").and_then(|value| value.as_str()),
        Some("ops")
    );
    assert_eq!(
        restored
            .aggregates
            .get("count")
            .and_then(|value| value.as_i64()),
        Some(1)
    );
    assert!(value_to_group(&json!("scalar"), &fallback).is_none());
    assert!(value_to_group(&json!({"groups": {"team": "eng"}}), &fallback).is_none());
}

#[test]
fn apply_groups_handles_empty_and_row_style_jq_output_unit() {
    let _guard = env_lock().lock().expect("env lock should not be poisoned");
    let groups = vec![Group {
        groups: row(json!({"team": "ops"})),
        aggregates: row(json!({"count": 2})),
        rows: vec![
            row(json!({"uid": "oistes"})),
            row(json!({"uid": "andreasd"})),
        ],
    }];

    let emptied = match apply(OutputItems::Groups(groups.clone()), "empty") {
        Ok(emptied) => emptied,
        Err(err)
            if matches!(
                err.downcast_ref::<super::JqError>(),
                Some(super::JqError::ExecutableNotFound { .. })
            ) =>
        {
            return;
        }
        Err(err) => panic!("empty jq output should keep group metadata: {err}"),
    };
    let OutputItems::Groups(emptied_groups) = emptied else {
        panic!("expected grouped output");
    };
    assert_eq!(emptied_groups[0].groups.get("team"), Some(&json!("ops")));
    assert!(emptied_groups[0].rows.is_empty());

    let projected = apply(
        OutputItems::Groups(groups),
        "{groups, aggregates, rows: (.rows | map({uid}))}",
    )
    .expect("group jq output with explicit rows should succeed");
    let OutputItems::Groups(projected_groups) = projected else {
        panic!("expected grouped output");
    };
    assert_eq!(
        projected_groups[0].rows[0]
            .get("uid")
            .and_then(|value| value.as_str()),
        Some("oistes")
    );
    assert_eq!(projected_groups[0].rows.len(), 2);
}

#[test]
fn apply_dispatch_and_scalar_row_helpers_cover_remaining_paths_unit() {
    let _guard = env_lock().lock().expect("env lock should not be poisoned");
    let rows = vec![
        row(json!({"uid": "oistes"})),
        row(json!({"uid": "andreasd"})),
    ];

    let row_items = match apply(
        OutputItems::Rows(rows.clone()),
        "map(select(.uid == \"oistes\"))",
    ) {
        Ok(items) => items,
        Err(err)
            if matches!(
                err.downcast_ref::<super::JqError>(),
                Some(super::JqError::ExecutableNotFound { .. })
            ) =>
        {
            return;
        }
        Err(err) => panic!("row apply should succeed: {err}"),
    };
    let OutputItems::Rows(filtered_rows) = row_items else {
        panic!("expected row output");
    };
    assert_eq!(filtered_rows.len(), 1);

    let emptied = match apply(OutputItems::Rows(rows), "empty") {
        Ok(items) => items,
        Err(err)
            if matches!(
                err.downcast_ref::<super::JqError>(),
                Some(super::JqError::ExecutableNotFound { .. })
            ) =>
        {
            return;
        }
        Err(err) => panic!("empty row apply should succeed: {err}"),
    };
    let OutputItems::Rows(emptied_rows) = emptied else {
        panic!("expected row output");
    };
    assert!(emptied_rows.is_empty());

    let groups = vec![Group {
        groups: row(json!({"team": "ops"})),
        aggregates: row(json!({"count": 2})),
        rows: vec![row(json!({"uid": "oistes"}))],
    }];
    let projected = match apply(OutputItems::Groups(groups), ".rows | map({uid})") {
        Ok(items) => items,
        Err(err)
            if matches!(
                err.downcast_ref::<super::JqError>(),
                Some(super::JqError::ExecutableNotFound { .. })
            ) =>
        {
            return;
        }
        Err(err) => panic!("group fallback apply should succeed: {err}"),
    };
    let OutputItems::Groups(groups) = projected else {
        panic!("expected grouped output");
    };
    assert_eq!(groups[0].rows.len(), 1);
    assert_eq!(
        groups[0].rows[0]
            .get("uid")
            .and_then(|value| value.as_str()),
        Some("oistes")
    );

    let scalar_rows = json_to_rows(json!(7));
    assert_eq!(scalar_rows.len(), 1);
    assert_eq!(
        scalar_rows[0].get("value").and_then(|value| value.as_i64()),
        Some(7)
    );
}

#[cfg(unix)]
fn make_fake_jq(script_body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let mut path = std::env::temp_dir();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be valid")
        .as_nanos();
    path.push(format!("osp-cli-fake-jq-{nonce}.sh"));
    fs::write(&path, format!("#!/bin/sh\n{script_body}\n"))
        .expect("fake jq script should be written");
    let mut perms = fs::metadata(&path)
        .expect("fake jq metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).expect("fake jq script should be executable");
    path
}

#[cfg(unix)]
#[test]
fn run_jq_with_program_surfaces_nonzero_exit_variants_unit() {
    let no_stderr = make_fake_jq("exit 7");
    let err = run_jq_with_program(
        no_stderr.to_str().expect("path should be utf8"),
        ".",
        &json!({"uid": "oistes"}),
    )
    .expect_err("nonzero exit without stderr should fail");
    assert!(matches!(
        err,
        super::JqError::FailedWithoutStderr { status_code: 7 }
            | super::JqError::FailedWithStderr { status_code: 7, .. }
    ));

    let with_stderr = make_fake_jq("echo nope >&2\nexit 9");
    let err = run_jq_with_program(
        with_stderr.to_str().expect("path should be utf8"),
        ".",
        &json!({"uid": "oistes"}),
    )
    .expect_err("nonzero exit with stderr should fail");
    assert!(matches!(
        err,
        super::JqError::FailedWithStderr { status_code: 9, ref stderr } if stderr.trim() == "nope"
    ));

    let _ = fs::remove_file(no_stderr);
    let _ = fs::remove_file(with_stderr);
}

#[test]
fn normalize_expression_accepts_double_quoted_filters_unit() {
    assert_eq!(
        normalize_expression("\"| map(.uid)\"").expect("double quoted jq should normalize"),
        "map(.uid)"
    );
}
