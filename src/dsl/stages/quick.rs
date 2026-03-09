use std::collections::HashSet;

use crate::core::{output_model::Group, row::Row};
use anyhow::{Result, anyhow};
use serde_json::Value;

use crate::dsl::{
    eval::{
        flatten::{coalesce_flat_row, flatten_row},
        matchers::{
            KeyMatches, contains_case_insensitive, eq_case_insensitive, match_row_keys_detailed,
            render_value,
        },
        resolve::{resolve_pairs, resolve_values_truthy},
    },
    parse::{
        key_spec::ExactMode,
        path::parse_path,
        quick::{QuickScope, parse_quick_spec},
    },
    stages::common::map_group_rows,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchMode {
    Single,
    Multi,
}

#[derive(Debug, Clone)]
struct MatchResult {
    matched: bool,
    key_hits: Vec<String>,
    value_hits: Vec<String>,
    is_projection: bool,
    synthetic: Row,
}

#[derive(Debug, Clone)]
pub(crate) struct QuickPlan {
    spec: crate::dsl::parse::quick::QuickSpec,
}

impl QuickPlan {
    fn apply_row(&self, row: Row, mode: MatchMode) -> Vec<Row> {
        apply_row_with_mode(row, &self.spec, mode)
    }
}

pub(crate) fn compile(raw_stage: &str) -> Result<QuickPlan> {
    let spec = parse_quick_spec(raw_stage);
    if spec.key_spec.token.trim().is_empty() {
        return Err(anyhow!("quick stage requires a search token"));
    }
    Ok(QuickPlan { spec })
}

pub fn apply(rows: Vec<Row>, raw_stage: &str) -> Result<Vec<Row>> {
    let plan = compile(raw_stage)?;

    // Quick mode is intentionally dual-purpose:
    // - multi-row input acts like a row filter
    // - single-row input acts more like projection/reshaping
    let mode = if rows.len() > 1 {
        MatchMode::Multi
    } else {
        MatchMode::Single
    };

    let mut out = Vec::new();
    for row in rows {
        out.extend(plan.apply_row(row, mode));
    }

    Ok(out)
}

pub fn apply_groups(groups: Vec<Group>, raw_stage: &str) -> Result<Vec<Group>> {
    let plan = compile(raw_stage)?;
    map_group_rows(groups, |rows| {
        let mode = if rows.len() > 1 {
            MatchMode::Multi
        } else {
            MatchMode::Single
        };
        let mut out = Vec::new();
        for row in rows {
            out.extend(plan.apply_row(row, mode));
        }
        Ok(out)
    })
}

pub(crate) fn stream_rows_with_plan<I>(
    rows: I,
    plan: QuickPlan,
) -> impl Iterator<Item = Result<Row>>
where
    I: IntoIterator<Item = Result<Row>>,
{
    let mut iter = rows.into_iter();
    let first = iter.next();
    let second = iter.next();

    // Quick semantics depend on whether the current payload is a single row or
    // a multi-row set. A two-row lookahead preserves that "magic" while still
    // allowing the common multi-row path to continue as a stream.
    let mode = if second.is_some() {
        MatchMode::Multi
    } else {
        MatchMode::Single
    };

    let mut seed = Vec::new();
    if let Some(row) = first {
        match row {
            Ok(row) => seed.extend(plan.apply_row(row, mode).into_iter().map(Ok)),
            Err(err) => seed.push(Err(err)),
        }
    }
    if let Some(row) = second {
        match row {
            Ok(row) => seed.extend(plan.apply_row(row, mode).into_iter().map(Ok)),
            Err(err) => seed.push(Err(err)),
        }
    }

    seed.into_iter().chain(iter.flat_map(move |row| {
        match row {
            Ok(row) => plan
                .apply_row(row, mode)
                .into_iter()
                .map(Ok)
                .collect::<Vec<_>>()
                .into_iter(),
            Err(err) => vec![Err(err)].into_iter(),
        }
    }))
}

fn apply_row_with_mode(
    row: Row,
    spec: &crate::dsl::parse::quick::QuickSpec,
    mode: MatchMode,
) -> Vec<Row> {
    if spec.key_spec.existence {
        let found = resolve_values_truthy(&row, &spec.key_spec.token, spec.key_spec.exact);
        let matched = if spec.key_spec.negated { !found } else { found };
        return if matched { vec![row] } else { Vec::new() };
    }

    let flat = flatten_row(&row);
    let (pairs, _) = resolve_pairs(&flat, &spec.key_spec.token);
    let synthetic = build_synthetic_map(&pairs, &flat);
    let mut result = match_row(&flat, &pairs, synthetic, spec);

    let keep = match spec.scope {
        QuickScope::KeyOnly => {
            if matches!(mode, MatchMode::Multi) {
                result.matched
            } else {
                spec.key_spec.negated || result.matched
            }
        }
        QuickScope::ValueOnly | QuickScope::KeyOrValue => {
            if matches!(mode, MatchMode::Multi) {
                result.matched
            } else {
                result.matched || spec.key_spec.negated
            }
        }
    };

    if !keep {
        return Vec::new();
    }

    if matches!(mode, MatchMode::Multi) && !result.is_projection {
        return vec![row];
    }

    transform_row(&flat, &mut result, spec).unwrap_or_default()
}

fn match_row(
    flat: &Row,
    pairs: &[(String, Value)],
    synthetic: Row,
    spec: &crate::dsl::parse::quick::QuickSpec,
) -> MatchResult {
    let matches = match_row_keys_detailed(flat, &spec.key_spec.token, spec.key_spec.exact);
    let mut key_hits = prefer_exact_keys(&matches, spec.key_spec.exact);
    let mut value_hits = Vec::new();
    let mut seen_values = HashSet::new();

    for (key, value) in pairs {
        let matched = match value {
            Value::Array(items) => items
                .iter()
                .any(|item| value_matches_token(item, &spec.key_spec.token, spec.key_spec.exact)),
            scalar => value_matches_token(scalar, &spec.key_spec.token, spec.key_spec.exact),
        };
        if matched && seen_values.insert(key.as_str()) {
            value_hits.push(key.clone());
        }
    }

    let mut matched = match spec.scope {
        QuickScope::KeyOnly => {
            if spec.key_not_equals {
                let key_set = key_hits.iter().collect::<HashSet<_>>();
                flat.keys().any(|key| !key_set.contains(key))
            } else {
                !key_hits.is_empty()
            }
        }
        QuickScope::ValueOnly => !value_hits.is_empty() || !synthetic.is_empty(),
        QuickScope::KeyOrValue => {
            !key_hits.is_empty() || !value_hits.is_empty() || !synthetic.is_empty()
        }
    };

    if spec.key_spec.negated {
        matched = !matched;
    }

    let mut is_projection = match spec.scope {
        QuickScope::ValueOnly | QuickScope::KeyOrValue => !synthetic.is_empty(),
        QuickScope::KeyOnly => false,
    };

    if key_hits_match_projection_token(&key_hits, &spec.key_spec.token) {
        is_projection = true;
    }

    if is_projection && !synthetic.is_empty() && matches!(spec.scope, QuickScope::KeyOrValue) {
        key_hits.clear();
    }

    MatchResult {
        matched,
        key_hits,
        value_hits,
        is_projection,
        synthetic,
    }
}

fn transform_row(
    flat: &Row,
    result: &mut MatchResult,
    spec: &crate::dsl::parse::quick::QuickSpec,
) -> Option<Vec<Row>> {
    let synthetic_keys = result.synthetic.keys().cloned().collect::<Vec<_>>();

    if result.is_projection && !spec.key_spec.negated {
        if !result.synthetic.is_empty() {
            let mut rows = Vec::new();
            let mut keys = result.synthetic.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                if let Some(value) = result.synthetic.get(&key) {
                    let mut projected = Row::new();
                    projected.insert(key.clone(), value.clone());
                    let mut coalesced = coalesce_flat_row(&projected);
                    coalesced = squeeze_single_entry(coalesced);
                    if !coalesced.is_empty() {
                        rows.push(coalesced);
                    }
                }
            }
            if !rows.is_empty() {
                return Some(rows);
            }
        }

        let mut selected = Vec::new();
        let mut seen = HashSet::new();
        extend_unique(&mut selected, &mut seen, &result.key_hits);
        extend_unique(&mut selected, &mut seen, &result.value_hits);
        extend_unique(&mut selected, &mut seen, &synthetic_keys);

        let mut projected = Row::new();
        for key in selected {
            if let Some(value) = flat
                .get(&key)
                .cloned()
                .or_else(|| result.synthetic.get(&key).cloned())
            {
                projected.insert(key, value);
            }
        }
        if projected.is_empty() {
            return None;
        }
        return Some(vec![coalesce_flat_row(&projected)]);
    }

    if spec.key_spec.negated {
        let mut new_row = flat.clone();
        let mut new_synthetic = result.synthetic.clone();
        let keys = union_keys(&result.key_hits, &result.value_hits);
        for key in keys {
            if let Some(value) = new_row.get(&key).cloned() {
                if result.value_hits.contains(&key) {
                    if let Value::Array(items) = value {
                        let remaining = items
                            .into_iter()
                            .filter(|item| {
                                !value_matches_token(
                                    item,
                                    &spec.key_spec.token,
                                    spec.key_spec.exact,
                                )
                            })
                            .collect::<Vec<_>>();
                        if remaining.is_empty() {
                            new_row.remove(&key);
                        } else {
                            new_row.insert(key.clone(), Value::Array(remaining));
                        }
                    } else if value_matches_token(&value, &spec.key_spec.token, spec.key_spec.exact)
                    {
                        new_row.remove(&key);
                    }
                } else if result.key_hits.contains(&key) {
                    new_row.remove(&key);
                }
            } else if let Some(value) = new_synthetic.get(&key).cloned() {
                if let Value::Array(items) = value {
                    let remaining = items
                        .into_iter()
                        .filter(|item| {
                            !value_matches_token(item, &spec.key_spec.token, spec.key_spec.exact)
                        })
                        .collect::<Vec<_>>();
                    if remaining.is_empty() {
                        new_synthetic.remove(&key);
                    } else {
                        new_synthetic.insert(key.clone(), Value::Array(remaining));
                    }
                } else if value_matches_token(&value, &spec.key_spec.token, spec.key_spec.exact) {
                    new_synthetic.remove(&key);
                }
            }
        }
        for (key, value) in new_synthetic {
            new_row.insert(key, value);
        }
        if new_row.is_empty() {
            return None;
        }
        return Some(vec![coalesce_flat_row(&new_row)]);
    }

    let mut filtered = Row::new();
    let keys = union_keys(&result.key_hits, &result.value_hits);
    for key in keys.into_iter().chain(result.synthetic.keys().cloned()) {
        let Some(value) = flat
            .get(&key)
            .cloned()
            .or_else(|| result.synthetic.get(&key).cloned())
        else {
            continue;
        };
        if result.value_hits.contains(&key)
            && let Value::Array(items) = value
        {
            let filtered_values = items
                .into_iter()
                .filter(|item| value_matches_token(item, &spec.key_spec.token, spec.key_spec.exact))
                .collect::<Vec<_>>();
            if filtered_values.is_empty() {
                continue;
            }
            filtered.insert(key.clone(), Value::Array(filtered_values));
            continue;
        }
        filtered.insert(key, value);
    }

    if filtered.is_empty() {
        None
    } else {
        let mut coalesced = coalesce_flat_row(&filtered);
        compact_sparse_arrays_in_row(&mut coalesced);
        Some(vec![coalesced])
    }
}

fn build_synthetic_map(pairs: &[(String, Value)], flat: &Row) -> Row {
    let mut out = Row::new();
    for (key, value) in pairs {
        if !flat.contains_key(key) {
            out.insert(key.clone(), value.clone());
        }
    }
    out
}

fn prefer_exact_keys(matches: &KeyMatches, _exact: ExactMode) -> Vec<String> {
    if !matches.exact.is_empty() {
        matches.exact.clone()
    } else {
        matches.partial.clone()
    }
}

fn key_hits_match_projection_token(key_hits: &[String], token: &str) -> bool {
    let mut names = key_hits.iter().filter_map(|key| last_segment_name(key));
    let Some(first) = names.next() else {
        return false;
    };

    if !eq_case_insensitive(&first, token) {
        return false;
    }

    names.all(|name| eq_case_insensitive(&name, &first))
}

fn extend_unique(out: &mut Vec<String>, seen: &mut HashSet<String>, keys: &[String]) {
    for key in keys {
        if seen.insert(key.clone()) {
            out.push(key.clone());
        }
    }
}

fn union_keys(left: &[String], right: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    extend_unique(&mut out, &mut seen, left);
    extend_unique(&mut out, &mut seen, right);
    out
}

fn value_matches_token(value: &Value, token: &str, exact: ExactMode) -> bool {
    match exact {
        ExactMode::CaseSensitive => {
            if let Value::Array(values) = value {
                return values
                    .iter()
                    .any(|item| value_matches_token(item, token, exact));
            }
            render_value(value) == token
        }
        ExactMode::CaseInsensitive => {
            if let Value::Array(values) = value {
                return values
                    .iter()
                    .any(|item| value_matches_token(item, token, exact));
            }
            eq_case_insensitive(&render_value(value), token)
        }
        ExactMode::None => {
            if let Value::Array(values) = value {
                return values
                    .iter()
                    .any(|item| value_matches_token(item, token, exact));
            }
            contains_case_insensitive(&render_value(value), token)
        }
    }
}

fn last_segment_name(key: &str) -> Option<String> {
    if let Ok(path) = parse_path(key)
        && let Some(segment) = path.segments.last()
        && let Some(name) = &segment.name
    {
        return Some(name.clone());
    }
    let last = key.rsplit('.').next().unwrap_or(key);
    Some(last.split('[').next().unwrap_or(last).to_string())
}

fn squeeze_single_entry(row: Row) -> Row {
    if row.len() != 1 {
        return row;
    }
    let (only_key, only_val) = match row.iter().next() {
        Some((key, value)) => (key.clone(), value.clone()),
        None => return row,
    };
    match only_val {
        Value::Array(items) => {
            let cleaned = items
                .into_iter()
                .filter(|item| !item.is_null())
                .collect::<Vec<_>>();
            if cleaned.len() == 1
                && let Value::Object(obj) = &cleaned[0]
            {
                return obj.clone();
            }
            if cleaned.is_empty() {
                return Row::new();
            }
            let mut out = Row::new();
            out.insert(only_key, Value::Array(cleaned));
            out
        }
        Value::Object(obj) => obj,
        _ => row,
    }
}

fn compact_sparse_arrays_in_row(row: &mut Row) {
    for value in row.values_mut() {
        compact_sparse_arrays(value);
    }
}

fn compact_sparse_arrays(value: &mut Value) {
    match value {
        Value::Array(items) => {
            for item in items.iter_mut() {
                compact_sparse_arrays(item);
            }
            if items.iter().any(|item| !item.is_null()) {
                items.retain(|item| !item.is_null());
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                compact_sparse_arrays(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;
