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
fn guide_view_commands_lower_to_key_value_sections_markdown_and_bordered_tables_unit() {
    let view = GuideView::from_text("Commands:\n  help  Show help\n");
    let model = DocumentModel::from_guide_view(&view);
    let BlockModel::Section(section) = &model.blocks[0] else {
        panic!("expected section");
    };
    assert_eq!(section.title.as_deref(), Some("Commands"));
    assert!(matches!(section.blocks[0], BlockModel::KeyValue(_)));

    let rendered = model.to_markdown_with_width(Some(80));
    assert!(rendered.contains("- `help` Show help"));
    assert!(rendered.contains("Show help"));
    assert!(!rendered.contains("| name"));

    let document = model.lower_to_render_document(
        LowerDocumentOptions {
            frame_style: SectionFrameStyle::Top,
            panel_kind: Some("help"),
            key_value_border: TableBorderStyle::Round,
            key_value_indent: None,
            key_value_gap: None,
            ruled_section_policy: crate::ui::chrome::RuledSectionPolicy::PerSection,
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
fn guide_view_section_lowering_preserves_custom_sections_notes_and_scalar_section_data_unit() {
    let mut view = GuideView::default();
    view.sections.push(
        GuideSection::new("Examples", GuideSectionKind::Custom)
            .paragraph("Run this first.")
            .entry("config", "Inspect config"),
    );
    view.sections.push(
        GuideSection::new("Keybindings", GuideSectionKind::Custom).data(json!([
            {"name": "Ctrl-D", "short_help": "exit"},
            {"name": "Ctrl-L", "short_help": "clear screen"}
        ])),
    );
    view.sections
        .push(GuideSection::new("Pipes", GuideSectionKind::Custom).data(json!(["F", "P", "S"])));
    view.notes
        .push("Use bare help for the REPL overview.".to_string());
    view.epilogue.push("More details later.".to_string());

    let model = DocumentModel::from_guide_view(&view);
    let section = model
        .blocks
        .iter()
        .find_map(|block| match block {
            BlockModel::Section(section) if section.title.as_deref() == Some("Examples") => {
                Some(section)
            }
            _ => None,
        })
        .expect("expected examples section");
    assert_eq!(section.title.as_deref(), Some("Examples"));
    assert!(matches!(section.blocks[0], BlockModel::Paragraph(_)));
    assert!(matches!(section.blocks[1], BlockModel::Blank));
    assert!(matches!(section.blocks[2], BlockModel::KeyValue(_)));
    assert!(matches!(
        model.blocks.last(),
        Some(BlockModel::Paragraph(_))
    ));

    let keybindings = model
        .blocks
        .iter()
        .find_map(|block| match block {
            BlockModel::Section(section) if section.title.as_deref() == Some("Keybindings") => {
                Some(section)
            }
            _ => None,
        })
        .expect("expected keybindings section");
    assert_eq!(keybindings.title.as_deref(), Some("Keybindings"));
    assert!(matches!(keybindings.blocks[0], BlockModel::KeyValue(_)));

    let pipes = model
        .blocks
        .iter()
        .find_map(|block| match block {
            BlockModel::Section(section) if section.title.as_deref() == Some("Pipes") => {
                Some(section)
            }
            _ => None,
        })
        .expect("expected pipes section");
    assert_eq!(pipes.title.as_deref(), Some("Pipes"));
    let BlockModel::List(list) = &pipes.blocks[0] else {
        panic!("expected scalar section data to lower to a list");
    };
    assert_eq!(list.items, vec!["F", "P", "S"]);
    assert!(list.inline_markup);
    assert!(matches!(
        list.layout,
        crate::ui::document::ValueLayout::AutoGrid
    ));

    let document = DocumentModel::from_guide_view(&GuideView {
        notes: vec!["Use bare help for the REPL overview.".to_string()],
        ..GuideView::default()
    })
    .lower_to_render_document(
        LowerDocumentOptions {
            frame_style: SectionFrameStyle::Top,
            panel_kind: Some("help"),
            key_value_border: TableBorderStyle::None,
            key_value_indent: None,
            key_value_gap: None,
            ruled_section_policy: crate::ui::chrome::RuledSectionPolicy::PerSection,
        },
        &mut 1,
    );

    let Block::Panel(panel) = &document.blocks[0] else {
        panic!("expected notes panel");
    };
    let Block::Line(first) = &panel.body.blocks[0] else {
        panic!("expected indented notes line");
    };
    assert_eq!(
        first
            .parts
            .iter()
            .map(|part| part.text.as_str())
            .collect::<Vec<_>>(),
        vec!["  Use bare help for the REPL overview."]
    );
}

#[test]
fn from_value_classifies_root_shapes_and_respects_preferred_key_order_unit() {
    let preferred = vec!["uid".to_string(), "city".to_string()];
    let scalar_object = json!({"uid": "alice", "mail": "a@uio.no"});
    let model = DocumentModel::from_value(&scalar_object, None);
    assert!(matches!(model.blocks[0], BlockModel::KeyValue(_)));

    let object_model = DocumentModel::from_value(
        &json!({"mail": "a@uio.no", "city": "Oslo", "uid": "alice"}),
        Some(&preferred),
    );
    let BlockModel::KeyValue(key_values) = &object_model.blocks[0] else {
        panic!("expected scalar object to lower to key/value rows");
    };
    assert_eq!(
        key_values
            .rows
            .iter()
            .map(|row| row.key.as_str())
            .collect::<Vec<_>>(),
        vec!["uid", "city", "mail"]
    );

    let list_model = DocumentModel::from_value(&json!(["alice", "bob"]), None);
    assert!(matches!(
        list_model.blocks.as_slice(),
        [BlockModel::List(_)]
    ));

    let table_model = DocumentModel::from_value(
        &json!([
            {"uid": "alice", "city": "Oslo"},
            {"uid": "bob", "city": "Bergen"}
        ]),
        None,
    );
    assert!(matches!(
        table_model.blocks.as_slice(),
        [BlockModel::Table(_)]
    ));

    let mixed_model = DocumentModel::from_value(
        &json!([
            {"uid": "alice"},
            ["nested", "array"],
            7
        ]),
        None,
    );
    assert!(matches!(mixed_model.blocks[0], BlockModel::KeyValue(_)));
    assert!(matches!(mixed_model.blocks[1], BlockModel::Blank));
    assert!(matches!(mixed_model.blocks[2], BlockModel::List(_)));
    assert!(matches!(mixed_model.blocks[3], BlockModel::Blank));
    assert!(matches!(mixed_model.blocks[4], BlockModel::Paragraph(_)));

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
            ruled_section_policy: crate::ui::chrome::RuledSectionPolicy::PerSection,
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
fn markdown_renderer_handles_empty_models_and_formats_lists_tables_and_key_value_blocks_unit() {
    let empty = DocumentModel { blocks: Vec::new() };
    assert_eq!(empty.to_markdown_with_width(Some(40)), "");

    let model = DocumentModel {
        blocks: vec![
            BlockModel::List(super::ListModel {
                items: vec!["alpha".to_string(), "beta".to_string()],
                indent: 0,
                inline_markup: false,
                layout: crate::ui::document::ValueLayout::Vertical,
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
            items: vec!["`alpha` beta".to_string(), "gamma".to_string()],
            indent: 2,
            inline_markup: true,
            layout: crate::ui::document::ValueLayout::Vertical,
        })],
    };

    let document = model.lower_to_render_document(
        LowerDocumentOptions {
            frame_style: SectionFrameStyle::Top,
            panel_kind: None,
            key_value_border: TableBorderStyle::None,
            key_value_indent: None,
            key_value_gap: None,
            ruled_section_policy: crate::ui::chrome::RuledSectionPolicy::PerSection,
        },
        &mut 11,
    );

    assert!(matches!(document.blocks[0], Block::Line(_)));
    assert!(matches!(document.blocks[1], Block::Line(_)));
    let Block::Line(first) = &document.blocks[0] else {
        panic!("expected line block");
    };
    assert_eq!(first.parts[0].text, "  ");
    assert_eq!(first.parts[1].text, "alpha");
    assert_eq!(
        first.parts[1].token,
        Some(crate::ui::style::StyleToken::Key)
    );
    assert_eq!(first.parts[2].text, " beta");
    assert_eq!(
        first.parts[2].token,
        Some(crate::ui::style::StyleToken::Value)
    );
}

#[test]
fn shared_top_bottom_policy_shares_section_separators_and_closes_once_unit() {
    let model = DocumentModel {
        blocks: vec![
            BlockModel::Section(super::SectionModel {
                title: Some("One".to_string()),
                blocks: vec![BlockModel::Paragraph("alpha".to_string())],
            }),
            BlockModel::Blank,
            BlockModel::Section(super::SectionModel {
                title: Some("Two".to_string()),
                blocks: vec![BlockModel::Paragraph("beta".to_string())],
            }),
        ],
    };

    let document = model.lower_to_render_document(
        LowerDocumentOptions {
            frame_style: SectionFrameStyle::TopBottom,
            panel_kind: None,
            key_value_border: TableBorderStyle::None,
            key_value_indent: None,
            key_value_gap: None,
            ruled_section_policy: crate::ui::RuledSectionPolicy::Shared,
        },
        &mut 1,
    );

    let Block::Panel(first) = &document.blocks[0] else {
        panic!("expected first panel");
    };
    assert_eq!(first.rules, crate::ui::document::PanelRules::Top);
    assert!(first.frame_style.is_none());

    let Block::Panel(second) = &document.blocks[2] else {
        panic!("expected second panel");
    };
    assert_eq!(second.rules, crate::ui::document::PanelRules::Both);
    assert!(second.frame_style.is_none());
}
