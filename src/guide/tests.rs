use super::{GuideEntry, GuideSection, GuideSectionKind, GuideView};
use crate::core::command_def::{ArgDef, CommandDef, FlagDef};
use crate::core::output_model::{OutputDocument, OutputDocumentKind, OutputItems, OutputResult};
use crate::ui::presentation::HelpLevel;
use serde_json::Value;
use serde_json::json;

#[test]
fn guide_view_from_text_preserves_usage_and_command_entries_unit() {
    let view = GuideView::from_text("Usage: osp theme <COMMAND>\n\nCommands:\n  list  Show\n");

    assert_eq!(view.usage, vec!["osp theme <COMMAND>".to_string()]);
    assert_eq!(view.commands[0].name, "list");
    assert_eq!(view.commands[0].short_help, "Show");
}

#[test]
fn filtered_for_help_level_hides_verbose_sections_until_requested_unit() {
    let mut view = GuideView::from_text(
        "Usage: osp [COMMAND]\n\nCommands:\n  help  Show help\n\nCommon Invocation Options:\n  --json  Render as JSON\n",
    );
    view.sections
        .push(GuideSection::new("Notes", GuideSectionKind::Notes).paragraph("extra note"));

    let tiny = view.filtered_for_help_level(HelpLevel::Tiny);
    let normal = view.filtered_for_help_level(HelpLevel::Normal);
    let verbose = view.filtered_for_help_level(HelpLevel::Verbose);

    assert!(!tiny.usage.is_empty());
    assert!(tiny.commands.is_empty());
    assert!(normal.common_invocation_options.is_empty());
    assert!(!normal.commands.is_empty());
    assert!(!normal.sections.is_empty());
    assert!(!verbose.common_invocation_options.is_empty());
}

#[test]
fn guide_view_from_command_def_builds_usage_commands_and_options_unit() {
    let view = GuideView::from_command_def(
        &CommandDef::new("theme")
            .about("Inspect and apply themes")
            .flag(FlagDef::new("raw").long("raw").help("Show raw values"))
            .arg(ArgDef::new("name").value_name("name"))
            .subcommand(CommandDef::new("list").about("List themes")),
    );

    assert_eq!(view.usage.len(), 1);
    assert_eq!(view.commands.len(), 1);
    assert_eq!(view.arguments.len(), 1);
    assert_eq!(view.options.len(), 1);
}

#[test]
fn help_section_builder_collects_blocks_unit() {
    let section = GuideSection::new("Notes", GuideSectionKind::Notes)
        .paragraph("first")
        .entry("show", "Display");

    assert_eq!(section.paragraphs, vec!["first".to_string()]);
    assert_eq!(section.entries.len(), 1);
}

#[test]
fn guide_view_projects_to_single_semantic_row_unit() {
    let view = GuideView::from_text("Commands:\n  list  Show\n");
    let rows = view.to_output_result().into_rows().expect("rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["commands"][0]["name"], "list");
    assert_eq!(rows[0]["commands"][0]["short_help"], "Show");
}

#[test]
fn guide_view_json_value_is_semantic_not_internal_shape_unit() {
    let view = GuideView::from_text("Usage: osp history <COMMAND>\n\nCommands:\n  list\n");
    let value = view.to_json_value();

    assert_eq!(value["usage"][0], "osp history <COMMAND>");
    assert_eq!(value["commands"][0]["name"], "list");
    assert!(value.get("sections").is_none());
}

#[test]
fn guide_view_round_trips_through_output_result_unit() {
    let view =
        GuideView::from_text("Usage: osp history <COMMAND>\n\nCommands:\n  help  Print help\n");
    let output = view.to_output_result();
    let rebuilt = GuideView::try_from_output_result(&output).expect("guide output");

    assert_eq!(rebuilt.usage[0], "osp history <COMMAND>");
    assert_eq!(rebuilt.commands[0].name, "help");
}

#[test]
fn guide_round_trip_preserves_authored_canonical_section_order_unit() {
    let view = GuideView {
        sections: vec![
            GuideSection::new("OSP", GuideSectionKind::Custom).paragraph("Welcome"),
            GuideSection::new("Usage", GuideSectionKind::Usage)
                .paragraph("[INVOCATION_OPTIONS] COMMAND [ARGS]..."),
            GuideSection::new("Commands", GuideSectionKind::Commands)
                .entry("doctor", "Run diagnostics checks"),
        ],
        ..GuideView::default()
    };

    let rebuilt = GuideView::try_from_output_result(&view.to_output_result()).expect("guide");

    assert_eq!(
        rebuilt
            .sections
            .iter()
            .map(|section| section.title.as_str())
            .collect::<Vec<_>>(),
        vec!["OSP", "Usage", "Commands"]
    );
    assert_eq!(
        rebuilt.usage,
        vec!["[INVOCATION_OPTIONS] COMMAND [ARGS]..."]
    );
    assert_eq!(rebuilt.commands[0].name, "doctor");

    let json = rebuilt.to_json_value();
    assert!(json.get("usage").is_none());
    assert!(json.get("commands").is_none());
    assert_eq!(json["sections"][1]["title"], "Usage");
    assert_eq!(json["sections"][2]["title"], "Commands");
}

#[test]
fn guide_view_output_result_carries_document_sidecar_unit() {
    let view =
        GuideView::from_text("Usage: osp history <COMMAND>\n\nCommands:\n  help  Print help\n");
    let output = view.to_output_result();

    assert!(matches!(
        output.document,
        Some(OutputDocument {
            kind: OutputDocumentKind::Guide,
            value: Value::Object(_),
        })
    ));
}

#[test]
fn guide_restore_does_not_guess_from_rows_when_document_is_present_unit() {
    let output = OutputResult {
        items: OutputItems::Rows(vec![
            json!({"commands": [{"name": "list"}]})
                .as_object()
                .cloned()
                .expect("object"),
        ]),
        document: Some(OutputDocument::new(
            OutputDocumentKind::Guide,
            json!([{"commands": [{"name": "list"}]}]),
        )),
        meta: Default::default(),
    };

    assert!(GuideView::try_from_output_result(&output).is_none());
}

#[test]
fn guide_view_accepts_legacy_summary_field_when_rehydrating_unit() {
    let output = OutputResult::from_rows(vec![
        json!({
            "commands": [
                {
                    "name": "list",
                    "summary": "Show"
                }
            ]
        })
        .as_object()
        .cloned()
        .expect("object"),
    ]);

    let rebuilt = GuideView::try_from_output_result(&output).expect("guide output");
    assert_eq!(rebuilt.commands[0].name, "list");
    assert_eq!(rebuilt.commands[0].short_help, "Show");
}

#[test]
fn guide_view_markdown_uses_headings_and_entry_tables_unit() {
    let view = GuideView {
        usage: vec!["history <COMMAND>".to_string()],
        commands: vec![GuideEntry {
            name: "list".to_string(),
            short_help: "List history entries".to_string(),
            display_indent: None,
            display_gap: None,
        }],
        options: vec![GuideEntry {
            name: "-h, --help".to_string(),
            short_help: "Print help".to_string(),
            display_indent: None,
            display_gap: None,
        }],
        ..GuideView::default()
    };

    let rendered = view.to_markdown();
    assert!(rendered.contains("## Usage"));
    assert!(rendered.contains("history <COMMAND>"));
    assert!(rendered.contains("## Commands"));
    assert!(rendered.contains("- `list` List history entries"));
    assert!(rendered.contains("## Options"));
    assert!(rendered.contains("- `-h, --help` Print help"));
    assert!(!rendered.contains("| name"));
}

#[test]
fn guide_view_markdown_bounds_padding_to_fit_width_unit() {
    let view = GuideView {
        commands: vec![
            GuideEntry {
                name: "plugins".to_string(),
                short_help: "Inspect and manage plugin providers".to_string(),
                display_indent: None,
                display_gap: None,
            },
            GuideEntry {
                name: "options".to_string(),
                short_help: "per invocation: --format/--json/--table/--value/--md, --mode, --color, --unicode/--ascii, -v/-q/-d, --cache, --plugin-provider".to_string(),
                display_indent: None,
                display_gap: None,
            },
        ],
        ..GuideView::default()
    };

    let rendered = view.to_markdown_with_width(Some(90));
    let lines = rendered.lines().collect::<Vec<_>>();
    assert!(
        lines.iter().any(|line| line.contains("- `plugins` ")),
        "expected bullet entry row in:\n{rendered}"
    );
    assert!(
        lines.iter().any(|line| line.contains("- `plugins` ")),
        "expected plugins row in:\n{rendered}"
    );
    assert!(
        lines.iter().any(|line| line.contains("- `options` ")),
        "expected options row in:\n{rendered}"
    );
}

#[test]
fn guide_value_lines_prefer_content_over_structure_labels_unit() {
    let view = GuideView {
        sections: vec![GuideSection {
            title: "Commands".to_string(),
            kind: GuideSectionKind::Commands,
            paragraphs: Vec::new(),
            entries: vec![GuideEntry {
                name: "config".to_string(),
                short_help: "Inspect and edit runtime config".to_string(),
                display_indent: None,
                display_gap: None,
            }],
            data: None,
        }],
        ..GuideView::default()
    };

    assert_eq!(
        view.to_value_lines(),
        vec!["Inspect and edit runtime config".to_string()]
    );
}

#[test]
fn guide_value_lines_preserve_json_object_value_order_unit() {
    let view = GuideView {
        sections: vec![GuideSection {
            title: "Session".to_string(),
            kind: GuideSectionKind::Custom,
            paragraphs: Vec::new(),
            entries: Vec::new(),
            data: Some(json!({
                "logged_in_as": "oistes",
                "theme": "rose-pine-moon",
                "version": "1.4.9"
            })),
        }],
        ..GuideView::default()
    };

    assert_eq!(
        view.to_value_lines(),
        vec![
            "oistes".to_string(),
            "rose-pine-moon".to_string(),
            "1.4.9".to_string()
        ]
    );
}
