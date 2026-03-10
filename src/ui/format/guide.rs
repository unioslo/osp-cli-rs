use serde_json::{Map, Value};

use crate::core::output_model::{OutputItems, OutputResult};
use crate::guide::GuideView;
use crate::ui::document::Document;
use crate::ui::document_model::{DocumentModel, LowerDocumentOptions};
use crate::ui::{RenderSettings, ResolvedRenderSettings};

pub(super) fn build_guide_document(
    output: &OutputResult,
    settings: &RenderSettings,
    resolved: &ResolvedRenderSettings,
    next_block_id: &mut u64,
) -> Document {
    if let Some(guide) = GuideView::try_from_output_result(output) {
        return DocumentModel::from_guide_view(&guide).lower_to_render_document(
            LowerDocumentOptions {
                frame_style: settings.chrome_frame,
                panel_kind: Some("guide"),
                key_value_border: resolved.help_table_border,
                key_value_indent: settings.help_entry_indent,
                key_value_gap: settings.help_entry_gap,
            },
            next_block_id,
        );
    }

    let value = output_to_value(output);
    let preferred_root_keys = match &output.items {
        OutputItems::Rows(rows) if rows.len() == 1 => Some(output.meta.key_index.as_slice()),
        _ => None,
    };
    DocumentModel::from_value(&value, preferred_root_keys).lower_to_render_document(
        LowerDocumentOptions {
            frame_style: settings.chrome_frame,
            panel_kind: Some("guide"),
            key_value_border: resolved.help_table_border,
            key_value_indent: None,
            key_value_gap: None,
        },
        next_block_id,
    )
}

fn output_to_value(output: &OutputResult) -> Value {
    match &output.items {
        OutputItems::Rows(rows) if rows.len() == 1 => rows
            .first()
            .cloned()
            .map(Value::Object)
            .unwrap_or_else(|| Value::Array(Vec::new())),
        OutputItems::Rows(rows) => {
            Value::Array(rows.iter().cloned().map(Value::Object).collect::<Vec<_>>())
        }
        OutputItems::Groups(groups) => Value::Array(
            groups
                .iter()
                .map(|group| {
                    let mut item = Map::new();
                    item.insert("groups".to_string(), Value::Object(group.groups.clone()));
                    item.insert(
                        "aggregates".to_string(),
                        Value::Object(group.aggregates.clone()),
                    );
                    item.insert(
                        "rows".to_string(),
                        Value::Array(
                            group
                                .rows
                                .iter()
                                .cloned()
                                .map(Value::Object)
                                .collect::<Vec<_>>(),
                        ),
                    );
                    Value::Object(item)
                })
                .collect::<Vec<_>>(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::build_guide_document;
    use crate::core::output::OutputFormat;
    use crate::core::output_model::OutputResult;
    use crate::guide::GuideView;
    use crate::ui::RenderSettings;
    use crate::ui::document::Block;
    use crate::ui::format::build_document_from_output;
    use crate::ui::renderer::render_document;
    use serde_json::json;

    #[test]
    fn guide_renderer_uses_borderless_table_for_multi_row_data_unit() {
        let rows = vec![
            json!({"name": "plugins", "short_help": "subcommands"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"name": "config", "short_help": "settings"})
                .as_object()
                .cloned()
                .expect("object"),
        ];
        let output = OutputResult::from_rows(rows);
        let document =
            build_document_from_output(&output, &RenderSettings::test_plain(OutputFormat::Guide));

        let Block::Table(table) = document.blocks.first().expect("table block") else {
            panic!("expected guide table");
        };
        assert_eq!(table.style, crate::ui::document::TableStyle::Guide);
    }

    #[test]
    fn guide_renderer_uses_key_value_lines_for_single_row_object_unit() {
        let output = OutputResult::from_rows(vec![
            json!({
                "uid": "oistes",
                "groups": ["ops", "vcs"],
            })
            .as_object()
            .cloned()
            .expect("object"),
        ]);
        let resolved = RenderSettings::test_plain(OutputFormat::Guide).resolve_render_settings();
        let mut next_block_id = 1u64;
        let document = build_guide_document(
            &output,
            &RenderSettings::test_plain(OutputFormat::Guide),
            &resolved,
            &mut next_block_id,
        );

        assert!(
            document
                .blocks
                .iter()
                .all(|block| matches!(block, Block::Line(_)))
        );
    }

    #[test]
    fn guide_renderer_renders_uniform_rows_without_table_borders_unit() {
        let output = OutputResult::from_rows(vec![
            json!({"name": "plugins", "short_help": "subcommands"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"name": "config", "short_help": "settings"})
                .as_object()
                .cloned()
                .expect("object"),
        ]);
        let settings = RenderSettings::test_plain(OutputFormat::Guide);
        let rendered = render_document(
            &build_document_from_output(&output, &settings),
            settings.resolve_render_settings(),
        );

        assert!(rendered.contains("name"));
        assert!(rendered.contains("plugins"));
        assert!(!rendered.contains('|'));
        assert!(!rendered.contains('+'));
    }

    #[test]
    fn guide_renderer_preserves_semantic_help_sections_unit() {
        let output =
            GuideView::from_text("Usage: osp history <COMMAND>\n\nCommands:\n  list  Show\n")
                .to_output_result();
        let settings = RenderSettings::test_plain(OutputFormat::Guide);
        let rendered = render_document(
            &build_document_from_output(&output, &settings),
            settings.resolve_render_settings(),
        );

        assert!(rendered.contains("Usage"));
        assert!(rendered.contains("Commands"));
        assert!(rendered.contains("list"));
    }
}
