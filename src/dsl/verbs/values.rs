//! `VALUE` / `VAL` extraction logic.
//!
//! `VALUE` is intentionally transforming rather than filtering. It keeps the
//! selector surface but changes matched leaves into `{value: ...}` rows.
//!
//! Examples:
//! - row input `VALUE uid` becomes `[{"value": "alice"}, ...]`
//! - semantic input `VALUE sections[0].entries[0].name` becomes a narrowed
//!   semantic tree whose targeted leaf is wrapped as `{"value": ...}`
//! - extracting sibling fields from the same object must keep field identity
//!   instead of collapsing into an unlabeled array

use crate::core::{output_model::Group, row::Row};
use anyhow::Result;
use serde_json::{Map, Value};

use crate::dsl::eval::resolve::resolve_values;

use crate::dsl::verbs::common::{map_group_rows, parse_terms};

use super::{json, selector};

#[derive(Debug, Clone, Default)]
pub(crate) struct ValuesPlan {
    selectors: Vec<selector::CompiledSelector>,
}

impl ValuesPlan {
    pub(crate) fn extract_row(&self, row: &Row) -> Vec<Row> {
        let mut out = Vec::new();

        if self.selectors.is_empty() {
            for value in row.values() {
                emit_value_rows(&mut out, value);
            }
            return out;
        }

        for selector in &self.selectors {
            for value in resolve_values(row, selector.token(), selector.exact()) {
                emit_value_rows(&mut out, &value);
            }
        }

        out
    }
}

pub(crate) fn compile(spec: &str) -> Result<ValuesPlan> {
    Ok(ValuesPlan {
        selectors: parse_terms(spec)?
            .into_iter()
            .map(|token| {
                selector::CompiledSelector::from_token(
                    token,
                    crate::dsl::parse::key_spec::ExactMode::CaseSensitive,
                )
            })
            .collect(),
    })
}

#[cfg(test)]
/// Extracts values from flat rows and emits `{value: ...}` rows.
///
/// With an empty spec, values from every field are emitted in row order.
pub fn apply(rows: Vec<Row>, spec: &str) -> Result<Vec<Row>> {
    let plan = compile(spec)?;
    apply_with_plan(rows, &plan)
}

pub(crate) fn apply_with_plan(rows: Vec<Row>, plan: &ValuesPlan) -> Result<Vec<Row>> {
    let mut out: Vec<Row> = Vec::new();

    for row in rows {
        out.extend(plan.extract_row(&row));
    }

    Ok(out)
}

pub(crate) fn apply_groups_with_plan(groups: Vec<Group>, plan: &ValuesPlan) -> Result<Vec<Group>> {
    map_group_rows(groups, |rows| {
        let mut out = Vec::new();
        for row in &rows {
            out.extend(plan.extract_row(row));
        }
        Ok(out)
    })
}

fn emit_value_rows(out: &mut Vec<Row>, value: &Value) {
    match value {
        Value::Array(values) => {
            for item in values {
                let mut row = Map::new();
                row.insert("value".to_string(), item.clone());
                out.push(row);
            }
        }
        _ => {
            let mut row = Map::new();
            row.insert("value".to_string(), value.clone());
            out.push(row);
        }
    }
}

pub(crate) fn apply_value_with_plan(value: Value, plan: &ValuesPlan) -> Result<Value> {
    if let Some(extracted) = try_extract_semantic_values(&value, plan) {
        return Ok(extracted);
    }

    match value {
        Value::Array(items) if items.iter().all(json::is_scalar_like) => Ok(Value::Array(
            items
                .into_iter()
                .map(|item| {
                    let mut row = Map::new();
                    row.insert("value".to_string(), item);
                    Value::Object(row)
                })
                .collect(),
        )),
        other => json::traverse_collections(other, |items| match items {
            crate::core::output_model::OutputItems::Rows(rows) => Ok(
                crate::core::output_model::OutputItems::Rows(apply_with_plan(rows, plan)?),
            ),
            crate::core::output_model::OutputItems::Groups(groups) => {
                Ok(crate::core::output_model::OutputItems::Groups(
                    apply_groups_with_plan(groups, plan)?,
                ))
            }
        }),
    }
}

fn try_extract_semantic_values(root: &Value, plan: &ValuesPlan) -> Option<Value> {
    if plan.selectors.is_empty() {
        return None;
    }

    let matches = selector::collect_compiled_matches(root, plan.selectors.iter());

    if matches.is_empty() {
        return if matches!(root, Value::Array(_)) {
            None
        } else {
            Some(Value::Null)
        };
    }

    // `VALUE` is an explicitly transforming stage. The semantic payload stays
    // canonical JSON, but the targeted leaves become `{value: ...}` rows so
    // downstream stages operate on a real transformed tree instead of relying
    // on collection-only row projection.
    let extracted = selector::transform_matches(root, &matches, false, value_to_rows);
    let extracted = collapse_extracted_field_wrappers(extracted);
    Some(collapse_root_value_collection(extracted))
}

fn value_to_rows(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(wrap_value_row).collect()),
        scalar => wrap_value_row(scalar),
    }
}

fn wrap_value_row(value: &Value) -> Value {
    let mut row = Map::new();
    row.insert("value".to_string(), value.clone());
    Value::Object(row)
}

fn collapse_extracted_field_wrappers(value: Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(collapse_extracted_field_wrappers)
                .collect(),
        ),
        Value::Object(map) => {
            let collapsed = map
                .into_iter()
                .map(|(key, value)| (key, collapse_extracted_field_wrappers(value)))
                .collect::<Map<_, _>>();

            if collapsed.len() == 1
                && let Some(only_value) = collapsed.values().next()
                && is_value_row(only_value)
            {
                return only_value.clone();
            }

            Value::Object(collapsed)
        }
        scalar => scalar,
    }
}

fn is_value_row(value: &Value) -> bool {
    matches!(value, Value::Object(map) if map.len() == 1 && map.contains_key("value"))
}

fn collapse_root_value_collection(value: Value) -> Value {
    let Value::Object(map) = value else {
        return value;
    };
    if map.len() != 1 {
        return Value::Object(map);
    }

    let (only_key, only_value) = map.into_iter().next().expect("single entry ensured above");
    // `VALUE` is fundamentally a row extractor. When the semantic transform
    // narrows the whole payload down to one extracted collection like
    // `{"commands": [{"value": ...}]}`, collapse that wrapper so downstream
    // stages see the value rows directly instead of a synthetic one-key shell.
    if matches!(&only_value, Value::Array(items) if items.iter().all(is_value_row)) {
        only_value
    } else {
        let mut restored = Map::new();
        restored.insert(only_key, only_value);
        Value::Object(restored)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{apply, apply_value_with_plan, compile};

    #[test]
    fn explodes_array_values() {
        let rows = vec![
            json!({"members": ["a", "b"]})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let output = apply(rows, "members").expect("values should work");
        assert_eq!(output.len(), 2);
    }

    #[test]
    fn emits_requested_scalar_values_and_ignores_missing_keys() {
        let rows = vec![
            json!({"uid": "oistes", "mail": "oistes@example.org"})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let output = apply(rows, "uid missing").expect("values should work");
        assert_eq!(output.len(), 1);
        assert_eq!(
            output[0].get("value").and_then(|value| value.as_str()),
            Some("oistes")
        );
    }

    #[test]
    fn empty_spec_emits_all_scalar_and_array_values_in_order() {
        let rows = vec![
            json!({"uid": "oistes", "members": ["a", "b"], "active": true})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let output = apply(rows, "").expect("empty values stage should enumerate all fields");
        let mut values = output
            .iter()
            .map(|row| {
                row.get("value")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null)
                    .to_string()
            })
            .collect::<Vec<_>>();
        values.sort();

        assert_eq!(values, vec!["\"a\"", "\"b\"", "\"oistes\"", "true"]);
    }

    #[test]
    fn resolves_nested_paths_and_quoted_terms() {
        let rows = vec![
            json!({
                "metadata": {"display,name": "Alice"},
                "members": [{"uid": "alice"}, {"uid": "bob"}]
            })
            .as_object()
            .cloned()
            .expect("object"),
        ];

        let output = apply(rows, "\"metadata.display,name\" members[].uid")
            .expect("nested values should work");
        let values = output
            .iter()
            .map(|row| row.get("value").cloned().expect("value"))
            .collect::<Vec<_>>();

        assert_eq!(values, vec![json!("Alice"), json!("alice"), json!("bob")]);
    }

    #[test]
    fn extracts_top_level_scalar_arrays_from_semantic_payloads() {
        let plan = compile("usage").expect("plan should compile");
        let extracted = apply_value_with_plan(
            json!({
                "usage": ["osp deploy <COMMAND>"],
                "notes": ["read this first"],
                "sections": [
                    {
                        "title": "Commands",
                        "entries": [
                            {"name": "deploy", "short_help": "Apply changes"}
                        ]
                    }
                ]
            }),
            &plan,
        )
        .expect("semantic value extraction should succeed");

        assert_eq!(
            extracted,
            json!([
                {"value": "osp deploy <COMMAND>"}
            ])
        );
    }

    #[test]
    fn extracts_addressed_nested_values_while_preserving_section_shell() {
        let plan = compile("sections[0].entries[0].name").expect("plan should compile");
        let extracted = apply_value_with_plan(
            json!({
                "preamble": ["Deploy commands"],
                "sections": [
                    {
                        "title": "Commands",
                        "kind": "commands",
                        "paragraphs": ["pick one"],
                        "entries": [
                            {"name": "deploy", "short_help": "Apply changes"},
                            {"name": "status", "short_help": "Inspect deployment"}
                        ]
                    }
                ]
            }),
            &plan,
        )
        .expect("semantic value extraction should succeed");

        assert_eq!(
            extracted,
            json!({
                "sections": [
                    {
                        "title": "Commands",
                        "kind": "commands",
                        "paragraphs": ["pick one"],
                        "entries": [
                            {"value": "deploy"}
                        ]
                    }
                ]
            })
        );
    }

    #[test]
    fn missing_semantic_value_path_returns_null() {
        let plan = compile("missing.path").expect("plan should compile");
        let extracted = apply_value_with_plan(
            json!({
                "usage": ["osp deploy <COMMAND>"]
            }),
            &plan,
        )
        .expect("semantic value extraction should succeed");

        assert_eq!(extracted, serde_json::Value::Null);
    }
}
