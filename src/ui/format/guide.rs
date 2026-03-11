use serde_json::Value;

use crate::core::output_model::{OutputResult, output_items_to_value};
use crate::guide::GuideView;
use crate::ui::document::Document;
use crate::ui::document_model::{DocumentModel, LowerDocumentOptions};
use crate::ui::resolution::ResolvedGuideRenderSettings;

pub(super) fn build_guide_document(
    output: &OutputResult,
    guide_settings: ResolvedGuideRenderSettings,
    next_block_id: &mut u64,
) -> Document {
    if let Some(guide) = GuideView::try_from_output_result(output) {
        return DocumentModel::from_guide_view(&guide).lower_to_render_document(
            LowerDocumentOptions {
                frame_style: guide_settings.frame_style,
                ruled_section_policy: guide_settings.ruled_section_policy,
                panel_kind: Some("guide"),
                key_value_border: guide_settings.help_chrome.table_border,
                key_value_indent: guide_settings.help_chrome.entry_indent,
                key_value_gap: guide_settings.help_chrome.entry_gap,
            },
            next_block_id,
        );
    }

    let value = output_to_value(output);
    DocumentModel::from_value(&value, None).lower_to_render_document(
        LowerDocumentOptions {
            frame_style: guide_settings.frame_style,
            ruled_section_policy: guide_settings.ruled_section_policy,
            panel_kind: Some("guide"),
            key_value_border: guide_settings.help_chrome.table_border,
            key_value_indent: None,
            key_value_gap: None,
        },
        next_block_id,
    )
}

fn output_to_value(output: &OutputResult) -> Value {
    output_items_to_value(&output.items)
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
        let settings = RenderSettings::test_plain(OutputFormat::Guide);
        let mut next_block_id = 1u64;
        let document = build_guide_document(
            &output,
            settings.resolve_guide_render_settings(),
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
    fn guide_renderer_preserves_input_object_key_order_unit() {
        let output = OutputResult::from_rows(vec![
            json!({
                "theme": "rose-pine-moon",
                "user": "oistes",
                "version": "1.4.9"
            })
            .as_object()
            .cloned()
            .expect("object"),
        ]);
        let settings = RenderSettings::test_plain(OutputFormat::Guide);
        let rendered = render_document(
            &build_document_from_output(&output, &settings),
            settings.resolve_render_settings(),
        );
        let theme = rendered.find("theme:").expect("theme row");
        let user = rendered.find("user:").expect("user row");
        let version = rendered.find("version:").expect("version row");

        assert!(theme < user);
        assert!(user < version);
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
