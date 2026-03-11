use crate::core::{
    output_model::{Group, OutputItems},
    row::Row,
};
use anyhow::Result;
use jaq_core::{
    Ctx, Vars, data,
    load::{Arena, File, Loader},
    unwrap_valr,
};
use jaq_json::Val;
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum JqError {
    #[error("JQ expects a jq expression")]
    MissingExpression,
    #[error("failed to convert payload into JSON input")]
    SerializePayload {
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to compile jq expression: {message}")]
    CompileFailed { message: String },
    #[error("jq evaluation failed: {message}")]
    EvaluationFailed { message: String },
    #[error("jq output is not valid JSON")]
    InvalidJsonOutput {
        #[source]
        source: serde_json::Error,
    },
}

type JaqFilter = jaq_core::Filter<data::JustLut<Val>>;

struct JaqProgram {
    filter: JaqFilter,
}

pub(crate) fn compile(spec: &str) -> std::result::Result<String, JqError> {
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

pub(crate) fn apply_with_expr(items: OutputItems, expr: &str) -> Result<OutputItems> {
    let program = compile_program(expr)?;
    match items {
        OutputItems::Rows(rows) => Ok(OutputItems::Rows(apply_rows(rows, &program)?)),
        OutputItems::Groups(groups) => Ok(OutputItems::Groups(apply_groups(groups, &program)?)),
    }
}

pub(crate) fn apply_value_with_expr(value: Value, expr: &str) -> Result<Value> {
    let program = compile_program(expr)?;
    Ok(run_jaq(&program, &value)?.unwrap_or(Value::Null))
}

fn apply_rows(rows: Vec<Row>, program: &JaqProgram) -> Result<Vec<Row>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let payload = Value::Array(rows.into_iter().map(Value::Object).collect());
    match run_jaq(program, &payload)? {
        None => Ok(Vec::new()),
        Some(value) => Ok(json_to_rows(value)),
    }
}

fn apply_groups(groups: Vec<Group>, program: &JaqProgram) -> Result<Vec<Group>> {
    let mut out = Vec::with_capacity(groups.len());
    for group in groups {
        let payload = group_to_value(&group);
        match run_jaq(program, &payload)? {
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

// Keep the public DSL verb name `JQ`, but execute it in-process through jaq
// so the pipeline does not depend on an external executable or child-process
// timing.
fn compile_program(expr: &str) -> std::result::Result<JaqProgram, JqError> {
    let arena = Arena::default();
    let defs = jaq_core::defs()
        .chain(jaq_std::defs())
        .chain(jaq_json::defs());
    let loader = Loader::new(defs);
    let modules = loader
        .load(
            &arena,
            File {
                path: (),
                code: expr,
            },
        )
        .map_err(|errors| JqError::CompileFailed {
            message: format!("{errors:?}"),
        })?;
    let funs = jaq_core::funs()
        .chain(jaq_std::funs())
        .chain(jaq_json::funs());
    let filter = jaq_core::Compiler::default()
        .with_funs(funs)
        .compile(modules)
        .map_err(|errors| JqError::CompileFailed {
            message: format!("{errors:?}"),
        })?;
    Ok(JaqProgram { filter })
}

fn run_jaq(program: &JaqProgram, payload: &Value) -> std::result::Result<Option<Value>, JqError> {
    let input = serde_json::from_value::<Val>(payload.clone())
        .map_err(|source| JqError::SerializePayload { source })?;
    let ctx = Ctx::<data::JustLut<Val>>::new(&program.filter.lut, Vars::new([]));
    let mut values = Vec::new();
    for value in program.filter.id.run((ctx, input)).map(unwrap_valr) {
        let value = value.map_err(|err| JqError::EvaluationFailed {
            message: err.to_string(),
        })?;
        values.push(jaq_value_to_json(&value)?);
    }

    match values.len() {
        0 => Ok(None),
        1 => Ok(values.into_iter().next()),
        _ => Ok(Some(Value::Array(values))),
    }
}

fn jaq_value_to_json(value: &Val) -> std::result::Result<Value, JqError> {
    serde_json::from_str(&value.to_string()).map_err(|source| JqError::InvalidJsonOutput { source })
}

#[cfg(test)]
mod tests;
