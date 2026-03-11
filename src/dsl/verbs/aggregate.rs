use std::{cmp::Ordering, fmt::Display};

use crate::core::{output_model::OutputItems, row::Row};
use anyhow::{Result, anyhow};
use serde_json::Value;

use crate::dsl::{
    eval::resolve::{resolve_values, resolve_values_truthy},
    parse::key_spec::KeySpec,
    verbs::common::{parse_alias_after_as, parse_stage_words},
};

use super::json;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AggregateFn {
    Count,
    Sum,
    Avg,
    Min,
    Max,
}

#[derive(Debug, Clone)]
struct AggregateSpec {
    function: AggregateFn,
    column_raw: Option<String>,
    alias: String,
}

#[derive(Debug, Clone)]
pub(crate) struct AggregatePlan {
    spec: AggregateSpec,
}

pub(crate) fn compile(spec: &str) -> Result<AggregatePlan> {
    Ok(AggregatePlan {
        spec: parse_aggregate_spec(spec)?,
    })
}

pub(crate) fn apply_with_plan(items: OutputItems, plan: &AggregatePlan) -> Result<OutputItems> {
    match items {
        OutputItems::Rows(rows) => {
            let value = aggregate_rows(&rows, &plan.spec);
            let mut row = Row::new();
            row.insert(plan.spec.alias.clone(), value);
            Ok(OutputItems::Rows(vec![row]))
        }
        OutputItems::Groups(groups) => {
            let enriched = groups
                .into_iter()
                .map(|mut group| {
                    let value = aggregate_rows(&group.rows, &plan.spec);
                    group.aggregates.insert(plan.spec.alias.clone(), value);
                    group
                })
                .collect::<Vec<_>>();
            Ok(OutputItems::Groups(enriched))
        }
    }
}

/// Implements the `C` count macro.
///
/// Flat rows become a single `{count}` row. Grouped input becomes one summary
/// row per group with the original group headers plus `count`.
pub fn count_macro(items: OutputItems, spec: &str) -> Result<OutputItems> {
    if !spec.trim().is_empty() {
        return Err(anyhow!("C takes no arguments"));
    }

    match items {
        OutputItems::Rows(rows) => {
            let mut row = Row::new();
            row.insert("count".to_string(), Value::from(rows.len() as i64));
            Ok(OutputItems::Rows(vec![row]))
        }
        OutputItems::Groups(groups) => {
            let rows = groups
                .into_iter()
                .map(|group| {
                    let mut row = group.groups;
                    row.insert("count".to_string(), Value::from(group.rows.len() as i64));
                    row
                })
                .collect::<Vec<_>>();
            Ok(OutputItems::Rows(rows))
        }
    }
}

fn parse_aggregate_spec(spec: &str) -> Result<AggregateSpec> {
    let words = parse_stage_words(spec)?;

    if words.is_empty() {
        return Err(anyhow!("A requires an aggregate function"));
    }

    let (function, mut column_raw, from_parenthesized) = parse_function_and_column(&words[0])?;
    let mut index = 1usize;

    if column_raw.is_none() && index < words.len() {
        if function == AggregateFn::Count && words.len() == 2 {
        } else if !words[index].eq_ignore_ascii_case("AS") {
            column_raw = Some(words[index].clone());
            index += 1;
        }
    }

    let alias = if let Some(alias) = parse_alias_after_as(&words, index, "A")? {
        alias
    } else if index < words.len() {
        words[index].clone()
    } else if let Some(column) = &column_raw {
        if from_parenthesized {
            format!("{}({column})", function.as_str())
        } else {
            column.clone()
        }
    } else {
        function.default_alias().to_string()
    };

    Ok(AggregateSpec {
        function,
        column_raw,
        alias,
    })
}

fn parse_function_and_column(input: &str) -> Result<(AggregateFn, Option<String>, bool)> {
    if let Some(open) = input.find('(') {
        if !input.ends_with(')') {
            return Err(anyhow!("A: malformed function call"));
        }
        let function_name = &input[..open];
        let column = &input[open + 1..input.len() - 1];
        let function = AggregateFn::parse(function_name)?;
        let column = if column.trim().is_empty() {
            None
        } else {
            Some(column.trim().to_string())
        };
        return Ok((function, column, true));
    }

    let function = AggregateFn::parse(input)?;
    Ok((function, None, false))
}

fn aggregate_rows(rows: &[Row], spec: &AggregateSpec) -> Value {
    let values = collect_column_values(rows, spec.column_raw.as_deref());

    match spec.function {
        AggregateFn::Count => Value::from(count_values(&values) as i64),
        AggregateFn::Sum => Value::from(sum_values(&values)),
        AggregateFn::Avg => {
            let numbers = numeric_values(&values);
            if numbers.is_empty() {
                Value::from(0.0)
            } else {
                Value::from(numbers.iter().sum::<f64>() / numbers.len() as f64)
            }
        }
        AggregateFn::Min => min_value(&values).unwrap_or(Value::Null),
        AggregateFn::Max => max_value(&values).unwrap_or(Value::Null),
    }
}

fn collect_column_values(rows: &[Row], column_raw: Option<&str>) -> Vec<Value> {
    match column_raw {
        None => rows.iter().map(|_| Value::Bool(true)).collect(),
        Some(column_raw) => {
            let key_spec = KeySpec::parse(column_raw);
            if key_spec.existence {
                rows.iter()
                    .map(|row| {
                        let found = resolve_values_truthy(row, &key_spec.token, key_spec.exact);
                        Value::Bool(if key_spec.negated { !found } else { found })
                    })
                    .collect()
            } else {
                rows.iter()
                    .flat_map(|row| resolve_values(row, &key_spec.token, key_spec.exact))
                    .flat_map(expand_array_value)
                    .collect()
            }
        }
    }
}

fn expand_array_value(value: Value) -> Vec<Value> {
    match value {
        Value::Array(values) => values,
        scalar => vec![scalar],
    }
}

fn count_values(values: &[Value]) -> usize {
    values.iter().filter(|value| !value.is_null()).count()
}

fn sum_values(values: &[Value]) -> f64 {
    numeric_values(values).iter().sum()
}

fn numeric_values(values: &[Value]) -> Vec<f64> {
    values
        .iter()
        .filter_map(|value| match value {
            Value::Number(number) => number.as_f64(),
            Value::String(text) => text.parse::<f64>().ok(),
            Value::Bool(flag) => Some(if *flag { 1.0 } else { 0.0 }),
            _ => None,
        })
        .collect()
}

fn min_value(values: &[Value]) -> Option<Value> {
    values
        .iter()
        .filter(|value| !value.is_null())
        .min_by(|left, right| compare_values(left, right))
        .cloned()
}

fn max_value(values: &[Value]) -> Option<Value> {
    values
        .iter()
        .filter(|value| !value.is_null())
        .max_by(|left, right| compare_values(left, right))
        .cloned()
}

fn compare_values(left: &Value, right: &Value) -> Ordering {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => a
            .as_f64()
            .partial_cmp(&b.as_f64())
            .unwrap_or(Ordering::Equal),
        (Value::String(a), Value::String(b)) => a.cmp(b),
        _ => value_to_string(left).cmp(&value_to_string(right)),
    }
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

impl AggregateFn {
    fn parse(value: &str) -> Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "count" => Ok(Self::Count),
            "sum" => Ok(Self::Sum),
            "avg" => Ok(Self::Avg),
            "min" => Ok(Self::Min),
            "max" => Ok(Self::Max),
            other => Err(anyhow!("A: unsupported function '{other}'")),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Count => "count",
            Self::Sum => "sum",
            Self::Avg => "avg",
            Self::Min => "min",
            Self::Max => "max",
        }
    }

    fn default_alias(self) -> &'static str {
        match self {
            Self::Count => "count",
            Self::Sum => "sum",
            Self::Avg => "avg",
            Self::Min => "min",
            Self::Max => "max",
        }
    }
}

impl Display for AggregateFn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

pub(crate) fn apply_value_with_plan(value: Value, plan: &AggregatePlan) -> Result<Value> {
    json::traverse_collections(value, |items| apply_with_plan(items, plan))
}

pub(crate) fn count_macro_value(value: Value, spec: &str) -> Result<Value> {
    json::traverse_collections(value, |items| count_macro(items, spec))
}

#[cfg(test)]
mod tests;
