use crate::core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use crate::core::output_model::{
    Group, OutputDocument, OutputDocumentKind, OutputItems, OutputMeta, OutputResult,
};
use crate::guide::{GuideSection, GuideSectionKind, GuideView};
use crate::row;
use serde_json::json;

use super::doc::{Block, SectionTitleChrome};
use super::settings::HelpTableChrome;
use super::{
    GuideDefaultFormat, HelpChromeSettings, HelpLayout, RenderProfile, RenderSettings,
    StructuredGuideRenderOptions, plan_output, render_guide_with_layout, render_output,
    render_output_for_copy, render_structured_output_with_guide_options, resolve_settings,
};

#[test]
fn planner_prefers_semantic_guide_before_non_explicit_baseline_unit() {
    let output = GuideView::from_text("Usage: osp history <COMMAND>\n\nCommands:\n  list  Show\n")
        .to_output_result();

    let mut inherited_json = RenderSettings::test_plain(OutputFormat::Json);
    inherited_json.guide_default_format = GuideDefaultFormat::Inherit;
    inherited_json.format_explicit = false;
    let inherited_plan = plan_output(&output, &inherited_json, super::RenderProfile::Normal);
    assert_eq!(inherited_plan.format, OutputFormat::Guide);

    let mut explicit_json = RenderSettings::test_plain(OutputFormat::Json);
    explicit_json.guide_default_format = GuideDefaultFormat::Guide;
    explicit_json.format_explicit = true;
    let explicit_plan = plan_output(&output, &explicit_json, super::RenderProfile::Normal);
    assert_eq!(explicit_plan.format, OutputFormat::Json);
}

#[test]
fn ui2_renders_generic_rows_as_markdown_table_unit() {
    let output = OutputResult::from_rows(vec![
        row! { "uid" => "alice", "mail" => "a@example.com" },
        row! { "uid" => "bob", "mail" => "b@example.com" },
    ]);
    let mut settings = RenderSettings::test_plain(OutputFormat::Markdown);
    settings.format_explicit = true;

    let rendered = render_output(&output, &settings);
    assert!(rendered.contains("| uid"));
    assert!(rendered.contains("alice"));
    assert!(rendered.contains("bob"));
}

#[test]
fn ui2_renders_guide_markdown_without_a_separate_pipeline_unit() {
    let output =
        GuideView::from_text("Usage: osp history <COMMAND>\n\nCommands:\n  list  List entries\n")
            .to_output_result();
    let mut settings = RenderSettings::test_plain(OutputFormat::Markdown);
    settings.format_explicit = true;

    let rendered = render_output(&output, &settings);
    assert!(rendered.contains("## Usage"));
    assert!(rendered.contains("## Commands"));
    assert!(rendered.contains("- `list` List entries"));
}

#[test]
fn ui2_structured_guide_options_are_owned_by_one_entrypoint_unit() {
    let guide = GuideView::from_text("Usage: osp history <COMMAND>\n");
    let output = guide.to_output_result();
    let settings = RenderSettings::test_plain(OutputFormat::Guide);

    let rendered = render_structured_output_with_guide_options(
        &output,
        &settings,
        StructuredGuideRenderOptions {
            source_guide: Some(&guide),
            layout: HelpLayout::Compact,
            title_prefix: Some("Demo"),
            show_footer_rule: Some(false),
        },
    );

    assert!(rendered.contains("Demo"));
    assert!(rendered.contains("osp history <COMMAND>"));
    assert!(!rendered.contains("---"));
}

#[test]
fn ui2_renders_single_row_output_as_aligned_key_value_unit() {
    let mut output = OutputResult::from_rows(vec![row! {
        "uid" => "alice",
        "display_name" => "Alice Example",
    }]);
    output.meta.key_index = vec!["uid".to_string(), "display_name".to_string()];
    let mut settings = RenderSettings::test_plain(OutputFormat::Mreg);
    settings.format_explicit = true;

    let rendered = render_output(&output, &settings);
    assert_eq!(
        rendered,
        "uid:          alice\ndisplay_name: Alice Example\n"
    );
}

#[test]
fn ui2_table_output_switches_between_ascii_and_unicode_unit() {
    let output =
        OutputResult::from_rows(vec![row! { "uid" => "alice", "mail" => "a@example.com" }]);

    let mut ascii = RenderSettings::test_plain(OutputFormat::Table);
    ascii.format_explicit = true;
    let ascii_rendered = render_output(&output, &ascii);
    assert!(ascii_rendered.contains('+'));
    assert!(ascii_rendered.contains('|'));

    let mut unicode = RenderSettings::test_plain(OutputFormat::Table);
    unicode.format_explicit = true;
    unicode.mode = crate::core::output::RenderMode::Rich;
    unicode.unicode = crate::core::output::UnicodeMode::Always;
    unicode.runtime.stdout_is_tty = true;
    let unicode_rendered = render_output(&output, &unicode);
    assert!(unicode_rendered.contains('┏'));
    assert!(unicode_rendered.contains('┃'));
}

#[test]
fn ui2_rich_help_layout_styles_section_titles_unit() {
    let guide = GuideView::from_text(
        "Usage: osp history <COMMAND>\n\nCommands:\n  list   List history entries\n",
    );
    let mut settings = RenderSettings::test_plain(OutputFormat::Guide);
    settings.mode = RenderMode::Rich;
    settings.color = ColorMode::Always;
    settings.unicode = UnicodeMode::Always;
    settings.runtime.stdout_is_tty = true;
    settings.theme_name = "dracula".to_string();
    settings.width = Some(60);

    let rendered = render_guide_with_layout(&guide, &settings, HelpLayout::Full);
    assert!(rendered.contains("\x1b[38;2;255;121;198mUsage\x1b[0m"));
    assert!(rendered.contains("\x1b[38;2;255;121;198mCommands\x1b[0m"));
}

#[test]
fn ui2_rich_table_output_styles_headers_and_numeric_values_unit() {
    let output = OutputResult::from_rows(vec![row! { "count" => "42", "name" => "alice" }]);
    let mut settings = RenderSettings::test_plain(OutputFormat::Table);
    settings.format_explicit = true;
    settings.mode = RenderMode::Rich;
    settings.color = ColorMode::Always;
    settings.unicode = UnicodeMode::Always;
    settings.runtime.stdout_is_tty = true;
    settings.theme_name = "dracula".to_string();

    let rendered = render_output(&output, &settings);
    assert!(rendered.contains("\x1b[38;2;189;147;249mcount\x1b[0m"));
    assert!(rendered.contains("\x1b[38;2;189;147;249mname\x1b[0m"));
    assert!(rendered.contains("\x1b[38;2;255;121;198m42\x1b[0m"));
}

#[test]
fn ui2_copy_safe_json_output_keeps_trailing_newline_unit() {
    let output = OutputResult::from_rows(vec![row! { "uid" => "alice" }]);
    let mut settings = RenderSettings::test_plain(OutputFormat::Json);
    settings.format_explicit = true;

    let rendered = render_output_for_copy(&output, &settings);
    assert!(rendered.ends_with('\n'));
    assert!(rendered.contains("\"uid\": \"alice\""));
}

#[test]
fn ui2_json_block_uses_row_payload_shape_even_with_semantic_document_unit() {
    let output = OutputResult::from_rows(vec![row! { "uid" => "alice" }]).with_document(
        OutputDocument::new(OutputDocumentKind::Guide, json!({"usage": ["osp history"]})),
    );
    let mut settings = RenderSettings::test_plain(OutputFormat::Json);
    settings.format_explicit = true;

    let plan = plan_output(&output, &settings, super::RenderProfile::Normal);
    let doc = super::lower::lower_output(&output, &plan);

    let Some(Block::Json(json)) = doc.blocks.first() else {
        panic!("expected json block");
    };
    assert!(json.text.contains("\"uid\": \"alice\""));
    assert!(!json.text.contains("\"usage\""));
}

#[test]
fn ui2_resolves_theme_catalog_from_caller_theme_name_unit() {
    let settings = RenderSettings::builder()
        .with_theme_name(" Rose_Pine Moon ")
        .build();

    let resolved = resolve_settings(&settings, RenderProfile::Normal);
    assert_eq!(resolved.theme_name, "rose-pine-moon");
    assert_eq!(resolved.theme.display_name(), "Rose Pine Moon");
    assert_eq!(resolved.theme.palette.title, "#e8dff6");
}

#[test]
fn ui2_renders_full_help_layout_as_top_level_render_block_unit() {
    let guide = GuideView::from_text(
        "Usage: osp history <COMMAND>\n\nCommands:\n  list   List history entries\n",
    );
    let mut settings = RenderSettings::test_plain(OutputFormat::Guide);
    settings.width = Some(80);

    let rendered = render_guide_with_layout(&guide, &settings, HelpLayout::Full);
    assert_eq!(
        rendered,
        "- Usage ------------------------------------------------------------------------\n  osp history <COMMAND>\n\n- Commands ---------------------------------------------------------------------\n  list   List history entries\n--------------------------------------------------------------------------------\n"
    );
}

#[test]
fn ui2_full_help_layout_titles_stay_flush_when_margin_is_set_unit() {
    let guide = GuideView::from_text("Usage: osp history <COMMAND>\n");
    let mut settings = RenderSettings::test_plain(OutputFormat::Guide);
    settings.width = Some(60);
    settings.margin = 2;

    let rendered = render_guide_with_layout(&guide, &settings, HelpLayout::Full);

    assert!(rendered.starts_with("- Usage"));
    assert!(rendered.contains("\n    osp history <COMMAND>\n"));
}

#[test]
fn ui2_full_help_layout_keeps_blank_between_paragraphs_and_data_unit() {
    let guide = GuideView {
        sections: vec![GuideSection {
            title: "OSP".to_string(),
            kind: GuideSectionKind::Custom,
            paragraphs: vec!["Welcome Demo!".to_string()],
            entries: Vec::new(),
            data: Some(serde_json::json!({
                "Logged in as": "oistes",
                "Theme": "Rose Pine Moon",
            })),
        }],
        ..Default::default()
    };
    let settings = RenderSettings::test_plain(OutputFormat::Guide);

    let rendered = render_guide_with_layout(&guide, &settings, HelpLayout::Full);

    assert!(rendered.contains("Welcome Demo!\n\n  Logged in as"));
    assert!(rendered.contains("Theme"));
}

#[test]
fn ui2_structured_guide_output_keeps_help_entry_indent_unit() {
    let output = GuideView::from_text(
        "Usage: osp history <COMMAND>\n\nCommands:\n  list   List history entries\n",
    )
    .to_output_result();
    let mut settings = RenderSettings::test_plain(OutputFormat::Guide);
    settings.width = Some(80);

    let rendered =
        super::render_structured_output_with_layout(&output, &settings, HelpLayout::Full);

    assert!(rendered.contains("- Commands "));
    assert!(rendered.contains("\n  list  List history entries\n"));
    assert!(!rendered.contains("\nlist  List history entries\n"));
}

#[test]
fn ui2_structured_guide_output_prefers_source_guide_hints_unit() {
    let guide = crate::guide::GuideView {
        preamble: vec!["Usage: osp history <COMMAND>".to_string()],
        commands: vec![crate::guide::GuideEntry {
            name: "list".to_string(),
            short_help: "List history entries".to_string(),
            display_indent: Some(">>".to_string()),
            display_gap: Some(" -> ".to_string()),
        }],
        ..Default::default()
    };
    let output = guide.to_output_result();
    let settings = RenderSettings::test_plain(OutputFormat::Guide);

    let rendered = super::render_structured_output_with_source_guide(
        &output,
        Some(&guide),
        &settings,
        HelpLayout::Full,
    );

    assert!(rendered.contains("\n>>list -> List history entries\n"));
}

#[test]
fn ui2_renders_compact_help_layout_as_top_level_render_block_unit() {
    let guide = GuideView::from_text(
        "Usage: osp history <COMMAND>\n\nCommands:\n  list   List history entries\n",
    );
    let settings = RenderSettings::test_plain(OutputFormat::Guide);

    let rendered = render_guide_with_layout(&guide, &settings, HelpLayout::Compact);
    assert_eq!(
        rendered,
        "Usage: osp history <COMMAND>\n\nCommands:\n  list   List history entries\n"
    );
}

#[test]
fn ui2_renders_minimal_help_layout_as_top_level_render_block_unit() {
    let guide = GuideView::from_text(
        "Usage: osp history <COMMAND>\n\nCommands:\n  list   List history entries\n",
    );
    let settings = RenderSettings::test_plain(OutputFormat::Guide);

    let rendered = render_guide_with_layout(&guide, &settings, HelpLayout::Minimal);
    assert_eq!(
        rendered,
        "Usage: osp history <COMMAND>\n\nCommands:\n  list   List history entries\n"
    );
}

#[test]
fn ui2_help_layout_lowering_uses_plain_sections_and_inline_usage_suffix_unit() {
    let guide = GuideView::from_text(
        "Usage: osp history <COMMAND>\n\nCommands:\n  list   List history entries\n",
    );
    let settings = RenderSettings::test_plain(OutputFormat::Guide);
    let plan = plan_output(&guide.to_output_result(), &settings, RenderProfile::Normal);

    let compact = super::lower::lower_guide_help_layout(&guide, &plan, HelpLayout::Compact, false);

    let Block::Section(usage) = &compact.blocks[0] else {
        panic!("expected usage section");
    };
    let Block::Blank = &compact.blocks[1] else {
        panic!("expected explicit blank between compact sections");
    };
    let Block::Section(commands) = &compact.blocks[2] else {
        panic!("expected commands section");
    };

    assert_eq!(usage.title.as_deref(), Some("Usage"));
    assert_eq!(
        usage.inline_title_suffix.as_deref(),
        Some("osp history <COMMAND>")
    );
    assert!(usage.blocks.is_empty());
    assert_eq!(usage.title_chrome, SectionTitleChrome::Plain);
    assert_eq!(commands.title.as_deref(), Some("Commands"));
    assert_eq!(commands.title_chrome, SectionTitleChrome::Plain);
}

#[test]
fn ui2_help_layout_lowering_makes_footer_a_real_block_unit() {
    let guide = GuideView::from_text(
        "Usage: osp history <COMMAND>\n\nCommands:\n  list   List history entries\n",
    );
    let settings = RenderSettings::test_plain(OutputFormat::Guide);
    let plan = plan_output(&guide.to_output_result(), &settings, RenderProfile::Normal);

    let full = super::lower::lower_guide_help_layout(&guide, &plan, HelpLayout::Full, true);

    let Some(Block::Rule) = full.blocks.last() else {
        panic!("expected lowered footer rule block");
    };
}

#[test]
fn ui2_lowering_carries_help_entry_defaults_in_ir_unit() {
    let guide = GuideView::from_text("Commands:\n  list   List history entries\n");
    let settings = RenderSettings::test_plain(OutputFormat::Guide);
    let plan = plan_output(&guide.to_output_result(), &settings, RenderProfile::Normal);
    let doc = super::lower::lower_guide_help_layout(&guide, &plan, HelpLayout::Compact, false);

    let Block::Section(block) = &doc.blocks[0] else {
        panic!("expected help section block");
    };
    let Block::GuideEntries(entries) = &block.blocks[0] else {
        panic!("expected guide entries block");
    };

    assert_eq!(entries.default_indent, "  ");
    assert_eq!(entries.default_gap, None);
}

#[test]
fn ui2_lowering_stamps_direct_guide_render_behavior_into_ir_unit() {
    let guide = GuideView {
        sections: vec![
            GuideSection::new("Notes", GuideSectionKind::Notes)
                .paragraph("Use `osp history list` for recent commands."),
        ],
        ..Default::default()
    };
    let output = guide.to_output_result();

    let guide_plan = plan_output(
        &output,
        &RenderSettings::test_plain(OutputFormat::Guide),
        RenderProfile::Normal,
    );
    let guide_doc = super::lower::lower_output(&output, &guide_plan);
    let Block::Section(guide_section) = &guide_doc.blocks[0] else {
        panic!("expected direct guide section");
    };
    let Block::Paragraph(guide_paragraph) = &guide_section.blocks[0] else {
        panic!("expected guide paragraph");
    };

    assert_eq!(guide_section.body_indent, 2);
    assert!(guide_section.trailing_newline);
    assert!(guide_paragraph.inline_markup);
    assert_eq!(guide_paragraph.indent, 0);

    let mut markdown_settings = RenderSettings::test_plain(OutputFormat::Markdown);
    markdown_settings.format_explicit = true;
    let markdown_plan = plan_output(&output, &markdown_settings, RenderProfile::Normal);
    let markdown_doc = super::lower::lower_output(&output, &markdown_plan);
    let Block::Section(markdown_section) = &markdown_doc.blocks[0] else {
        panic!("expected markdown guide section");
    };

    assert_eq!(markdown_section.body_indent, 0);
    assert!(!markdown_section.trailing_newline);
}

#[test]
fn ui2_help_chrome_settings_override_indent_gap_and_spacing_unit() {
    let guide = crate::guide::GuideView {
        commands: vec![crate::guide::GuideEntry {
            name: "show".to_string(),
            short_help: "Display current value".to_string(),
            display_indent: None,
            display_gap: None,
        }],
        options: vec![crate::guide::GuideEntry {
            name: "-h, --help".to_string(),
            short_help: "Print help".to_string(),
            display_indent: None,
            display_gap: None,
        }],
        ..Default::default()
    };
    let mut settings = RenderSettings::test_plain(OutputFormat::Guide);
    settings.help_chrome = HelpChromeSettings {
        table_chrome: HelpTableChrome::None,
        entry_indent: Some(4),
        entry_gap: Some(3),
        section_spacing: Some(0),
    };

    let rendered = render_guide_with_layout(&guide, &settings, HelpLayout::Compact);

    assert_eq!(
        rendered,
        "Commands:\n    show   Display current value\nOptions:\n    -h, --help   Print help\n"
    );
}

#[test]
fn ui_grouped_outputs_lower_and_render_with_one_group_owner_unit() {
    let output = OutputResult {
        items: OutputItems::Groups(vec![
            Group {
                groups: row! { "team" => "prod" },
                aggregates: row! { "count" => 2 },
                rows: vec![row! { "uid" => "alice" }, row! { "uid" => "bob" }],
            },
            Group {
                groups: row! { "team" => "stage" },
                aggregates: row! { "count" => 1 },
                rows: vec![row! { "uid" => "carol" }],
            },
        ]),
        document: None,
        meta: OutputMeta {
            key_index: vec!["team".to_string(), "count".to_string(), "uid".to_string()],
            column_align: Vec::new(),
            wants_copy: false,
            grouped: true,
            render_recommendation: None,
        },
    };

    let table_plan = plan_output(
        &output,
        &RenderSettings::test_plain(OutputFormat::Table),
        RenderProfile::Normal,
    );
    let table_doc = super::lower::lower_output(&output, &table_plan);
    assert!(matches!(table_doc.blocks[0], Block::Table(_)));
    assert!(matches!(table_doc.blocks[1], Block::Blank));
    assert!(matches!(table_doc.blocks[2], Block::Table(_)));
    let Block::Table(first_group) = &table_doc.blocks[0] else {
        panic!("expected first grouped table");
    };
    assert_eq!(first_group.summary[0].key, "team");
    assert_eq!(first_group.summary[1].key, "count");

    let mut json_settings = RenderSettings::test_plain(OutputFormat::Json);
    json_settings.format_explicit = true;
    let rendered_json = render_output(&output, &json_settings);
    assert!(rendered_json.contains("\"groups\""));
    assert!(rendered_json.contains("\"aggregates\""));
    assert!(rendered_json.contains("\"rows\""));
}

#[test]
fn ui_help_layout_lowers_mixed_structured_section_data_through_one_pipeline_unit() {
    let guide = GuideView {
        sections: vec![
            GuideSection::new("Session", GuideSectionKind::Custom).data(json!({
                "profile": "prod",
                "theme": "rose-pine-moon"
            })),
            GuideSection::new("Examples", GuideSectionKind::Custom).data(json!([
                "osp history list",
                "osp history clear",
                "osp history last",
                "osp history search",
                "osp history export",
                "osp history import"
            ])),
            GuideSection::new("Shortcuts", GuideSectionKind::Custom).data(json!([
                {"name": "list", "short_help": "List history"},
                {"name": "clear", "short_help": "Clear history"}
            ])),
            GuideSection::new("Matrix", GuideSectionKind::Custom).data(json!([
                {"uid": "alice", "state": "ok"},
                {"uid": "bob", "state": "warn"}
            ])),
        ],
        ..Default::default()
    };
    let mut settings = RenderSettings::test_plain(OutputFormat::Guide);
    settings.width = Some(40);
    let plan = plan_output(&guide.to_output_result(), &settings, RenderProfile::Normal);
    let doc = super::lower::lower_guide_help_layout(&guide, &plan, HelpLayout::Full, false);

    let Block::Section(session) = &doc.blocks[0] else {
        panic!("expected session section");
    };
    assert!(matches!(session.blocks[0], Block::KeyValue(_)));
    let Block::Section(examples) = &doc.blocks[2] else {
        panic!("expected examples section");
    };
    let Block::List(list) = &examples.blocks[0] else {
        panic!("expected scalar list");
    };
    assert!(list.auto_grid);
    let Block::Section(shortcuts) = &doc.blocks[4] else {
        panic!("expected shortcuts section");
    };
    assert!(matches!(shortcuts.blocks[0], Block::GuideEntries(_)));
    let Block::Section(matrix) = &doc.blocks[6] else {
        panic!("expected matrix section");
    };
    assert!(matches!(matrix.blocks[0], Block::Table(_)));

    let rendered = render_guide_with_layout(&guide, &settings, HelpLayout::Full);
    assert!(rendered.contains("Session"));
    assert!(rendered.contains("profile"));
    assert!(rendered.contains("osp history list"));
    assert!(rendered.contains("list"));
    assert!(rendered.contains("alice"));
}
