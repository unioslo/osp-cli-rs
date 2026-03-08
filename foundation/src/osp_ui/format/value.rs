use crate::osp_core::row::Row;

use crate::osp_ui::document::ValueBlock;

use super::common::value_to_display;

pub fn build_value_block(rows: &[Row]) -> ValueBlock {
    ValueBlock {
        values: rows
            .iter()
            .filter_map(|row| row.get("value"))
            .map(value_to_display)
            .collect(),
    }
}
