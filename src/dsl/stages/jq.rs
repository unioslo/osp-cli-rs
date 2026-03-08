use std::io::{ErrorKind, Write};
use std::process::{Command, Stdio};
use std::thread;

use crate::core::{
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
    #[error("jq failed with status {status_code}: {stderr}")]
    FailedWithStderr { status_code: i32, stderr: String },
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
        return Err(JqError::FailedWithStderr {
            status_code: output.status.code().unwrap_or(-1),
            stderr,
        });
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
mod tests;
