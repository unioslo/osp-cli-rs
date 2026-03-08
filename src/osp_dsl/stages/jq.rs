use std::io::{ErrorKind, Write};
use std::process::{Command, Stdio};
use std::thread;

use crate::osp_core::{
    output_model::{Group, OutputItems},
    row::Row,
};
use anyhow::Result;
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum JqError {
    #[error("JQ expects a jq expression")]
    MissingExpression,
    #[error("failed to encode jq payload")]
    SerializePayload {
        #[source]
        source: serde_json::Error,
    },
    #[error("jq executable not found in PATH: {source}")]
    ExecutableNotFound { source: std::io::Error },
    #[error("jq process I/O failed: {source}")]
    Io { source: std::io::Error },
    #[error("jq stdin writer thread panicked")]
    StdinWriterPanicked,
    #[error("jq failed with status {status_code}")]
    FailedWithoutStderr { status_code: i32 },
    #[error("jq failed: {stderr}")]
    FailedWithStderr { stderr: String },
    #[error("jq output is not valid JSON")]
    InvalidJsonOutput {
        #[source]
        source: serde_json::Error,
    },
}

pub fn apply(items: OutputItems, spec: &str) -> Result<OutputItems> {
    let expr = normalize_expression(spec)?;
    match items {
        OutputItems::Rows(rows) => Ok(OutputItems::Rows(apply_rows(rows, &expr)?)),
        OutputItems::Groups(groups) => Ok(OutputItems::Groups(apply_groups(groups, &expr)?)),
    }
}

fn normalize_expression(spec: &str) -> std::result::Result<String, JqError> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Err(JqError::MissingExpression);
    }

    let mut expr = trimmed.to_string();
    if expr.len() >= 2 {
        let first = expr.chars().next().unwrap_or_default();
        let last = expr.chars().last().unwrap_or_default();
        if (first == '"' || first == '\'') && first == last {
            expr = expr[1..expr.len() - 1].to_string();
        }
    }
    if let Some(rest) = expr.strip_prefix('|') {
        expr = rest.trim_start().to_string();
    }
    if expr.trim().is_empty() {
        return Err(JqError::MissingExpression);
    }
    Ok(expr)
}

fn apply_rows(rows: Vec<Row>, expr: &str) -> Result<Vec<Row>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let payload = Value::Array(rows.into_iter().map(Value::Object).collect());
    match run_jq(expr, &payload)? {
        None => Ok(Vec::new()),
        Some(value) => Ok(json_to_rows(value)),
    }
}

fn apply_groups(groups: Vec<Group>, expr: &str) -> Result<Vec<Group>> {
    let mut out = Vec::with_capacity(groups.len());
    for group in groups {
        let payload = group_to_value(&group);
        match run_jq(expr, &payload)? {
            None => out.push(Group {
                groups: group.groups,
                aggregates: group.aggregates,
                rows: Vec::new(),
            }),
            Some(value) => {
                if let Some(replacement) = value_to_group(&value, &group) {
                    out.push(replacement);
                } else {
                    let rows = json_to_rows(value);
                    out.push(Group {
                        groups: group.groups,
                        aggregates: group.aggregates,
                        rows,
                    });
                }
            }
        }
    }
    Ok(out)
}

fn group_to_value(group: &Group) -> Value {
    let rows = group
        .rows
        .iter()
        .cloned()
        .map(Value::Object)
        .collect::<Vec<_>>();
    let mut payload = serde_json::Map::new();
    payload.insert("groups".to_string(), Value::Object(group.groups.clone()));
    payload.insert(
        "aggregates".to_string(),
        Value::Object(group.aggregates.clone()),
    );
    payload.insert("rows".to_string(), Value::Array(rows));
    Value::Object(payload)
}

fn value_to_group(value: &Value, fallback: &Group) -> Option<Group> {
    let Value::Object(map) = value else {
        return None;
    };
    if !map.contains_key("rows") {
        return None;
    }

    let rows_value = map.get("rows").cloned().unwrap_or(Value::Array(Vec::new()));
    let groups_value = map.get("groups");
    let aggregates_value = map.get("aggregates");

    let groups = match groups_value {
        Some(Value::Object(obj)) => obj.clone(),
        _ => fallback.groups.clone(),
    };
    let aggregates = match aggregates_value {
        Some(Value::Object(obj)) => obj.clone(),
        _ => fallback.aggregates.clone(),
    };

    Some(Group {
        groups,
        aggregates,
        rows: json_to_rows(rows_value),
    })
}

fn json_to_rows(value: Value) -> Vec<Row> {
    match value {
        Value::Array(values) => values.into_iter().flat_map(json_value_to_row).collect(),
        other => json_value_to_row(other),
    }
}

fn json_value_to_row(value: Value) -> Vec<Row> {
    match value {
        Value::Object(map) => vec![map],
        other => {
            let mut row = Row::new();
            row.insert("value".to_string(), other);
            vec![row]
        }
    }
}

fn run_jq(expr: &str, payload: &Value) -> std::result::Result<Option<Value>, JqError> {
    run_jq_with_program("jq", expr, payload)
}

fn run_jq_with_program(
    program: &str,
    expr: &str,
    payload: &Value,
) -> std::result::Result<Option<Value>, JqError> {
    let input =
        serde_json::to_string(payload).map_err(|source| JqError::SerializePayload { source })?;
    let mut child = Command::new(program)
        .arg(expr)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| JqError::ExecutableNotFound { source })?;

    let writer = child.stdin.take().map(|mut stdin| {
        let input = input.into_bytes();
        thread::spawn(move || stdin.write_all(&input))
    });

    let output = child
        .wait_with_output()
        .map_err(|source| JqError::Io { source })?;
    if let Some(writer) = writer {
        match writer.join() {
            Ok(Ok(())) => {}
            Ok(Err(err)) if !output.status.success() && err.kind() == ErrorKind::BrokenPipe => {}
            Ok(Err(source)) => return Err(JqError::Io { source }),
            Err(_) => return Err(JqError::StdinWriterPanicked),
        }
    }

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            return Err(JqError::FailedWithoutStderr {
                status_code: output.status.code().unwrap_or(-1),
            });
        }
        return Err(JqError::FailedWithStderr { stderr });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let parsed = serde_json::from_str::<Value>(trimmed)
        .map_err(|source| JqError::InvalidJsonOutput { source })?;
    Ok(Some(parsed))
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use crate::osp_core::output_model::{Group, OutputItems};
    use serde_json::json;

    use super::{
        apply, apply_rows, group_to_value, json_to_rows, normalize_expression, run_jq,
        run_jq_with_program, value_to_group,
    };

    fn row(value: serde_json::Value) -> crate::osp_core::row::Row {
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

        let fallback =
            value_to_group(&payload, &group).expect("original payload should round trip");
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
}
