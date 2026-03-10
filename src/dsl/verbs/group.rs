//! Grouping stage (`G`) for the canonical DSL.
//!
//! Grouping works on dimensions rather than structures:
//! - grouping by scalar keys is allowed
//! - grouping by selectors/fan-out paths is allowed
//! - grouping by plain objects or arrays of objects should fail and push the
//!   user toward a leaf field or `?field`

use std::collections::HashMap;

use crate::core::{output_model::Group, row::Row};
use anyhow::{Result, anyhow};
use serde_json::Value;

use crate::dsl::{
    eval::{
        flatten::{coalesce_flat_row, flatten_row},
        matchers::match_row_keys_detailed,
        resolve::{enumerate_path_values, is_truthy},
    },
    parse::{
        key_spec::{ExactMode, KeySpec},
        path::{PathExpression, expression_to_flat_key, parse_path},
    },
    verbs::common::{parse_optional_alias_after_key, parse_stage_words},
};

use super::json;

#[derive(Debug, Clone)]
struct GroupKeyPlan {
    key_spec: KeySpec,
    header_key: String,
    flat_hint: Option<String>,
    allow_multiple: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct GroupPlan {
    keys: Vec<GroupKeyPlan>,
}

pub(crate) fn compile(spec: &str) -> Result<GroupPlan> {
    Ok(GroupPlan {
        keys: parse_group_plan(spec)?,
    })
}

pub(crate) fn group_rows_with_plan(rows: Vec<Row>, plan: &GroupPlan) -> Result<Vec<Group>> {
    let mut buckets = GroupBuckets::default();

    for row in rows {
        let flat = flatten_row(&row);
        let headers = resolve_group_headers(&flat, &plan.keys)?;
        for header in headers {
            buckets.push(Group {
                groups: header,
                aggregates: Row::new(),
                rows: vec![row.clone()],
            })?;
        }
    }

    Ok(buckets.finish())
}

pub(crate) fn regroup_groups_with_plan(groups: Vec<Group>, plan: &GroupPlan) -> Result<Vec<Group>> {
    let mut buckets = GroupBuckets::default();

    for group in groups {
        let base_headers = group.groups;
        let base_aggregates = group.aggregates;
        for row in group.rows {
            let flat = flatten_row(&row);
            let headers = resolve_group_headers(&flat, &plan.keys)?;
            for header in headers {
                let mut merged_headers = base_headers.clone();
                merged_headers.extend(header);
                buckets.push(Group {
                    groups: merged_headers,
                    aggregates: base_aggregates.clone(),
                    rows: vec![row.clone()],
                })?;
            }
        }
    }

    Ok(buckets.finish())
}

fn parse_group_plan(spec: &str) -> Result<Vec<GroupKeyPlan>> {
    let words = parse_stage_words(spec)?;

    if words.is_empty() {
        return Err(anyhow!("G requires one or more keys"));
    }

    let mut plan = Vec::new();
    let mut index = 0usize;
    while index < words.len() {
        let token = words[index].clone();
        let (alias, consumed) = parse_optional_alias_after_key(&words, index, "G")?;
        let header_key = alias.unwrap_or_else(|| canonical_header_key(&token));
        index += consumed;
        let key_spec = KeySpec::parse(&token);
        let (flat_hint, allow_multiple) = classify_group_key(&key_spec.token);

        plan.push(GroupKeyPlan {
            key_spec,
            header_key,
            flat_hint,
            allow_multiple,
        });
    }

    Ok(plan)
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

fn resolve_group_headers(row: &Row, plan: &[GroupKeyPlan]) -> Result<Vec<Row>> {
    let mut combinations = vec![Row::new()];

    for key_plan in plan {
        let values = resolve_group_values(row, key_plan)?;
        let mut next = Vec::new();

        for combination in &combinations {
            for value in &values {
                let mut candidate = combination.clone();
                candidate.insert(key_plan.header_key.clone(), value.clone());
                next.push(candidate);
            }
        }

        combinations = next;
    }

    Ok(combinations)
}

fn resolve_group_values(row: &Row, key_plan: &GroupKeyPlan) -> Result<Vec<Value>> {
    let values = resolve_group_pairs(row, key_plan)?
        .into_iter()
        .map(|(_, value)| value)
        .collect::<Vec<_>>();

    if key_plan.key_spec.existence {
        let found = values.iter().any(is_truthy);
        let value = if key_plan.key_spec.negated {
            !found
        } else {
            found
        };
        return Ok(vec![Value::Bool(value)]);
    }

    if values.is_empty() {
        Ok(vec![Value::Null])
    } else {
        Ok(values)
    }
}

fn resolve_group_pairs(row: &Row, key_plan: &GroupKeyPlan) -> Result<Vec<(String, Value)>> {
    if let Some(expr) = selector_expression(&key_plan.key_spec.token) {
        let nested = Value::Object(coalesce_flat_row(row));
        return Ok(enumerate_path_values(&nested, &expr));
    }

    let matches = match_row_keys_detailed(row, &key_plan.key_spec.token, key_plan.key_spec.exact);
    let mut keys = select_match_keys(&matches, key_plan.key_spec.exact);

    if keys.is_empty()
        && let Some(flat_hint) = &key_plan.flat_hint
        && row.contains_key(flat_hint)
    {
        keys.push(flat_hint.clone());
    }

    reject_structured_container_token(row, key_plan, &keys)?;

    if !key_plan.allow_multiple && keys.len() > 1 {
        return Err(anyhow!(
            "G: token '{}' matched multiple keys: {}",
            key_plan.key_spec.token,
            keys.join(", ")
        ));
    }

    Ok(keys
        .into_iter()
        .filter_map(|key| row.get(&key).cloned().map(|value| (key, value)))
        .collect())
}

fn reject_structured_container_token(
    row: &Row,
    key_plan: &GroupKeyPlan,
    keys: &[String],
) -> Result<()> {
    if key_plan.key_spec.existence || key_plan.allow_multiple || keys.is_empty() {
        return Ok(());
    }

    let Some(flat_hint) = &key_plan.flat_hint else {
        return Ok(());
    };
    if row.contains_key(flat_hint) {
        return Ok(());
    }

    if keys.iter().all(|key| is_descendant_key(key, flat_hint)) {
        return Err(anyhow!(
            "G: token '{}' refers to structured content; pick a leaf field, a selector, or use ?{}",
            key_plan.key_spec.token,
            key_plan.key_spec.token
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

fn select_match_keys(
    matches: &crate::dsl::eval::matchers::KeyMatches,
    exact: ExactMode,
) -> Vec<String> {
    if exact != ExactMode::None {
        return matches.exact.clone();
    }
    if !matches.exact.is_empty() {
        return matches.exact.clone();
    }
    matches.partial.clone()
}

#[derive(Default)]
struct GroupBuckets {
    groups: Vec<Group>,
    index_by_key: HashMap<String, usize>,
}

impl GroupBuckets {
    fn push(&mut self, group: Group) -> Result<()> {
        let key = serde_json::to_string(&group.groups)
            .map_err(|error| anyhow!("failed to encode group key: {error}"))?;

        if let Some(existing) = self.index_by_key.get(&key) {
            self.groups[*existing].rows.extend(group.rows);
            return Ok(());
        }

        let position = self.groups.len();
        self.index_by_key.insert(key, position);
        self.groups.push(group);
        Ok(())
    }

    fn finish(self) -> Vec<Group> {
        self.groups
    }
}

pub(crate) fn apply_value_with_plan(value: Value, plan: &GroupPlan) -> Result<Value> {
    json::traverse_collections(value, |items| match items {
        crate::core::output_model::OutputItems::Rows(rows) => Ok(
            crate::core::output_model::OutputItems::Groups(group_rows_with_plan(rows, plan)?),
        ),
        crate::core::output_model::OutputItems::Groups(groups) => Ok(
            crate::core::output_model::OutputItems::Groups(regroup_groups_with_plan(groups, plan)?),
        ),
    })
}
