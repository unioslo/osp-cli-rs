use crate::osp_core::output::OutputFormat;
use crate::osp_core::output_model::{ColumnAlignment, Group, OutputItems, OutputResult};
use crate::osp_core::row::Row;

use crate::osp_ui::document::{Block, Document, JsonBlock, TableStyle};
use crate::osp_ui::{RenderBackend, RenderSettings, ResolvedRenderSettings};

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
    let resolved = settings.resolve_render_settings();
    build_document_from_output_resolved(output, settings, &resolved)
}

pub fn build_document_from_output_resolved(
    output: &OutputResult,
    settings: &RenderSettings,
    resolved: &ResolvedRenderSettings,
) -> Document {
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
            blocks: vec![Block::Table({
                let mut block = table::build_table_block(
                    rows,
                    style,
                    Some(&output.meta.key_index),
                    allocate_block_id(next_block_id),
                );
                block.align = table_alignments_for_headers(
                    &block.headers,
                    &output.meta.key_index,
                    &output.meta.column_align,
                );
                block
            })],
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
                    block.align = table_alignments_for_headers(
                        &block.headers,
                        &output.meta.key_index,
                        &output.meta.column_align,
                    );
                    block.header_pairs = group_header_pairs(group, Some(&output.meta.key_index));
                    Block::Table(block)
                })
                .collect(),
        },
    }
}

fn table_alignments_for_headers(
    headers: &[String],
    key_index: &[String],
    column_align: &[ColumnAlignment],
) -> Option<Vec<crate::osp_ui::document::TableAlign>> {
    if key_index.is_empty() || column_align.is_empty() {
        return None;
    }

    let align_by_key = key_index
        .iter()
        .cloned()
        .zip(column_align.iter().copied())
        .collect::<std::collections::BTreeMap<String, ColumnAlignment>>();

    let out = headers
        .iter()
        .map(|header| {
            align_by_key
                .get(header)
                .copied()
                .map(table_align_from_output)
                .unwrap_or(crate::osp_ui::document::TableAlign::Default)
        })
        .collect::<Vec<_>>();

    if out
        .iter()
        .all(|align| matches!(align, crate::osp_ui::document::TableAlign::Default))
    {
        None
    } else {
        Some(out)
    }
}

fn table_align_from_output(value: ColumnAlignment) -> crate::osp_ui::document::TableAlign {
    match value {
        ColumnAlignment::Default => crate::osp_ui::document::TableAlign::Default,
        ColumnAlignment::Left => crate::osp_ui::document::TableAlign::Left,
        ColumnAlignment::Center => crate::osp_ui::document::TableAlign::Center,
        ColumnAlignment::Right => crate::osp_ui::document::TableAlign::Right,
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
    use super::{
        build_document, build_document_from_output, build_document_from_output_resolved,
        group_header_pairs, materialize_rows, resolve_format, resolve_output_format,
        table_alignments_for_headers,
    };
    use crate::osp_core::output::{OutputFormat, RenderMode};
    use crate::osp_core::output_model::{
        ColumnAlignment, Group, OutputItems, OutputMeta, OutputResult,
    };
    use crate::osp_core::row::Row;
    use crate::osp_ui::RenderSettings;
    use crate::osp_ui::document::{Block, TableAlign, TableStyle};
    use serde_json::json;

    fn settings(format: OutputFormat) -> RenderSettings {
        RenderSettings {
            mode: RenderMode::Plain,
            width: Some(100),
            grid_padding: 2,
            theme_name: "plain".to_string(),
            ..RenderSettings::test_plain(format)
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
                column_align: Vec::new(),
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

    #[test]
    fn row_table_document_preserves_alignment_metadata() {
        let mut row = Row::new();
        row.insert("name".to_string(), json!("alice"));
        row.insert("count".to_string(), json!(2));
        let output = OutputResult {
            items: OutputItems::Rows(vec![row]),
            meta: OutputMeta {
                key_index: vec!["name".to_string(), "count".to_string()],
                column_align: vec![ColumnAlignment::Left, ColumnAlignment::Right],
                wants_copy: false,
                grouped: false,
            },
        };

        let document = build_document_from_output(&output, &settings(OutputFormat::Table));
        let Block::Table(table) = &document.blocks[0] else {
            panic!("expected table block");
        };
        assert_eq!(
            table.align,
            Some(vec![
                crate::osp_ui::document::TableAlign::Left,
                crate::osp_ui::document::TableAlign::Right
            ])
        );
    }

    #[test]
    fn grouped_table_document_preserves_alignment_metadata() {
        let mut group_fields = Row::new();
        group_fields.insert("group".to_string(), json!("ops"));
        let mut row = Row::new();
        row.insert("uid".to_string(), json!("alice"));
        row.insert("count".to_string(), json!(2));
        let output = OutputResult {
            items: OutputItems::Groups(vec![Group {
                groups: group_fields,
                aggregates: Row::new(),
                rows: vec![row],
            }]),
            meta: OutputMeta {
                key_index: vec!["group".to_string(), "uid".to_string(), "count".to_string()],
                column_align: vec![
                    ColumnAlignment::Default,
                    ColumnAlignment::Left,
                    ColumnAlignment::Right,
                ],
                wants_copy: false,
                grouped: true,
            },
        };

        let document = build_document_from_output(&output, &settings(OutputFormat::Table));
        let Block::Table(table) = &document.blocks[0] else {
            panic!("expected table block");
        };
        assert_eq!(
            table.align,
            Some(vec![
                crate::osp_ui::document::TableAlign::Left,
                crate::osp_ui::document::TableAlign::Right
            ])
        );
    }

    #[test]
    fn grouped_markdown_document_preserves_header_pairs_and_alignment() {
        let mut group_fields = Row::new();
        group_fields.insert("group".to_string(), json!("ops"));
        let mut aggregates = Row::new();
        aggregates.insert("count".to_string(), json!(2));
        let mut row = Row::new();
        row.insert("uid".to_string(), json!("alice"));
        row.insert("score".to_string(), json!(42));
        let output = OutputResult {
            items: OutputItems::Groups(vec![Group {
                groups: group_fields,
                aggregates,
                rows: vec![row],
            }]),
            meta: OutputMeta {
                key_index: vec![
                    "group".to_string(),
                    "count".to_string(),
                    "uid".to_string(),
                    "score".to_string(),
                ],
                column_align: vec![
                    ColumnAlignment::Default,
                    ColumnAlignment::Default,
                    ColumnAlignment::Left,
                    ColumnAlignment::Right,
                ],
                wants_copy: false,
                grouped: true,
            },
        };

        let document = build_document_from_output(&output, &settings(OutputFormat::Markdown));
        let Block::Table(table) = &document.blocks[0] else {
            panic!("expected table block");
        };
        assert_eq!(table.style, TableStyle::Markdown);
        assert_eq!(
            table.header_pairs,
            vec![
                ("group".to_string(), json!("ops")),
                ("count".to_string(), json!(2))
            ]
        );
        assert_eq!(
            table.align,
            Some(vec![
                crate::osp_ui::document::TableAlign::Left,
                crate::osp_ui::document::TableAlign::Right
            ])
        );
    }

    #[test]
    fn build_document_wrapper_and_resolve_format_cover_value_and_explicit_modes() {
        let rows = vec![json!({"value": 7}).as_object().cloned().expect("object")];

        assert_eq!(
            resolve_format(&rows, OutputFormat::Auto),
            OutputFormat::Value
        );
        assert_eq!(
            resolve_format(&rows, OutputFormat::Json),
            OutputFormat::Json
        );

        let document = build_document(&rows, &settings(OutputFormat::Value));
        assert!(matches!(document.blocks[0], Block::Value(_)));
    }

    #[test]
    fn mreg_and_value_documents_materialize_group_rows_consistently() {
        let group = Group {
            groups: json!({"group": "ops"})
                .as_object()
                .cloned()
                .expect("object"),
            aggregates: json!({"count": 2}).as_object().cloned().expect("object"),
            rows: vec![],
        };
        let output = OutputResult {
            items: OutputItems::Groups(vec![group.clone()]),
            meta: OutputMeta {
                key_index: vec!["group".to_string(), "count".to_string()],
                ..OutputMeta::default()
            },
        };

        let resolved = settings(OutputFormat::Mreg).resolve_render_settings();
        let document =
            build_document_from_output_resolved(&output, &settings(OutputFormat::Mreg), &resolved);
        assert!(!document.blocks.is_empty());

        let value_output = OutputResult {
            items: OutputItems::Groups(vec![group]),
            meta: OutputMeta::default(),
        };
        let value_document =
            build_document_from_output(&value_output, &settings(OutputFormat::Value));
        assert!(matches!(value_document.blocks[0], Block::Value(_)));

        let rows = materialize_rows(&value_output);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("group"), Some(&json!("ops")));
        assert_eq!(rows[0].get("count"), Some(&json!(2)));
    }

    #[test]
    fn header_pairs_and_alignments_skip_defaults_and_deduplicate_keys() {
        let group = Group {
            groups: json!({"group": "ops"})
                .as_object()
                .cloned()
                .expect("object"),
            aggregates: json!({"count": 2}).as_object().cloned().expect("object"),
            rows: vec![],
        };

        let pairs = group_header_pairs(
            &group,
            Some(&[
                "count".to_string(),
                "group".to_string(),
                "group".to_string(),
            ]),
        );
        assert_eq!(
            pairs,
            vec![
                ("count".to_string(), json!(2)),
                ("group".to_string(), json!("ops"))
            ]
        );

        let align = table_alignments_for_headers(
            &["group".to_string(), "count".to_string()],
            &["group".to_string(), "count".to_string()],
            &[ColumnAlignment::Default, ColumnAlignment::Default],
        );
        assert!(align.is_none());

        let align = table_alignments_for_headers(
            &["group".to_string(), "count".to_string()],
            &["group".to_string(), "count".to_string()],
            &[ColumnAlignment::Center, ColumnAlignment::Right],
        );
        assert_eq!(align, Some(vec![TableAlign::Center, TableAlign::Right]));
    }
}
