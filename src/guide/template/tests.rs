use super::{GuideTemplateBlock, GuideTemplateInclude, parse_markdown_template};
use serde_json::json;

#[test]
fn markdown_template_parses_headings_and_includes_unit() {
    let parsed = parse_markdown_template("# Title\n\nHello *there*\n\n{{ help }}");
    assert_eq!(
        parsed,
        vec![
            GuideTemplateBlock::Heading("Title".to_string()),
            GuideTemplateBlock::Paragraph("Hello *there*".to_string()),
            GuideTemplateBlock::Include(GuideTemplateInclude::Help),
        ]
    );
}

#[test]
fn markdown_template_treats_underscore_emphasis_like_markdown_unit() {
    let parsed = parse_markdown_template("Muted _text_ and **strong**.");
    assert_eq!(
        parsed,
        vec![GuideTemplateBlock::Paragraph(
            "Muted *text* and **strong**.".to_string()
        )]
    );
}

#[test]
fn markdown_template_parses_items_code_blocks_and_overview_include_unit() {
    let parsed = parse_markdown_template(
        "## Details\n\nLine ~~gone~~ <em>kept</em>\n\n{{ overview }}\n\n- first\n- second\n\n```sh\necho hi\npwd\n```\n",
    );
    assert_eq!(
        parsed,
        vec![
            GuideTemplateBlock::Heading("Details".to_string()),
            GuideTemplateBlock::Paragraph("Line ~~gone~~ <em>kept</em>".to_string()),
            GuideTemplateBlock::Include(GuideTemplateInclude::Overview),
            GuideTemplateBlock::Paragraph("- first".to_string()),
            GuideTemplateBlock::Paragraph("- second".to_string()),
            GuideTemplateBlock::Paragraph("`echo hi`".to_string()),
            GuideTemplateBlock::Paragraph("`pwd`".to_string()),
        ]
    );
}

#[test]
fn markdown_template_parses_osp_code_blocks_as_semantic_data_unit() {
    let parsed = parse_markdown_template(
        "## Keybindings\n\n```osp\n[{\"name\":\"Ctrl-D\",\"short_help\":\"exit\"}]\n```\n",
    );

    assert_eq!(
        parsed,
        vec![
            GuideTemplateBlock::Heading("Keybindings".to_string()),
            GuideTemplateBlock::Data(json!([
                {"name": "Ctrl-D", "short_help": "exit"}
            ])),
        ]
    );
}

#[test]
fn markdown_template_keeps_non_osp_json_fences_literal_unit() {
    let parsed = parse_markdown_template("## Data\n\n```json\n[{\"name\":\"Ctrl-D\"}]\n```\n");

    assert_eq!(
        parsed,
        vec![
            GuideTemplateBlock::Heading("Data".to_string()),
            GuideTemplateBlock::Paragraph("`[{\"name\":\"Ctrl-D\"}]`".to_string()),
        ]
    );
}

#[test]
fn markdown_template_keeps_invalid_osp_fences_literal_unit() {
    let parsed = parse_markdown_template("## Data\n\n```osp\n[{name:\"Ctrl-D\"}]\n```\n");

    assert_eq!(
        parsed,
        vec![
            GuideTemplateBlock::Heading("Data".to_string()),
            GuideTemplateBlock::Paragraph("`[{name:\"Ctrl-D\"}]`".to_string()),
        ]
    );
}

#[test]
fn markdown_template_keeps_invalid_osp_blocks_visible_as_literal_code_unit() {
    let parsed = parse_markdown_template("```osp\n[{\"name\":\"Ctrl-D\"\n```\n");

    assert_eq!(
        parsed,
        vec![GuideTemplateBlock::Paragraph(
            "`[{\"name\":\"Ctrl-D\"`".to_string()
        )]
    );
}
