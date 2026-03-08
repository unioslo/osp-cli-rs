use std::collections::BTreeSet;

use crate::osp_core::row::Row;
use serde_json::Value;

use crate::osp_ui::document::{TableBlock, TableStyle};

pub fn build_table_block(
    rows: &[Row],
    style: TableStyle,
    preferred_key_order: Option<&[String]>,
    block_id: u64,
) -> TableBlock {
    let headers = collect_headers(rows, preferred_key_order);
    let rendered_rows = rows
        .iter()
        .map(|row| {
            headers
                .iter()
                .map(|key| {
                    row.get(key)
                        .cloned()
                        .unwrap_or_else(|| Value::String(String::new()))
                })
                .collect::<Vec<Value>>()
        })
        .collect::<Vec<Vec<Value>>>();

    TableBlock {
        block_id,
        style,
        headers,
        rows: rendered_rows,
        header_pairs: Vec::new(),
        align: None,
        shrink_to_fit: true,
        depth: 0,
    }
}

fn collect_headers(rows: &[Row], preferred_key_order: Option<&[String]>) -> Vec<String> {
    let mut headers = Vec::new();
    let mut remaining = BTreeSet::new();

    for row in rows {
        for key in row.keys() {
            remaining.insert(key.clone());
        }
    }

    if let Some(order) = preferred_key_order {
        for key in order {
            if remaining.remove(key) {
                headers.push(key.clone());
            }
        }
    }

    headers.extend(remaining);
    headers
}

#[cfg(test)]
mod tests {
    use super::build_table_block;
    use crate::osp_ui::document::TableStyle;
    use crate::osp_core::row::Row;
    use serde_json::json;

    #[test]
    fn respects_preferred_key_order_when_available() {
        let mut row = Row::new();
        row.insert("uid".to_string(), json!("alice"));
        row.insert("group".to_string(), json!("ops"));
        let preferred = vec!["group".to_string(), "uid".to_string()];
        let table = build_table_block(&[row], TableStyle::Grid, Some(&preferred), 1);
        assert_eq!(table.headers, preferred);
    }

    #[test]
    fn appends_remaining_headers_after_preferred_order() {
        let mut row = Row::new();
        row.insert("uid".to_string(), json!("alice"));
        row.insert("group".to_string(), json!("ops"));
        row.insert("role".to_string(), json!("admin"));
        let preferred = vec!["group".to_string()];
        let table = build_table_block(&[row], TableStyle::Grid, Some(&preferred), 1);
        assert_eq!(table.headers[0], "group");
        assert_eq!(
            table.headers[1..].to_vec(),
            vec!["role".to_string(), "uid".to_string()]
        );
    }

    #[test]
    fn missing_values_render_as_empty_cells_not_null() {
        let mut row1 = Row::new();
        row1.insert("host".to_string(), json!("login1.uio.no"));
        row1.insert("vlan".to_string(), json!("303"));
        let mut row2 = Row::new();
        row2.insert("host".to_string(), json!("login2.uio.no"));

        let preferred = vec!["host".to_string(), "vlan".to_string()];
        let table = build_table_block(&[row1, row2], TableStyle::Grid, Some(&preferred), 1);
        assert_eq!(table.rows[1][1], json!(""));
    }

    #[test]
    fn collects_headers_in_sorted_order_without_preferred_keys() {
        let mut row = Row::new();
        row.insert("uid".to_string(), json!("alice"));
        row.insert("group".to_string(), json!("ops"));
        row.insert("role".to_string(), json!("admin"));

        let table = build_table_block(&[row], TableStyle::Grid, None, 1);

        assert_eq!(
            table.headers,
            vec!["group".to_string(), "role".to_string(), "uid".to_string()]
        );
    }
}
