//! Document-lowering stage of the UI pipeline.
//!
//! This module exists to convert rows and semantic outputs into a structured
//! document model before terminal rendering happens.
//!
//! High level flow:
//!
//! - choose an output format from explicit settings, metadata, or shape
//! - lower the input into a [`crate::ui::Document`]
//! - leave terminal-specific styling and width-sensitive text rendering to the
//!   renderer layer
//!
//! Contract:
//!
//! - this module shapes documents
//! - it should not emit ANSI strings directly or depend on terminal I/O

use crate::core::output::OutputFormat;
use crate::core::output_model::{
    ColumnAlignment, Group, OutputItems, OutputResult, RenderRecommendation,
};
use crate::core::row::Row;
use crate::guide::GuideView;

use crate::ui::document::{Block, Document, JsonBlock, TableStyle, ValueBlock};
use crate::ui::{RenderBackend, RenderSettings, ResolvedRenderPlan};

mod common;
mod guide;
pub mod help;
#[cfg(test)]
mod message;
mod mreg;
mod table;
mod value;

#[cfg(test)]
/// Builds a document from row data using the provided render settings.
pub fn build_document(rows: &[Row], settings: &RenderSettings) -> Document {
    build_document_from_output(
        &OutputResult {
            items: OutputItems::Rows(rows.to_vec()),
            document: None,
            meta: Default::default(),
        },
        settings,
    )
}

/// Builds a document from a structured output result.
pub fn build_document_from_output(output: &OutputResult, settings: &RenderSettings) -> Document {
    let plan = settings.resolve_render_plan(output);
    build_document_from_output_plan(output, &plan)
}

/// Builds a document from a structured output result using a pre-resolved plan.
///
/// The returned document reflects the format selected by [`resolve_output_format`].
pub fn build_document_from_output_plan(
    output: &OutputResult,
    plan: &ResolvedRenderPlan,
) -> Document {
    let mut next_block_id = 1u64;
    match plan.format {
        OutputFormat::Guide => guide::build_guide_document(output, plan.guide, &mut next_block_id),
        OutputFormat::Json => Document {
            blocks: vec![Block::Json(build_json_block_from_output(output))],
        },
        OutputFormat::Table => build_table_document(output, TableStyle::Grid, &mut next_block_id),
        OutputFormat::Markdown => {
            build_table_document(output, TableStyle::Markdown, &mut next_block_id)
        }
        OutputFormat::Mreg => {
            let rows = materialize_rows(output);
            let width_hint = plan.render.width.unwrap_or(100).max(24);
            let prefer_stacked_object_lists = plan.render.backend == RenderBackend::Rich;
            Document {
                blocks: mreg::build_mreg_blocks(
                    &rows,
                    mreg::MregBuildOptions {
                        key_order: Some(&output.meta.key_index),
                        short_list_max: plan.mreg.short_list_max,
                        medium_list_max: plan.mreg.medium_list_max,
                        width_hint,
                        indent_size: plan.mreg.indent_size,
                        prefer_stacked_object_lists,
                        stack_min_col_width: plan.mreg.stack_min_col_width,
                        stack_overflow_ratio: plan.mreg.stack_overflow_ratio,
                    },
                    &mut next_block_id,
                ),
            }
        }
        OutputFormat::Value => {
            if let Some(guide) = GuideView::try_from_output_result(output) {
                // Value mode should remain useful for semantic help/intro
                // payloads. Falling back to the generic value block would
                // print nothing because those rows do not have a `value` key.
                return Document {
                    blocks: vec![Block::Value(ValueBlock {
                        values: guide.to_value_lines(),
                    })],
                };
            }
            let rows = materialize_rows(output);
            Document {
                blocks: vec![Block::Value(value::build_value_block(
                    &rows,
                    Some(&output.meta.key_index),
                ))],
            }
        }
        OutputFormat::Auto => unreachable!("auto format is resolved above"),
    }
}

#[cfg(test)]
/// Resolves the output format that would be chosen for the given rows.
pub fn resolve_format(rows: &[Row], format: OutputFormat) -> OutputFormat {
    resolve_output_format(
        &OutputResult {
            items: OutputItems::Rows(rows.to_vec()),
            document: None,
            meta: Default::default(),
        },
        &RenderSettings::test_plain(format),
    )
}

/// Resolves the output format for a result and render settings.
///
/// Explicit format settings take precedence, then metadata recommendations,
/// then automatic inference from the output shape.
pub fn resolve_output_format(output: &OutputResult, settings: &RenderSettings) -> OutputFormat {
    if settings.format_explicit && !matches!(settings.format, OutputFormat::Auto) {
        return settings.format;
    }

    if let Some(recommended) = output.meta.render_recommendation {
        return match recommended {
            RenderRecommendation::Format(format) => format,
            RenderRecommendation::Guide => OutputFormat::Guide,
        };
    }

    if !matches!(settings.format, OutputFormat::Auto) {
        return settings.format;
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
) -> Option<Vec<crate::ui::document::TableAlign>> {
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
                .unwrap_or(crate::ui::document::TableAlign::Default)
        })
        .collect::<Vec<_>>();

    if out
        .iter()
        .all(|align| matches!(align, crate::ui::document::TableAlign::Default))
    {
        None
    } else {
        Some(out)
    }
}

fn table_align_from_output(value: ColumnAlignment) -> crate::ui::document::TableAlign {
    match value {
        ColumnAlignment::Default => crate::ui::document::TableAlign::Default,
        ColumnAlignment::Left => crate::ui::document::TableAlign::Left,
        ColumnAlignment::Center => crate::ui::document::TableAlign::Center,
        ColumnAlignment::Right => crate::ui::document::TableAlign::Right,
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
        build_document, build_document_from_output, build_document_from_output_plan,
        group_header_pairs, materialize_rows, resolve_format, resolve_output_format,
        table_alignments_for_headers,
    };
    use crate::core::output::{OutputFormat, RenderMode};
    use crate::core::output_model::{
        ColumnAlignment, Group, OutputItems, OutputMeta, OutputResult, RenderRecommendation,
    };
    use crate::core::row::Row;
    use crate::ui::RenderSettings;
    use crate::ui::document::{Block, TableAlign, TableStyle};
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
            document: None,
            meta: OutputMeta::default(),
        };

        assert_eq!(
            resolve_output_format(&output, &RenderSettings::test_plain(OutputFormat::Auto)),
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
            document: None,
            meta: OutputMeta {
                key_index: vec!["group".to_string(), "count".to_string(), "uid".to_string()],
                column_align: Vec::new(),
                wants_copy: false,
                grouped: true,
                render_recommendation: None,
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
            document: None,
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
            document: None,
            meta: OutputMeta {
                key_index: vec!["name".to_string(), "count".to_string()],
                column_align: vec![ColumnAlignment::Left, ColumnAlignment::Right],
                wants_copy: false,
                grouped: false,
                render_recommendation: None,
            },
        };

        let document = build_document_from_output(&output, &settings(OutputFormat::Table));
        let Block::Table(table) = &document.blocks[0] else {
            panic!("expected table block");
        };
        assert_eq!(
            table.align,
            Some(vec![
                crate::ui::document::TableAlign::Left,
                crate::ui::document::TableAlign::Right
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
            document: None,
            meta: OutputMeta {
                key_index: vec!["group".to_string(), "uid".to_string(), "count".to_string()],
                column_align: vec![
                    ColumnAlignment::Default,
                    ColumnAlignment::Left,
                    ColumnAlignment::Right,
                ],
                wants_copy: false,
                grouped: true,
                render_recommendation: None,
            },
        };

        let document = build_document_from_output(&output, &settings(OutputFormat::Table));
        let Block::Table(table) = &document.blocks[0] else {
            panic!("expected table block");
        };
        assert_eq!(
            table.align,
            Some(vec![
                crate::ui::document::TableAlign::Left,
                crate::ui::document::TableAlign::Right
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
            document: None,
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
                render_recommendation: None,
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
                crate::ui::document::TableAlign::Left,
                crate::ui::document::TableAlign::Right
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
    fn recommendation_beats_inherited_format_unit() {
        let rows = vec![json!({"value": 7}).as_object().cloned().expect("object")];
        let mut settings = RenderSettings::test_plain(OutputFormat::Json);
        settings.format_explicit = false;
        let mut output = OutputResult::from_rows(rows);
        output.meta.render_recommendation = Some(RenderRecommendation::Format(OutputFormat::Value));

        assert_eq!(
            resolve_output_format(&output, &settings),
            OutputFormat::Value
        );
    }

    #[test]
    fn explicit_format_beats_recommendation_unit() {
        let rows = vec![json!({"value": 7}).as_object().cloned().expect("object")];
        let mut settings = RenderSettings::test_plain(OutputFormat::Json);
        settings.format_explicit = true;
        let mut output = OutputResult::from_rows(rows);
        output.meta.render_recommendation = Some(RenderRecommendation::Format(OutputFormat::Value));

        assert_eq!(
            resolve_output_format(&output, &settings),
            OutputFormat::Json
        );
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
            document: None,
            meta: OutputMeta {
                key_index: vec!["group".to_string(), "count".to_string()],
                ..OutputMeta::default()
            },
        };

        let plan = settings(OutputFormat::Mreg).resolve_render_plan(&output);
        let document = build_document_from_output_plan(&output, &plan);
        assert!(!document.blocks.is_empty());

        let value_output = OutputResult {
            items: OutputItems::Groups(vec![group]),
            document: None,
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
