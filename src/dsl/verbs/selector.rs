//! Shared selector-engine substrate for semantic DSL verbs.
//!
//! Long-term architecture rule:
//! - selector verbs (`P`, `F`, path quick, `?path`, `K`, `V`, `VALUE`, `U`)
//!   should flow through addressed match selection plus one structural rewrite
//!   operation
//! - collection verbs (`S`, `G`, `A`, `C`, `Z`, `L`, `JQ`) should stay on the
//!   row/group bridge
//!
//! Keep those engines separate. Selector semantics are about preserving and
//! rewriting document structure. Collection semantics are about operating on
//! row/group datasets. Mixing the two inside each verb is what causes the code
//! count and semantic drift to grow again.
//!
//! This module owns both selector branches:
//! - structural addressed rewrite for path-shaped selectors
//! - permissive descendant traversal for bare-token selectors
//!
//! That keeps the semantic fork explicit in one place instead of spreading it
//! across verb-local helpers and generic JSON utilities.
//!
//! Examples:
//! - `name` stays permissive and may match descendant keys or values
//! - `sections[0].entries[1].name` is strict and addressed
//! - `a.b` means path semantics, not "find some flattened key that happens to
//!   render as `a.b`"

use crate::core::row::Row;
use anyhow::Result;
use serde_json::Value;
use std::collections::HashSet;

use crate::dsl::{
    eval::resolve::{
        AddressStep, AddressedValue, compact_sparse_arrays, materialize_path_matches,
        resolve_descendant_matches, resolve_path_matches,
    },
    eval::{matchers::match_row_keys, resolve::evaluate_path},
    parse::{
        key_spec::{ExactMode, KeySpec},
        path::{PathExpression, expression_to_flat_key, is_structural_path_token, parse_path},
    },
};

use super::json;

/// Compile-time split between structural path semantics and permissive
/// descendant matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SelectorMode {
    StructuralPath,
    PermissiveDescendant,
}

/// Parsed selector token plus the compile-time mode it should use.
///
/// Selector verbs should carry this instead of threading raw `KeySpec` and
/// `SelectorMode` separately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompiledSelector {
    pub(crate) key_spec: KeySpec,
    pub(crate) mode: SelectorMode,
    path: Option<PathExpression>,
}

/// Classifies `token` into the selector mode it should use.
///
/// This decision should happen during verb compilation so execution does not
/// have to keep re-guessing semantics from token shape.
pub(crate) fn classify_token(token: &str) -> SelectorMode {
    if token_uses_structural_path(token) {
        SelectorMode::StructuralPath
    } else {
        SelectorMode::PermissiveDescendant
    }
}

/// Classifies a parsed [`KeySpec`].
///
/// Callers should prefer this over classifying raw stage text so operator
/// prefixes like `!path` and `?path` do not accidentally leak into selector
/// semantics.
pub(crate) fn classify_key_spec(spec: &KeySpec) -> SelectorMode {
    classify_token(&spec.token)
}

impl CompiledSelector {
    pub(crate) fn parse(raw: &str) -> Self {
        Self::from_key_spec(KeySpec::parse(raw))
    }

    pub(crate) fn from_token(token: String, exact: ExactMode) -> Self {
        Self::from_key_spec(KeySpec {
            token,
            negated: false,
            existence: false,
            exact,
            strict_ambiguous: false,
        })
    }

    pub(crate) fn from_key_spec(key_spec: KeySpec) -> Self {
        let mode = classify_key_spec(&key_spec);
        let path = parse_path(&key_spec.token).ok();
        Self {
            key_spec,
            mode,
            path,
        }
    }

    pub(crate) fn is_structural(&self) -> bool {
        matches!(self.mode, SelectorMode::StructuralPath)
    }

    pub(crate) fn resolve_matches(&self, root: &Value) -> Vec<AddressedValue> {
        match self.mode {
            SelectorMode::StructuralPath => resolve_path_matches(root, self.token(), self.exact()),
            SelectorMode::PermissiveDescendant => {
                resolve_descendant_matches(root, self.token(), self.exact())
            }
        }
    }

    pub(crate) fn token(&self) -> &str {
        &self.key_spec.token
    }

    pub(crate) fn exact(&self) -> ExactMode {
        self.key_spec.exact
    }

    pub(crate) fn path(&self) -> Option<&PathExpression> {
        self.path.as_ref()
    }

    pub(crate) fn collect_dynamic_column(
        &self,
        nested_row: &Value,
    ) -> Option<(String, Vec<Value>)> {
        let path = self.path()?;
        if !self.is_structural() || !path_has_selectors(path) {
            return None;
        }

        let values = evaluate_path(nested_row, path);
        if values.is_empty() {
            return None;
        }

        Some((self.label(), values))
    }

    pub(crate) fn matched_flat_keys(&self, flat_row: &Row) -> Vec<String> {
        if self.is_structural() {
            let Some(path) = self.path() else {
                return Vec::new();
            };
            let Some(exact) = expression_to_flat_key(path) else {
                return Vec::new();
            };
            return flat_row
                .keys()
                .filter(|key| *key == &exact)
                .cloned()
                .collect();
        }

        match_row_keys(flat_row, self.token(), self.exact())
            .into_iter()
            .map(ToOwned::to_owned)
            .collect()
    }

    pub(crate) fn label(&self) -> String {
        if let Some(path) = self.path()
            && let Some(segment) = path.segments.last()
            && let Some(name) = &segment.name
        {
            return name.clone();
        }

        let token = self.token();
        let last = token.rsplit('.').next().unwrap_or(token);
        let head = last.split('[').next().unwrap_or(last);
        if head.is_empty() {
            "value".to_string()
        } else {
            head.to_string()
        }
    }

    pub(crate) fn matches_dynamic_label(&self, label: &str) -> bool {
        if self.label() == label {
            return true;
        }

        let mut row = Row::new();
        row.insert(label.to_string(), Value::Null);
        !match_row_keys(&row, self.token(), self.exact()).is_empty()
    }
}

/// Returns whether `token` should use the structural selector engine rather
/// than permissive descendant matching.
///
/// Bare names like `name` intentionally stay on the permissive path for now.
/// Dotted, indexed, sliced, fanout, or absolute selectors are structural.
pub(crate) fn token_uses_structural_path(token: &str) -> bool {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return false;
    }

    let Ok(path) = parse_path(trimmed) else {
        return false;
    };

    is_structural_path_token(trimmed, &path)
}

/// Collects and deduplicates addressed matches from compiled selectors.
pub(crate) fn collect_compiled_matches<'a, I>(root: &Value, selectors: I) -> Vec<AddressedValue>
where
    I: IntoIterator<Item = &'a CompiledSelector>,
{
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    for selector in selectors {
        for entry in selector.resolve_matches(root) {
            if seen.insert(entry.flat_key.clone()) {
                out.push(entry);
            }
        }
    }

    out
}

/// Rebuilds only the addressed matches and restores the surviving envelope.
pub(crate) fn project_matches(root: &Value, matches: &[AddressedValue]) -> Value {
    json::project_addressed_matches(root, matches)
}

/// Rebuilds addressed matches while preserving original array positions until a
/// later explicit compact pass.
pub(crate) fn project_matches_unfinalized(root: &Value, matches: &[AddressedValue]) -> Value {
    json::project_addressed_matches_unfinalized(root, matches)
}

/// Removes addressed matches while preserving real `null` values elsewhere.
pub(crate) fn remove_matches(root: Value, matches: &[AddressedValue]) -> Value {
    json::remove_addressed_matches(root, matches)
}

/// Applies a leaf transform over addressed matches and rebuilds the result.
pub(crate) fn transform_matches<F>(
    root: &Value,
    matches: &[AddressedValue],
    preserve_terminal_parent_envelope: bool,
    transform: F,
) -> Value
where
    F: Fn(&Value) -> Value + Copy,
{
    json::transform_addressed_matches(root, matches, preserve_terminal_parent_envelope, transform)
}

/// Rebuilds several structural selectors without compacting array holes yet.
pub(crate) fn project_compiled_unfinalized<'a, I>(root: &Value, selectors: I) -> Value
where
    I: IntoIterator<Item = &'a CompiledSelector>,
{
    let matches = collect_compiled_matches(root, selectors);
    if matches.is_empty() {
        Value::Null
    } else {
        project_matches_unfinalized(root, &matches)
    }
}

/// Removes the union of several compiled structural selectors.
pub(crate) fn remove_compiled<'a, I>(root: Value, selectors: I) -> Value
where
    I: IntoIterator<Item = &'a CompiledSelector>,
{
    let matches = collect_compiled_matches(&root, selectors);
    if matches.is_empty() {
        root
    } else {
        remove_matches(root, &matches)
    }
}

/// Projects structural addressed matches into row output.
///
/// This intentionally differs from `project_matches`: row-mode selector verbs
/// want tabular leaf rows when the matches are array elements, not rebuilt
/// document envelopes. Non-leaf structural matches still materialize through
/// the addressed tree and compact sparse holes before returning.
pub(crate) fn project_row_matches(matches: &[AddressedValue]) -> Vec<Row> {
    if matches.is_empty() {
        return Vec::new();
    }

    if matches
        .iter()
        .all(|entry| matches!(entry.address.last(), Some(AddressStep::Index(_))))
    {
        return matches
            .iter()
            .map(addressed_leaf_row)
            .filter(|row| !row.is_empty())
            .collect();
    }

    let mut projected = materialize_path_matches(matches);
    compact_sparse_arrays(&mut projected);
    match projected {
        Value::Null => Vec::new(),
        Value::Object(map) => vec![map],
        _ => Vec::new(),
    }
}

/// Recursive descendant filter for permissive quick/filter matching.
///
/// Contract:
///
/// - objects narrow to the child fields that matched
/// - arrays keep only matching elements
/// - leaf array records may stay whole when that still narrows the array
/// - if whole-element retention would make the array branch a no-op, the
///   element narrows instead
/// - unrelated ancestor siblings do not survive just because a descendant did
///
/// Structural path selectors should use the addressed rewrite helpers above
/// instead.
pub(crate) fn filter_descendants<F>(value: Value, predicate: F) -> Result<Value>
where
    F: Fn(&Row) -> bool + Copy,
{
    filter_descendants_with_options(value, predicate, true)
}

pub(crate) fn filter_descendants_with_options<F>(
    value: Value,
    predicate: F,
    allow_container_key_match: bool,
) -> Result<Value>
where
    F: Fn(&Row) -> bool + Copy,
{
    filter_descendants_in_context(value, predicate, false, allow_container_key_match)
}

fn filter_descendants_in_context<F>(
    value: Value,
    predicate: F,
    preserve_item_siblings: bool,
    allow_container_key_match: bool,
) -> Result<Value>
where
    F: Fn(&Row) -> bool + Copy,
{
    match value {
        Value::Object(map) => filter_object_descendants(
            map,
            predicate,
            preserve_item_siblings,
            allow_container_key_match,
        ),
        Value::Array(items) => {
            filter_array_descendants(items, predicate, allow_container_key_match)
        }
        scalar => {
            if predicate(&single_value_row(&scalar)) {
                Ok(scalar)
            } else {
                Ok(Value::Null)
            }
        }
    }
}

fn filter_array_descendants<F>(
    items: Vec<Value>,
    predicate: F,
    allow_container_key_match: bool,
) -> Result<Value>
where
    F: Fn(&Row) -> bool + Copy,
{
    let mut blunt = Vec::new();
    let mut narrow = Vec::new();
    let original = items.clone();

    for item in items {
        // Arrays keep matching elements as the main retention unit. Leaf rows
        // still get a narrower fallback so a singleton match does not degrade
        // into a fake no-op like `[{"name":"doctor"}] | doctor`.
        let narrowed = match &item {
            Value::Object(map) if json::is_leaf_record_map(map) => filter_descendants_in_context(
                item.clone(),
                predicate,
                false,
                allow_container_key_match,
            )?,
            Value::Object(_) => filter_descendants_in_context(
                item.clone(),
                predicate,
                true,
                allow_container_key_match,
            )?,
            _ => filter_descendants_in_context(
                item.clone(),
                predicate,
                false,
                allow_container_key_match,
            )?,
        };

        if json::is_structurally_empty(&narrowed) {
            continue;
        }

        narrow.push(narrowed.clone());
        if should_keep_array_item_whole(&item) {
            blunt.push(item);
        } else {
            blunt.push(narrowed);
        }
    }

    let blunt_value = Value::Array(blunt);
    if blunt_value == Value::Array(original) {
        Ok(Value::Array(narrow))
    } else {
        Ok(blunt_value)
    }
}

fn filter_object_descendants<F>(
    map: serde_json::Map<String, Value>,
    predicate: F,
    preserve_item_siblings: bool,
    allow_container_key_match: bool,
) -> Result<Value>
where
    F: Fn(&Row) -> bool + Copy,
{
    let mut out = serde_json::Map::new();
    let mut deferred_siblings = Vec::new();
    let mut preserved_child = false;

    for (key, child) in map {
        let keep_as_envelope_field = json::is_envelope_field(&child);
        let keep_whole_by_key = allow_container_key_match
            && matches!(child, Value::Object(_) | Value::Array(_))
            && predicate(&single_field_row(&key, &Value::Null));
        if keep_whole_by_key {
            preserved_child = true;
            out.insert(key, child);
            continue;
        }

        let transformed = match &child {
            Value::Object(_) | Value::Array(_) => filter_descendants_in_context(
                child.clone(),
                predicate,
                false,
                allow_container_key_match,
            )?,
            _ if should_match_field_as_whole(&child)
                && predicate(&single_field_row(&key, &child)) =>
            {
                match &child {
                    Value::Array(_) => {
                        let narrowed = filter_descendants_in_context(
                            child.clone(),
                            predicate,
                            false,
                            allow_container_key_match,
                        )?;
                        if json::is_structurally_empty(&narrowed) {
                            child.clone()
                        } else {
                            narrowed
                        }
                    }
                    _ => child.clone(),
                }
            }
            _ => Value::Null,
        };

        if !json::is_structurally_empty(&transformed) {
            preserved_child = true;
            out.insert(key, transformed);
        } else if preserve_item_siblings && keep_as_envelope_field {
            deferred_siblings.push((key, child));
        }
    }

    if preserve_item_siblings && preserved_child {
        for (key, value) in deferred_siblings {
            out.entry(key).or_insert(value);
        }
    }

    Ok(Value::Object(out))
}

/// Recursive envelope-preserving traversal for permissive descendant
/// projection.
///
/// Leaf record objects are projected through `project_leaf_rows`. Non-leaf
/// containers recurse first, preserving envelope metadata around surviving
/// descendants. If that recursive descent finds nothing, `project_leaf_rows`
/// gets one more chance on the whole object so field-relative selectors like
/// `entries[]` can still project from the current shell. Collection arrays use
/// `project_collections` as the degrade bridge when descendant recursion
/// produces nothing.
pub(crate) fn project_descendants<FLeaf, FCollections>(
    value: Value,
    project_leaf_rows: FLeaf,
    project_collections: FCollections,
) -> Result<Value>
where
    FLeaf: Fn(Vec<Row>) -> Result<Value> + Copy,
    FCollections: Fn(Value) -> Result<Value> + Copy,
{
    match value {
        Value::Object(map) if json::is_leaf_record_map(&map) => project_leaf_rows(vec![map]),
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, child) in &map {
                let projected =
                    project_descendants(child.clone(), project_leaf_rows, project_collections)?;
                if !json::is_structurally_empty(&projected) {
                    out.insert(key.clone(), projected);
                }
            }
            if !out.is_empty() {
                for (key, child) in &map {
                    if !out.contains_key(key) && json::is_envelope_field(child) {
                        out.insert(key.clone(), child.clone());
                    }
                }
                Ok(Value::Object(out))
            } else {
                project_leaf_rows(vec![map])
            }
        }
        Value::Array(items) if json::is_collection_array(&items) => {
            let mut out = Vec::new();
            for item in &items {
                let projected =
                    project_descendants(item.clone(), project_leaf_rows, project_collections)?;
                if !json::is_structurally_empty(&projected) {
                    out.push(projected);
                }
            }
            if !out.is_empty() {
                Ok(Value::Array(out))
            } else {
                project_collections(Value::Array(items))
            }
        }
        Value::Array(items) => Ok(Value::Array(
            items
                .into_iter()
                .flat_map(|item| {
                    match project_descendants(item, project_leaf_rows, project_collections) {
                        Ok(Value::Array(values)) => values.into_iter().map(Ok).collect::<Vec<_>>(),
                        Ok(other) if !json::is_structurally_empty(&other) => vec![Ok(other)],
                        Ok(_) => Vec::new(),
                        Err(err) => vec![Err(err)],
                    }
                })
                .collect::<Result<Vec<_>>>()?,
        )),
        _ => Ok(Value::Null),
    }
}

fn single_field_row(key: &str, value: &Value) -> Row {
    let mut row = Row::new();
    row.insert(key.to_string(), value.clone());
    row
}

fn path_has_selectors(path: &PathExpression) -> bool {
    path.segments
        .iter()
        .any(|segment| !segment.selectors.is_empty())
}

fn addressed_leaf_row(entry: &AddressedValue) -> Row {
    match &entry.value {
        Value::Object(map) => map.clone(),
        scalar => {
            let mut row = Row::new();
            row.insert("value".to_string(), scalar.clone());
            row
        }
    }
}

fn single_value_row(value: &Value) -> Row {
    single_field_row("value", value)
}

fn should_match_field_as_whole(value: &Value) -> bool {
    !matches!(value, Value::Array(_) | Value::Object(_))
}

fn should_keep_array_item_whole(value: &Value) -> bool {
    matches!(
        value,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
    ) || matches!(value, Value::Object(map) if json::is_leaf_record_map(map))
}
