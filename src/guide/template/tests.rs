use super::{GuideTemplateBlock, GuideTemplateInclude, parse_markdown_template};
use serde_json::json;

#[test]
fn markdown_template_parses_headings_includes_lists_and_inline_markdown_unit() {
    let parsed = parse_markdown_template("# Title\n\nHello *there*\n\n{{ help }}");
    assert_eq!(
        parsed,
        vec![
            GuideTemplateBlock::Heading("Title".to_string()),
            GuideTemplateBlock::Paragraph("Hello *there*".to_string()),
            GuideTemplateBlock::Include(GuideTemplateInclude::Help),
        ]
    );
    assert_eq!(
        parse_markdown_template("Muted _text_ and **strong**."),
        vec![GuideTemplateBlock::Paragraph(
            "Muted *text* and **strong**.".to_string()
        )]
    );
    assert_eq!(
        parse_markdown_template(
            "## Details\n\nLine ~~gone~~ <em>kept</em>\n\n{{ overview }}\n\n- first\n- second\n\n```sh\necho hi\npwd\n```\n",
        ),
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
fn markdown_template_parses_valid_osp_code_blocks_as_semantic_data_unit() {
    assert_eq!(
        parse_markdown_template(
            "## Keybindings\n\n```osp\n[{\"name\":\"Ctrl-D\",\"short_help\":\"exit\"}]\n```\n",
        ),
        vec![
            GuideTemplateBlock::Heading("Keybindings".to_string()),
            GuideTemplateBlock::Data(json!([
                {"name": "Ctrl-D", "short_help": "exit"}
            ])),
        ]
    );
}

#[test]
fn markdown_template_keeps_non_semantic_or_invalid_code_blocks_literal_unit() {
    assert_eq!(
        parse_markdown_template("## Data\n\n```json\n[{\"name\":\"Ctrl-D\"}]\n```\n"),
        vec![
            GuideTemplateBlock::Heading("Data".to_string()),
            GuideTemplateBlock::Paragraph("`[{\"name\":\"Ctrl-D\"}]`".to_string()),
        ]
    );
    assert_eq!(
        parse_markdown_template("## Data\n\n```osp\n[{name:\"Ctrl-D\"}]\n```\n"),
        vec![
            GuideTemplateBlock::Heading("Data".to_string()),
            GuideTemplateBlock::Paragraph("`[{name:\"Ctrl-D\"}]`".to_string()),
        ]
    );
    assert_eq!(
        parse_markdown_template("```osp\n[{\"name\":\"Ctrl-D\"\n```\n"),
        vec![GuideTemplateBlock::Paragraph(
            "`[{\"name\":\"Ctrl-D\"`".to_string()
        )]
    );
}
