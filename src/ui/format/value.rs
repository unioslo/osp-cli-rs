use crate::core::row::Row;

use crate::ui::document::ValueBlock;

use super::common::value_to_display;

/// Builds a value block from the surviving field values of each row.
///
/// Value mode is the RHS renderer for the post-pipeline result. It should not
/// require a literal `value` column: if a pipeline narrows a row down to
/// `{"muted": "#89b4fa"}`, value mode should render `#89b4fa`.
pub fn build_value_block(rows: &[Row], key_order: Option<&[String]>) -> ValueBlock {
    ValueBlock {
        values: rows
            .iter()
            .flat_map(|row| row_value_displays(row, key_order))
            .collect(),
    }
}

fn row_value_displays(row: &Row, key_order: Option<&[String]>) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();

    if let Some(order) = key_order {
        for key in order {
            if !seen.insert(key.clone()) {
                continue;
            }
            if let Some(value) = row.get(key)
                && !value.is_null()
            {
                out.push(value_to_display(value));
            }
        }
    }

    let mut extras = row
        .keys()
        .filter(|key| !seen.contains(*key))
        .cloned()
        .collect::<Vec<_>>();
    extras.sort();

    for key in extras {
        if let Some(value) = row.get(&key)
            && !value.is_null()
        {
            out.push(value_to_display(value));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::build_value_block;
    use serde_json::json;

    #[test]
    fn single_non_value_column_renders_its_rhs_value_unit() {
        let rows = vec![
            json!({"muted": "#89b4fa"})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let block = build_value_block(&rows, Some(&["muted".to_string()]));
        assert_eq!(block.values, vec!["#89b4fa".to_string()]);
    }

    #[test]
    fn multi_column_rows_follow_key_order_and_skip_nulls_unit() {
        let rows = vec![
            json!({
                "id": "catppuccin",
                "base": null,
                "muted": "#89b4fa",
            })
            .as_object()
            .cloned()
            .expect("object"),
        ];

        let block = build_value_block(
            &rows,
            Some(&["id".to_string(), "base".to_string(), "muted".to_string()]),
        );
        assert_eq!(
            block.values,
            vec!["catppuccin".to_string(), "#89b4fa".to_string()]
        );
    }
}
