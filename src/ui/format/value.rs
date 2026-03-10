use crate::core::row::Row;

use crate::ui::document::ValueBlock;

use super::common::value_to_display;

/// Builds a value block from the `"value"` field of each row.
///
/// Rows without a `"value"` field are skipped.
pub fn build_value_block(rows: &[Row]) -> ValueBlock {
    ValueBlock {
        values: rows
            .iter()
            .filter_map(|row| row.get("value"))
            .map(value_to_display)
            .collect(),
    }
}
