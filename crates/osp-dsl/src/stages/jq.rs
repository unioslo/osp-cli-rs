use std::process::{Command, Stdio};

use anyhow::{Result, anyhow};
use osp_core::{
    output_model::{Group, OutputItems},
    row::Row,
};
use serde_json::Value;

pub fn apply(items: OutputItems, spec: &str) -> Result<OutputItems> {
    let expr = normalize_expression(spec)?;
    match items {
        OutputItems::Rows(rows) => Ok(OutputItems::Rows(apply_rows(rows, &expr)?)),
        OutputItems::Groups(groups) => Ok(OutputItems::Groups(apply_groups(groups, &expr)?)),
    }
}

fn normalize_expression(spec: &str) -> Result<String> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("JQ expects a jq expression"));
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
        return Err(anyhow!("JQ expects a jq expression"));
    }
    Ok(expr)
}

fn apply_rows(rows: Vec<Row>, expr: &str) -> Result<Vec<Row>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let payload = Value::Array(rows.into_iter().map(|row| Value::Object(row)).collect());
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

fn run_jq(expr: &str, payload: &Value) -> Result<Option<Value>> {
    let input = serde_json::to_string(payload)?;
    let mut child = Command::new("jq")
        .arg(expr)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| anyhow!("jq executable not found in PATH: {err}"))?;

    if let Some(stdin) = child.stdin.as_mut() {
        use std::io::Write;
        stdin.write_all(input.as_bytes())?;
    }
    let output = child.wait_with_output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            return Err(anyhow!(
                "jq failed with status {}",
                output.status.code().unwrap_or(-1)
            ));
        }
        return Err(anyhow!("jq failed: {stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let parsed = serde_json::from_str::<Value>(trimmed)
        .map_err(|_| anyhow!("jq output is not valid JSON"))?;
    Ok(Some(parsed))
}
