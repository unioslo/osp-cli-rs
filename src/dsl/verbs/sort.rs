use std::{cmp::Ordering, net::IpAddr};

use crate::core::{
    output_model::{Group, OutputItems},
    row::Row,
};
use anyhow::{Result, anyhow};
use serde_json::Value;

use crate::dsl::{
    eval::resolve::resolve_first_value,
    parse::key_spec::KeySpec,
    verbs::common::{parse_optional_alias_after_key, parse_stage_words},
};

use super::json;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortCast {
    Auto,
    Num,
    Str,
    Ip,
}

#[derive(Debug, Clone)]
struct SortKeySpec {
    key_spec: KeySpec,
    descending: bool,
    cast: SortCast,
}

#[derive(Debug, Clone)]
pub(crate) struct SortPlan {
    keys: Vec<SortKeySpec>,
}

pub(crate) fn compile(spec: &str) -> Result<SortPlan> {
    Ok(SortPlan {
        keys: parse_sort_spec(spec)?,
    })
}

pub(crate) fn apply_with_plan(items: OutputItems, plan: &SortPlan) -> Result<OutputItems> {
    match items {
        OutputItems::Rows(mut rows) => {
            rows.sort_by(|left, right| compare_rows(left, right, &plan.keys));
            Ok(OutputItems::Rows(rows))
        }
        OutputItems::Groups(mut groups) => {
            groups.sort_by(|left, right| compare_groups(left, right, &plan.keys));
            Ok(OutputItems::Groups(groups))
        }
    }
}

fn parse_sort_spec(spec: &str) -> Result<Vec<SortKeySpec>> {
    let words = parse_stage_words(spec)?;

    if words.is_empty() {
        return Err(anyhow!("S requires one or more keys"));
    }

    let mut keys = Vec::new();
    let mut index = 0usize;
    while index < words.len() {
        let token = &words[index];
        let (alias, consumed) = parse_optional_alias_after_key(&words, index, "S")?;

        let descending = token.starts_with('!');
        let raw_key = if descending { &token[1..] } else { token };
        let mut key = SortKeySpec {
            key_spec: KeySpec::parse(raw_key),
            descending,
            cast: SortCast::Auto,
        };

        if let Some(alias) = alias.as_deref() {
            key.cast = parse_sort_cast(alias)?;
        }
        index += consumed;

        keys.push(key);
    }

    Ok(keys)
}

fn parse_sort_cast(raw: &str) -> Result<SortCast> {
    match raw.to_ascii_lowercase().as_str() {
        "auto" => Ok(SortCast::Auto),
        "num" | "number" => Ok(SortCast::Num),
        "str" | "string" => Ok(SortCast::Str),
        "ip" => Ok(SortCast::Ip),
        other => Err(anyhow!("S: unsupported cast '{other}'")),
    }
}

fn compare_rows(left: &Row, right: &Row, keys: &[SortKeySpec]) -> Ordering {
    for key in keys {
        let left_value = resolve_first_value(left, &key.key_spec.token, key.key_spec.exact);
        let right_value = resolve_first_value(right, &key.key_spec.token, key.key_spec.exact);

        let mut ordering =
            compare_optional_values(left_value.as_ref(), right_value.as_ref(), key.cast);
        if key.descending {
            ordering = ordering.reverse();
        }

        if ordering != Ordering::Equal {
            return ordering;
        }
    }

    Ordering::Equal
}

fn compare_groups(left: &Group, right: &Group, keys: &[SortKeySpec]) -> Ordering {
    let left_row = merged_group_row(left);
    let right_row = merged_group_row(right);
    compare_rows(&left_row, &right_row, keys)
}

fn merged_group_row(group: &Group) -> Row {
    let mut merged = group.groups.clone();
    merged.extend(group.aggregates.clone());
    merged
}

fn compare_optional_values(
    left: Option<&Value>,
    right: Option<&Value>,
    cast: SortCast,
) -> Ordering {
    match (left, right) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(left), Some(right)) => compare_values(left, right, cast),
    }
}

fn compare_values(left: &Value, right: &Value, cast: SortCast) -> Ordering {
    match cast {
        SortCast::Num => compare_numbers(left, right),
        SortCast::Str => to_string_normalized(left).cmp(&to_string_normalized(right)),
        SortCast::Ip => compare_ip(left, right),
        SortCast::Auto => compare_auto(left, right),
    }
}

fn compare_auto(left: &Value, right: &Value) -> Ordering {
    let left_num = to_f64(left);
    let right_num = to_f64(right);
    if let (Some(left_num), Some(right_num)) = (left_num, right_num) {
        return left_num.partial_cmp(&right_num).unwrap_or(Ordering::Equal);
    }

    let left_ip = to_ip(left);
    let right_ip = to_ip(right);
    if let (Some(left_ip), Some(right_ip)) = (left_ip, right_ip) {
        return left_ip.cmp(&right_ip);
    }

    to_string_normalized(left).cmp(&to_string_normalized(right))
}

fn compare_numbers(left: &Value, right: &Value) -> Ordering {
    match (to_f64(left), to_f64(right)) {
        (Some(left_num), Some(right_num)) => {
            left_num.partial_cmp(&right_num).unwrap_or(Ordering::Equal)
        }
        _ => to_string_normalized(left).cmp(&to_string_normalized(right)),
    }
}

fn compare_ip(left: &Value, right: &Value) -> Ordering {
    match (to_ip(left), to_ip(right)) {
        (Some(left_ip), Some(right_ip)) => left_ip.cmp(&right_ip),
        _ => to_string_normalized(left).cmp(&to_string_normalized(right)),
    }
}

fn to_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.parse::<f64>().ok(),
        Value::Bool(flag) => Some(if *flag { 1.0 } else { 0.0 }),
        _ => None,
    }
}

fn to_ip(value: &Value) -> Option<IpAddr> {
    match value {
        Value::String(text) => text.parse::<IpAddr>().ok(),
        _ => None,
    }
}

fn to_string_normalized(value: &Value) -> String {
    match value {
        Value::String(text) => text.to_ascii_lowercase(),
        _ => value.to_string().to_ascii_lowercase(),
    }
}

pub(crate) fn apply_value_with_plan(value: Value, plan: &SortPlan) -> Result<Value> {
    match value {
        Value::Array(items) if json::is_collection_array(&items) => {
            json::apply_collection_stage(Value::Array(items), |items| apply_with_plan(items, plan))
        }
        Value::Array(mut items) => {
            items.sort_by(json::compare_scalar_values);
            Ok(Value::Array(items))
        }
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, child) in map {
                out.insert(key, apply_value_with_plan(child, plan)?);
            }
            Ok(Value::Object(out))
        }
        scalar => Ok(scalar),
    }
}
