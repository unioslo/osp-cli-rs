use crate::core::{output_model::Group, row::Row};
use anyhow::{Result, anyhow};
use regex::Regex;
use serde_json::Value;

use crate::dsl::{
    eval::{
        matchers::{contains_case_insensitive, eq_case_insensitive, render_value},
        resolve::{is_truthy, resolve_values, resolve_values_truthy},
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

pub(crate) fn apply_with_plan(rows: Vec<Row>, plan: &FilterPlan) -> Result<Vec<Row>> {
    let mut out = Vec::new();

    for row in rows {
        if plan.matches(&row) {
            out.push(row);
        }
    }

    Ok(out)
}

pub(crate) fn apply_groups_with_plan(groups: Vec<Group>, plan: &FilterPlan) -> Result<Vec<Group>> {
    let mut out = Vec::new();

    for mut group in groups {
        if plan.matches(&group.groups) || plan.matches(&group.aggregates) {
            out.push(group);
            continue;
        }

        group.rows.retain(|row| plan.matches(row));
        if !group.rows.is_empty() {
            out.push(group);
        }
    }

    Ok(out)
}

pub(crate) fn apply_value_with_plan(value: Value, plan: &FilterPlan) -> Result<Value> {
    if let Some(filtered) = try_apply_addressed_filter(&value, &plan.parsed) {
        return Ok(filtered);
    }
    selector::filter_descendants(value, |row| plan.matches(row))
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

fn try_apply_addressed_filter(root: &Value, spec: &ParsedFilterSpec) -> Option<Value> {
    if !spec.column.is_structural() {
        return None;
    }

    let matches = spec.column.resolve_matches(root);
    if matches.is_empty() {
        return Some(if spec.negated {
            root.clone()
        } else {
            Value::Null
        });
    }

    let value_spec = spec.value.as_ref();
    let survivors = matches
        .into_iter()
        .filter(|entry| {
            let positive = if spec.existence_check {
                is_truthy(&entry.value)
            } else {
                let Some(value_spec) = value_spec else {
                    return false;
                };
                matches_value(&entry.value, spec.operator, value_spec)
            };
            if spec.negated { !positive } else { positive }
        })
        .collect::<Vec<_>>();

    if survivors.is_empty() {
        return Some(Value::Null);
    }

    Some(selector::project_matches(root, &survivors))
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
        _ => None,
    }
}

fn parse_timestamp(input: &str) -> Option<i64> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    let (date_part, time_part) = if let Some((date, time)) = trimmed.split_once('T') {
        (date, Some(time))
    } else if let Some((date, time)) = trimmed.split_once(' ') {
        (date, Some(time))
    } else {
        (trimmed, None)
    };

    let (year, month, day) = parse_date(date_part)?;
    let (hour, minute, second, offset_minutes) = match time_part {
        Some(time) => parse_time(time)?,
        None => (0, 0, 0, 0),
    };

    let days = days_from_civil(year, month, day);
    let seconds = days
        .checked_mul(86_400)?
        .checked_add(i64::from(hour) * 3_600)?
        .checked_add(i64::from(minute) * 60)?
        .checked_add(i64::from(second))?;
    seconds.checked_sub(i64::from(offset_minutes) * 60)
}

fn parse_date(input: &str) -> Option<(i32, u32, u32)> {
    let mut parts = input.split('-');
    let year = parts.next()?.parse::<i32>().ok()?;
    let month = parts.next()?.parse::<u32>().ok()?;
    let day = parts.next()?.parse::<u32>().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some((year, month, day))
}

fn parse_time(input: &str) -> Option<(u32, u32, u32, i32)> {
    let mut clock = input.trim();
    let mut offset_minutes = 0i32;

    if let Some(stripped) = clock.strip_suffix('Z') {
        clock = stripped;
    } else if let Some((time_part, offset_part)) = split_tz_offset(clock) {
        clock = time_part;
        offset_minutes = parse_offset_minutes(offset_part)?;
    }

    let mut parts = clock.split(':');
    let hour = parts.next()?.parse::<u32>().ok()?;
    let minute = parts.next()?.parse::<u32>().ok()?;
    let second = parts.next().and_then(parse_second_component).unwrap_or(0);
    if parts.next().is_some() || hour > 23 || minute > 59 || second > 59 {
        return None;
    }

    Some((hour, minute, second, offset_minutes))
}

fn split_tz_offset(input: &str) -> Option<(&str, &str)> {
    let bytes = input.as_bytes();
    for index in (1..bytes.len()).rev() {
        let ch = bytes[index] as char;
        if matches!(ch, '+' | '-') {
            return Some((&input[..index], &input[index..]));
        }
    }
    None
}

fn parse_offset_minutes(input: &str) -> Option<i32> {
    let sign = match input.as_bytes().first().copied()? as char {
        '+' => 1,
        '-' => -1,
        _ => return None,
    };
    let rest = &input[1..];
    let (hours, minutes) = if let Some((hours, minutes)) = rest.split_once(':') {
        (hours, minutes)
    } else if rest.len() == 4 {
        (&rest[..2], &rest[2..])
    } else {
        return None;
    };

    let hours = hours.parse::<i32>().ok()?;
    let minutes = minutes.parse::<i32>().ok()?;
    if hours > 23 || minutes > 59 {
        return None;
    }

    Some(sign * (hours * 60 + minutes))
}

fn parse_second_component(input: &str) -> Option<u32> {
    let whole = input
        .split_once('.')
        .map(|(whole, _)| whole)
        .unwrap_or(input);
    whole.parse::<u32>().ok()
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    // Howard Hinnant's civil-from-days algorithm:
    // https://howardhinnant.github.io/date_algorithms.html
    let year = i64::from(year) - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month = i64::from(month);
    let day = i64::from(day);
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
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
