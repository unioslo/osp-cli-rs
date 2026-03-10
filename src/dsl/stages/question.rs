use crate::core::{
    output_model::{Group, OutputItems},
    row::Row,
};
use anyhow::Result;
use serde_json::Value;

use crate::dsl::stages::{common::map_group_rows, quick};

/// Applies the `?` stage to flat or grouped output.
///
/// With an empty spec, empty values are removed from rows. Otherwise the stage
/// delegates to quick matching using `?`-prefixed semantics.
pub fn apply(items: OutputItems, spec: &str) -> Result<OutputItems> {
    let trimmed = spec.trim();
    if trimmed.is_empty() {
        return Ok(clean_items(items));
    }

    let raw = format!("?{trimmed}");
    let out = match items {
        OutputItems::Rows(rows) => OutputItems::Rows(quick::apply(rows, &raw)?),
        OutputItems::Groups(groups) => OutputItems::Groups(apply_groups_quick(groups, &raw)?),
    };
    Ok(out)
}

fn clean_items(items: OutputItems) -> OutputItems {
    match items {
        OutputItems::Rows(rows) => OutputItems::Rows(clean_rows(rows)),
        OutputItems::Groups(groups) => OutputItems::Groups(
            groups
                .into_iter()
                .map(|group| Group {
                    groups: group.groups,
                    aggregates: group.aggregates,
                    rows: clean_rows(group.rows),
                })
                .collect(),
        ),
    }
}

fn clean_rows(rows: Vec<Row>) -> Vec<Row> {
    rows.into_iter().filter_map(clean_row).collect()
}

pub(crate) fn clean_row(row: Row) -> Option<Row> {
    let cleaned = row
        .into_iter()
        .filter(|(_, value)| !is_empty_value(value))
        .collect::<Row>();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn is_empty_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.is_empty(),
        Value::Array(items) => items.is_empty(),
        _ => false,
    }
}

fn apply_groups_quick(groups: Vec<Group>, raw: &str) -> Result<Vec<Group>> {
    map_group_rows(groups, |rows| quick::apply(rows, raw))
}

#[cfg(test)]
mod tests {
    use crate::core::output_model::{Group, OutputItems};
    use serde_json::json;

    use super::apply;

    fn row(value: serde_json::Value) -> crate::core::row::Row {
        value
            .as_object()
            .cloned()
            .expect("fixture should be an object")
    }

    #[test]
    fn empty_spec_cleans_rows_and_drops_empty_results() {
        let items = OutputItems::Rows(vec![
            row(json!({"uid": "oistes", "mail": "", "tags": [], "note": null})),
            row(json!({"mail": "", "tags": [], "note": null})),
        ]);

        let cleaned = apply(items, "   ").expect("cleaning should succeed");
        let OutputItems::Rows(rows) = cleaned else {
            panic!("expected row output");
        };

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].len(), 1);
        assert_eq!(
            rows[0].get("uid").and_then(|value| value.as_str()),
            Some("oistes")
        );
    }

    #[test]
    fn empty_spec_cleans_group_rows_without_touching_group_metadata() {
        let items = OutputItems::Groups(vec![Group {
            groups: row(json!({"team": "ops"})),
            aggregates: row(json!({"count": 2})),
            rows: vec![
                row(json!({"uid": "oistes", "mail": ""})),
                row(json!({"mail": "", "tags": []})),
            ],
        }]);

        let cleaned = apply(items, "").expect("group cleaning should succeed");
        let OutputItems::Groups(groups) = cleaned else {
            panic!("expected grouped output");
        };

        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0]
                .groups
                .get("team")
                .and_then(|value| value.as_str()),
            Some("ops")
        );
        assert_eq!(
            groups[0]
                .aggregates
                .get("count")
                .and_then(|value| value.as_i64()),
            Some(2)
        );
        assert_eq!(groups[0].rows.len(), 1);
        assert_eq!(groups[0].rows[0].len(), 1);
        assert_eq!(
            groups[0].rows[0]
                .get("uid")
                .and_then(|value| value.as_str()),
            Some("oistes")
        );
    }

    #[test]
    fn non_empty_spec_reuses_quick_filter_for_rows_and_groups() {
        let rows = OutputItems::Rows(vec![
            row(json!({"uid": "oistes"})),
            row(json!({"mail": "other@example.org"})),
        ]);
        let filtered = apply(rows, "uid").expect("row filter should succeed");
        let OutputItems::Rows(rows) = filtered else {
            panic!("expected row output");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].get("uid").and_then(|value| value.as_str()),
            Some("oistes")
        );

        let groups = OutputItems::Groups(vec![Group {
            groups: row(json!({"team": "ops"})),
            aggregates: row(json!({"count": 2})),
            rows: vec![
                row(json!({"uid": "oistes"})),
                row(json!({"mail": "other@example.org"})),
            ],
        }]);
        let filtered = apply(groups, "uid").expect("group filter should succeed");
        let OutputItems::Groups(groups) = filtered else {
            panic!("expected grouped output");
        };
        assert_eq!(groups[0].rows.len(), 1);
        assert_eq!(
            groups[0].rows[0]
                .get("uid")
                .and_then(|value| value.as_str()),
            Some("oistes")
        );
    }
}
