use crate::core::document::{DocBlock, Document, DocumentEntry, HelpLevel};
use crate::core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use crate::core::output_model::{
    ColumnAlignment, Group, OutputDocument, OutputItems, OutputMeta, OutputResult,
};

use crate::row;
use serde_json::json;

use super::doc::{Block, SectionTitleChrome};
use super::{HelpLayout, RenderSettings, RenderTarget, plan_format, render, resolve_settings};

#[test]
fn planner_prefers_semantic_guide_before_non_explicit_baseline_unit() {
    let output =
        Document::from_help_text("Usage: osp history <COMMAND>\n\nCommands:\n  list  Show\n")
            .to_output_result();

    let mut inherited_json = RenderSettings::test_plain(OutputFormat::Json);
    inherited_json.format_explicit = false;
    let inherited_plan = plan_format(&output, &inherited_json, super::RenderTarget::OneShotScreen);
    assert_eq!(inherited_plan.format, OutputFormat::Auto);

    let mut explicit_json = RenderSettings::test_plain(OutputFormat::Json);
    explicit_json.format_explicit = true;
    let explicit_plan = plan_format(&output, &explicit_json, super::RenderTarget::OneShotScreen);
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

    let rendered = render(&output, &crate::ui::RenderRequest::new(&settings));
    assert!(rendered.contains("| uid"));
    assert!(rendered.contains("alice"));
    assert!(rendered.contains("bob"));
}

#[test]
fn semantic_document_blocks_use_shared_markdown_rendering_unit() {
    let document = Document {
        blocks: vec![
            DocBlock::Section {
                role: Default::default(),
                title: "Options".to_string(),
                level: HelpLevel::Normal,
                blocks: vec![DocBlock::Entries {
                    entries: vec![DocumentEntry {
                        role: Default::default(),
                        name: "--json".to_string(),
                        description: "Emit JSON".to_string(),
                    }],
                }],
            },
            DocBlock::List {
                items: vec!["first".to_string(), "second".to_string()],
            },
        ],
    };
    let mut settings = RenderSettings::test_plain(OutputFormat::Markdown);
    settings.format_explicit = true;

    let rendered = render(
        &document.to_output_result(),
        &crate::ui::RenderRequest::new(&settings),
    );

    assert!(rendered.contains("## Options"));
    assert!(rendered.contains("- `--json` Emit JSON"));
    assert!(rendered.contains("- first\n- second"));
    let document = Document {
        blocks: vec![
            DocBlock::Section {
                role: Default::default(),
                title: "First".into(),
                level: HelpLevel::Normal,
                blocks: vec![DocBlock::Entries {
                    entries: vec![DocumentEntry {
                        role: Default::default(),
                        name: "zebra".into(),
                        description: "same".into(),
                    }],
                }],
            },
            DocBlock::Section {
                role: Default::default(),
                title: "Second".into(),
                level: HelpLevel::Normal,
                blocks: vec![DocBlock::Entries {
                    entries: vec![DocumentEntry {
                        role: Default::default(),
                        name: "apple".into(),
                        description: "same".into(),
                    }],
                }],
            },
        ],
    };
    let output = crate::dsl::apply_output_pipeline(
        document.to_output_result(),
        &["S name".into(), "L 1".into(), "F name=apple".into()],
    )
    .unwrap();
    assert!(output.meta.document_rows);
    assert!(output.document.is_none());
    assert_eq!(output.as_rows().unwrap()[0]["section"], "Second");
    let mut settings = RenderSettings::test_plain(OutputFormat::Markdown);
    settings.format_explicit = true;
    let rendered = render(&output, &crate::ui::RenderRequest::new(&settings));
    assert!(rendered.contains("## Second"));
    assert!(!rendered.contains("First"));
    let projected = crate::dsl::apply_output_pipeline(output, &["P name".into()]).unwrap();
    assert!(!projected.meta.document_rows);
    assert!(
        render(
            &projected,
            &crate::ui::RenderRequest::new(&RenderSettings::test_plain(OutputFormat::Markdown))
        )
        .contains("| name")
    );
}

#[test]
fn ui2_terminal_tables_preserve_first_column_for_all_overflow_modes_unit() {
    let message =
        "Not authorized for vcenter vcsa-test03.uio.no; request access or choose another value";
    let output = OutputResult::from_rows(vec![row! {
        "message" => message,
        "provider" => "vmware",
        "next" => "Request access or choose another value",
    }]);

    for overflow in [
        super::TableOverflow::Clip,
        super::TableOverflow::Ellipsis,
        super::TableOverflow::Wrap,
    ] {
        let mut settings = RenderSettings::test_plain(OutputFormat::Table);
        settings.format_explicit = true;
        settings.width = Some(48);
        settings.runtime.stdout_is_tty = true;
        settings.table_overflow = overflow;

        let rendered = render(&output, &crate::ui::RenderRequest::new(&settings));

        assert!(rendered.contains(message), "{overflow:?}: {rendered}");
        assert!(rendered.contains("+2 hidden: provider, next"));
    }
}

#[test]
fn ui2_structured_guide_options_are_owned_by_one_entrypoint_unit() {
    let guide = Document::from_help_text("Usage: osp history <COMMAND>\n");
    let output = guide.to_output_result();
    let settings = RenderSettings::test_plain(OutputFormat::Auto);

    let rendered = render(
        &output,
        &crate::ui::RenderRequest {
            layout: Some(HelpLayout::Compact),
            title_prefix: Some("Demo"),
            show_footer_rule: Some(false),
            ..crate::ui::RenderRequest::new(&settings)
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

    let rendered = render(&output, &crate::ui::RenderRequest::new(&settings));
    assert_eq!(
        rendered,
        "uid:          alice\ndisplay_name: Alice Example\n"
    );
}

#[test]
fn ui2_mreg_renders_scalar_arrays_inline_or_as_hanging_lists_unit() {
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

    let rendered = render(&output, &crate::ui::RenderRequest::new(&settings));

    assert!(rendered.contains("uid:"));
    assert!(rendered.contains("oistes\n"));
    assert!(rendered.contains("eduPersonAffiliation (3):"));
    assert!(rendered.contains("eduPersonAffiliation (3):\n  employee"));
    assert!(rendered.contains("filegroups (4):"));
    assert!(rendered.contains("oistes\n"));
    assert!(rendered.contains("filegroups (4):\n  oistes"));
    assert!(rendered.contains("netgroups (6):"));
    assert!(rendered.contains("ansatt-373034"));
    assert!(rendered.contains("it-uio-ms365-ansatt-publisert"));
    assert!(!rendered.contains("[\"employee\",\"member\",\"staff\"]"));
}

#[test]
fn ui2_mreg_scalar_arrays_that_fit_stay_inline_unit() {
    let mut output = OutputResult::from_rows(vec![row! {
        "uid" => "alice",
        "groups" => json!(["red", "blue", "green"]),
    }]);
    output.meta.key_index = vec!["uid".to_string(), "groups".to_string()];
    let mut settings = RenderSettings::test_plain(OutputFormat::Mreg);
    settings.format_explicit = true;

    let rendered = render(&output, &crate::ui::RenderRequest::new(&settings));

    assert_eq!(
        rendered,
        "uid:        alice\ngroups (3): red, blue, green\n"
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

    let rendered = render(&output, &crate::ui::RenderRequest::new(&settings));

    assert!(rendered.contains("- uid: oistes"));
    assert!(rendered.contains("- filegroups (4):"));
    assert!(rendered.contains("  - oistes"));
    assert!(rendered.contains("  - ucore"));
    assert!(rendered.contains("  - vortex-opptak"));
    assert!(!rendered.contains("[\"oistes\",\"ucore\",\"usit\",\"vortex-opptak\"]"));
}

#[test]
fn ui2_rich_mreg_inline_lists_keep_key_and_value_styling_unit() {
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

    let rendered = render(&output, &crate::ui::RenderRequest::new(&settings));

    assert!(rendered.contains("\x1b[38;2;189;147;249meduPersonAffiliation (3)\x1b[0m"));
    for value in ["employee", "member", "staff"] {
        assert!(rendered.contains(&format!("\x1b[38;2;248;248;242m{value}\x1b[0m")));
    }
}

#[test]
fn ui2_mreg_renders_nested_object_fields_recursively_unit() {
    let mut output = OutputResult::from_rows(vec![row! {
        "uid" => "alice",
        "profile" => json!({
            "owner": "alice",
            "groups": ["dev", "ops"],
            "evidence": [null, "pending", {"source": "inventory"}, ["first", "second"]],
        }),
    }]);
    output.meta.key_index = vec!["uid".to_string(), "profile".to_string()];
    let mut settings = RenderSettings::test_plain(OutputFormat::Mreg);
    settings.format_explicit = true;

    let rendered = render(&output, &crate::ui::RenderRequest::new(&settings));

    assert!(rendered.contains("uid:"));
    assert!(rendered.contains("profile:"));
    assert!(rendered.contains("owner:"));
    assert!(
        rendered
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .contains("groups (2): dev"),
        "{rendered}"
    );
    assert!(rendered.contains("ops"));
    assert!(rendered.contains("evidence (4)"), "{rendered}");
    for marker in ["[1]", "[2]", "[3]", "[4]"] {
        assert!(rendered.contains(marker), "{rendered}");
    }
    for value in ["pending", "source:", "inventory", "first", "second"] {
        assert!(rendered.contains(value), "{rendered}");
    }
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
    settings.width = Some(240);
    settings.width_max = 0;
    settings.format_explicit = true;

    let rendered = render(&output, &crate::ui::RenderRequest::new(&settings));

    assert!(rendered.contains("name: it-usit-gsd-drift"));
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

    let rendered = render(&output, &crate::ui::RenderRequest::new(&settings));

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
    let ascii_rendered = render(&output, &crate::ui::RenderRequest::new(&ascii));
    assert!(ascii_rendered.contains('+'));
    assert!(ascii_rendered.contains('|'));

    let mut unicode = RenderSettings::test_plain(OutputFormat::Table);
    unicode.format_explicit = true;
    unicode.mode = crate::core::output::RenderMode::Rich;
    unicode.unicode = crate::core::output::UnicodeMode::Always;
    unicode.runtime.stdout_is_tty = true;
    let unicode_rendered = render(&output, &crate::ui::RenderRequest::new(&unicode));
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

    let rendered = render(&output, &crate::ui::RenderRequest::new(&settings));
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
            title_field: None,
            display_notes: Vec::new(),
            context: Default::default(),
            key_index: vec!["name".to_string(), "count".to_string(), "state".to_string()],
            unix_timestamp_columns: Vec::new(),
            display_rules: Vec::new(),
            display_columns: None,
            display_column_labels: None,
            column_align: vec![
                ColumnAlignment::Left,
                ColumnAlignment::Right,
                ColumnAlignment::Center,
            ],
            wants_copy: false,
            wants_page: false,
            wants_editor: false,
            grouped: false,
            document_rows: false,
            document_row_roles: Vec::new(),
            document_entry_gap: None,
            document_literal_commands: Vec::new(),
            document_command_prefixes: false,
            document_long_help: false,
            render_recommendation: None,
            display_limit: None,
        },
    };
    let mut settings = RenderSettings::test_plain(OutputFormat::Table);
    settings.format_explicit = true;

    let rendered = render(&output, &crate::ui::RenderRequest::new(&settings));

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
            title_field: None,
            display_notes: Vec::new(),
            context: Default::default(),
            key_index: vec!["name".to_string(), "count".to_string(), "state".to_string()],
            unix_timestamp_columns: Vec::new(),
            display_rules: Vec::new(),
            display_columns: None,
            display_column_labels: None,
            column_align: vec![
                ColumnAlignment::Left,
                ColumnAlignment::Right,
                ColumnAlignment::Center,
            ],
            wants_copy: false,
            wants_page: false,
            wants_editor: false,
            grouped: false,
            document_rows: false,
            document_row_roles: Vec::new(),
            document_entry_gap: None,
            document_literal_commands: Vec::new(),
            document_command_prefixes: false,
            document_long_help: false,
            render_recommendation: None,
            display_limit: None,
        },
    };
    let mut settings = RenderSettings::test_plain(OutputFormat::Markdown);
    settings.format_explicit = true;

    let rendered = render(&output, &crate::ui::RenderRequest::new(&settings));

    assert!(rendered.contains("| :----- | -----: | :-----: |"));
    assert!(rendered.contains("| alice |    42 |  ok   |"));
}

#[test]
fn ui2_copy_safe_json_output_keeps_trailing_newline_unit() {
    let output = OutputResult::from_rows(vec![row! { "uid" => "alice" }]);
    let mut settings = RenderSettings::test_plain(OutputFormat::Json);
    settings.format_explicit = true;

    let rendered = render(
        &output,
        &crate::ui::RenderRequest {
            target: crate::ui::RenderTarget::Clipboard,
            ..crate::ui::RenderRequest::new(&settings)
        },
    );
    assert!(rendered.ends_with('\n'));
    assert!(rendered.contains("\"uid\": \"alice\""));
}

#[test]
fn ui2_json_export_preserves_semantic_document_unit() {
    let output = OutputResult::from_rows(vec![row! { "uid" => "alice" }]).with_document(
        OutputDocument::Semantic(
            serde_json::from_value(json!({"blocks": [{"text": "osp history"}]}))
                .expect("semantic document"),
        ),
    );
    let mut settings = RenderSettings::test_plain(OutputFormat::Json);
    settings.format_explicit = true;

    let rendered = render(&output, &crate::ui::RenderRequest::new(&settings));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&rendered).unwrap(),
        json!({"blocks": [{"text": "osp history"}]})
    );
}

#[test]
fn ui2_json_export_preserves_an_explicit_json_document_unit() {
    let value = json!({
        "mreg": {"name": "db01.uio.no"},
        "ldap": {"host": "db01.uio.no"}
    });
    let output = OutputResult::from_rows(vec![row! {
        "mreg" => json!({"name": "db01.uio.no"}),
        "ldap" => json!({"host": "db01.uio.no"}),
    }])
    .with_document(OutputDocument::Json(value.clone()));
    let mut settings = RenderSettings::test_plain(OutputFormat::Json);
    settings.format_explicit = true;

    let rendered = render(&output, &crate::ui::RenderRequest::new(&settings));

    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&rendered).expect("valid JSON"),
        value
    );
}

#[test]
fn ui2_resolves_theme_catalog_from_caller_theme_name_unit() {
    let settings = RenderSettings {
        theme_name: " Rose_Pine Moon ".into(),
        ..RenderSettings::default()
    };

    let resolved = resolve_settings(&settings, RenderTarget::OneShotScreen);
    assert_eq!(resolved.theme_name, "rose-pine-moon");
    assert_eq!(resolved.theme.display_name(), "Rose Pine Moon");
    assert_eq!(resolved.theme.palette.title, "#e8dff6");
}

#[test]
fn ui2_full_help_layout_keeps_usage_inline_with_explicit_margin_unit() {
    let guide = Document::from_help_text("Usage: osp history <COMMAND>\n");
    let mut settings = RenderSettings::test_plain(OutputFormat::Auto);
    settings.width = Some(60);
    settings.margin = 2;
    settings.runtime.stdout_is_tty = true;
    settings.runtime.command_scope = crate::core::CommandScope::ReplRoot;

    let rendered = render(
        &guide.to_output_result(),
        &crate::ui::RenderRequest {
            layout: Some(HelpLayout::Full),
            ..crate::ui::RenderRequest::new(&settings)
        },
    );

    assert!(rendered.starts_with("  Usage: osp history <COMMAND>\n"));
}

#[test]
fn ui2_full_help_layout_keeps_blank_between_paragraphs_and_data_unit() {
    let guide = Document {
        blocks: vec![DocBlock::section(
            "OSP".to_string(),
            vec![
                DocBlock::Paragraph {
                    text: "Welcome Demo!".to_string(),
                },
                DocBlock::Data {
                    value: serde_json::json!({
                        "Logged in as": "oistes",
                        "Theme": "Rose Pine Moon",
                    }),
                },
            ],
        )],
    };
    let settings = RenderSettings::test_plain(OutputFormat::Auto);

    let rendered = render(
        &guide.to_output_result(),
        &crate::ui::RenderRequest {
            layout: Some(HelpLayout::Full),
            ..crate::ui::RenderRequest::new(&settings)
        },
    );

    assert!(rendered.contains("Welcome Demo!\n\nLogged in as"));
    assert!(rendered.contains("Theme"));
}

#[test]
fn ui2_structured_guide_output_keeps_help_entry_indent_unit() {
    let output = Document::from_help_text(
        "Usage: osp history <COMMAND>\n\nCommands:\n  list   List history entries\n",
    )
    .to_output_result();
    let mut settings = RenderSettings::test_plain(OutputFormat::Auto);
    settings.width = Some(80);

    let rendered = super::render(
        &output,
        &crate::ui::RenderRequest {
            layout: Some(HelpLayout::Full),
            ..crate::ui::RenderRequest::new(&settings)
        },
    );

    assert!(rendered.contains("Commands "));
    assert!(rendered.contains("\nlist  List history entries\n"));
    assert!(!rendered.contains("\n  list  List history entries\n"));
}

#[test]
fn ui2_structured_document_output_uses_presentation_metadata_unit() {
    let document = Document::from_help_text("Commands:\n  list  List history entries\n");
    let mut output = document.to_output_result();
    output.meta.document_entry_gap = Some(4);
    let settings = RenderSettings::test_plain(OutputFormat::Auto);
    let rendered = super::render(
        &output,
        &crate::ui::RenderRequest {
            layout: Some(HelpLayout::Full),
            ..crate::ui::RenderRequest::new(&settings)
        },
    );
    assert!(rendered.contains("\nlist    List history entries\n"));
}

#[test]
fn ui2_help_layout_lowering_uses_plain_sections_and_inline_usage_suffix_unit() {
    let guide = Document::from_help_text(
        "Usage: osp history <COMMAND>\n\nCommands:\n  list   List history entries\n",
    );
    let settings = RenderSettings::test_plain(OutputFormat::Auto);
    let output = guide.to_output_result();
    let mut plan = plan_format(&output, &settings, RenderTarget::OneShotScreen);
    let projection = super::projection::project(&output, plan.format, &plan.settings);
    super::plan::plan_layout(&projection, &mut plan);

    let compact =
        super::lower::lower_document_help_layout(&guide, &plan, HelpLayout::Compact, false);

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
    let guide = Document::from_help_text(
        "Usage: osp history <COMMAND>\n\nCommands:\n  list   List history entries\n",
    );
    let settings = RenderSettings::test_plain(OutputFormat::Auto);
    let output = guide.to_output_result();
    let mut plan = plan_format(&output, &settings, RenderTarget::OneShotScreen);
    let projection = super::projection::project(&output, plan.format, &plan.settings);
    super::plan::plan_layout(&projection, &mut plan);

    let full = super::lower::lower_document_help_layout(&guide, &plan, HelpLayout::Full, true);

    let Some(Block::Rule) = full.blocks.last() else {
        panic!("expected lowered footer rule block");
    };
}

#[test]
fn ui2_lowering_carries_help_entry_defaults_in_ir_unit() {
    let guide = Document::from_help_text("Commands:\n  list   List history entries\n");
    let settings = RenderSettings::test_plain(OutputFormat::Auto);
    let output = guide.to_output_result();
    let mut plan = plan_format(&output, &settings, RenderTarget::OneShotScreen);
    let projection = super::projection::project(&output, plan.format, &plan.settings);
    super::plan::plan_layout(&projection, &mut plan);
    let doc = super::lower::lower_document_help_layout(&guide, &plan, HelpLayout::Compact, false);

    let Block::Section(block) = &doc.blocks[0] else {
        panic!("expected help section block");
    };
    let Block::Entries(entries) = &block.blocks[0] else {
        panic!("expected guide entries block");
    };

    assert_eq!(entries.default_indent, "");
    assert_eq!(entries.default_gap, None);
}

#[test]
fn ui2_lowering_stamps_direct_guide_render_behavior_into_ir_unit() {
    let guide = Document {
        blocks: vec![DocBlock::section(
            "Notes",
            vec![DocBlock::Paragraph {
                text: ("Use `osp history list` for recent commands.").into(),
            }],
        )],
    };
    let output = guide.to_output_result();

    let mut guide_plan = plan_format(
        &output,
        &RenderSettings::test_plain(OutputFormat::Auto),
        RenderTarget::OneShotScreen,
    );
    let projection = super::projection::project(&output, guide_plan.format, &guide_plan.settings);
    super::plan::plan_layout(&projection, &mut guide_plan);
    let guide_doc = super::lower::lower_output(&projection, &guide_plan);
    let Block::Section(guide_section) = &guide_doc.blocks[0] else {
        panic!("expected direct guide section");
    };
    let Block::Paragraph(guide_paragraph) = &guide_section.blocks[0] else {
        panic!("expected guide paragraph");
    };

    assert_eq!(guide_section.body_indent, 0);
    assert!(guide_section.trailing_newline);
    assert!(guide_paragraph.inline_markup);
    assert_eq!(guide_paragraph.indent, 0);

    let mut markdown_settings = RenderSettings::test_plain(OutputFormat::Markdown);
    markdown_settings.format_explicit = true;
    let mut markdown_plan = plan_format(&output, &markdown_settings, RenderTarget::OneShotScreen);
    let projection =
        super::projection::project(&output, markdown_plan.format, &markdown_plan.settings);
    super::plan::plan_layout(&projection, &mut markdown_plan);
    let markdown_doc = super::lower::lower_output(&projection, &markdown_plan);
    let Block::Section(markdown_section) = &markdown_doc.blocks[0] else {
        panic!("expected markdown guide section");
    };

    assert_eq!(markdown_section.body_indent, 0);
    assert!(!markdown_section.trailing_newline);
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
            title_field: None,
            display_notes: Vec::new(),
            context: Default::default(),
            key_index: vec!["team".to_string(), "count".to_string(), "uid".to_string()],
            unix_timestamp_columns: Vec::new(),
            display_rules: Vec::new(),
            display_columns: None,
            display_column_labels: None,
            column_align: Vec::new(),
            wants_copy: false,
            wants_page: false,
            wants_editor: false,
            grouped: true,
            document_rows: false,
            document_row_roles: Vec::new(),
            document_entry_gap: None,
            document_literal_commands: Vec::new(),
            document_command_prefixes: false,
            document_long_help: false,
            render_recommendation: None,
            display_limit: None,
        },
    };

    let mut table_plan = plan_format(
        &output,
        &RenderSettings::test_plain(OutputFormat::Table),
        RenderTarget::OneShotScreen,
    );
    let projection = super::projection::project(&output, table_plan.format, &table_plan.settings);
    super::plan::plan_layout(&projection, &mut table_plan);
    let table_doc = super::lower::lower_output(&projection, &table_plan);
    let Block::Table(first_group) = &table_doc.blocks[0] else {
        panic!("expected first grouped table");
    };
    assert_eq!(first_group.summary[0].key, "team");
    assert_eq!(first_group.summary[1].key, "count");

    let mut json_settings = RenderSettings::test_plain(OutputFormat::Json);
    json_settings.format_explicit = true;
    let rendered_json = render(&output, &crate::ui::RenderRequest::new(&json_settings));
    assert!(rendered_json.contains("\"groups\""));
    assert!(rendered_json.contains("\"aggregates\""));
    assert!(rendered_json.contains("\"rows\""));
}

#[test]
fn ui_help_layout_lowers_mixed_structured_section_data_to_local_block_kinds_unit() {
    let guide = Document {
        blocks: vec![
            DocBlock::section(
                "Session",
                vec![DocBlock::Data {
                    value: json!({
                        "profile": "prod",
                        "theme": "rose-pine-moon"
                    }),
                }],
            ),
            DocBlock::section(
                "Examples",
                vec![DocBlock::Data {
                    value: json!([
                        "osp history list",
                        "osp history clear",
                        "osp history last",
                        "osp history search",
                        "osp history export",
                        "osp history import"
                    ]),
                }],
            ),
            DocBlock::section(
                "Shortcuts",
                vec![DocBlock::Data {
                    value: json!([
                        {"name": "list", "description": "List history"},
                        {"name": "clear", "description": "Clear history"}
                    ]),
                }],
            ),
            DocBlock::section(
                "Matrix",
                vec![DocBlock::Data {
                    value: json!([
                        {"uid": "alice", "state": "ok"},
                        {"uid": "bob", "state": "warn"}
                    ]),
                }],
            ),
        ],
    };
    let mut settings = RenderSettings::test_plain(OutputFormat::Auto);
    settings.width = Some(40);
    settings.runtime.stdout_is_tty = true;
    let output = guide.to_output_result();
    let mut plan = plan_format(&output, &settings, RenderTarget::OneShotScreen);
    let projection = super::projection::project(&output, plan.format, &plan.settings);
    super::plan::plan_layout(&projection, &mut plan);
    let doc = super::lower::lower_document_help_layout(&guide, &plan, HelpLayout::Full, false);

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
    assert!(matches!(shortcuts.blocks[0], Block::Table(_)));
    let Block::Section(matrix) = &doc.blocks[6] else {
        panic!("expected matrix section");
    };
    assert!(matches!(matrix.blocks[0], Block::Table(_)));

    // Verify the lowered grid reaches the terminal with every copyable example
    // intact, rather than checking only the intermediate block types.
    let rendered = super::render(
        &guide.to_output_result(),
        &crate::ui::RenderRequest {
            layout: Some(HelpLayout::Full),
            ..crate::ui::RenderRequest::new(&settings)
        },
    );
    for example in ["list", "clear", "last", "search", "export", "import"] {
        assert!(
            rendered.contains(&format!("osp history {example}")),
            "{rendered}"
        );
    }
    assert!(rendered.contains("alice") && rendered.contains("bob"));
    assert!(
        rendered
            .lines()
            .all(|line| unicode_width::UnicodeWidthStr::width(line) <= 40)
    );
}

mod integration;

#[path = "../../tests/support/output.rs"]
mod output_support;
#[test]
fn contextual_messages_preserve_event_order_and_theme_controls() {
    use crate::core::{MessageLevel, TextRole, TextSpan};
    use crate::ui::messages::MessageBuffer;
    let mut messages = MessageBuffer::new();
    messages.warning("Task still running.");
    messages.push_titled(
        MessageLevel::Error,
        "Host lookup failed",
        "No matching host found.",
    );
    let mut settings = RenderSettings {
        theme_name: "dracula".into(),
        color: ColorMode::Always,
        unicode: UnicodeMode::Always,
        width: Some(72),
        chrome_frame: crate::ui::SectionFrameStyle::TopBottom,
        ruled_section_policy: crate::ui::RuledSectionPolicy::PerSection,
        ..Default::default()
    };
    settings.runtime.stdout_is_tty = true;
    let rendered = crate::ui::render_messages(
        &messages,
        &crate::ui::MessageRequest::new(&settings, MessageLevel::Warning),
    );
    let plain = crate::ui::tests::output_support::strip_ansi(&rendered);
    assert!(plain.find("Warning").unwrap() < plain.find("Host lookup failed").unwrap());
    assert!(!plain.contains("Errors") && !plain.contains("Warnings"));
    assert!(rendered.contains("\x1b[38;2;241;250;140m"));
    assert!(rendered.contains("\x1b[1;38;2;255;85;85m"));
    assert!(rendered.contains("\x1b[38;2;248;248;242mNo matching host found."));
    let document = Document {
        blocks: vec![DocBlock::Preformatted {
            lines: vec![vec![
                TextSpan::new("label ", TextRole::Key),
                TextSpan::new("activity ", TextRole::Info),
                TextSpan::new("complete ", TextRole::Success),
                TextSpan::new("attention ", TextRole::Warning),
                TextSpan::new("failed ", TextRole::Error),
                TextSpan::new("pending ", TextRole::Muted),
                TextSpan::new("message", TextRole::Text),
            ]],
        }],
    }
    .to_output_result();
    settings.width = Some(120);
    let rendered = render(&document, &crate::ui::RenderRequest::new(&settings));
    for styled in [
        "\x1b[38;2;189;147;249mlabel ",
        "\x1b[38;2;139;233;253mactivity ",
        "\x1b[38;2;80;250;123mcomplete ",
        "\x1b[38;2;241;250;140mattention ",
        "\x1b[1;38;2;255;85;85mfailed ",
        "\x1b[38;2;104;121;173mpending ",
        "\x1b[38;2;248;248;242mmessage",
    ] {
        assert!(rendered.contains(styled), "{rendered:?}");
    }
    settings.color = ColorMode::Never;
    settings.unicode = UnicodeMode::Never;
    let uncolored = crate::ui::render_messages(
        &messages,
        &crate::ui::MessageRequest::new(&settings, MessageLevel::Warning),
    );
    assert!(uncolored.contains("Host lookup failed") && !uncolored.contains('\x1b'));
    assert_eq!(
        render(&document, &crate::ui::RenderRequest::new(&settings)),
        "label activity complete attention failed pending message\n"
    );
}

#[path = "tests/guide.rs"]
mod guide;

#[test]
fn human_display_metadata_preserves_typed_exports() {
    use crate::core::{DisplayPart, DisplayRule};
    let raw = serde_json::json!({
        "provider":"cloud:dev", "action":"set_expiry", "duration":1.969, "owners":["ops","web"],
        "compute":{"cpu":2,"memory_mib":8192}, "os":"Debian 13",
        "sources":["cloud"],"conflicts":[{"source":"inventory","value":"Debian 12"}],
        "addresses":[{"address":"192.0.2.1","family":4}],
        "address_conflicts":[{"source":"inventory","value":[{"address":"192.0.2.2","family":4}]}]
    });
    let mut output = crate::core::OutputResult::from_rows(vec![raw.as_object().unwrap().clone()]);
    output.meta.display_columns = Some(
        [
            "provider",
            "action",
            "owners",
            "compute",
            "os",
            "addresses",
            "duration",
        ]
        .map(str::to_owned)
        .into(),
    );
    output.meta.display_rules = vec![
        DisplayRule::DurationSeconds {
            field: "duration".into(),
        },
        DisplayRule::StripQualifier {
            field: "provider".into(),
            separator: ":".into(),
            qualifier: "dev".into(),
        },
        DisplayRule::Replace {
            field: "action".into(),
            from: "_".into(),
            to: "-".into(),
        },
        DisplayRule::Join {
            field: "owners".into(),
            separator: " / ".into(),
        },
        DisplayRule::Divide {
            field: "compute.memory_mib".into(),
            divisor: 1024,
        },
        DisplayRule::Compose {
            field: "compute".into(),
            parts: vec![
                DisplayPart::Field {
                    field: "cpu".into(),
                },
                DisplayPart::Literal {
                    text: " CPU / ".into(),
                },
                DisplayPart::Field {
                    field: "memory_mib".into(),
                },
                DisplayPart::Literal {
                    text: " GiB".into(),
                },
            ],
        },
        DisplayRule::SourceValues {
            field: "os".into(),
            sources: "sources".into(),
            conflicts: "conflicts".into(),
            default_source: "Authority".into(),
            labels: Default::default(),
        },
        DisplayRule::SourceValues {
            field: "addresses".into(),
            sources: "absent".into(),
            conflicts: "address_conflicts".into(),
            default_source: "Authority".into(),
            labels: Default::default(),
        },
        DisplayRule::Labels {
            field: "addresses".into(),
            labels: [
                ("address".into(), "Address".into()),
                ("family".into(), "Family".into()),
            ]
            .into(),
        },
    ];
    output.meta.display_rules.push(DisplayRule::Compose {
        field: "action".into(),
        parts: vec![
            DisplayPart::Literal {
                text: "Action: ".into(),
            },
            DisplayPart::Field {
                field: String::new(),
            },
        ],
    });
    let settings = crate::ui::RenderSettings::test_plain(crate::core::OutputFormat::Mreg);
    let human = crate::ui::render(&output, &crate::ui::RenderRequest::new(&settings));
    for expected in [
        "cloud",
        "Action: set-expiry",
        "2s",
        "ops / web",
        "2 CPU / 8 GiB",
        "Debian 13 (cloud) / Debian 12 (inventory)",
        "Address",
        "Family",
        "Authority",
        "192.0.2.2",
    ] {
        assert!(human.contains(expected), "{expected}: {human}");
    }
    assert_eq!(
        output.as_rows().unwrap()[0],
        raw.as_object().unwrap().clone()
    );
    let settings = crate::ui::RenderSettings::test_plain(crate::core::OutputFormat::Json);
    let json = crate::ui::render(&output, &crate::ui::RenderRequest::new(&settings));
    let exported: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(exported, serde_json::json!([raw]));
    output.meta.display_columns = None;
    output.meta.display_rules = vec![DisplayRule::Compose {
        field: String::new(),
        parts: vec![
            DisplayPart::Literal {
                text: "Provider ".into(),
            },
            DisplayPart::Field {
                field: "provider".into(),
            },
        ],
    }];
    let settings = crate::ui::RenderSettings::test_plain(crate::core::OutputFormat::Value);
    assert_eq!(
        crate::ui::render(&output, &crate::ui::RenderRequest::new(&settings)),
        "Provider cloud:dev\n"
    );
    assert_eq!(
        output.as_rows().unwrap()[0],
        raw.as_object().unwrap().clone()
    );
}
