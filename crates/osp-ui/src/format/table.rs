use std::collections::BTreeSet;

use osp_core::row::Row;
use serde_json::Value;

use crate::document::{TableBlock, TableStyle};

pub fn build_table_block(
    rows: &[Row],
    style: TableStyle,
    preferred_key_order: Option<&[String]>,
) -> TableBlock {
    let headers = collect_headers(rows, preferred_key_order);
    let rendered_rows = rows
        .iter()
        .map(|row| {
            headers
                .iter()
                .map(|key| row.get(key).cloned().unwrap_or(Value::Null))
                .collect::<Vec<Value>>()
        })
        .collect::<Vec<Vec<Value>>>();

    TableBlock {
        style,
        headers,
        rows: rendered_rows,
        header_pairs: Vec::new(),
        align: None,
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
    use crate::document::TableStyle;
    use osp_core::row::Row;
    use serde_json::json;

    #[test]
    fn respects_preferred_key_order_when_available() {
        let mut row = Row::new();
        row.insert("uid".to_string(), json!("alice"));
        row.insert("group".to_string(), json!("ops"));
        let preferred = vec!["group".to_string(), "uid".to_string()];
        let table = build_table_block(&[row], TableStyle::Grid, Some(&preferred));
        assert_eq!(table.headers, preferred);
    }

    #[test]
    fn appends_remaining_headers_after_preferred_order() {
        let mut row = Row::new();
        row.insert("uid".to_string(), json!("alice"));
        row.insert("group".to_string(), json!("ops"));
        row.insert("role".to_string(), json!("admin"));
        let preferred = vec!["group".to_string()];
        let table = build_table_block(&[row], TableStyle::Grid, Some(&preferred));
        assert_eq!(table.headers[0], "group");
        assert_eq!(
            table.headers[1..].to_vec(),
            vec!["role".to_string(), "uid".to_string()]
        );
    }
}
