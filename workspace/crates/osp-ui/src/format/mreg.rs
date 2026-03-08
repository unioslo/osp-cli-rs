use std::collections::BTreeSet;

use osp_core::row::Row;
use serde_json::{Map, Value};

use crate::display::value_to_display;
use crate::document::{Block, MregBlock, MregEntry, MregRow, MregValue, TableBlock, TableStyle};
use crate::width::display_width;

#[derive(Debug, Clone, Copy)]
pub struct MregBuildOptions<'a> {
    pub key_order: Option<&'a [String]>,
    pub short_list_max: usize,
    pub medium_list_max: usize,
    pub width_hint: usize,
    pub indent_size: usize,
    pub prefer_stacked_object_lists: bool,
    pub stack_min_col_width: usize,
    pub stack_overflow_ratio: usize,
}

pub fn build_mreg_blocks(
    rows: &[Row],
    options: MregBuildOptions<'_>,
    next_block_id: &mut u64,
) -> Vec<Block> {
    let mut blocks = Vec::new();
    for row in rows {
        let mut builder = MregBuilder::new(options, next_block_id);
        builder.visit_object(row, 0, options.key_order);
        builder.flush_entries();
        blocks.extend(builder.blocks);
    }
    blocks
}

struct MregBuilder<'a> {
    blocks: Vec<Block>,
    entries: Vec<MregEntry>,
    short_list_max: usize,
    medium_list_max: usize,
    width_hint: usize,
    indent_size: usize,
    prefer_stacked_object_lists: bool,
    stack_min_col_width: usize,
    stack_overflow_ratio: usize,
    next_block_id: &'a mut u64,
}

impl<'a> MregBuilder<'a> {
    fn new(options: MregBuildOptions<'_>, next_block_id: &'a mut u64) -> Self {
        Self {
            blocks: Vec::new(),
            entries: Vec::new(),
            short_list_max: options.short_list_max.max(1),
            medium_list_max: options
                .medium_list_max
                .max(options.short_list_max.max(1) + 1),
            width_hint: options.width_hint.max(24),
            indent_size: options.indent_size.max(1),
            prefer_stacked_object_lists: options.prefer_stacked_object_lists,
            stack_min_col_width: options.stack_min_col_width.max(1),
            stack_overflow_ratio: options.stack_overflow_ratio.max(100),
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

    fn push_vertical_list_entry(&mut self, key: String, depth: usize, values: Vec<Value>) {
        self.entries.push(MregEntry {
            key,
            depth,
            value: MregValue::VerticalList(values),
        });
    }

    fn push_grid_entry(&mut self, key: String, depth: usize, values: Vec<Value>) {
        self.entries.push(MregEntry {
            key,
            depth,
            value: MregValue::Grid(values),
        });
    }

    fn visit_object(
        &mut self,
        map: &Map<String, Value>,
        depth: usize,
        preferred_order: Option<&[String]>,
    ) {
        for key in ordered_keys(map, preferred_order) {
            let value = map
                .get(&key)
                .expect("ordered_keys must only return keys present in the source object");
            self.visit_value(&key, value, depth);
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
                    let stack_due_to_nested_shape = object_list_contains_nested_structures(items);
                    let stack_due_to_width = should_stack_object_list_table(
                        &table,
                        self.width_hint,
                        self.indent_size,
                        depth + 1,
                        self.stack_min_col_width,
                        self.stack_overflow_ratio,
                    );
                    let stack_due_to_backend_bias = self.prefer_stacked_object_lists
                        && width_constrained_object_table(
                            &table,
                            self.width_hint,
                            self.indent_size,
                            depth + 1,
                        );

                    if stack_due_to_nested_shape || stack_due_to_width || stack_due_to_backend_bias
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
                    let first = items
                        .first()
                        .expect("non-empty list branch must have a first item");
                    self.push_scalar_entry(key_with_count, depth, first.clone());
                    return;
                }

                if items.len() <= self.medium_list_max {
                    self.push_vertical_list_entry(key_with_count, depth, items.clone());
                } else {
                    self.push_grid_entry(key_with_count, depth, items.clone());
                }
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

fn object_list_contains_nested_structures(items: &[Value]) -> bool {
    items.iter().any(|item| match item {
        Value::Object(map) => map.values().any(value_is_nested_structure),
        _ => false,
    })
}

fn value_is_nested_structure(value: &Value) -> bool {
    match value {
        Value::Object(_) => true,
        Value::Array(items) => items
            .iter()
            .any(|item| matches!(item, Value::Object(_) | Value::Array(_))),
        _ => false,
    }
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

    if table.headers.len() >= 6 {
        return true;
    }

    let columns = table.headers.len().max(1);
    let border = columns * 3 + 1;
    let content_budget = available_width.saturating_sub(border);
    let avg_column_budget = content_budget / columns;
    let shrunken_widths = shrink_column_widths_to_fit(table, available_width);
    let unreadable_columns = shrunken_widths
        .iter()
        .filter(|width| **width < stack_min_col_width.max(4))
        .count();

    // If we're forced into aggressively tiny columns, stacked output is easier to scan.
    let min_col_budget = stack_min_col_width.max(4);
    let overflow_ratio = stack_overflow_ratio.max(100);
    let ratio_trigger =
        estimated_width.saturating_mul(100) > available_width.saturating_mul(overflow_ratio);
    let unreadable_ratio_trigger = unreadable_columns * 3 >= columns.max(1);

    avg_column_budget < min_col_budget || ratio_trigger || unreadable_ratio_trigger
}

fn width_constrained_object_table(
    table: &TableBlock,
    width_hint: usize,
    indent_size: usize,
    depth: usize,
) -> bool {
    let available_width = width_hint
        .saturating_sub(depth.saturating_mul(indent_size))
        .max(24);
    estimate_table_width(table) > available_width
}

fn shrink_column_widths_to_fit(table: &TableBlock, available_width: usize) -> Vec<usize> {
    let min_width = 4usize;
    let mut widths = table
        .headers
        .iter()
        .map(|header| display_width(header).max(min_width))
        .collect::<Vec<_>>();

    if widths.is_empty() {
        return widths;
    }

    for row in &table.rows {
        for (index, cell) in row.iter().enumerate() {
            if let Some(width) = widths.get_mut(index) {
                *width = (*width).max(display_width(&value_to_display(cell)).max(min_width));
            }
        }
    }

    while widths.iter().sum::<usize>() + widths.len() * 3 + 1 > available_width {
        let (index, width) = widths
            .iter_mut()
            .enumerate()
            .max_by_key(|(_, width)| **width)
            .expect("non-empty widths must have a widest column");
        if *width <= min_width {
            break;
        }
        let _ = index;
        *width -= 1;
    }

    widths
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

#[cfg(test)]
mod tests {
    use super::{
        MregBuildOptions, build_mreg_blocks, estimate_table_width, ordered_keys,
        shrink_column_widths_to_fit, value_is_nested_structure,
    };
    use crate::document::{Block, MregBlock, MregValue, TableBlock};
    use serde_json::{Map, Value, json};

    fn mreg_blocks(blocks: &[Block]) -> impl Iterator<Item = &MregBlock> {
        blocks.iter().filter_map(|block| match block {
            Block::Mreg(mreg) => Some(mreg),
            _ => None,
        })
    }

    fn table_blocks(blocks: &[Block]) -> impl Iterator<Item = &TableBlock> {
        blocks.iter().filter_map(|block| match block {
            Block::Table(table) => Some(table),
            _ => None,
        })
    }

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

    fn wide_flat_row() -> Map<String, Value> {
        let mut row = Map::new();
        row.insert(
            "records".to_string(),
            Value::Array(vec![
                json!({
                    "id": 1,
                    "name": "alpha.example.org",
                    "owner": "ops",
                    "status": "active",
                    "created_at": "2026-01-01T00:00:00+01:00",
                    "updated_at": "2026-01-02T00:00:00+01:00",
                    "region": "eu-west-1",
                    "environment": "production"
                }),
                json!({
                    "id": 2,
                    "name": "beta.example.org",
                    "owner": "ops",
                    "status": "active",
                    "created_at": "2026-01-03T00:00:00+01:00",
                    "updated_at": "2026-01-04T00:00:00+01:00",
                    "region": "eu-west-1",
                    "environment": "production"
                }),
            ]),
        );
        row
    }

    fn nested_object_list_row() -> Map<String, Value> {
        let mut row = Map::new();
        row.insert(
            "networks".to_string(),
            Value::Array(vec![
                json!({
                    "policy": Value::Null,
                    "communities": [
                        {
                            "id": 3,
                            "name": "laptops",
                            "description": "Laptops",
                        },
                        {
                            "id": 2,
                            "name": "workstations",
                            "description": "Workstations",
                        }
                    ],
                    "network": "129.240.130.0/24",
                    "description": "knh-klientnett-2 (statisk DHCP)",
                    "vlan": 200,
                }),
                json!({
                    "policy": Value::Null,
                    "communities": [],
                    "network": "2001:700:100:4003::/64",
                    "description": "usit-knh",
                    "vlan": 200,
                }),
            ]),
        );
        row
    }

    #[test]
    fn models_list_thresholds_as_scalar_vertical_and_grid() {
        let mut next_block_id = 1;
        let mut row = Map::new();
        row.insert("single".to_string(), json!(["a"]));
        row.insert("pair".to_string(), json!(["a", "b"]));
        row.insert("many".to_string(), json!(["a", "b", "c"]));

        let blocks = build_mreg_blocks(
            &[row],
            MregBuildOptions {
                key_order: None,
                short_list_max: 1,
                medium_list_max: 2,
                width_hint: 80,
                indent_size: 2,
                prefer_stacked_object_lists: false,
                stack_min_col_width: 10,
                stack_overflow_ratio: 200,
            },
            &mut next_block_id,
        );

        let mreg = mreg_blocks(&blocks).next().expect("mreg block");

        assert!(
            mreg.rows[0]
                .entries
                .iter()
                .any(|entry| matches!(entry.value, MregValue::Scalar(_)))
        );
        assert!(
            mreg.rows[0]
                .entries
                .iter()
                .any(|entry| matches!(entry.value, MregValue::VerticalList(_)))
        );
        assert!(
            mreg.rows[0]
                .entries
                .iter()
                .any(|entry| matches!(entry.value, MregValue::Grid(_)))
        );
    }

    #[test]
    fn keeps_object_lists_as_tables_in_plain_mode() {
        let mut next_block_id = 1;
        let blocks = build_mreg_blocks(
            &[sample_row()],
            MregBuildOptions {
                key_order: None,
                short_list_max: 1,
                medium_list_max: 5,
                width_hint: 80,
                indent_size: 2,
                prefer_stacked_object_lists: false,
                stack_min_col_width: 10,
                stack_overflow_ratio: 200,
            },
            &mut next_block_id,
        );
        assert!(blocks.iter().any(|block| matches!(block, Block::Table(_))));
    }

    #[test]
    fn stacks_wide_flat_object_lists_when_width_is_tight() {
        let mut next_block_id = 1;
        let blocks = build_mreg_blocks(
            &[wide_flat_row()],
            MregBuildOptions {
                key_order: None,
                short_list_max: 1,
                medium_list_max: 5,
                width_hint: 40,
                indent_size: 2,
                prefer_stacked_object_lists: false,
                stack_min_col_width: 10,
                stack_overflow_ratio: 200,
            },
            &mut next_block_id,
        );
        assert!(!blocks.iter().any(|block| matches!(block, Block::Table(_))));
        let has_separator = mreg_blocks(&blocks).any(|mreg| {
            mreg.rows.iter().any(|row| {
                row.entries
                    .iter()
                    .any(|entry| matches!(entry.value, MregValue::Separator))
            })
        });
        assert!(has_separator);
    }

    #[test]
    fn stacks_nested_object_lists_even_when_width_is_wide() {
        let mut next_block_id = 1;
        let blocks = build_mreg_blocks(
            &[nested_object_list_row()],
            MregBuildOptions {
                key_order: None,
                short_list_max: 1,
                medium_list_max: 5,
                width_hint: 120,
                indent_size: 2,
                prefer_stacked_object_lists: false,
                stack_min_col_width: 10,
                stack_overflow_ratio: 200,
            },
            &mut next_block_id,
        );

        assert!(matches!(blocks.first(), Some(Block::Mreg(_))));
        let has_separator = mreg_blocks(&blocks).any(|mreg| {
            mreg.rows.iter().any(|row| {
                row.entries
                    .iter()
                    .any(|entry| matches!(entry.value, MregValue::Separator))
            })
        });
        assert!(has_separator);
        let has_communities_group = mreg_blocks(&blocks).any(|mreg| {
            mreg.rows.iter().any(|row| {
                row.entries
                    .iter()
                    .any(|entry| entry.key.starts_with("communities"))
            })
        });
        assert!(has_communities_group);
    }

    #[test]
    fn nested_object_values_become_group_entries() {
        let mut next_block_id = 1;
        let mut row = Map::new();
        row.insert(
            "owner".to_string(),
            json!({
                "uid": "alice",
                "mail": "alice@uio.no"
            }),
        );

        let blocks = build_mreg_blocks(
            &[row],
            MregBuildOptions {
                key_order: None,
                short_list_max: 1,
                medium_list_max: 5,
                width_hint: 80,
                indent_size: 2,
                prefer_stacked_object_lists: false,
                stack_min_col_width: 10,
                stack_overflow_ratio: 200,
            },
            &mut next_block_id,
        );

        let mreg = mreg_blocks(&blocks).next().expect("mreg block");
        let entries = &mreg.rows[0].entries;
        assert!(
            entries
                .iter()
                .any(|entry| entry.key == "owner" && matches!(entry.value, MregValue::Group))
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.key == "uid" && matches!(entry.value, MregValue::Scalar(_)))
        );
    }

    #[test]
    fn mixed_object_and_scalar_lists_keep_table_shape() {
        let mut next_block_id = 1;
        let mut row = Map::new();
        row.insert(
            "records".to_string(),
            Value::Array(vec![json!({"name": "alpha", "count": 1}), json!("raw")]),
        );

        let blocks = build_mreg_blocks(
            &[row],
            MregBuildOptions {
                key_order: None,
                short_list_max: 1,
                medium_list_max: 5,
                width_hint: 80,
                indent_size: 2,
                prefer_stacked_object_lists: false,
                stack_min_col_width: 10,
                stack_overflow_ratio: 200,
            },
            &mut next_block_id,
        );

        let table = table_blocks(&blocks).next().expect("table block");
        assert_eq!(table.headers, vec!["name".to_string(), "count".to_string()]);
        assert_eq!(table.rows[1], vec![json!("raw"), json!("raw")]);
    }

    #[test]
    fn preferred_key_order_is_respected_for_scalar_entries() {
        let mut next_block_id = 1;
        let mut row = Map::new();
        row.insert("uid".to_string(), json!("alice"));
        row.insert("mail".to_string(), json!("alice@uio.no"));
        row.insert("name".to_string(), json!("Alice"));
        let preferred = vec!["name".to_string(), "uid".to_string()];

        let blocks = build_mreg_blocks(
            &[row],
            MregBuildOptions {
                key_order: Some(&preferred),
                short_list_max: 1,
                medium_list_max: 5,
                width_hint: 80,
                indent_size: 2,
                prefer_stacked_object_lists: false,
                stack_min_col_width: 10,
                stack_overflow_ratio: 200,
            },
            &mut next_block_id,
        );

        let mreg = mreg_blocks(&blocks).next().expect("mreg block");
        let keys = mreg.rows[0]
            .entries
            .iter()
            .map(|entry| entry.key.as_str())
            .collect::<Vec<_>>();
        assert_eq!(keys, vec!["name", "uid", "mail"]);
    }

    #[test]
    fn empty_object_lists_fall_back_to_vertical_lists() {
        let mut next_block_id = 1;
        let mut row = Map::new();
        row.insert(
            "records".to_string(),
            Value::Array(vec![json!({}), json!({})]),
        );

        let blocks = build_mreg_blocks(
            &[row],
            MregBuildOptions {
                key_order: None,
                short_list_max: 1,
                medium_list_max: 5,
                width_hint: 80,
                indent_size: 2,
                prefer_stacked_object_lists: false,
                stack_min_col_width: 10,
                stack_overflow_ratio: 200,
            },
            &mut next_block_id,
        );

        let mreg = mreg_blocks(&blocks).next().expect("mreg block");
        assert!(mreg.rows[0].entries.iter().any(|entry| {
            entry.key == "records (2)"
                && matches!(entry.value, MregValue::VerticalList(ref values) if values == &vec![json!({}), json!({})])
        }));
    }

    #[test]
    fn backend_bias_can_stack_width_constrained_object_tables() {
        let mut next_block_id = 1;
        let mut row = Map::new();
        row.insert(
            "records".to_string(),
            Value::Array(vec![
                json!({
                    "name": "alpha-host-1",
                    "description": "secondary login node",
                }),
                json!({
                    "name": "beta-host-2",
                    "description": "backup login node",
                }),
            ]),
        );

        let plain_blocks = build_mreg_blocks(
            &[row.clone()],
            MregBuildOptions {
                key_order: None,
                short_list_max: 1,
                medium_list_max: 5,
                width_hint: 36,
                indent_size: 2,
                prefer_stacked_object_lists: false,
                stack_min_col_width: 10,
                stack_overflow_ratio: 200,
            },
            &mut next_block_id,
        );
        let stacked_blocks = build_mreg_blocks(
            &[row],
            MregBuildOptions {
                key_order: None,
                short_list_max: 1,
                medium_list_max: 5,
                width_hint: 36,
                indent_size: 2,
                prefer_stacked_object_lists: true,
                stack_min_col_width: 10,
                stack_overflow_ratio: 200,
            },
            &mut next_block_id,
        );

        assert!(table_blocks(&plain_blocks).next().is_some());
        assert!(table_blocks(&stacked_blocks).next().is_none());
        assert!(mreg_blocks(&stacked_blocks).any(|mreg| {
            mreg.rows.iter().any(|row| {
                row.entries
                    .iter()
                    .any(|entry| matches!(entry.value, MregValue::Separator))
            })
        }));
    }

    #[test]
    fn single_item_lists_become_scalar_entries() {
        let mut next_block_id = 1;
        let mut row = Map::new();
        row.insert("single".to_string(), json!(["a"]));

        let blocks = build_mreg_blocks(
            &[row],
            MregBuildOptions {
                key_order: None,
                short_list_max: 1,
                medium_list_max: 5,
                width_hint: 80,
                indent_size: 2,
                prefer_stacked_object_lists: false,
                stack_min_col_width: 10,
                stack_overflow_ratio: 200,
            },
            &mut next_block_id,
        );

        let mreg = mreg_blocks(&blocks).next().expect("mreg block");
        assert!(mreg.rows[0].entries.iter().any(|entry| {
            entry.key == "single"
                && matches!(entry.value, MregValue::Scalar(ref value) if value == &json!("a"))
        }));
    }

    #[test]
    fn stacked_object_lists_skip_non_object_items() {
        let mut next_block_id = 1;
        let mut row = Map::new();
        row.insert(
            "records".to_string(),
            Value::Array(vec![
                json!({
                    "name": "alpha-host-1",
                    "description": "secondary login node",
                    "meta": { "type": "node" }
                }),
                json!("raw"),
            ]),
        );

        let blocks = build_mreg_blocks(
            &[row],
            MregBuildOptions {
                key_order: None,
                short_list_max: 1,
                medium_list_max: 5,
                width_hint: 120,
                indent_size: 2,
                prefer_stacked_object_lists: false,
                stack_min_col_width: 10,
                stack_overflow_ratio: 200,
            },
            &mut next_block_id,
        );

        let mreg = mreg_blocks(&blocks).next().expect("mreg block");
        assert!(
            mreg.rows[0]
                .entries
                .iter()
                .any(|entry| entry.key == "records (2)" && matches!(entry.value, MregValue::Group))
        );
        assert!(!mreg_blocks(&blocks).any(|mreg| {
            mreg.rows.iter().any(|row| {
                row.entries.iter().any(|entry| {
                    matches!(entry.value, MregValue::Scalar(ref value) if value == &json!("raw"))
                })
            })
        }));
    }

    #[test]
    fn ordered_keys_deduplicates_preferred_entries() {
        let mut map = Map::new();
        map.insert("uid".to_string(), json!("alice"));
        map.insert("mail".to_string(), json!("alice@uio.no"));

        let keys = ordered_keys(
            &map,
            Some(&["mail".to_string(), "mail".to_string(), "uid".to_string()]),
        );

        assert_eq!(keys, vec!["mail".to_string(), "uid".to_string()]);
    }

    #[test]
    fn nested_structure_helper_detects_objects_and_nested_arrays() {
        assert!(value_is_nested_structure(&json!({"uid": "alice"})));
        assert!(value_is_nested_structure(&json!([{"uid": "alice"}])));
        assert!(!value_is_nested_structure(&json!(["alice", "bob"])));
    }

    #[test]
    fn shrink_column_widths_stops_at_minimum_width() {
        let table = TableBlock {
            block_id: 1,
            style: crate::document::TableStyle::Grid,
            headers: vec!["a".to_string(), "b".to_string()],
            rows: vec![vec![json!("x"), json!("y")]],
            header_pairs: Vec::new(),
            align: None,
            shrink_to_fit: false,
            depth: 0,
        };

        let widths = shrink_column_widths_to_fit(&table, 1);

        assert_eq!(widths, vec![4, 4]);
    }

    #[test]
    fn shrink_column_widths_handles_empty_tables() {
        let table = TableBlock {
            block_id: 1,
            style: crate::document::TableStyle::Grid,
            headers: Vec::new(),
            rows: vec![Vec::new()],
            header_pairs: Vec::new(),
            align: None,
            shrink_to_fit: false,
            depth: 0,
        };

        let widths = shrink_column_widths_to_fit(&table, 10);

        assert!(widths.is_empty());
    }

    #[test]
    fn estimate_table_width_accounts_for_cell_content() {
        let table = TableBlock {
            block_id: 1,
            style: crate::document::TableStyle::Grid,
            headers: vec!["name".to_string(), "value".to_string()],
            rows: vec![vec![
                json!("alpha"),
                json!("this is longer than the header"),
            ]],
            header_pairs: Vec::new(),
            align: None,
            shrink_to_fit: false,
            depth: 0,
        };

        assert!(estimate_table_width(&table) > "name".len() + "value".len() + 7);
    }
}
