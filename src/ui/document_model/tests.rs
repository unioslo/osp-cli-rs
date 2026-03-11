use serde_json::json;

use super::{
    BlockModel, DocumentModel, KeyValueBlockModel, KeyValueMarkdownStyle, KeyValueRowModel,
    LowerDocumentOptions,
};
use crate::guide::{GuideSection, GuideSectionKind, GuideView};
use crate::ui::TableAlign;
use crate::ui::TableBorderStyle;
use crate::ui::chrome::SectionFrameStyle;
use crate::ui::document::Block;

#[test]
fn guide_view_lowers_commands_to_key_value_sections_unit() {
    let view = GuideView::from_text("Commands:\n  help  Show help\n");
    let model = DocumentModel::from_guide_view(&view);
    let BlockModel::Section(section) = &model.blocks[0] else {
        panic!("expected section");
    };
    assert_eq!(section.title.as_deref(), Some("Commands"));
    assert!(matches!(section.blocks[0], BlockModel::KeyValue(_)));
}

#[test]
fn markdown_from_guide_view_uses_entry_lines_unit() {
    let view = GuideView::from_text("Commands:\n  help  Show help\n");
    let rendered = DocumentModel::from_guide_view(&view).to_markdown_with_width(Some(80));
    assert!(rendered.contains("- `help` Show help"));
    assert!(rendered.contains("Show help"));
    assert!(!rendered.contains("| name"));
}

#[test]
fn scalar_object_value_renders_as_key_value_group_unit() {
    let value = json!({"uid": "alice", "mail": "a@uio.no"});
    let model = DocumentModel::from_value(&value, None);
    assert!(matches!(model.blocks[0], BlockModel::KeyValue(_)));
}

#[test]
fn help_key_values_can_lower_to_bordered_tables_unit() {
    let view = GuideView::from_text("Commands:\n  help  Show help\n");
    let model = DocumentModel::from_guide_view(&view);
    let document = model.lower_to_render_document(
        LowerDocumentOptions {
            frame_style: SectionFrameStyle::Top,
            panel_kind: Some("help"),
            key_value_border: TableBorderStyle::Round,
            key_value_indent: None,
            key_value_gap: None,
        },
        &mut 1,
    );
    let crate::ui::document::Block::Panel(panel) = &document.blocks[0] else {
        panic!("expected panel");
    };
    assert!(matches!(
        panel.body.blocks[0],
        crate::ui::document::Block::Table(_)
    ));
}

#[test]
fn guide_view_custom_sections_keep_paragraph_entry_and_epilogue_separation_unit() {
    let mut view = GuideView::default();
    view.sections.push(
        GuideSection::new("Examples", GuideSectionKind::Custom)
            .paragraph("Run this first.")
            .entry("config", "Inspect config"),
    );
    view.epilogue.push("More details later.".to_string());

    let model = DocumentModel::from_guide_view(&view);
    let BlockModel::Section(section) = &model.blocks[0] else {
        panic!("expected section");
    };
    assert_eq!(section.title.as_deref(), Some("Examples"));
    assert!(matches!(section.blocks[0], BlockModel::Paragraph(_)));
    assert!(matches!(section.blocks[1], BlockModel::Blank));
    assert!(matches!(section.blocks[2], BlockModel::KeyValue(_)));
    assert!(matches!(model.blocks[1], BlockModel::Blank));
    assert!(matches!(model.blocks[2], BlockModel::Blank));
    assert!(matches!(model.blocks[3], BlockModel::Paragraph(_)));
}

#[test]
fn value_arrays_lower_to_tables_lists_and_blank_separated_blocks_unit() {
    let table_model = DocumentModel::from_value(
        &json!([
            {"uid": "alice", "city": "Oslo"},
            {"uid": "bob", "city": "Bergen"}
        ]),
        None,
    );
    assert!(matches!(table_model.blocks[0], BlockModel::Table(_)));

    let list_model = DocumentModel::from_value(&json!(["alice", "bob"]), None);
    assert!(matches!(list_model.blocks[0], BlockModel::List(_)));

    let mixed_model = DocumentModel::from_value(&json!([{"uid": "alice"}, 7]), None);
    assert!(matches!(mixed_model.blocks[0], BlockModel::KeyValue(_)));
    assert!(matches!(mixed_model.blocks[1], BlockModel::Blank));
    assert!(matches!(mixed_model.blocks[2], BlockModel::Paragraph(_)));
}

#[test]
fn lower_key_value_rows_respect_gap_overrides_and_empty_values_unit() {
    let model = DocumentModel {
        blocks: vec![BlockModel::KeyValue(KeyValueBlockModel {
            key_header: None,
            value_header: None,
            rows: vec![
                KeyValueRowModel {
                    key: "uid".to_string(),
                    value: "alice".to_string(),
                    indent: Some("  ".to_string()),
                    gap: Some(" -> ".to_string()),
                },
                KeyValueRowModel {
                    key: "note".to_string(),
                    value: String::new(),
                    indent: None,
                    gap: None,
                },
            ],
            border_override: None,
            markdown_style: KeyValueMarkdownStyle::Table,
        })],
    };

    let document = model.lower_to_render_document(
        LowerDocumentOptions {
            frame_style: SectionFrameStyle::Top,
            panel_kind: None,
            key_value_border: TableBorderStyle::None,
            key_value_indent: None,
            key_value_gap: None,
        },
        &mut 7,
    );

    let crate::ui::document::Block::Line(first) = &document.blocks[0] else {
        panic!("expected line block");
    };
    assert_eq!(first.parts[0].text, "  ");
    assert_eq!(first.parts[2].text, " -> ");

    let crate::ui::document::Block::Line(second) = &document.blocks[1] else {
        panic!("expected line block");
    };
    assert_eq!(second.parts[2].text, ":");
    assert_eq!(second.parts.len(), 3);
}

#[test]
fn markdown_renderer_returns_empty_string_for_empty_models_unit() {
    let model = DocumentModel { blocks: Vec::new() };
    assert_eq!(model.to_markdown_with_width(Some(40)), "");
}

#[test]
fn markdown_renderer_formats_lists_tables_and_key_value_blocks_unit() {
    let model = DocumentModel {
        blocks: vec![
            BlockModel::List(super::ListModel {
                items: vec!["alpha".to_string(), "beta".to_string()],
            }),
            BlockModel::Blank,
            BlockModel::KeyValue(KeyValueBlockModel {
                key_header: Some("name".to_string()),
                value_header: Some("short_help".to_string()),
                rows: vec![
                    KeyValueRowModel {
                        key: "config".to_string(),
                        value: "Show config".to_string(),
                        indent: None,
                        gap: None,
                    },
                    KeyValueRowModel {
                        key: "theme".to_string(),
                        value: String::new(),
                        indent: None,
                        gap: None,
                    },
                ],
                border_override: None,
                markdown_style: KeyValueMarkdownStyle::Lines,
            }),
            BlockModel::Blank,
            BlockModel::Table(super::TableModel {
                headers: vec!["name".to_string(), "score".to_string()],
                rows: vec![
                    vec![json!("alice"), json!(3)],
                    vec![json!("bob"), json!(12)],
                ],
                align: Some(vec![TableAlign::Left, TableAlign::Right]),
                border_override: None,
            }),
        ],
    };

    let rendered = model.to_markdown_with_width(Some(18));
    assert!(rendered.contains("- alpha\n- beta"));
    assert!(rendered.contains("- `config` Show config"));
    assert!(rendered.contains("- `theme`"));
    assert!(rendered.contains("| name  | score |"));
    assert!(rendered.contains(":"));
    assert!(rendered.contains("---"));
}

#[test]
fn lower_document_renders_lists_when_model_contains_list_blocks_unit() {
    let model = DocumentModel {
        blocks: vec![BlockModel::List(super::ListModel {
            items: vec!["alpha".to_string(), "beta".to_string()],
        })],
    };

    let document = model.lower_to_render_document(
        LowerDocumentOptions {
            frame_style: SectionFrameStyle::Top,
            panel_kind: None,
            key_value_border: TableBorderStyle::None,
            key_value_indent: None,
            key_value_gap: None,
        },
        &mut 11,
    );

    assert!(matches!(document.blocks[0], Block::Line(_)));
    assert!(matches!(document.blocks[1], Block::Line(_)));
}
