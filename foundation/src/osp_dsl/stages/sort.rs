use std::{cmp::Ordering, net::IpAddr};

use anyhow::{Result, anyhow};
use crate::osp_core::{
    output_model::{Group, OutputItems},
    row::Row,
};
use serde_json::Value;

use crate::osp_dsl::{
    eval::resolve::resolve_first_value,
    parse::key_spec::KeySpec,
    stages::common::{parse_optional_alias_after_key, parse_stage_words},
};

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

pub fn apply(items: OutputItems, spec: &str) -> Result<OutputItems> {
    let keys = parse_sort_spec(spec)?;

    match items {
        OutputItems::Rows(mut rows) => {
            rows.sort_by(|left, right| compare_rows(left, right, &keys));
            Ok(OutputItems::Rows(rows))
        }
        OutputItems::Groups(mut groups) => {
            groups.sort_by(|left, right| compare_groups(left, right, &keys));
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

        if let Some(alias) = alias {
            key.cast = parse_sort_cast(&alias)?;
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

#[cfg(test)]
mod tests {
    use crate::osp_core::output_model::{Group, OutputItems};
    use serde_json::json;

    use super::apply;

    #[test]
    fn sort_numeric_strings_ascending() {
        let rows = vec![
            json!({"vlan": "300"}).as_object().cloned().expect("object"),
            json!({"vlan": "75"}).as_object().cloned().expect("object"),
            json!({"vlan": "100"}).as_object().cloned().expect("object"),
        ];

        let output = apply(OutputItems::Rows(rows), "vlan").expect("sort should work");
        match output {
            OutputItems::Rows(rows) => {
                let values = rows
                    .iter()
                    .filter_map(|row| row.get("vlan").and_then(|value| value.as_str()))
                    .collect::<Vec<_>>();
                assert_eq!(values, vec!["75", "100", "300"]);
            }
            OutputItems::Groups(_) => panic!("expected rows"),
        }
    }

    #[test]
    fn sort_numeric_strings_descending() {
        let rows = vec![
            json!({"vlan": "300"}).as_object().cloned().expect("object"),
            json!({"vlan": "75"}).as_object().cloned().expect("object"),
            json!({"vlan": "100"}).as_object().cloned().expect("object"),
        ];

        let output = apply(OutputItems::Rows(rows), "!vlan").expect("sort should work");
        match output {
            OutputItems::Rows(rows) => {
                let values = rows
                    .iter()
                    .filter_map(|row| row.get("vlan").and_then(|value| value.as_str()))
                    .collect::<Vec<_>>();
                assert_eq!(values, vec!["300", "100", "75"]);
            }
            OutputItems::Groups(_) => panic!("expected rows"),
        }
    }

    #[test]
    fn sort_multiple_keys_with_direction() {
        let rows = vec![
            json!({"dept": "eng", "score": "100"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"dept": "sales", "score": "200"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"dept": "eng", "score": "50"})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let output = apply(OutputItems::Rows(rows), "dept !score").expect("sort should work");
        match output {
            OutputItems::Rows(rows) => {
                let result = rows
                    .iter()
                    .map(|row| {
                        (
                            row.get("dept")
                                .and_then(|value| value.as_str())
                                .unwrap_or_default(),
                            row.get("score")
                                .and_then(|value| value.as_str())
                                .unwrap_or_default(),
                        )
                    })
                    .collect::<Vec<_>>();
                assert_eq!(
                    result,
                    vec![("eng", "100"), ("eng", "50"), ("sales", "200")]
                );
            }
            OutputItems::Groups(_) => panic!("expected rows"),
        }
    }

    #[test]
    fn sort_uses_first_list_element_when_value_is_list() {
        let rows = vec![
            json!({"host": "a", "vlan": ["300"]})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"host": "b", "vlan": ["75"]})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"host": "c", "vlan": ["100"]})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let output = apply(OutputItems::Rows(rows), "vlan").expect("sort should work");
        match output {
            OutputItems::Rows(rows) => {
                let hosts = rows
                    .iter()
                    .filter_map(|row| row.get("host").and_then(|value| value.as_str()))
                    .collect::<Vec<_>>();
                assert_eq!(hosts, vec!["b", "c", "a"]);
            }
            OutputItems::Groups(_) => panic!("expected rows"),
        }
    }

    #[test]
    fn sort_supports_explicit_ip_and_string_casts() {
        let rows = vec![
            json!({"host": "b", "ip": "10.0.0.20"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"host": "a", "ip": "10.0.0.3"})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let output =
            apply(OutputItems::Rows(rows.clone()), "ip as ip").expect("ip sort should work");
        let OutputItems::Rows(rows) = output else {
            panic!("expected rows");
        };
        let hosts = rows
            .iter()
            .filter_map(|row| row.get("host").and_then(|value| value.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(hosts, vec!["a", "b"]);

        let output = apply(OutputItems::Rows(rows.clone()), "!host as str")
            .expect("string cast should work");
        let OutputItems::Rows(rows) = output else {
            panic!("expected rows");
        };
        let hosts = rows
            .iter()
            .filter_map(|row| row.get("host").and_then(|value| value.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(hosts, vec!["b", "a"]);
    }

    #[test]
    fn sort_places_missing_values_last_and_sorts_groups_using_merged_rows() {
        let rows = vec![
            json!({"host": "missing"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"host": "present", "score": "20"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"host": "small", "score": "5"})
                .as_object()
                .cloned()
                .expect("object"),
        ];
        let output = apply(OutputItems::Rows(rows), "score").expect("sort should work");
        let OutputItems::Rows(rows) = output else {
            panic!("expected rows");
        };
        let hosts = rows
            .iter()
            .filter_map(|row| row.get("host").and_then(|value| value.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(hosts, vec!["small", "present", "missing"]);

        let groups = vec![
            Group {
                groups: json!({"dept": "sales"})
                    .as_object()
                    .cloned()
                    .expect("object"),
                aggregates: json!({"count": 2}).as_object().cloned().expect("object"),
                rows: Vec::new(),
            },
            Group {
                groups: json!({"dept": "eng"}).as_object().cloned().expect("object"),
                aggregates: json!({"count": 5}).as_object().cloned().expect("object"),
                rows: Vec::new(),
            },
        ];
        let output = apply(OutputItems::Groups(groups), "!count").expect("group sort should work");
        let OutputItems::Groups(groups) = output else {
            panic!("expected groups");
        };
        let depts = groups
            .iter()
            .filter_map(|group| group.groups.get("dept").and_then(|value| value.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(depts, vec!["eng", "sales"]);
    }

    #[test]
    fn sort_rejects_missing_keys_and_unknown_casts() {
        let err =
            apply(OutputItems::Rows(Vec::new()), "").expect_err("missing sort spec should fail");
        assert!(err.to_string().contains("S requires one or more keys"));

        let rows = vec![json!({"host": "a"}).as_object().cloned().expect("object")];
        let err = apply(OutputItems::Rows(rows), "host as wat")
            .expect_err("unknown sort cast should fail");
        assert!(err.to_string().contains("unsupported cast 'wat'"));
    }

    #[test]
    fn sort_auto_cast_orders_booleans_before_strings() {
        let rows = vec![
            json!({"host": "c", "enabled": "zzz"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"host": "b", "enabled": true})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"host": "a", "enabled": false})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let output = apply(OutputItems::Rows(rows), "enabled").expect("sort should work");
        let OutputItems::Rows(rows) = output else {
            panic!("expected rows");
        };
        let hosts = rows
            .iter()
            .filter_map(|row| row.get("host").and_then(|value| value.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(hosts, vec!["a", "b", "c"]);
    }

    #[test]
    fn explicit_ip_and_number_casts_fall_back_to_string_comparison() {
        let rows = vec![
            json!({"host": "b", "ip": "banana", "score": "20a"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"host": "a", "ip": "apple", "score": "100a"})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let output = apply(OutputItems::Rows(rows.clone()), "ip as ip").expect("sort should work");
        let OutputItems::Rows(rows) = output else {
            panic!("expected rows");
        };
        let hosts = rows
            .iter()
            .filter_map(|row| row.get("host").and_then(|value| value.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(hosts, vec!["a", "b"]);

        let output =
            apply(OutputItems::Rows(rows.clone()), "score as num").expect("sort should work");
        let OutputItems::Rows(rows) = output else {
            panic!("expected rows");
        };
        let hosts = rows
            .iter()
            .filter_map(|row| row.get("host").and_then(|value| value.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(hosts, vec!["a", "b"]);
    }
}
