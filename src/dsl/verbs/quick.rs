use std::{borrow::Cow, collections::HashSet};

use crate::core::{output_model::Group, row::Row};
use anyhow::{Result, anyhow};
use serde_json::Value;

use crate::dsl::{
    eval::{
        flatten::{coalesce_flat_row, flatten_row},
        matchers::{
            KeyMatches, contains_case_insensitive, eq_case_insensitive,
            fuzzy_contains_case_insensitive, match_row_keys_detailed,
            match_row_keys_detailed_fuzzy, render_value,
        },
        resolve::{compact_sparse_arrays, is_truthy, resolve_pairs, resolve_values_truthy},
    },
    parse::{
        key_spec::ExactMode,
        path::{Selector, parse_path},
        quick::{QuickScope, parse_quick_spec},
    },
    verbs::common::map_group_rows,
};

use super::{json, selector};

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
    spec: CompiledQuickSpec,
}

#[derive(Debug, Clone)]
struct CompiledQuickSpec {
    scope: QuickScope,
    selector: selector::CompiledSelector,
    key_not_equals: bool,
    fuzzy: bool,
}

impl CompiledQuickSpec {
    fn from_parsed(spec: crate::dsl::parse::quick::QuickSpec) -> Self {
        Self {
            scope: spec.scope,
            selector: selector::CompiledSelector::from_key_spec(spec.key_spec),
            key_not_equals: spec.key_not_equals,
            fuzzy: spec.fuzzy,
        }
    }

    fn token(&self) -> &str {
        self.selector.token()
    }

    fn exact(&self) -> ExactMode {
        self.selector.exact()
    }

    fn negated(&self) -> bool {
        self.selector.key_spec.negated
    }

    fn existence(&self) -> bool {
        self.selector.key_spec.existence
    }

    fn is_structural(&self) -> bool {
        self.selector.is_structural()
    }

    fn resolve_matches(&self, root: &Value) -> Vec<crate::dsl::eval::resolve::AddressedValue> {
        self.selector.resolve_matches(root)
    }
}

impl QuickPlan {
    fn apply_row(&self, row: Row, mode: MatchMode) -> Vec<Row> {
        apply_row_with_mode(row, &self.spec, mode)
    }

    pub(crate) fn matches_row_filter_mode(&self, row: &Row) -> bool {
        !self.apply_row(row.clone(), MatchMode::Multi).is_empty()
    }
}

pub(crate) fn compile(raw_stage: &str) -> Result<QuickPlan> {
    let spec = CompiledQuickSpec::from_parsed(parse_quick_spec(raw_stage));
    let token = spec.token().trim();
    if token.is_empty() {
        return Err(anyhow!("quick stage requires a search token"));
    }
    if spec.fuzzy {
        if spec.existence() {
            return Err(anyhow!(
                "% quick does not support existence filters; use plain ?path or literal quick"
            ));
        }
        if !matches!(spec.exact(), ExactMode::None) || spec.key_not_equals {
            return Err(anyhow!(
                "% quick does not support exact-match key operators; use plain quick operators"
            ));
        }
        if spec.is_structural() {
            return Err(anyhow!(
                "% quick does not support path selectors; use plain path quick instead"
            ));
        }
    }
    Ok(QuickPlan { spec })
}

pub(crate) fn apply_with_plan(rows: Vec<Row>, plan: &QuickPlan) -> Result<Vec<Row>> {
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

pub(crate) fn apply_groups_with_plan(groups: Vec<Group>, plan: &QuickPlan) -> Result<Vec<Group>> {
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
    // a multi-row set. A two-row lookahead preserves that magic while still
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

fn apply_row_with_mode(row: Row, spec: &CompiledQuickSpec, mode: MatchMode) -> Vec<Row> {
    if let Some(transformed) = try_apply_path_scoped_row(&row, spec) {
        return transformed;
    }

    if spec.existence() {
        let found = resolve_values_truthy(&row, spec.token(), spec.exact());
        let matched = if spec.negated() { !found } else { found };
        return if matched { vec![row] } else { Vec::new() };
    }

    let flat = flatten_row(&row);
    let (pairs, _) = resolve_pairs(&flat, spec.token());
    let synthetic = build_synthetic_map(&pairs, &flat);
    let mut result = match_row(&flat, &pairs, synthetic, spec);

    let keep = match spec.scope {
        QuickScope::KeyOnly => {
            if matches!(mode, MatchMode::Multi) {
                result.matched
            } else {
                spec.negated() || result.matched
            }
        }
        QuickScope::ValueOnly | QuickScope::KeyOrValue => {
            if matches!(mode, MatchMode::Multi) {
                result.matched
            } else {
                result.matched || spec.negated()
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
    spec: &CompiledQuickSpec,
) -> MatchResult {
    let matches = if spec.fuzzy {
        match_row_keys_detailed_fuzzy(flat, spec.token(), spec.exact())
    } else {
        match_row_keys_detailed(flat, spec.token(), spec.exact())
    };
    let mut key_hits = prefer_exact_keys(&matches, spec.exact());
    let mut value_hits = Vec::new();
    let mut seen_values = HashSet::new();

    for (key, value) in pairs {
        let matched = match value {
            Value::Array(items) => items
                .iter()
                .any(|item| value_matches_token(item, spec.token(), spec.exact(), spec.fuzzy)),
            scalar => value_matches_token(scalar, spec.token(), spec.exact(), spec.fuzzy),
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

    if spec.negated() {
        matched = !matched;
    }

    let mut is_projection = match spec.scope {
        QuickScope::ValueOnly | QuickScope::KeyOrValue => !synthetic.is_empty(),
        QuickScope::KeyOnly => false,
    };

    if key_hits_match_projection_token(&key_hits, spec.token()) {
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
    spec: &CompiledQuickSpec,
) -> Option<Vec<Row>> {
    let synthetic_keys = result.synthetic.keys().cloned().collect::<Vec<_>>();

    if result.is_projection && !spec.negated() {
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
        let selected = expand_object_array_parent_keys(selected, &[flat, &result.synthetic]);

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
        let restored = restore_row_envelope(flat, projected, spec.is_structural());
        return Some(vec![restored]);
    }

    if spec.negated() {
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
                                !value_matches_token(item, spec.token(), spec.exact(), spec.fuzzy)
                            })
                            .collect::<Vec<_>>();
                        if remaining.is_empty() {
                            new_row.remove(&key);
                        } else {
                            new_row.insert(key.clone(), Value::Array(remaining));
                        }
                    } else if value_matches_token(&value, spec.token(), spec.exact(), spec.fuzzy) {
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
                            !value_matches_token(item, spec.token(), spec.exact(), spec.fuzzy)
                        })
                        .collect::<Vec<_>>();
                    if remaining.is_empty() {
                        new_synthetic.remove(&key);
                    } else {
                        new_synthetic.insert(key.clone(), Value::Array(remaining));
                    }
                } else if value_matches_token(&value, spec.token(), spec.exact(), spec.fuzzy) {
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
        let mut restored = restore_row_envelope(flat, new_row, spec.is_structural());
        compact_sparse_arrays_in_row(&mut restored);
        return Some(vec![restored]);
    }

    let mut filtered = Row::new();
    let keys = expand_object_array_parent_keys(
        union_keys(&result.key_hits, &result.value_hits)
            .into_iter()
            .chain(result.synthetic.keys().cloned())
            .collect(),
        &[flat, &result.synthetic],
    );
    for key in keys {
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
                .filter(|item| value_matches_token(item, spec.token(), spec.exact(), spec.fuzzy))
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
        let mut coalesced = restore_row_envelope(flat, filtered, spec.is_structural());
        compact_sparse_arrays_in_row(&mut coalesced);
        Some(vec![coalesced])
    }
}

fn restore_row_envelope(flat: &Row, narrowed: Row, structural: bool) -> Row {
    if structural {
        return coalesce_flat_row(&narrowed);
    }

    let original = Value::Object(coalesce_flat_row(flat));
    let narrowed = Value::Object(coalesce_flat_row(&narrowed));
    match json::preserve_envelope_fields(original, narrowed) {
        Value::Object(map) => map,
        _ => Row::new(),
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

fn expand_object_array_parent_keys(selected: Vec<String>, sources: &[&Row]) -> Vec<String> {
    let mut expanded = Vec::new();
    let mut seen = HashSet::new();

    for key in selected {
        if seen.insert(key.clone()) {
            expanded.push(key.clone());
        }

        let Some(parent) = object_array_parent_prefix(&key) else {
            continue;
        };
        let child_prefix = format!("{parent}.");
        for source in sources {
            for candidate in source.keys() {
                if candidate.starts_with(&child_prefix) && seen.insert(candidate.clone()) {
                    expanded.push(candidate.clone());
                }
            }
        }
    }

    expanded
}

fn object_array_parent_prefix(key: &str) -> Option<String> {
    let path = parse_path(key).ok()?;
    let mut prefix = String::new();
    let mut parent = None;

    for (segment_index, segment) in path.segments.iter().enumerate() {
        if !prefix.is_empty() {
            prefix.push('.');
        }
        let name = segment.name.as_ref()?;
        prefix.push_str(name);

        if !segment.selectors.is_empty() {
            for selector in &segment.selectors {
                let Selector::Index(index) = selector else {
                    return None;
                };
                if *index < 0 {
                    return None;
                }
                prefix.push('[');
                prefix.push_str(&index.to_string());
                prefix.push(']');
            }

            if segment_index + 1 < path.segments.len() {
                parent = Some(prefix.clone());
            }
        }
    }

    parent
}

fn value_matches_token(value: &Value, token: &str, exact: ExactMode, fuzzy: bool) -> bool {
    let token = unescape_search_token(token);
    match exact {
        ExactMode::CaseSensitive => {
            if let Value::Array(values) = value {
                return values
                    .iter()
                    .any(|item| value_matches_token(item, &token, exact, fuzzy));
            }
            render_value(value) == token
        }
        ExactMode::CaseInsensitive => {
            if let Value::Array(values) = value {
                return values
                    .iter()
                    .any(|item| value_matches_token(item, &token, exact, fuzzy));
            }
            eq_case_insensitive(&render_value(value), &token)
        }
        ExactMode::None => {
            if let Value::Array(values) = value {
                return values
                    .iter()
                    .any(|item| value_matches_token(item, &token, exact, fuzzy));
            }
            if fuzzy {
                return fuzzy_contains_case_insensitive(&render_value(value), &token);
            }
            contains_case_insensitive(&render_value(value), &token)
        }
    }
}

fn unescape_search_token(token: &str) -> Cow<'_, str> {
    if !token.contains('\\') {
        return Cow::Borrowed(token);
    }

    // Row quick is search-first UX. If the user escapes punctuation like
    // `theme\.name`, treat that as the literal text they can already see in
    // the rendered row instead of forcing parser trivia on them.
    let mut out = String::with_capacity(token.len());
    let mut chars = token.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some(escaped) => out.push(escaped),
                None => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }

    Cow::Owned(out)
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

pub(crate) fn apply_value(value: Value, raw_stage: &str) -> Result<Value> {
    let plan = compile(raw_stage)?;
    apply_value_with_plan(value, &plan)
}

pub(crate) fn apply_value_with_plan(value: Value, plan: &QuickPlan) -> Result<Value> {
    if let Some(transformed) = try_apply_path_scoped_value(&value, &plan.spec) {
        return Ok(transformed);
    }
    selector::filter_descendants_with_options(
        value,
        |row| plan.matches_row_filter_mode(row),
        !plan.spec.fuzzy,
    )
}

fn try_apply_path_scoped_row(row: &Row, spec: &CompiledQuickSpec) -> Option<Vec<Row>> {
    if !spec.is_structural() || spec.key_not_equals {
        return None;
    }

    // Canonicalize through flatten/coalesce so path-scoped quick behaves the
    // same on already-flat rows and nested row-shaped JSON.
    let canonical = Value::Object(coalesce_flat_row(&flatten_row(row)));
    let matches = spec.resolve_matches(&canonical);

    if spec.existence() {
        let found = matches.iter().any(|entry| is_truthy(&entry.value));
        let keep = if spec.negated() { !found } else { found };
        return Some(if keep { vec![row.clone()] } else { Vec::new() });
    }

    if spec.negated() {
        return Some(match selector::remove_matches(canonical, &matches) {
            Value::Null => Vec::new(),
            Value::Object(map) => vec![map],
            _ => Vec::new(),
        });
    }

    if matches.is_empty() {
        return None;
    }

    // Row quick deliberately strips the structural envelope and emits tabular
    // rows when the addressed hits are array elements. The value-path sibling
    // keeps the envelope by routing through `selector::project_matches`.
    Some(selector::project_row_matches(&matches))
}

fn try_apply_path_scoped_value(root: &Value, spec: &CompiledQuickSpec) -> Option<Value> {
    if !spec.is_structural() || spec.key_not_equals {
        return None;
    }

    let matches = spec.resolve_matches(root);

    if spec.existence() {
        let found = matches.iter().any(|entry| is_truthy(&entry.value));
        let keep = if spec.negated() { !found } else { found };
        return Some(if keep { root.clone() } else { Value::Null });
    }

    if spec.negated() {
        return Some(selector::remove_matches(root.clone(), &matches));
    }

    if matches.is_empty() {
        return Some(Value::Null);
    }

    Some(selector::project_matches(root, &matches))
}

#[cfg(test)]
mod tests;
