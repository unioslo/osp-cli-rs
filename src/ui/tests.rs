use crate::core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use crate::core::output_model::{
    ColumnAlignment, Group, OutputDocument, OutputDocumentKind, OutputItems, OutputMeta,
    OutputResult,
};
use crate::guide::{GuideSection, GuideSectionKind, GuideView};
use crate::row;
use serde_json::json;
use unicode_width::UnicodeWidthStr;

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
fn ui2_terminal_tables_honor_width_for_clip_ellipsis_and_wrap_unit() {
    let output = OutputResult::from_rows(vec![row! {
        "provider" => "vmware",
        "message" => "Not authorized for vcenter vcsa-test03.uio.no",
        "next" => "Request access or choose another value",
    }]);

    for (overflow, expected) in [
        (super::TableOverflow::Clip, "Not authorized"),
        (super::TableOverflow::Ellipsis, "…"),
        (super::TableOverflow::Wrap, "another value"),
    ] {
        let mut settings = RenderSettings::test_plain(OutputFormat::Table);
        settings.format_explicit = true;
        settings.width = Some(48);
        settings.table_overflow = overflow;

        let rendered = render_output(&output, &settings);

        assert!(rendered.contains(expected), "{overflow:?}: {rendered}");
        assert!(
            rendered
                .lines()
                .all(|line| UnicodeWidthStr::width(line) <= 48),
            "{overflow:?} exceeded width:\n{rendered}"
        );
    }
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
fn ui2_mreg_renders_scalar_array_fields_as_multiline_lists_unit() {
    let mut output = OutputResult::from_rows(vec![row! {
        "uid" => "oistes",
        "eduPersonAffiliation" => json!(["employee", "member", "staff"]),
        "filegroups" => json!(["oistes", "ucore", "usit", "vortex-opptak"]),
        "netgroups" => json!([
            "ansatt-373034",
            "ansatt-tekadm-373034",
            "dia-drs-vaktsjefer",
            "it-uio-azure-users",
            "it-uio-ms365-ansatt",
            "it-uio-ms365-ansatt-publisert",
        ]),
    }]);
    output.meta.key_index = vec![
        "uid".to_string(),
        "eduPersonAffiliation".to_string(),
        "filegroups".to_string(),
        "netgroups".to_string(),
    ];
    let mut settings = RenderSettings::test_plain(OutputFormat::Mreg);
    settings.format_explicit = true;
    settings.width = Some(70);

    let rendered = render_output(&output, &settings);

    assert!(rendered.contains("uid:"));
    assert!(rendered.contains("oistes\n"));
    assert!(rendered.contains("eduPersonAffiliation (3):"));
    assert!(rendered.contains("employee\n"));
    assert!(rendered.contains("member\n"));
    assert!(rendered.contains("staff\n"));
    assert!(rendered.contains("filegroups (4):"));
    assert!(rendered.contains("oistes\n"));
    assert!(rendered.contains("vortex-opptak\n"));
    assert!(rendered.contains("netgroups (6):"));
    assert!(rendered.contains("ansatt-373034"));
    assert!(rendered.contains("it-uio-ms365-ansatt-publisert"));
    assert!(!rendered.contains("[\"employee\",\"member\",\"staff\"]"));
}

#[test]
fn ui2_mreg_multiline_scalar_arrays_keep_continuation_alignment_unit() {
    let mut output = OutputResult::from_rows(vec![row! {
        "uid" => "alice",
        "groups" => json!(["red", "blue", "green"]),
    }]);
    output.meta.key_index = vec!["uid".to_string(), "groups".to_string()];
    let mut settings = RenderSettings::test_plain(OutputFormat::Mreg);
    settings.format_explicit = true;

    let rendered = render_output(&output, &settings);

    assert_eq!(
        rendered,
        "uid:        alice\ngroups (3): red\n            blue\n            green\n"
    );
}

#[test]
fn ui2_markdown_renders_single_row_scalar_arrays_as_multiline_key_values_unit() {
    let mut output = OutputResult::from_rows(vec![row! {
        "uid" => "oistes",
        "filegroups" => json!(["oistes", "ucore", "usit", "vortex-opptak"]),
    }]);
    output.meta.key_index = vec!["uid".to_string(), "filegroups".to_string()];
    let mut settings = RenderSettings::test_plain(OutputFormat::Markdown);
    settings.format_explicit = true;

    let rendered = render_output(&output, &settings);

    assert!(rendered.contains("- uid: oistes"));
    assert!(rendered.contains("- filegroups (4):"));
    assert!(rendered.contains("  - oistes"));
    assert!(rendered.contains("  - ucore"));
    assert!(rendered.contains("  - vortex-opptak"));
    assert!(!rendered.contains("[\"oistes\",\"ucore\",\"usit\",\"vortex-opptak\"]"));
}

#[test]
fn ui2_rich_mreg_multiline_lists_keep_key_and_value_styling_unit() {
    let output = OutputResult::from_rows(vec![row! {
        "eduPersonAffiliation" => json!(["employee", "member", "staff"]),
    }]);
    let mut settings = RenderSettings::test_plain(OutputFormat::Mreg);
    settings.format_explicit = true;
    settings.mode = RenderMode::Rich;
    settings.color = ColorMode::Always;
    settings.unicode = UnicodeMode::Always;
    settings.runtime.stdout_is_tty = true;
    settings.theme_name = "dracula".to_string();

    let rendered = render_output(&output, &settings);

    assert!(rendered.contains("\x1b[38;2;189;147;249meduPersonAffiliation (3)\x1b[0m"));
    assert!(rendered.contains("\x1b[38;2;248;248;242memployee\x1b[0m"));
    assert!(rendered.contains("\x1b[38;2;248;248;242mmember\x1b[0m"));
    assert!(rendered.contains("\x1b[38;2;248;248;242mstaff\x1b[0m"));
}

#[test]
fn ui2_mreg_renders_nested_object_fields_recursively_unit() {
    let mut output = OutputResult::from_rows(vec![row! {
        "uid" => "alice",
        "profile" => json!({
            "owner": "alice",
            "groups": ["dev", "ops"],
        }),
    }]);
    output.meta.key_index = vec!["uid".to_string(), "profile".to_string()];
    let mut settings = RenderSettings::test_plain(OutputFormat::Mreg);
    settings.format_explicit = true;

    let rendered = render_output(&output, &settings);

    assert!(rendered.contains("uid:"));
    assert!(rendered.contains("profile:"));
    assert!(rendered.contains("owner:"));
    assert!(rendered.contains("groups (2): dev"));
    assert!(rendered.contains("ops"));
    assert!(!rendered.contains("{\"owner\":\"alice\""));
}

#[test]
fn ui2_mreg_renders_nested_object_arrays_as_tables_unit() {
    let mut output = OutputResult::from_rows(vec![row! {
        "name" => "it-usit-gsd-drift",
        "siteadmins" => json!({
            "current": [
                {
                    "name": "iti-ops@usit.uio.no",
                    "role": "primary",
                    "hosts": 242,
                },
            ],
            "expired": [
                {
                    "name": "iti-ssd@usit.uio.no",
                    "role": "primary",
                    "status": "expired",
                    "valid_from": "2023-01-01T00:00:00Z",
                    "valid_to": "2026-05-31T00:00:00Z",
                    "replaces": "it-drift-gd-gsd@usit.uio.no",
                    "replaced_by": "iti-ops@usit.uio.no",
                    "hosts": 3,
                },
                {
                    "name": "it-drift-gd-gsd@usit.uio.no",
                    "role": "primary",
                    "status": "expired",
                    "valid_from": "2022-06-01T00:00:00Z",
                    "valid_to": "2023-01-01T00:00:00Z",
                    "replaced_by": "iti-ssd@usit.uio.no",
                    "hosts": 0,
                },
            ],
        }),
    }]);
    output.meta.key_index = vec!["name".to_string(), "siteadmins".to_string()];
    let mut settings = RenderSettings::test_plain(OutputFormat::Mreg);
    settings.format_explicit = true;

    let rendered = render_output(&output, &settings);

    assert!(rendered.contains("name:       it-usit-gsd-drift"));
    assert!(rendered.contains("siteadmins:"));
    assert!(rendered.contains("current (1):"));
    assert!(rendered.contains("expired (2):"));
    assert!(rendered.contains("iti-ssd@usit.uio.no"));
    assert!(rendered.contains("it-drift-gd-gsd@usit.uio.no"));
    assert!(rendered.contains("replaced_by"));
    assert!(!rendered.contains("[1]:"));
}

#[test]
fn ui2_markdown_renders_nested_object_fields_recursively_unit() {
    let mut output = OutputResult::from_rows(vec![row! {
        "uid" => "alice",
        "profile" => json!({
            "owner": "alice",
            "groups": ["dev", "ops"],
        }),
    }]);
    output.meta.key_index = vec!["uid".to_string(), "profile".to_string()];
    let mut settings = RenderSettings::test_plain(OutputFormat::Markdown);
    settings.format_explicit = true;

    let rendered = render_output(&output, &settings);

    assert!(rendered.contains("- uid: alice"));
    assert!(rendered.contains("- profile:"));
    assert!(rendered.contains("  - owner: alice"));
    assert!(rendered.contains("  - groups (2):"));
    assert!(rendered.contains("    - dev"));
    assert!(rendered.contains("    - ops"));
    assert!(!rendered.contains("{\"owner\":\"alice\""));
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
fn ui2_terminal_table_honors_column_alignment_metadata_unit() {
    let output = OutputResult {
        items: OutputItems::Rows(vec![row! {
            "name" => "alice",
            "count" => "42",
            "state" => "ok",
        }]),
        document: None,
        meta: OutputMeta {
            key_index: vec!["name".to_string(), "count".to_string(), "state".to_string()],
            column_align: vec![
                ColumnAlignment::Left,
                ColumnAlignment::Right,
                ColumnAlignment::Center,
            ],
            wants_copy: false,
            grouped: false,
            render_recommendation: None,
        },
    };
    let mut settings = RenderSettings::test_plain(OutputFormat::Table);
    settings.format_explicit = true;

    let rendered = render_output(&output, &settings);

    assert!(rendered.contains("| name  | count | state |"));
    assert!(rendered.contains("| alice |    42 |  ok   |"));
}

#[test]
fn ui2_markdown_table_honors_column_alignment_metadata_unit() {
    let output = OutputResult {
        items: OutputItems::Rows(vec![row! {
            "name" => "alice",
            "count" => "42",
            "state" => "ok",
        }]),
        document: None,
        meta: OutputMeta {
            key_index: vec!["name".to_string(), "count".to_string(), "state".to_string()],
            column_align: vec![
                ColumnAlignment::Left,
                ColumnAlignment::Right,
                ColumnAlignment::Center,
            ],
            wants_copy: false,
            grouped: false,
            render_recommendation: None,
        },
    };
    let mut settings = RenderSettings::test_plain(OutputFormat::Markdown);
    settings.format_explicit = true;

    let rendered = render_output(&output, &settings);

    assert!(rendered.contains("| :----- | -----: | :-----: |"));
    assert!(rendered.contains("| alice |    42 |  ok   |"));
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
fn ui2_json_block_preserves_an_explicit_json_document_unit() {
    let value = json!({
        "mreg": {"name": "db01.uio.no"},
        "ldap": {"host": "db01.uio.no"}
    });
    let output = OutputResult::from_rows(vec![row! {
        "mreg" => json!({"name": "db01.uio.no"}),
        "ldap" => json!({"host": "db01.uio.no"}),
    }])
    .with_document(OutputDocument::new(OutputDocumentKind::Json, value.clone()));
    let mut settings = RenderSettings::test_plain(OutputFormat::Json);
    settings.format_explicit = true;

    let rendered = render_output(&output, &settings);

    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&rendered).expect("valid JSON"),
        value
    );
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
fn ui_help_layout_lowers_mixed_structured_section_data_to_local_block_kinds_unit() {
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
}
