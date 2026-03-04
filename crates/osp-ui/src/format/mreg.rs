use osp_core::row::Row;
use serde_json::Value;

use crate::document::{MregBlock, MregEntry, MregRow, MregValue};

pub fn build_mreg_block(rows: &[Row]) -> MregBlock {
    let mut block_rows = Vec::new();

    for row in rows {
        let mut keys = row.keys().cloned().collect::<Vec<String>>();
        keys.sort();

        let entries = keys
            .into_iter()
            .filter_map(|key| row.get(&key).map(|value| (key, value)))
            .map(|(key, value)| MregEntry {
                key,
                depth: 0,
                value: match value {
                    Value::Array(items) if !items.is_empty() => MregValue::List(items.clone()),
                    _ => MregValue::Scalar(value.clone()),
                },
            })
            .collect::<Vec<MregEntry>>();

        block_rows.push(MregRow { entries });
    }

    MregBlock { rows: block_rows }
}
