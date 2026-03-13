use super::{GuideEntry, GuideSection, GuideSectionKind, GuideView};
use crate::core::command_def::{ArgDef, CommandDef, FlagDef};
use crate::core::output_model::{OutputDocument, OutputDocumentKind, OutputItems, OutputResult};
use crate::ui::presentation::HelpLevel;
use serde_json::Value;
use serde_json::json;

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
fn guide_view_from_command_def_builds_usage_arguments_and_options_unit() {
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
fn guide_output_restore_round_trips_canonical_and_authored_ordered_shapes_unit() {
    let cases = vec![
        (
            GuideView::from_text("Usage: osp history <COMMAND>\n\nCommands:\n  help  Print help\n"),
            vec![],
            Some(vec!["osp history <COMMAND>".to_string()]),
            Some(vec!["help".to_string()]),
            false,
        ),
        (
            GuideView {
                sections: vec![
                    GuideSection::new("OSP", GuideSectionKind::Custom).paragraph("Welcome"),
                    GuideSection::new("Usage", GuideSectionKind::Usage)
                        .paragraph("[INVOCATION_OPTIONS] COMMAND [ARGS]..."),
                    GuideSection::new("Commands", GuideSectionKind::Commands)
                        .entry("doctor", "Run diagnostics checks"),
                ],
                ..GuideView::default()
            },
            vec![
                "OSP".to_string(),
                "Usage".to_string(),
                "Commands".to_string(),
            ],
            Some(vec!["[INVOCATION_OPTIONS] COMMAND [ARGS]...".to_string()]),
            Some(vec!["doctor".to_string()]),
            true,
        ),
        (
            GuideView {
                sections: vec![
                    GuideSection::new("Usage", GuideSectionKind::Usage)
                        .paragraph("[INVOCATION_OPTIONS] COMMAND [ARGS]..."),
                    GuideSection::new("Commands", GuideSectionKind::Commands)
                        .entry("help", "Show this command overview."),
                ],
                ..GuideView::default()
            },
            vec![],
            Some(vec!["[INVOCATION_OPTIONS] COMMAND [ARGS]...".to_string()]),
            Some(vec!["help".to_string()]),
            false,
        ),
    ];

    for (view, expected_sections, expected_usage, expected_commands, ordered_sections) in cases {
        let output = view.to_output_result();
        assert!(matches!(
            output.document,
            Some(OutputDocument {
                kind: OutputDocumentKind::Guide,
                value: Value::Object(_),
            })
        ));

        let rebuilt = GuideView::try_from_output_result(&output).expect("guide output");
        assert_eq!(
            rebuilt
                .sections
                .iter()
                .map(|section| section.title.clone())
                .collect::<Vec<_>>(),
            expected_sections
        );
        if let Some(expected_usage) = expected_usage {
            assert_eq!(rebuilt.usage, expected_usage);
        }
        if let Some(expected_commands) = expected_commands {
            assert_eq!(
                rebuilt
                    .commands
                    .iter()
                    .map(|entry| entry.name.clone())
                    .collect::<Vec<_>>(),
                expected_commands
            );
        }

        let json = rebuilt.to_json_value();
        if ordered_sections {
            assert!(json.get("usage").is_none());
            assert!(json.get("commands").is_none());
            assert_eq!(
                json["sections"]
                    .as_array()
                    .expect("ordered sections array")
                    .iter()
                    .map(|section| section["title"].as_str().unwrap_or_default().to_string())
                    .collect::<Vec<_>>(),
                expected_sections
            );
        } else {
            assert_eq!(json["usage"][0], rebuilt.usage[0]);
            assert_eq!(json["commands"][0]["name"], rebuilt.commands[0].name);
        }
    }
}

#[test]
fn guide_restore_prefers_document_and_accepts_legacy_row_shapes_unit() {
    let invalid_document_output = OutputResult {
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
    assert!(GuideView::try_from_output_result(&invalid_document_output).is_none());

    let legacy_summary_output = OutputResult::from_rows(vec![
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
    let rebuilt = GuideView::try_from_output_result(&legacy_summary_output).expect("guide output");
    assert_eq!(rebuilt.commands[0].name, "list");
    assert_eq!(rebuilt.commands[0].short_help, "Show");
}

#[test]
fn guide_markdown_surfaces_sections_and_bounds_entry_rows_unit() {
    let view = GuideView {
        usage: vec!["history <COMMAND>".to_string()],
        commands: vec![
            GuideEntry {
                name: "list".to_string(),
                short_help: "List history entries".to_string(),
                display_indent: None,
                display_gap: None,
            },
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
        options: vec![GuideEntry {
            name: "-h, --help".to_string(),
            short_help: "Print help".to_string(),
            display_indent: None,
            display_gap: None,
        }],
        ..GuideView::default()
    };

    let default_markdown = view.to_markdown();
    assert!(default_markdown.contains("## Usage"));
    assert!(default_markdown.contains("history <COMMAND>"));
    assert!(default_markdown.contains("## Commands"));
    assert!(default_markdown.contains("- `list` List history entries"));
    assert!(default_markdown.contains("## Options"));
    assert!(default_markdown.contains("- `-h, --help` Print help"));
    assert!(!default_markdown.contains("| name"));

    let bounded_markdown = view.to_markdown_with_width(Some(90));
    let lines = bounded_markdown.lines().collect::<Vec<_>>();
    assert!(
        lines.iter().any(|line| line.contains("- `plugins` ")),
        "expected plugins row in:\n{bounded_markdown}"
    );
    assert!(
        lines.iter().any(|line| line.contains("- `options` ")),
        "expected options row in:\n{bounded_markdown}"
    );
}

#[test]
fn guide_value_lines_use_semantic_content_and_preserve_object_value_order_unit() {
    let cases = vec![
        (
            GuideView {
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
            },
            vec!["Inspect and edit runtime config".to_string()],
        ),
        (
            GuideView {
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
            },
            vec![
                "oistes".to_string(),
                "rose-pine-moon".to_string(),
                "1.4.9".to_string(),
            ],
        ),
    ];

    for (view, expected) in cases {
        assert_eq!(view.to_value_lines(), expected);
    }
}
