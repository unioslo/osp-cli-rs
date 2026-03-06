use std::collections::BTreeSet;

use osp_core::row::Row;
use serde_json::{Map, Value};
use unicode_width::UnicodeWidthStr;

use crate::display::value_to_display;
use crate::document::{Block, MregBlock, MregEntry, MregRow, MregValue, TableBlock, TableStyle};

pub fn build_mreg_blocks(
    rows: &[Row],
    key_order: Option<&[String]>,
    short_list_max: usize,
    width_hint: usize,
    indent_size: usize,
    prefer_stacked_object_lists: bool,
    stack_min_col_width: usize,
    stack_overflow_ratio: usize,
    next_block_id: &mut u64,
) -> Vec<Block> {
    let mut blocks = Vec::new();
    for row in rows {
        let mut builder = MregBuilder::new(
            short_list_max.max(1),
            width_hint.max(24),
            indent_size.max(1),
            prefer_stacked_object_lists,
            stack_min_col_width.max(1),
            stack_overflow_ratio.max(100),
            next_block_id,
        );
        builder.visit_object(row, 0, key_order);
        builder.flush_entries();
        blocks.extend(builder.blocks);
    }
    blocks
}

struct MregBuilder<'a> {
    blocks: Vec<Block>,
    entries: Vec<MregEntry>,
    short_list_max: usize,
    width_hint: usize,
    indent_size: usize,
    prefer_stacked_object_lists: bool,
    stack_min_col_width: usize,
    stack_overflow_ratio: usize,
    next_block_id: &'a mut u64,
}

impl<'a> MregBuilder<'a> {
    fn new(
        short_list_max: usize,
        width_hint: usize,
        indent_size: usize,
        prefer_stacked_object_lists: bool,
        stack_min_col_width: usize,
        stack_overflow_ratio: usize,
        next_block_id: &'a mut u64,
    ) -> Self {
        Self {
            blocks: Vec::new(),
            entries: Vec::new(),
            short_list_max,
            width_hint,
            indent_size,
            prefer_stacked_object_lists,
            stack_min_col_width,
            stack_overflow_ratio,
            next_block_id,
        }
    }

    fn flush_entries(&mut self) {
        if self.entries.is_empty() {
            return;
        }
        let block_id = self.allocate_block_id();
        let entries = std::mem::take(&mut self.entries);
        self.blocks.push(Block::Mreg(MregBlock {
            block_id,
            rows: vec![MregRow { entries }],
        }));
    }

    fn push_group_entry(&mut self, key: String, depth: usize) {
        self.entries.push(MregEntry {
            key,
            depth,
            value: MregValue::Group,
        });
    }

    fn push_scalar_entry(&mut self, key: String, depth: usize, value: Value) {
        self.entries.push(MregEntry {
            key,
            depth,
            value: MregValue::Scalar(value),
        });
    }

    fn push_separator_entry(&mut self, depth: usize) {
        self.entries.push(MregEntry {
            key: String::new(),
            depth,
            value: MregValue::Separator,
        });
    }

    fn push_list_entry(&mut self, key: String, depth: usize, values: Vec<Value>) {
        self.entries.push(MregEntry {
            key,
            depth,
            value: MregValue::List(values),
        });
    }

    fn visit_object(
        &mut self,
        map: &Map<String, Value>,
        depth: usize,
        preferred_order: Option<&[String]>,
    ) {
        for key in ordered_keys(map, preferred_order) {
            if let Some(value) = map.get(&key) {
                self.visit_value(&key, value, depth);
            }
        }
    }

    fn visit_value(&mut self, key: &str, value: &Value, depth: usize) {
        match value {
            Value::Object(map) => {
                self.push_group_entry(key.to_string(), depth);
                self.visit_object(map, depth + 1, None);
            }
            Value::Array(items) => {
                if items.is_empty() {
                    self.push_scalar_entry(key.to_string(), depth, Value::String("[]".to_string()));
                    return;
                }

                let key_with_count = label_with_count(key, items.len());
                if items.iter().any(Value::is_object)
                    && let Some(mut table) = table_block_from_object_list(items, depth + 1)
                {
                    if self.prefer_stacked_object_lists
                        && should_stack_object_list_table(
                            &table,
                            self.width_hint,
                            self.indent_size,
                            depth + 1,
                            self.stack_min_col_width,
                            self.stack_overflow_ratio,
                        )
                    {
                        self.push_group_entry(key_with_count, depth);
                        self.visit_object_list(items, depth + 1);
                        return;
                    }
                    self.push_group_entry(key_with_count, depth);
                    self.flush_entries();
                    table.block_id = self.allocate_block_id();
                    self.blocks.push(Block::Table(table));
                    return;
                }

                if items.len() <= self.short_list_max {
                    if let Some(first) = items.first() {
                        self.push_scalar_entry(key_with_count, depth, first.clone());
                    }
                    return;
                }

                self.push_list_entry(key_with_count, depth, items.clone());
            }
            _ => self.push_scalar_entry(key.to_string(), depth, value.clone()),
        }
    }

    fn visit_object_list(&mut self, items: &[Value], depth: usize) {
        for (index, item) in items.iter().enumerate() {
            let Value::Object(map) = item else {
                continue;
            };
            if index > 0 {
                self.push_separator_entry(depth);
            }
            self.visit_object(map, depth, None);
        }
    }
    fn allocate_block_id(&mut self) -> u64 {
        let id = *self.next_block_id;
        *self.next_block_id = self.next_block_id.saturating_add(1);
        id
    }
}

fn label_with_count(key: &str, count: usize) -> String {
    if count > 1 {
        format!("{key} ({count})")
    } else {
        key.to_string()
    }
}

fn ordered_keys(map: &Map<String, Value>, preferred_order: Option<&[String]>) -> Vec<String> {
    let mut keys = Vec::new();
    let mut seen = BTreeSet::new();

    if let Some(order) = preferred_order {
        for key in order {
            if map.contains_key(key) && seen.insert(key.clone()) {
                keys.push(key.clone());
            }
        }
    }

    for key in map.keys() {
        if seen.insert(key.clone()) {
            keys.push(key.clone());
        }
    }

    keys
}

fn table_block_from_object_list(items: &[Value], depth: usize) -> Option<TableBlock> {
    let mut headers = Vec::new();
    let mut seen = BTreeSet::new();

    for item in items {
        let Value::Object(map) = item else {
            continue;
        };
        for key in map.keys() {
            if seen.insert(key.clone()) {
                headers.push(key.clone());
            }
        }
    }

    if headers.is_empty() {
        return None;
    }

    let mut rows = Vec::new();
    for item in items {
        if let Value::Object(map) = item {
            rows.push(
                headers
                    .iter()
                    .map(|header| map.get(header).cloned().unwrap_or(Value::Null))
                    .collect(),
            );
        } else {
            rows.push(headers.iter().map(|_| item.clone()).collect());
        }
    }

    Some(TableBlock {
        block_id: 0,
        style: TableStyle::Grid,
        headers,
        rows,
        header_pairs: Vec::new(),
        align: None,
        shrink_to_fit: false,
        depth,
    })
}

fn should_stack_object_list_table(
    table: &TableBlock,
    width_hint: usize,
    indent_size: usize,
    depth: usize,
    stack_min_col_width: usize,
    stack_overflow_ratio: usize,
) -> bool {
    let estimated_width = estimate_table_width(table).max(1);
    let available_width = width_hint
        .saturating_sub(depth.saturating_mul(indent_size))
        .max(24);

    if estimated_width <= available_width {
        return false;
    }

    let columns = table.headers.len().max(1);
    let border = columns * 3 + 1;
    let content_budget = available_width.saturating_sub(border);
    let avg_column_budget = content_budget / columns;

    // If we're forced into aggressively tiny columns, stacked output is easier to scan.
    let min_col_budget = stack_min_col_width.max(4);
    let overflow_ratio = stack_overflow_ratio.max(100);
    let ratio_trigger =
        estimated_width.saturating_mul(100) > available_width.saturating_mul(overflow_ratio);

    avg_column_budget < min_col_budget || ratio_trigger
}

fn estimate_table_width(table: &TableBlock) -> usize {
    let mut widths = table
        .headers
        .iter()
        .map(|header| display_width(header).max(1))
        .collect::<Vec<usize>>();

    for row in &table.rows {
        for (idx, cell) in row.iter().enumerate() {
            if let Some(width) = widths.get_mut(idx) {
                *width = (*width).max(display_width(&value_to_display(cell)).max(1));
            }
        }
    }

    widths.iter().sum::<usize>() + widths.len() * 3 + 1
}

fn display_width(value: &str) -> usize {
    UnicodeWidthStr::width(value)
}

#[cfg(test)]
mod tests {
    use super::build_mreg_blocks;
    use crate::document::{Block, MregValue};
    use serde_json::{Map, Value, json};

    fn sample_row() -> Map<String, Value> {
        let mut row = Map::new();
        row.insert(
            "networks".to_string(),
            Value::Array(vec![
                json!({
                    "policy": Value::Null,
                    "network": "129.240.130.0/24",
                    "description": "knh-klientnett-2 (statisk DHCP)",
                    "vlan": 200
                }),
                json!({
                    "policy": Value::Null,
                    "network": "2001:700:100:4003::/64",
                    "description": "usit-knh",
                    "vlan": 200
                }),
            ]),
        );
        row
    }

    #[test]
    fn keeps_object_lists_as_tables_in_plain_mode() {
        let mut next_block_id = 1;
        let blocks = build_mreg_blocks(
            &[sample_row()],
            None,
            1,
            80,
            2,
            false,
            10,
            200,
            &mut next_block_id,
        );
        assert!(blocks.iter().any(|block| matches!(block, Block::Table(_))));
    }

    #[test]
    fn stacks_object_lists_when_rich_width_is_tight() {
        let mut next_block_id = 1;
        let blocks = build_mreg_blocks(
            &[sample_row()],
            None,
            1,
            40,
            2,
            true,
            10,
            200,
            &mut next_block_id,
        );
        assert!(!blocks.iter().any(|block| matches!(block, Block::Table(_))));
        let has_separator = blocks.iter().any(|block| match block {
            Block::Mreg(mreg) => mreg.rows.iter().any(|row| {
                row.entries
                    .iter()
                    .any(|entry| matches!(entry.value, MregValue::Separator))
            }),
            _ => false,
        });
        assert!(has_separator);
    }
}
