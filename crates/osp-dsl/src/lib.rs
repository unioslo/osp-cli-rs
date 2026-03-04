use anyhow::{Result, anyhow};
use osp_core::row::Row;
use regex::Regex;
use serde_json::{Map, Value};

#[derive(Debug, Clone)]
pub struct Pipeline {
    pub command: String,
    pub stages: Vec<String>,
}

pub fn parse_pipeline(line: &str) -> Pipeline {
    let mut segments: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escape = false;

    for ch in line.chars() {
        if escape {
            current.push(ch);
            escape = false;
            continue;
        }
        if ch == '\\' {
            current.push(ch);
            escape = true;
            continue;
        }
        if ch == '\'' && !in_double {
            in_single = !in_single;
            current.push(ch);
            continue;
        }
        if ch == '"' && !in_single {
            in_double = !in_double;
            current.push(ch);
            continue;
        }
        if ch == '|' && !in_single && !in_double {
            let trimmed = current.trim();
            if !trimmed.is_empty() {
                segments.push(trimmed.to_string());
            } else {
                segments.push(String::new());
            }
            current.clear();
            continue;
        }
        current.push(ch);
    }
    if !current.trim().is_empty() {
        segments.push(current.trim().to_string());
    }

    let command = segments.first().cloned().unwrap_or_default();
    let stages = if segments.len() > 1 {
        segments[1..]
            .iter()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string())
            .collect()
    } else {
        Vec::new()
    };

    Pipeline { command, stages }
}

pub fn apply_pipeline(mut rows: Vec<Row>, stages: &[String]) -> Result<Vec<Row>> {
    for stage in stages {
        rows = apply_stage(rows, stage)?;
    }
    Ok(rows)
}

fn apply_stage(rows: Vec<Row>, stage: &str) -> Result<Vec<Row>> {
    let stage = stage.trim();
    if stage.is_empty() {
        return Ok(rows);
    }
    let (verb, spec) = split_verb(stage);
    match verb.as_str() {
        "P" => project(rows, &spec),
        "V" => values(rows, &spec),
        "F" => filter(rows, &spec),
        other => Err(anyhow!("unsupported DSL verb: {other}")),
    }
}

fn split_verb(stage: &str) -> (String, String) {
    let mut parts = stage.splitn(2, char::is_whitespace);
    let verb = parts.next().unwrap_or_default().to_ascii_uppercase();
    let spec = parts.next().unwrap_or_default().trim().to_string();
    if verb.is_empty() {
        ("".to_string(), stage.to_string())
    } else {
        (verb, spec)
    }
}

fn parse_keys(spec: &str) -> Vec<String> {
    spec.split(|c: char| c == ',' || c.is_whitespace())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn project(rows: Vec<Row>, spec: &str) -> Result<Vec<Row>> {
    let keys = parse_keys(spec);
    if keys.is_empty() {
        return Err(anyhow!("P requires one or more keys"));
    }

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let mut projected = Map::new();
        for key in &keys {
            if let Some(value) = row.get(key) {
                projected.insert(key.clone(), value.clone());
            }
        }
        out.push(projected);
    }

    Ok(out)
}

fn values(rows: Vec<Row>, spec: &str) -> Result<Vec<Row>> {
    let keys = parse_keys(spec);
    let mut out: Vec<Row> = Vec::new();

    for row in rows {
        if keys.is_empty() {
            for value in row.values() {
                emit_value_rows(&mut out, value);
            }
            continue;
        }

        for key in &keys {
            if let Some(value) = row.get(key) {
                emit_value_rows(&mut out, value);
            }
        }
    }

    Ok(out)
}

fn emit_value_rows(out: &mut Vec<Row>, value: &Value) {
    match value {
        Value::Array(values) => {
            for item in values {
                let mut row = Map::new();
                row.insert("value".to_string(), item.clone());
                out.push(row);
            }
        }
        _ => {
            let mut row = Map::new();
            row.insert("value".to_string(), value.clone());
            out.push(row);
        }
    }
}

fn filter(rows: Vec<Row>, spec: &str) -> Result<Vec<Row>> {
    if spec.trim().is_empty() {
        return Err(anyhow!("F requires a predicate"));
    }

    if let Some((key, op, rhs)) = split_predicate(spec) {
        let rhs = unquote(rhs.trim());
        let mut out = Vec::new();
        for row in rows {
            if let Some(value) = row.get(key.trim())
                && match_value(value, &op, rhs)
            {
                out.push(row);
            }
        }
        return Ok(out);
    }

    let query = spec.to_ascii_lowercase();
    let mut out = Vec::new();
    for row in rows {
        if row.iter().any(|(k, v)| {
            k.to_ascii_lowercase().contains(&query)
                || value_to_string(v).to_ascii_lowercase().contains(&query)
        }) {
            out.push(row);
        }
    }
    Ok(out)
}

fn split_predicate(spec: &str) -> Option<(&str, String, &str)> {
    for op in ["<=", ">=", "!=", "==", "~", "=", "<", ">"] {
        if let Some(index) = spec.find(op) {
            let left = &spec[..index];
            let right = &spec[index + op.len()..];
            return Some((left, op.to_string(), right));
        }
    }
    None
}

fn match_value(value: &Value, op: &str, rhs: &str) -> bool {
    if let Value::Array(items) = value {
        return items.iter().any(|item| match_value(item, op, rhs));
    }

    let left_str = value_to_string(value);
    match op {
        "=" => left_str.eq_ignore_ascii_case(rhs),
        "==" => left_str == rhs,
        "!=" => !left_str.eq_ignore_ascii_case(rhs),
        "~" => Regex::new(rhs)
            .map(|re| re.is_match(&left_str))
            .unwrap_or(false),
        ">" | "<" | ">=" | "<=" => compare_numbers(value, rhs, op),
        _ => false,
    }
}

fn compare_numbers(left: &Value, rhs: &str, op: &str) -> bool {
    let left_num = value_to_f64(left);
    let right_num = rhs.parse::<f64>().ok();
    let (Some(left_num), Some(right_num)) = (left_num, right_num) else {
        return false;
    };

    match op {
        ">" => left_num > right_num,
        "<" => left_num < right_num,
        ">=" => left_num >= right_num,
        "<=" => left_num <= right_num,
        _ => false,
    }
}

fn value_to_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(v) => v.to_string(),
        Value::Number(v) => v.to_string(),
        Value::String(v) => v.clone(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}

fn unquote(input: &str) -> &str {
    if input.len() >= 2
        && ((input.starts_with('"') && input.ends_with('"'))
            || (input.starts_with('\'') && input.ends_with('\'')))
    {
        return &input[1..input.len() - 1];
    }

    input
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{apply_pipeline, parse_pipeline};

    #[test]
    fn parse_pipeline_splits_command_and_stages() {
        let parsed = parse_pipeline("ldap user oistes | P uid,cn | F uid=oistes");
        assert_eq!(parsed.command, "ldap user oistes");
        assert_eq!(parsed.stages, vec!["P uid,cn", "F uid=oistes"]);
    }

    #[test]
    fn value_stage_expands_arrays() {
        let rows = vec![
            json!({"members": ["oistes", "andreasd"]})
                .as_object()
                .cloned()
                .expect("fixture must be object"),
        ];
        let output =
            apply_pipeline(rows, &["V members".to_string()]).expect("pipeline should pass");
        assert_eq!(output.len(), 2);
    }

    #[test]
    fn filter_stage_matches_key_equals() {
        let rows = vec![
            json!({"uid": "oistes"})
                .as_object()
                .cloned()
                .expect("fixture must be object"),
        ];
        let output =
            apply_pipeline(rows, &["F uid=oistes".to_string()]).expect("pipeline should pass");
        assert_eq!(output.len(), 1);
    }
}
