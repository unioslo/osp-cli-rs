use crate::core::row::Row;
use serde_json::Value;

#[cfg(test)]
pub fn apply(
    items: crate::core::output_model::OutputItems,
    spec: &str,
) -> anyhow::Result<crate::core::output_model::OutputItems> {
    let stage = if spec.trim().is_empty() {
        crate::dsl::compiled::CompiledStage::Clean
    } else {
        crate::dsl::compiled::CompiledStage::Question(super::quick::compile(&format!(
            "?{}",
            spec.trim()
        ))?)
    };
    crate::dsl::engine::apply_stage(items.into(), &stage).map(Into::into)
}

pub(crate) fn clean_rows(rows: Vec<Row>) -> Vec<Row> {
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

pub(crate) fn is_empty_value(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.is_empty(),
        Value::Array(items) => items.is_empty(),
        _ => false,
    }
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
