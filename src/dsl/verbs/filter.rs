#[cfg(test)]
use crate::core::output_model::Group;
use crate::core::row::Row;
use anyhow::{Result, anyhow};
use regex::Regex;

use crate::dsl::{
    eval::{
        matchers::{contains_case_insensitive, eq_case_insensitive, render_value},
        resolve::{resolve_values, resolve_values_truthy},
    },
    parse::key_spec::{ExactMode, KeySpec},
    verbs::common::parse_stage_words,
};

use super::selector;

#[derive(Debug, Clone)]
pub(crate) struct FilterPlan {
    parsed: ParsedFilterSpec,
}

impl FilterPlan {
    pub(crate) fn matches(&self, row: &Row) -> bool {
        evaluate_row(row, &self.parsed)
    }
}

pub(crate) fn compile(spec: &str) -> Result<FilterPlan> {
    Ok(FilterPlan {
        parsed: parse_filter_spec(spec)?,
    })
}

#[cfg(test)]
/// Filters flat rows according to the predicate in `spec`.
pub fn apply(rows: Vec<Row>, spec: &str) -> Result<Vec<Row>> {
    let plan = compile(spec)?;
    apply_with_plan(rows, &plan)
}

#[cfg(test)]
/// Filters grouped output according to `spec`.
///
/// Group headers and aggregates are tested first. Otherwise the predicate is
/// applied to member rows and empty groups are dropped.
pub fn apply_groups(groups: Vec<Group>, spec: &str) -> Result<Vec<Group>> {
    let plan = compile(spec)?;
    apply_groups_with_plan(groups, &plan)
}

#[cfg(test)]
pub(crate) fn apply_with_plan(rows: Vec<Row>, plan: &FilterPlan) -> Result<Vec<Row>> {
    let mut out = Vec::new();

    for row in rows {
        if plan.matches(&row) {
            out.push(row);
        }
    }

    Ok(out)
}

pub(crate) fn apply_set(
    mut set: crate::dsl::model::RowSet,
    plan: &FilterPlan,
) -> Result<crate::dsl::model::RowSet> {
    let grouped = set.grouped;
    set.partitions.retain_mut(|partition| {
        if grouped {
            let header = crate::core::output_model::group_header_row(partition);
            let selector = &plan.parsed.column.key_spec;
            if !resolve_values(&header, &selector.token, selector.exact).is_empty() {
                return plan.matches(&header);
            }
        }
        partition.rows.retain(|row| plan.matches(row));
        !grouped || !partition.rows.is_empty()
    });
    Ok(set)
}

#[cfg(test)]
pub(crate) fn apply_groups_with_plan(groups: Vec<Group>, plan: &FilterPlan) -> Result<Vec<Group>> {
    Ok(apply_set(
        crate::dsl::model::RowSet {
            partitions: groups,
            grouped: true,
        },
        plan,
    )?
    .partitions)
}

#[derive(Debug, Clone)]
struct ParsedFilterSpec {
    column: selector::CompiledSelector,
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
    regex: Option<Regex>,
}

fn parse_filter_spec(spec: &str) -> Result<ParsedFilterSpec> {
    let words = parse_stage_words(spec)?;

    if words.is_empty() {
        return Err(anyhow!("F requires a predicate"));
    }

    let column = selector::CompiledSelector::from_key_spec(KeySpec::parse(&words[0]));
    let mut index = 1usize;

    let mut operator = Operator::Eq;
    let mut rhs_token: Option<String> = None;
    let mut value = ComparisonValue {
        text: String::new(),
        exact: false,
        strict: false,
        negated: false,
        regex: None,
    };

    if let Some(token) = words.get(index) {
        if let Some(parsed_op) = parse_operator_token(token) {
            operator = parsed_op;
            index += 1;
            let rhs = words
                .get(index)
                .ok_or_else(|| anyhow!("F: missing value after operator"))?;
            rhs_token = Some(rhs.to_string());
            let rhs_spec = KeySpec::parse(rhs);
            value = ComparisonValue {
                text: rhs_spec.token,
                exact: matches!(parsed_op, Operator::Eq | Operator::Ne),
                strict: rhs_spec.exact == ExactMode::CaseSensitive,
                negated: rhs_spec.negated,
                regex: None,
            };
        } else {
            rhs_token = Some(token.to_string());
            let rhs_spec = KeySpec::parse(token);
            value = ComparisonValue {
                text: rhs_spec.token,
                exact: rhs_spec.exact != ExactMode::None,
                strict: rhs_spec.exact == ExactMode::CaseSensitive,
                negated: rhs_spec.negated,
                regex: None,
            };
        }
        index += 1;
    }

    // A date and clock separated by whitespace are one timestamp operand.
    // Consume both explicitly rather than silently treating it as midnight.
    if index + 1 == words.len() && parse_date(&value.text).is_some() {
        let timestamp = format!("{} {}", value.text, words[index]);
        if words[index].contains(':') {
            value.text = timestamp;
            index += 1;
        }
    }
    if index < words.len() {
        return Err(anyhow!(
            "F: unexpected trailing predicate text; chain predicates with `| F ...`"
        ));
    }

    if matches!(
        operator,
        Operator::Gt | Operator::Ge | Operator::Lt | Operator::Le
    ) && value.text.get(..10).and_then(parse_date).is_some()
        && parse_timestamp(&value.text).is_none()
    {
        return Err(anyhow!(
            "F: invalid or ambiguous local timestamp; specify an RFC3339 time with Z or an explicit UTC offset"
        ));
    }

    let original_operator = operator;
    if matches!(operator, Operator::Ne) {
        operator = Operator::Eq;
        value.exact = true;
    }

    let negated =
        column.key_spec.negated || matches!(original_operator, Operator::Ne) || value.negated;
    if matches!(operator, Operator::Regex) {
        value.regex =
            Some(Regex::new(&value.text).map_err(|err| anyhow!("F: invalid regex: {err}"))?);
    }

    let existence_check = column.key_spec.existence || rhs_token.is_none();

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
        let found =
            resolve_values_truthy(row, &spec.column.key_spec.token, spec.column.key_spec.exact);
        return if spec.column.key_spec.negated {
            !found
        } else {
            found
        };
    }

    let values = resolve_values(row, &spec.column.key_spec.token, spec.column.key_spec.exact);
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
        Operator::Regex => rhs
            .regex
            .as_ref()
            .is_some_and(|regex| regex.is_match(&render_value(value))),
        Operator::Eq | Operator::Ne => compare_text_or_bool(value, rhs),
    }
}

fn compare_numbers(left: &serde_json::Value, rhs: &str, operator: Operator) -> bool {
    let left_num = value_to_f64(left);
    let right_num = rhs.parse::<f64>().ok();
    if let (Some(left_num), Some(right_num)) = (left_num, right_num) {
        return match operator {
            Operator::Gt => left_num > right_num,
            Operator::Ge => left_num >= right_num,
            Operator::Lt => left_num < right_num,
            Operator::Le => left_num <= right_num,
            _ => false,
        };
    }

    let left_time = value_to_timestamp(left);
    let right_time = parse_timestamp(rhs);
    if let (Some(left_time), Some(right_time)) = (left_time, right_time) {
        return match operator {
            Operator::Gt => left_time > right_time,
            Operator::Ge => left_time >= right_time,
            Operator::Lt => left_time < right_time,
            Operator::Le => left_time <= right_time,
            _ => false,
        };
    }

    false
}

fn compare_text_or_bool(left: &serde_json::Value, rhs: &ComparisonValue) -> bool {
    let left_rendered = render_value(left);

    if let serde_json::Value::Bool(flag) = left {
        if eq_case_insensitive(&rhs.text, "true") {
            return *flag;
        }
        if eq_case_insensitive(&rhs.text, "false") {
            return !*flag;
        }
    }

    if rhs.strict {
        if rhs.exact {
            return left_rendered == rhs.text;
        }
        return left_rendered.contains(&rhs.text);
    }

    if rhs.exact {
        eq_case_insensitive(&left_rendered, &rhs.text)
    } else {
        contains_case_insensitive(&left_rendered, &rhs.text)
    }
}

fn value_to_f64(value: &serde_json::Value) -> Option<f64> {
    match value {
        serde_json::Value::Number(number) => number.as_f64(),
        serde_json::Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    }
}

fn value_to_timestamp(value: &serde_json::Value) -> Option<i64> {
    match value {
        serde_json::Value::String(text) => parse_timestamp(text),
        serde_json::Value::Number(number) => number.as_i64(),
        _ => None,
    }
}

pub(crate) fn parse_timestamp(input: &str) -> Option<i64> {
    use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
    let text = input.trim();
    if let Ok(time) = DateTime::parse_from_rfc3339(text) {
        return Some(time.timestamp());
    }
    let naive = [
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M",
    ]
    .iter()
    .find_map(|format| NaiveDateTime::parse_from_str(text, format).ok())
    .or_else(|| parse_date(text)?.and_hms_opt(0, 0, 0))?;
    Local
        .from_local_datetime(&naive)
        .single()
        .map(|time| time.timestamp())
}

fn parse_date(input: &str) -> Option<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(input, "%Y-%m-%d").ok()
}

#[cfg(test)]
mod tests {
    use crate::core::output_model::Group;
    use serde_json::json;

    use super::{apply, apply_groups, parse_filter_spec, parse_timestamp};

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
    fn invalid_regex_fails_at_parse_time() {
        let rows = vec![
            json!({"status": "active"})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let error = apply(rows, "status ~ [unterminated").expect_err("regex should fail");
        assert!(
            error.to_string().contains("invalid regex"),
            "unexpected error: {error}"
        );
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

    #[test]
    fn parses_timestamps_for_ordered_comparison() {
        assert_eq!(
            parse_timestamp("2026-02-13"),
            parse_timestamp("2026-02-13 00:00:00")
        );
        assert!(
            parse_timestamp("2026-02-13T20:00:00+00:00") > parse_timestamp("2026-02-13 00:00:00")
        );
    }

    #[test]
    fn groups_keep_matching_rows_when_headers_do_not_match() {
        let groups = vec![Group {
            groups: json!({"team": "ops"}).as_object().cloned().expect("object"),
            aggregates: serde_json::Map::new(),
            rows: vec![
                json!({"uid": "alice", "score": 9})
                    .as_object()
                    .cloned()
                    .expect("object"),
                json!({"uid": "bob", "score": 15})
                    .as_object()
                    .cloned()
                    .expect("object"),
            ],
        }];

        let output = apply_groups(groups, "score >= 10").expect("group filter should work");

        assert_eq!(output.len(), 1);
        assert_eq!(output[0].rows.len(), 1);
        assert_eq!(output[0].rows[0].get("uid"), Some(&json!("bob")));
    }

    #[test]
    fn supports_numeric_timestamp_and_missing_negated_comparisons() {
        let rows = vec![
            json!({"uid": "alice", "score": 10, "created": "2024-01-01T12:00:00Z"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"uid": "bob", "score": 2, "created": "2023-01-01T12:00:00Z"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"uid": "carol"})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let numeric = apply(rows.clone(), "score >= 10").expect("numeric comparison should work");
        assert_eq!(numeric.len(), 1);
        assert_eq!(numeric[0].get("uid"), Some(&json!("alice")));

        let timestamp =
            apply(rows.clone(), "created > 2023-12-31T23:59:59Z").expect("time comparison");
        assert_eq!(timestamp.len(), 1);
        assert_eq!(timestamp[0].get("uid"), Some(&json!("alice")));

        let negated_missing = apply(rows, "score != 2").expect("negated missing should work");
        assert_eq!(negated_missing.len(), 2);
        assert!(
            negated_missing
                .iter()
                .any(|row| row.get("uid") == Some(&json!("alice")))
        );
        assert!(
            negated_missing
                .iter()
                .any(|row| row.get("uid") == Some(&json!("carol")))
        );
    }

    #[test]
    fn supports_array_regex_and_boolean_matches() {
        let rows = vec![
            json!({"uid": "alice", "tags": ["dev", "ops"], "enabled": true})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"uid": "bob", "tags": ["sales"], "enabled": false})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let array_match = apply(rows.clone(), "tags ops").expect("array contains should work");
        assert_eq!(array_match.len(), 1);
        assert_eq!(array_match[0].get("uid"), Some(&json!("alice")));

        let regex = apply(rows.clone(), "uid ~ ^a").expect("regex match should work");
        assert_eq!(regex.len(), 1);
        assert_eq!(regex[0].get("uid"), Some(&json!("alice")));

        let boolean = apply(rows, "enabled false").expect("bool compare should work");
        assert_eq!(boolean.len(), 1);
        assert_eq!(boolean[0].get("uid"), Some(&json!("bob")));
    }

    #[test]
    fn parse_treats_prefixed_path_filters_as_structural_selectors() {
        let parsed =
            parse_filter_spec("!sections[0].entries[0].name").expect("filter should parse");

        assert!(parsed.column.is_structural());
    }
}
