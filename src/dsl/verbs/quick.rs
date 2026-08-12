use std::borrow::Cow;
use std::collections::HashSet;

use anyhow::{Result, anyhow};
use serde_json::Value;

use crate::core::{output_model::Group, row::Row};
use crate::dsl::{
    eval::{
        flatten::{coalesce_flat_row, flatten_row},
        matchers::{
            contains_case_insensitive, fuzzy_contains_case_insensitive, match_row_keys_detailed,
            match_row_keys_detailed_fuzzy, render_value,
        },
        resolve::{is_truthy, resolve_pairs, resolve_values_truthy},
    },
    parse::{
        key_spec::ExactMode,
        quick::{QuickScope, parse_quick_spec},
    },
    verbs::common::map_group_rows,
};

use super::selector;

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
}

impl QuickPlan {
    pub(crate) fn matches_row_filter_mode(&self, row: &Row) -> bool {
        matches_row(row, &self.spec)
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
    Ok(rows
        .into_iter()
        .filter(|row| plan.matches_row_filter_mode(row))
        .collect())
}

pub(crate) fn apply_groups_with_plan(groups: Vec<Group>, plan: &QuickPlan) -> Result<Vec<Group>> {
    map_group_rows(groups, |rows| apply_with_plan(rows, plan))
}

fn matches_row(row: &Row, spec: &CompiledQuickSpec) -> bool {
    if let Some(matched) =
        match_structural_value(&Value::Object(coalesce_flat_row(&flatten_row(row))), spec)
    {
        return matched;
    }

    if spec.existence() {
        let found = resolve_values_truthy(row, spec.token(), spec.exact());
        return if spec.negated() { !found } else { found };
    }

    let flat = flatten_row(row);
    let (pairs, _) = resolve_pairs(&flat, spec.token());
    let matches = if spec.fuzzy {
        match_row_keys_detailed_fuzzy(&flat, spec.token(), spec.exact())
    } else {
        match_row_keys_detailed(&flat, spec.token(), spec.exact())
    };
    let key_hits = if matches.exact.is_empty() {
        matches.partial
    } else {
        matches.exact
    };
    let value_hit = pairs
        .iter()
        .any(|(_, value)| value_matches_token(value, spec.token(), spec.exact(), spec.fuzzy));
    let synthetic_hit = pairs.iter().any(|(key, _)| !flat.contains_key(key));

    let matched = match spec.scope {
        QuickScope::KeyOnly if spec.key_not_equals => {
            let key_set = key_hits.iter().collect::<HashSet<_>>();
            flat.keys().any(|key| !key_set.contains(key))
        }
        QuickScope::KeyOnly => !key_hits.is_empty(),
        QuickScope::ValueOnly => value_hit || synthetic_hit,
        QuickScope::KeyOrValue => !key_hits.is_empty() || value_hit || synthetic_hit,
    };

    if spec.negated() { !matched } else { matched }
}

fn value_matches_token(value: &Value, token: &str, exact: ExactMode, fuzzy: bool) -> bool {
    let token = unescape_search_token(token);
    match exact {
        ExactMode::CaseSensitive => match value {
            Value::Array(values) => values
                .iter()
                .any(|item| value_matches_token(item, &token, exact, fuzzy)),
            scalar => render_value(scalar) == token,
        },
        ExactMode::CaseInsensitive => match value {
            Value::Array(values) => values
                .iter()
                .any(|item| value_matches_token(item, &token, exact, fuzzy)),
            scalar => render_value(scalar).eq_ignore_ascii_case(&token),
        },
        ExactMode::None => match value {
            Value::Array(values) => values
                .iter()
                .any(|item| value_matches_token(item, &token, exact, fuzzy)),
            scalar if fuzzy => fuzzy_contains_case_insensitive(&render_value(scalar), &token),
            scalar => contains_case_insensitive(&render_value(scalar), &token),
        },
    }
}

fn unescape_search_token(token: &str) -> Cow<'_, str> {
    if !token.contains('\\') {
        return Cow::Borrowed(token);
    }

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

pub(crate) fn apply_value(value: Value, raw_stage: &str) -> Result<Value> {
    let plan = compile(raw_stage)?;
    apply_value_with_plan(value, &plan)
}

pub(crate) fn apply_value_with_plan(value: Value, plan: &QuickPlan) -> Result<Value> {
    apply_value_filter(value, plan, false)
}

pub(crate) fn apply_value_with_plan_preserving_matching_rows(
    value: Value,
    plan: &QuickPlan,
) -> Result<Value> {
    apply_value_filter(value, plan, true)
}

fn apply_value_filter(
    value: Value,
    plan: &QuickPlan,
    preserve_matching_leaf_rows: bool,
) -> Result<Value> {
    if plan.spec.is_structural() && plan.spec.negated() && !plan.spec.existence() {
        return Ok(selector::remove_compiled(
            value,
            std::iter::once(&plan.spec.selector),
        ));
    }
    if let Some(matched) = match_structural_value(&value, &plan.spec) {
        return Ok(if matched { value } else { Value::Null });
    }
    selector::filter_descendants_preserving_matching_rows_with_options(
        value,
        |row| plan.matches_row_filter_mode(row),
        !plan.spec.fuzzy,
        preserve_matching_leaf_rows,
    )
}

fn match_structural_value(root: &Value, spec: &CompiledQuickSpec) -> Option<bool> {
    if !spec.is_structural() || spec.key_not_equals {
        return None;
    }

    let matches = spec.selector.resolve_matches(root);
    let found = if spec.existence() {
        matches.iter().any(|entry| is_truthy(&entry.value))
    } else if matches.is_empty() {
        return None;
    } else {
        true
    };
    Some(if spec.negated() { !found } else { found })
}

#[cfg(test)]
mod tests;
