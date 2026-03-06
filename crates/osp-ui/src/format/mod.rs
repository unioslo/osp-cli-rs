use osp_core::output::OutputFormat;
use osp_core::output_model::{Group, OutputItems, OutputResult};
use osp_core::row::Row;

use crate::document::{Block, Document, JsonBlock, TableStyle};
use crate::{RenderBackend, RenderSettings};

mod common;
pub mod help;
pub mod message;
mod mreg;
mod table;
mod value;

pub use help::build_help_document;
pub use message::{MessageContent, MessageFormatter, MessageKind, MessageOptions, MessageRules};

pub fn build_document(rows: &[Row], settings: &RenderSettings) -> Document {
    build_document_from_output(
        &OutputResult {
            items: OutputItems::Rows(rows.to_vec()),
            meta: Default::default(),
        },
        settings,
    )
}

pub fn build_document_from_output(output: &OutputResult, settings: &RenderSettings) -> Document {
    let format = resolve_output_format(output, settings.format);
    let mut next_block_id = 1u64;
    match format {
        OutputFormat::Json => Document {
            blocks: vec![Block::Json(build_json_block_from_output(output))],
        },
        OutputFormat::Table => build_table_document(output, TableStyle::Grid, &mut next_block_id),
        OutputFormat::Markdown => {
            build_table_document(output, TableStyle::Markdown, &mut next_block_id)
        }
        OutputFormat::Mreg => {
            let rows = materialize_rows(output);
            let resolved = settings.resolve_render_settings();
            let width_hint = resolved.width.unwrap_or(100).max(24);
            let prefer_stacked_object_lists = resolved.backend == RenderBackend::Rich;
            Document {
                blocks: mreg::build_mreg_blocks(
                    &rows,
                    mreg::MregBuildOptions {
                        key_order: Some(&output.meta.key_index),
                        short_list_max: settings.short_list_max,
                        medium_list_max: settings.medium_list_max,
                        width_hint,
                        indent_size: settings.indent_size.max(1),
                        prefer_stacked_object_lists,
                        stack_min_col_width: settings.mreg_stack_min_col_width.max(1),
                        stack_overflow_ratio: settings.mreg_stack_overflow_ratio.max(100),
                    },
                    &mut next_block_id,
                ),
            }
        }
        OutputFormat::Value => {
            let rows = materialize_rows(output);
            Document {
                blocks: vec![Block::Value(value::build_value_block(&rows))],
            }
        }
        OutputFormat::Auto => unreachable!("auto format is resolved above"),
    }
}

pub fn resolve_format(rows: &[Row], format: OutputFormat) -> OutputFormat {
    resolve_output_format(
        &OutputResult {
            items: OutputItems::Rows(rows.to_vec()),
            meta: Default::default(),
        },
        format,
    )
}

pub fn resolve_output_format(output: &OutputResult, format: OutputFormat) -> OutputFormat {
    if !matches!(format, OutputFormat::Auto) {
        return format;
    }

    if matches!(output.items, OutputItems::Groups(_)) {
        return OutputFormat::Table;
    }

    let rows = materialize_rows(output);
    if rows
        .iter()
        .all(|row| row.len() == 1 && row.contains_key("value"))
    {
        OutputFormat::Value
    } else if rows.len() <= 1 {
        OutputFormat::Mreg
    } else {
        OutputFormat::Table
    }
}

fn build_table_document(
    output: &OutputResult,
    style: TableStyle,
    next_block_id: &mut u64,
) -> Document {
    match &output.items {
        OutputItems::Rows(rows) => Document {
            blocks: vec![Block::Table(table::build_table_block(
                rows,
                style,
                Some(&output.meta.key_index),
                allocate_block_id(next_block_id),
            ))],
        },
        OutputItems::Groups(groups) => Document {
            blocks: groups
                .iter()
                .map(|group| {
                    let mut rows = group.rows.clone();
                    if rows.is_empty() {
                        rows.push(merge_group_header(group));
                    }
                    let mut block = table::build_table_block(
                        &rows,
                        style,
                        Some(&output.meta.key_index),
                        allocate_block_id(next_block_id),
                    );
                    block.header_pairs = group_header_pairs(group, Some(&output.meta.key_index));
                    Block::Table(block)
                })
                .collect(),
        },
    }
}

fn allocate_block_id(next_block_id: &mut u64) -> u64 {
    let id = *next_block_id;
    *next_block_id = next_block_id.saturating_add(1);
    id
}

fn build_json_block_from_output(output: &OutputResult) -> JsonBlock {
    let payload = match &output.items {
        OutputItems::Rows(rows) => serde_json::Value::Array(
            rows.iter()
                .cloned()
                .map(serde_json::Value::Object)
                .collect(),
        ),
        OutputItems::Groups(groups) => serde_json::Value::Array(
            groups
                .iter()
                .map(|group| {
                    let mut item = serde_json::Map::new();
                    item.insert(
                        "groups".to_string(),
                        serde_json::Value::Object(group.groups.clone()),
                    );
                    item.insert(
                        "aggregates".to_string(),
                        serde_json::Value::Object(group.aggregates.clone()),
                    );
                    item.insert(
                        "rows".to_string(),
                        serde_json::Value::Array(
                            group
                                .rows
                                .iter()
                                .cloned()
                                .map(serde_json::Value::Object)
                                .collect(),
                        ),
                    );
                    serde_json::Value::Object(item)
                })
                .collect(),
        ),
    };

    JsonBlock { payload }
}

fn materialize_rows(output: &OutputResult) -> Vec<Row> {
    match &output.items {
        OutputItems::Rows(rows) => rows.clone(),
        OutputItems::Groups(groups) => {
            let mut out = Vec::new();
            for group in groups {
                if group.rows.is_empty() {
                    out.push(merge_group_header(group));
                    continue;
                }
                for row in &group.rows {
                    out.push(merge_group_row(group, row));
                }
            }
            out
        }
    }
}

fn merge_group_header(group: &Group) -> Row {
    let mut row = group.groups.clone();
    for (key, value) in &group.aggregates {
        row.insert(key.clone(), value.clone());
    }
    row
}

fn merge_group_row(group: &Group, row: &Row) -> Row {
    let mut merged = group.groups.clone();
    for (key, value) in &group.aggregates {
        merged.insert(key.clone(), value.clone());
    }
    for (key, value) in row {
        merged.insert(key.clone(), value.clone());
    }
    merged
}

fn group_header_pairs(
    group: &Group,
    preferred_key_order: Option<&[String]>,
) -> Vec<(String, serde_json::Value)> {
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut ordered = Vec::new();

    if let Some(order) = preferred_key_order {
        for key in order {
            if group.groups.contains_key(key) || group.aggregates.contains_key(key) {
                ordered.push(key.clone());
            }
        }
    }

    for key in group.groups.keys() {
        ordered.push(key.clone());
    }
    for key in group.aggregates.keys() {
        ordered.push(key.clone());
    }

    for key in ordered {
        if !seen.insert(key.clone()) {
            continue;
        }
        if let Some(value) = group.groups.get(&key) {
            out.push((key.clone(), value.clone()));
            continue;
        }
        if let Some(value) = group.aggregates.get(&key) {
            out.push((key.clone(), value.clone()));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::{build_document_from_output, resolve_output_format};
    use crate::document::{Block, TableStyle};
    use crate::{RenderRuntime, RenderSettings};
    use osp_core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
    use osp_core::output_model::{Group, OutputItems, OutputMeta, OutputResult};
    use osp_core::row::Row;
    use serde_json::json;

    fn settings(format: OutputFormat) -> RenderSettings {
        RenderSettings {
            format,
            mode: RenderMode::Plain,
            color: ColorMode::Never,
            unicode: UnicodeMode::Never,
            width: Some(100),
            margin: 0,
            indent_size: 2,
            short_list_max: 1,
            medium_list_max: 5,
            grid_padding: 2,
            grid_columns: None,
            column_weight: 3,
            table_overflow: crate::TableOverflow::Clip,
            mreg_stack_min_col_width: 10,
            mreg_stack_overflow_ratio: 200,
            theme_name: "plain".to_string(),
            theme: None,
            style_overrides: crate::style::StyleOverrides::default(),
            runtime: RenderRuntime::default(),
        }
    }

    #[test]
    fn auto_format_uses_table_for_grouped_output() {
        let mut group_fields = Row::new();
        group_fields.insert("group".to_string(), json!("a"));
        let mut row = Row::new();
        row.insert("uid".to_string(), json!("oistes"));
        let output = OutputResult {
            items: OutputItems::Groups(vec![Group {
                groups: group_fields,
                aggregates: Row::new(),
                rows: vec![row],
            }]),
            meta: OutputMeta::default(),
        };

        assert_eq!(
            resolve_output_format(&output, OutputFormat::Auto),
            OutputFormat::Table
        );
    }

    #[test]
    fn grouped_table_document_populates_header_pairs() {
        let mut group_fields = Row::new();
        group_fields.insert("group".to_string(), json!("ops"));
        let mut aggregates = Row::new();
        aggregates.insert("count".to_string(), json!(2));
        let mut row = Row::new();
        row.insert("uid".to_string(), json!("alice"));
        let output = OutputResult {
            items: OutputItems::Groups(vec![Group {
                groups: group_fields,
                aggregates,
                rows: vec![row],
            }]),
            meta: OutputMeta {
                key_index: vec!["group".to_string(), "count".to_string(), "uid".to_string()],
                wants_copy: false,
                grouped: true,
            },
        };

        let document = build_document_from_output(&output, &settings(OutputFormat::Table));
        let Block::Table(table) = &document.blocks[0] else {
            panic!("expected table block");
        };
        assert_eq!(table.style, TableStyle::Grid);
        assert_eq!(
            table.header_pairs,
            vec![
                ("group".to_string(), json!("ops")),
                ("count".to_string(), json!(2))
            ]
        );
        assert_eq!(table.headers, vec!["uid".to_string()]);
    }

    #[test]
    fn grouped_json_document_keeps_group_structure() {
        let mut group_fields = Row::new();
        group_fields.insert("group".to_string(), json!("ops"));
        let mut row = Row::new();
        row.insert("uid".to_string(), json!("alice"));
        let output = OutputResult {
            items: OutputItems::Groups(vec![Group {
                groups: group_fields,
                aggregates: Row::new(),
                rows: vec![row],
            }]),
            meta: OutputMeta::default(),
        };
        let document = build_document_from_output(&output, &settings(OutputFormat::Json));
        let Block::Json(json_block) = &document.blocks[0] else {
            panic!("expected json block");
        };
        let payload = json_block.payload.as_array().expect("array payload");
        let first = payload.first().expect("first group");
        assert!(first.get("groups").is_some());
        assert!(first.get("rows").is_some());
    }
}
