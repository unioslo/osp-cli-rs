use std::collections::HashMap;

use anyhow::{Result, anyhow};
use osp_core::{output_model::Group, row::Row};
use serde_json::Value;

use crate::{
    eval::{
        flatten::{coalesce_flat_row, flatten_row},
        matchers::match_row_keys_detailed,
        resolve::{enumerate_path_values, is_truthy},
    },
    parse::{
        key_spec::{ExactMode, KeySpec},
        path::{PathExpression, expression_to_flat_key, parse_path},
    },
    stages::common::{parse_optional_alias_after_key, parse_stage_words},
};

#[derive(Debug, Clone)]
struct GroupSpec {
    key_spec: KeySpec,
    header_key: String,
    flat_hint: Option<String>,
    allow_multiple: bool,
}

pub fn group_rows(rows: Vec<Row>, spec: &str) -> Result<Vec<Group>> {
    let specs = parse_group_specs(spec)?;
    let mut buckets: Vec<Group> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();

    for row in rows {
        let flat = flatten_row(&row);
        let combinations = build_header_combinations(&flat, &specs)?;
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
        let base_aggregates = group.aggregates;
        for row in group.rows {
            let flat = flatten_row(&row);
            let combinations = build_header_combinations(&flat, &specs)?;
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
                        aggregates: base_aggregates.clone(),
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
        let key_spec = KeySpec::parse(&token);
        let (flat_hint, allow_multiple) = classify_group_key(&key_spec.token);

        specs.push(GroupSpec {
            key_spec,
            header_key,
            flat_hint,
            allow_multiple,
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

fn classify_group_key(token: &str) -> (Option<String>, bool) {
    let Ok(path) = parse_path(token) else {
        return (None, token.contains("[]"));
    };

    let allow_multiple = token.contains("[]")
        || path
            .segments
            .iter()
            .any(|segment| !segment.selectors.is_empty());
    (expression_to_flat_key(&path), allow_multiple)
}

fn build_header_combinations(row: &Row, specs: &[GroupSpec]) -> Result<Vec<Row>> {
    let mut combinations = vec![Row::new()];

    for spec in specs {
        let values = resolve_group_values(row, spec)?;
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

    Ok(combinations)
}

fn resolve_group_values(row: &Row, spec: &GroupSpec) -> Result<Vec<Value>> {
    let values = resolve_group_pairs(row, spec)?
        .into_iter()
        .map(|(_, value)| value)
        .collect::<Vec<_>>();

    if spec.key_spec.existence {
        let found = values.iter().any(is_truthy);
        let value = if spec.key_spec.negated { !found } else { found };
        return Ok(vec![Value::Bool(value)]);
    }

    if values.is_empty() {
        Ok(vec![Value::Null])
    } else {
        Ok(values)
    }
}

fn resolve_group_pairs(row: &Row, spec: &GroupSpec) -> Result<Vec<(String, Value)>> {
    if let Some(expr) = selector_expression(&spec.key_spec.token) {
        let nested = Value::Object(coalesce_flat_row(row));
        return Ok(enumerate_path_values(&nested, &expr));
    }

    let matches = match_row_keys_detailed(row, &spec.key_spec.token, spec.key_spec.exact);
    let mut keys = select_match_keys(&matches, spec.key_spec.exact);

    if keys.is_empty()
        && let Some(flat_hint) = &spec.flat_hint
        && row.contains_key(flat_hint)
    {
        keys.push(flat_hint.clone());
    }

    reject_structured_container_token(row, spec, &keys)?;

    if !spec.allow_multiple && keys.len() > 1 {
        return Err(anyhow!(
            "G: token '{}' matched multiple keys: {}",
            spec.key_spec.token,
            keys.join(", ")
        ));
    }

    Ok(keys
        .into_iter()
        .filter_map(|key| row.get(&key).cloned().map(|value| (key, value)))
        .collect())
}

fn reject_structured_container_token(row: &Row, spec: &GroupSpec, keys: &[String]) -> Result<()> {
    if spec.key_spec.existence || spec.allow_multiple || keys.is_empty() {
        return Ok(());
    }

    let Some(flat_hint) = &spec.flat_hint else {
        return Ok(());
    };
    if row.contains_key(flat_hint) {
        return Ok(());
    }

    if keys.iter().all(|key| is_descendant_key(key, flat_hint)) {
        return Err(anyhow!(
            "G: token '{}' refers to structured content; pick a leaf field, a selector, or use ?{}",
            spec.key_spec.token,
            spec.key_spec.token
        ));
    }

    Ok(())
}

fn is_descendant_key(key: &str, prefix: &str) -> bool {
    key.strip_prefix(prefix)
        .is_some_and(|suffix| suffix.starts_with('.') || suffix.starts_with('['))
}

fn selector_expression(token: &str) -> Option<PathExpression> {
    let path = parse_path(token).ok()?;
    if path
        .segments
        .iter()
        .any(|segment| !segment.selectors.is_empty())
    {
        Some(path)
    } else {
        None
    }
}

fn select_match_keys(matches: &crate::eval::matchers::KeyMatches, exact: ExactMode) -> Vec<String> {
    if exact != ExactMode::None {
        return matches.exact.clone();
    }
    if !matches.exact.is_empty() {
        return matches.exact.clone();
    }
    matches.partial.clone()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{group_rows, regroup_groups};
    use osp_core::output_model::Group;

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

    #[test]
    fn ambiguous_fuzzy_key_errors() {
        let rows = vec![
            json!({"asset.id": 1, "owner.id": 2})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let error = group_rows(rows, "id").expect_err("group should reject ambiguous key");
        assert!(error.to_string().contains("matched multiple keys"));
    }

    #[test]
    fn regroup_preserves_aggregates() {
        let groups = vec![Group {
            groups: json!({"dept": "sales"})
                .as_object()
                .cloned()
                .expect("object"),
            aggregates: json!({"total": 300}).as_object().cloned().expect("object"),
            rows: vec![
                json!({"team": "ops"}).as_object().cloned().expect("object"),
                json!({"team": "infra"})
                    .as_object()
                    .cloned()
                    .expect("object"),
            ],
        }];

        let regrouped = regroup_groups(groups, "team").expect("regroup should work");
        assert_eq!(regrouped.len(), 2);
        assert!(
            regrouped
                .iter()
                .all(|group| group.aggregates.get("total") == Some(&json!(300)))
        );
    }

    #[test]
    fn grouping_structured_container_requires_leaf_or_selector() {
        let rows = vec![
            json!({"servers": [{"role": "web"}, {"role": "db"}]})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let error = group_rows(rows, "servers").expect_err("group should reject container token");
        assert!(error.to_string().contains("structured content"));
    }
}
