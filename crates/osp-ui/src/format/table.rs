use std::collections::BTreeSet;

use osp_core::row::Row;

use crate::document::{TableBlock, TableStyle};

use super::common::value_to_display;

pub fn build_table_block(rows: &[Row], style: TableStyle) -> TableBlock {
    let headers = collect_headers(rows);
    let rendered_rows = rows
        .iter()
        .map(|row| {
            headers
                .iter()
                .map(|key| row.get(key).map(value_to_display).unwrap_or_default())
                .collect::<Vec<String>>()
        })
        .collect::<Vec<Vec<String>>>();

    TableBlock {
        style,
        headers,
        rows: rendered_rows,
    }
}

fn collect_headers(rows: &[Row]) -> Vec<String> {
    let mut set = BTreeSet::new();
    for row in rows {
        for key in row.keys() {
            set.insert(key.clone());
        }
    }
    set.into_iter().collect()
}
