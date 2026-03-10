use std::{cmp::Ordering, fmt::Display};

use crate::core::{output_model::OutputItems, row::Row};
use anyhow::{Result, anyhow};
use serde_json::Value;

use crate::dsl::{
    eval::resolve::{resolve_values, resolve_values_truthy},
    parse::key_spec::KeySpec,
    stages::common::{parse_alias_after_as, parse_stage_words},
};

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

/// Applies an aggregate stage to flat rows or grouped output.
///
/// Row input is reduced to a single output row. Grouped input stores the
/// aggregate result in each group's aggregate map.
pub fn apply(items: OutputItems, spec: &str) -> Result<OutputItems> {
    let parsed = parse_aggregate_spec(spec)?;
    match items {
        OutputItems::Rows(rows) => {
            let value = aggregate_rows(&rows, &parsed);
            let mut row = Row::new();
            row.insert(parsed.alias, value);
            Ok(OutputItems::Rows(vec![row]))
        }
        OutputItems::Groups(groups) => {
            let enriched = groups
                .into_iter()
                .map(|mut group| {
                    let value = aggregate_rows(&group.rows, &parsed);
                    group.aggregates.insert(parsed.alias.clone(), value);
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
            // `A count alias` form
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

#[cfg(test)]
mod tests {
    use crate::core::output_model::{Group, OutputItems};
    use serde_json::json;

    use super::{apply, count_macro};

    #[test]
    fn aggregate_count_global() {
        let rows = vec![
            json!({"id": 1}).as_object().cloned().expect("object"),
            json!({"id": 2}).as_object().cloned().expect("object"),
        ];

        let output = apply(OutputItems::Rows(rows), "count total").expect("aggregate should work");
        match output {
            OutputItems::Rows(rows) => {
                assert_eq!(
                    rows[0].get("total").and_then(|value| value.as_i64()),
                    Some(2)
                );
            }
            OutputItems::Groups(_) => panic!("expected rows"),
        }
    }

    #[test]
    fn aggregate_sum_and_avg() {
        let rows = vec![
            json!({"numbers": [1, 2]})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"numbers": [3]})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let output = apply(OutputItems::Rows(rows.clone()), "sum(numbers[]) total")
            .expect("aggregate should work");
        match output {
            OutputItems::Rows(rows) => {
                assert_eq!(
                    rows[0].get("total").and_then(|value| value.as_f64()),
                    Some(6.0)
                );
            }
            OutputItems::Groups(_) => panic!("expected rows"),
        }

        let output = apply(OutputItems::Rows(rows), "avg(numbers[]) average")
            .expect("aggregate should work");
        match output {
            OutputItems::Rows(rows) => {
                assert_eq!(
                    rows[0].get("average").and_then(|value| value.as_f64()),
                    Some(2.0)
                );
            }
            OutputItems::Groups(_) => panic!("expected rows"),
        }
    }

    #[test]
    fn aggregate_on_groups_adds_aggregates() {
        let groups = vec![Group {
            groups: json!({"dept": "sales"})
                .as_object()
                .cloned()
                .expect("object"),
            aggregates: serde_json::Map::new(),
            rows: vec![
                json!({"amount": 100}).as_object().cloned().expect("object"),
                json!({"amount": 200}).as_object().cloned().expect("object"),
            ],
        }];

        let output =
            apply(OutputItems::Groups(groups), "sum(amount) total").expect("aggregate should work");
        match output {
            OutputItems::Groups(groups) => {
                assert_eq!(
                    groups[0]
                        .aggregates
                        .get("total")
                        .and_then(|value| value.as_f64()),
                    Some(300.0)
                );
            }
            OutputItems::Rows(_) => panic!("expected groups"),
        }
    }

    #[test]
    fn count_macro_returns_count_rows() {
        let rows = vec![
            json!({"id": 1}).as_object().cloned().expect("object"),
            json!({"id": 2}).as_object().cloned().expect("object"),
        ];

        let output = count_macro(OutputItems::Rows(rows), "").expect("count should work");
        match output {
            OutputItems::Rows(rows) => {
                assert_eq!(
                    rows[0].get("count").and_then(|value| value.as_i64()),
                    Some(2)
                );
            }
            OutputItems::Groups(_) => panic!("expected rows"),
        }
    }

    #[test]
    fn aggregate_supports_min_max_and_existence_count() {
        let rows = vec![
            json!({"score": 10, "enabled": true, "name": "beta"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"score": 3, "enabled": false, "name": "alpha"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"name": "gamma"})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let min = apply(OutputItems::Rows(rows.clone()), "min(score) lowest")
            .expect("min aggregate should work");
        let OutputItems::Rows(min_rows) = min else {
            panic!("expected row output");
        };
        assert_eq!(
            min_rows[0].get("lowest").and_then(|value| value.as_i64()),
            Some(3)
        );

        let max = apply(OutputItems::Rows(rows.clone()), "max(name) highest")
            .expect("max aggregate should work");
        let OutputItems::Rows(max_rows) = max else {
            panic!("expected row output");
        };
        assert_eq!(
            max_rows[0].get("highest").and_then(|value| value.as_str()),
            Some("gamma")
        );

        let count = apply(OutputItems::Rows(rows), "count(?enabled) enabled_count")
            .expect("existence count should work");
        let OutputItems::Rows(count_rows) = count else {
            panic!("expected row output");
        };
        assert_eq!(
            count_rows[0]
                .get("enabled_count")
                .and_then(|value| value.as_i64()),
            Some(3)
        );
    }

    #[test]
    fn aggregate_parses_default_aliases_and_group_count_macro() {
        let rows = vec![
            json!({"amount": 4}).as_object().cloned().expect("object"),
            json!({"amount": 6}).as_object().cloned().expect("object"),
        ];
        let summed = apply(OutputItems::Rows(rows), "sum(amount)").expect("sum should work");
        let OutputItems::Rows(rows) = summed else {
            panic!("expected row output");
        };
        assert_eq!(
            rows[0].get("sum(amount)").and_then(|value| value.as_f64()),
            Some(10.0)
        );

        let grouped = OutputItems::Groups(vec![Group {
            groups: json!({"dept": "sales"})
                .as_object()
                .cloned()
                .expect("object"),
            aggregates: serde_json::Map::new(),
            rows: vec![
                json!({"id": 1}).as_object().cloned().expect("object"),
                json!({"id": 2}).as_object().cloned().expect("object"),
            ],
        }]);
        let counted = count_macro(grouped, "").expect("count macro should work for groups");
        let OutputItems::Rows(rows) = counted else {
            panic!("expected row output");
        };
        assert_eq!(
            rows[0].get("dept").and_then(|value| value.as_str()),
            Some("sales")
        );
        assert_eq!(
            rows[0].get("count").and_then(|value| value.as_i64()),
            Some(2)
        );
    }

    #[test]
    fn aggregate_rejects_invalid_forms() {
        let rows = OutputItems::Rows(vec![json!({"id": 1}).as_object().cloned().expect("object")]);

        let missing_fn = apply(rows.clone(), "").expect_err("missing function should fail");
        assert!(
            missing_fn
                .to_string()
                .contains("A requires an aggregate function")
        );

        let malformed = apply(rows.clone(), "sum(id").expect_err("malformed function should fail");
        assert!(malformed.to_string().contains("malformed function call"));

        let unsupported =
            apply(rows.clone(), "median(id)").expect_err("unsupported function should fail");
        assert!(
            unsupported
                .to_string()
                .contains("unsupported function 'median'")
        );

        let count_err = count_macro(rows, "extra").expect_err("C should reject arguments");
        assert!(count_err.to_string().contains("C takes no arguments"));
    }

    #[test]
    fn aggregate_supports_alias_after_as_and_mixed_numeric_inputs() {
        let rows = vec![
            json!({"value": "4"}).as_object().cloned().expect("object"),
            json!({"value": true}).as_object().cloned().expect("object"),
            json!({"value": 2}).as_object().cloned().expect("object"),
        ];

        let output =
            apply(OutputItems::Rows(rows), "sum(value) AS total").expect("sum alias should work");
        let OutputItems::Rows(rows) = output else {
            panic!("expected row output");
        };
        assert_eq!(
            rows[0].get("total").and_then(|value| value.as_f64()),
            Some(7.0)
        );
    }

    #[test]
    fn aggregate_handles_empty_inputs_and_parenthesized_count_aliases() {
        let empty_rows = OutputItems::Rows(vec![
            json!({"value": null}).as_object().cloned().expect("object"),
        ]);

        let avg = apply(empty_rows.clone(), "avg(value) average").expect("avg should work");
        let OutputItems::Rows(avg_rows) = avg else {
            panic!("expected row output");
        };
        assert_eq!(
            avg_rows[0].get("average").and_then(|value| value.as_f64()),
            Some(0.0)
        );

        let min = apply(empty_rows, "min(value) lowest").expect("min should work");
        let OutputItems::Rows(min_rows) = min else {
            panic!("expected row output");
        };
        assert_eq!(min_rows[0].get("lowest"), Some(&json!(null)));

        let count_rows = vec![
            json!({"enabled": true})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"enabled": false})
                .as_object()
                .cloned()
                .expect("object"),
        ];
        let counted =
            apply(OutputItems::Rows(count_rows), "count(enabled) AS matches").expect("count");
        let OutputItems::Rows(rows) = counted else {
            panic!("expected row output");
        };
        assert_eq!(
            rows[0].get("matches").and_then(|value| value.as_i64()),
            Some(2)
        );
    }

    #[test]
    fn aggregate_prefers_alias_token_for_two_word_count_form() {
        let rows = vec![
            json!({"id": 1}).as_object().cloned().expect("object"),
            json!({"id": 2}).as_object().cloned().expect("object"),
            json!({"id": 3}).as_object().cloned().expect("object"),
        ];

        let output = apply(OutputItems::Rows(rows), "count total").expect("count should work");
        let OutputItems::Rows(rows) = output else {
            panic!("expected row output");
        };
        assert_eq!(
            rows[0].get("total").and_then(|value| value.as_i64()),
            Some(3)
        );
    }

    #[test]
    fn aggregate_space_separated_column_form_keeps_column_name_as_alias() {
        let rows = vec![
            json!({"amount": 4}).as_object().cloned().expect("object"),
            json!({"amount": 6}).as_object().cloned().expect("object"),
        ];

        let output = apply(OutputItems::Rows(rows), "sum amount").expect("sum should work");
        let OutputItems::Rows(rows) = output else {
            panic!("expected row output");
        };
        assert_eq!(
            rows[0].get("amount").and_then(|value| value.as_f64()),
            Some(10.0)
        );
    }
}
