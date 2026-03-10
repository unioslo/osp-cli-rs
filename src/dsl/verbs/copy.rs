use crate::core::row::Row;

/// Returns `rows` unchanged.
///
/// The canonical DSL keeps this local even though the behavior is trivial so
/// execution ownership stays in `src/dsl/verbs/*`.
pub fn apply(rows: Vec<Row>) -> Vec<Row> {
    rows
}

#[cfg(test)]
mod tests {
    use super::apply;
    use serde_json::json;

    #[test]
    fn copy_stage_is_identity_for_rows_unit() {
        let rows = vec![
            json!({"uid": "alice"})
                .as_object()
                .cloned()
                .expect("object"),
        ];
        assert_eq!(apply(rows.clone()), rows);
    }
}
