use anyhow::{Result, anyhow};
use osp_core::{output_model::Group, row::Row};
use regex::Regex;

use crate::{
    eval::{
        matchers::render_value,
        resolve::{resolve_values, resolve_values_truthy},
    },
    parse::key_spec::{ExactMode, KeySpec},
    stages::common::parse_stage_words,
};

pub fn apply(rows: Vec<Row>, spec: &str) -> Result<Vec<Row>> {
    let parsed = parse_filter_spec(spec)?;
    let mut out = Vec::new();

    for row in rows {
        if evaluate_row(&row, &parsed) {
            out.push(row);
        }
    }

    Ok(out)
}

pub fn apply_groups(groups: Vec<Group>, spec: &str) -> Result<Vec<Group>> {
    let parsed = parse_filter_spec(spec)?;
    let mut out = Vec::new();

    for mut group in groups {
        if evaluate_row(&group.groups, &parsed) || evaluate_row(&group.aggregates, &parsed) {
            out.push(group);
            continue;
        }

        group.rows.retain(|row| evaluate_row(row, &parsed));
        if !group.rows.is_empty() {
            out.push(group);
        }
    }

    Ok(out)
}

#[derive(Debug, Clone)]
struct ParsedFilterSpec {
    column: KeySpec,
    operator: Operator,
    value: Option<ComparisonValue>,
    negated: bool,
    existence_check: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Operator {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
    Regex,
}

#[derive(Debug, Clone)]
struct ComparisonValue {
    text: String,
    exact: bool,
    strict: bool,
    negated: bool,
}

fn parse_filter_spec(spec: &str) -> Result<ParsedFilterSpec> {
    let words = parse_stage_words(spec)?;

    if words.is_empty() {
        return Err(anyhow!("F requires a predicate"));
    }

    let column = KeySpec::parse(&words[0]);
    let mut index = 1usize;

    let mut operator = Operator::Eq;
    let mut rhs_token: Option<String> = None;
    let mut value = ComparisonValue {
        text: String::new(),
        exact: false,
        strict: false,
        negated: false,
    };

    if let Some(token) = words.get(index) {
        if let Some(parsed_op) = parse_operator_token(token) {
            operator = parsed_op;
            index += 1;
            let rhs = words
                .get(index)
                .ok_or_else(|| anyhow!("F: missing value after operator"))?;
            rhs_token = Some(rhs.clone());
            let rhs_spec = KeySpec::parse(rhs);
            value = ComparisonValue {
                text: rhs_spec.token,
                exact: matches!(parsed_op, Operator::Eq | Operator::Ne),
                strict: rhs_spec.exact == ExactMode::CaseSensitive,
                negated: rhs_spec.negated,
            };
        } else {
            rhs_token = Some(token.clone());
            let rhs_spec = KeySpec::parse(token);
            value = ComparisonValue {
                text: rhs_spec.token,
                exact: rhs_spec.exact != ExactMode::None,
                strict: rhs_spec.exact == ExactMode::CaseSensitive,
                negated: rhs_spec.negated,
            };
        }
    }

    if matches!(operator, Operator::Ne) {
        operator = Operator::Eq;
        value.exact = true;
    }

    let negated = column.negated
        || matches!(
            parse_operator_token(words.get(1).map(|s| s.as_str()).unwrap_or("")),
            Some(Operator::Ne)
        )
        || value.negated;

    let existence_check = column.existence || rhs_token.is_none();

    Ok(ParsedFilterSpec {
        column,
        operator,
        value: rhs_token.map(|_| value),
        negated,
        existence_check,
    })
}

fn parse_operator_token(token: &str) -> Option<Operator> {
    match token {
        "=" | "==" => Some(Operator::Eq),
        "!=" => Some(Operator::Ne),
        ">" => Some(Operator::Gt),
        ">=" => Some(Operator::Ge),
        "<" => Some(Operator::Lt),
        "<=" => Some(Operator::Le),
        "~" => Some(Operator::Regex),
        _ => None,
    }
}

fn evaluate_row(row: &Row, spec: &ParsedFilterSpec) -> bool {
    if spec.existence_check {
        let found = resolve_values_truthy(row, &spec.column.token, spec.column.exact);
        return if spec.column.negated { !found } else { found };
    }

    let values = resolve_values(row, &spec.column.token, spec.column.exact);
    if values.is_empty() {
        return spec.negated;
    }

    let Some(value_spec) = &spec.value else {
        return false;
    };

    let positive = values
        .iter()
        .any(|value| matches_value(value, spec.operator, value_spec));

    if spec.negated { !positive } else { positive }
}

fn matches_value(value: &serde_json::Value, operator: Operator, rhs: &ComparisonValue) -> bool {
    if let serde_json::Value::Array(items) = value {
        return items.iter().any(|item| matches_scalar(item, operator, rhs));
    }

    matches_scalar(value, operator, rhs)
}

fn matches_scalar(value: &serde_json::Value, operator: Operator, rhs: &ComparisonValue) -> bool {
    match operator {
        Operator::Gt | Operator::Ge | Operator::Lt | Operator::Le => {
            compare_numbers(value, &rhs.text, operator)
        }
        Operator::Regex => Regex::new(&rhs.text)
            .map(|regex| regex.is_match(&render_value(value)))
            .unwrap_or(false),
        Operator::Eq | Operator::Ne => compare_text_or_bool(value, rhs),
    }
}

fn compare_numbers(left: &serde_json::Value, rhs: &str, operator: Operator) -> bool {
    let left_num = value_to_f64(left);
    let right_num = rhs.parse::<f64>().ok();
    let (Some(left_num), Some(right_num)) = (left_num, right_num) else {
        return false;
    };

    match operator {
        Operator::Gt => left_num > right_num,
        Operator::Ge => left_num >= right_num,
        Operator::Lt => left_num < right_num,
        Operator::Le => left_num <= right_num,
        _ => false,
    }
}

fn compare_text_or_bool(left: &serde_json::Value, rhs: &ComparisonValue) -> bool {
    let left_rendered = render_value(left);

    if let serde_json::Value::Bool(flag) = left {
        if rhs.text.eq_ignore_ascii_case("true") {
            return *flag;
        }
        if rhs.text.eq_ignore_ascii_case("false") {
            return !*flag;
        }
    }

    if rhs.strict {
        if rhs.exact {
            return left_rendered == rhs.text;
        }
        return left_rendered.contains(&rhs.text);
    }

    let left_lower = left_rendered.to_ascii_lowercase();
    let rhs_lower = rhs.text.to_ascii_lowercase();
    if rhs.exact {
        left_lower == rhs_lower
    } else {
        left_lower.contains(&rhs_lower)
    }
}

fn value_to_f64(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(number) => number.as_f64(),
        serde_json::Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::apply;

    #[test]
    fn filters_on_equals_predicate() {
        let rows = vec![
            json!({"uid": "oistes"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"uid": "andreasd"})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let output = apply(rows, "uid=oistes").expect("filter should work");
        assert_eq!(output.len(), 1);
    }

    #[test]
    fn supports_spaced_contains_syntax() {
        let rows = vec![
            json!({"status": "active"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"status": "inactive"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"status": "pending"})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let output = apply(rows, "status active").expect("filter should work");
        assert_eq!(output.len(), 2);
    }

    #[test]
    fn supports_existence_checks() {
        let rows = vec![
            json!({"name": "a", "val": null})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"name": "b", "val": "x"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"name": "c"}).as_object().cloned().expect("object"),
        ];

        let output = apply(rows, "?val").expect("filter should work");
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].get("name").and_then(|v| v.as_str()), Some("b"));
    }

    #[test]
    fn supports_negated_missing_keys() {
        let rows = vec![
            json!({"name": "a", "val": 1})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"name": "b"}).as_object().cloned().expect("object"),
        ];

        let output = apply(rows, "!val=1").expect("filter should work");
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].get("name").and_then(|v| v.as_str()), Some("b"));
    }
}
