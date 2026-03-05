use std::collections::HashMap;

use anyhow::{Result, anyhow};
use osp_core::{output_model::Group, row::Row};
use serde_json::Value;

use crate::{
    eval::resolve::{is_truthy, resolve_values},
    parse::key_spec::KeySpec,
    stages::common::{parse_optional_alias_after_key, parse_stage_words},
};

#[derive(Debug, Clone)]
struct GroupSpec {
    key_spec: KeySpec,
    header_key: String,
}

pub fn group_rows(rows: Vec<Row>, spec: &str) -> Result<Vec<Group>> {
    let specs = parse_group_specs(spec)?;
    let mut buckets: Vec<Group> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();

    for row in rows {
        let combinations = build_header_combinations(&row, &specs);
        for header in combinations {
            let key = serde_json::to_string(&header)
                .map_err(|error| anyhow!("failed to encode group key: {error}"))?;
            if let Some(existing) = index.get(&key) {
                buckets[*existing].rows.push(row.clone());
            } else {
                let position = buckets.len();
                index.insert(key, position);
                buckets.push(Group {
                    groups: header,
                    aggregates: Row::new(),
                    rows: vec![row.clone()],
                });
            }
        }
    }

    Ok(buckets)
}

pub fn regroup_groups(groups: Vec<Group>, spec: &str) -> Result<Vec<Group>> {
    let specs = parse_group_specs(spec)?;
    let mut buckets: Vec<Group> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();

    for group in groups {
        let base_headers = group.groups;
        for row in group.rows {
            let combinations = build_header_combinations(&row, &specs);
            for header in combinations {
                let mut merged_headers = base_headers.clone();
                merged_headers.extend(header);

                let key = serde_json::to_string(&merged_headers)
                    .map_err(|error| anyhow!("failed to encode regroup key: {error}"))?;
                if let Some(existing) = index.get(&key) {
                    buckets[*existing].rows.push(row.clone());
                } else {
                    let position = buckets.len();
                    index.insert(key, position);
                    buckets.push(Group {
                        groups: merged_headers,
                        aggregates: Row::new(),
                        rows: vec![row.clone()],
                    });
                }
            }
        }
    }

    Ok(buckets)
}

fn parse_group_specs(spec: &str) -> Result<Vec<GroupSpec>> {
    let words = parse_stage_words(spec)?;

    if words.is_empty() {
        return Err(anyhow!("G requires one or more keys"));
    }

    let mut specs = Vec::new();
    let mut index = 0usize;
    while index < words.len() {
        let token = words[index].clone();
        let (alias, consumed) = parse_optional_alias_after_key(&words, index, "G")?;
        let header_key = alias.unwrap_or_else(|| canonical_header_key(&token));
        index += consumed;

        specs.push(GroupSpec {
            key_spec: KeySpec::parse(&token),
            header_key,
        });
    }

    Ok(specs)
}

fn canonical_header_key(token: &str) -> String {
    token
        .trim_start_matches('!')
        .trim_start_matches('?')
        .trim_start_matches('=')
        .replace("[]", "")
        .trim_start_matches('.')
        .to_string()
}

fn build_header_combinations(row: &Row, specs: &[GroupSpec]) -> Vec<Row> {
    let mut combinations = vec![Row::new()];

    for spec in specs {
        let values = resolve_group_values(row, spec);
        let mut next = Vec::new();

        for combination in &combinations {
            for value in &values {
                let mut candidate = combination.clone();
                candidate.insert(spec.header_key.clone(), value.clone());
                next.push(candidate);
            }
        }

        combinations = next;
    }

    combinations
}

fn resolve_group_values(row: &Row, spec: &GroupSpec) -> Vec<Value> {
    if spec.key_spec.existence {
        let found = resolve_values(row, &spec.key_spec.token, spec.key_spec.exact)
            .iter()
            .any(is_truthy);
        let value = if spec.key_spec.negated { !found } else { found };
        return vec![Value::Bool(value)];
    }

    let values = resolve_values(row, &spec.key_spec.token, spec.key_spec.exact);
    if values.is_empty() {
        vec![Value::Null]
    } else {
        values
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::group_rows;

    #[test]
    fn group_by_scalar_key() {
        let rows = vec![
            json!({"dept": "sales", "id": 1})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"dept": "sales", "id": 2})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"dept": "eng", "id": 3})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let groups = group_rows(rows, "dept").expect("group should work");
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn group_with_alias() {
        let rows = vec![
            json!({"dept": "sales"})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let groups = group_rows(rows, "dept AS department").expect("group should work");
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0]
                .groups
                .get("department")
                .and_then(|value| value.as_str()),
            Some("sales")
        );
    }

    #[test]
    fn group_existence_token() {
        let rows = vec![
            json!({"name": "a", "vlan": "100"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"name": "b"}).as_object().cloned().expect("object"),
        ];

        let groups = group_rows(rows, "?vlan").expect("group should work");
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn group_fanout_from_list() {
        let rows = vec![
            json!({"name": "a", "tags": ["x", "y"]})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let groups = group_rows(rows, "tags[]").expect("group should work");
        assert_eq!(groups.len(), 2);
    }
}
