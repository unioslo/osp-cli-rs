use osp_cli::core::command_def::{
    ArgDef, CommandDef, CommandPolicyDef, FlagDef, ValueChoice, ValueKind,
};
use osp_cli::core::command_policy::VisibilityMode;
use osp_cli::core::output::OutputFormat;
use osp_cli::dsl::{apply_output_pipeline, parse_pipeline};
use osp_cli::guide::{GuideSection, GuideSectionKind, GuideView};
use osp_cli::ui::{RenderSettings, render_output};
use serde_json::json;

fn run_guide_pipeline(
    view: GuideView,
    pipeline: &str,
) -> osp_cli::core::output_model::OutputResult {
    let parsed = parse_pipeline(&format!("fixture | {pipeline}")).expect("pipeline should parse");
    apply_output_pipeline(view.to_output_result(), &parsed.stages).expect("pipeline should succeed")
}

fn sample_guide() -> GuideView {
    GuideView::from_text(
        "Usage: osp history <COMMAND>\n\nCommands:\n  list   List history entries\n  clear  Clear history entries\n",
    )
}

#[test]
fn guide_payload_narrowing_restores_and_renders_as_markdown_guide() {
    let output = run_guide_pipeline(sample_guide(), "list | ? | L 1");
    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should still restore");
    assert_eq!(rebuilt.commands.len(), 1);
    assert_eq!(rebuilt.commands[0].name, "list");

    let mut settings = RenderSettings::test_plain(OutputFormat::Markdown);
    settings.width = Some(80);
    let markdown = render_output(&output, &settings);

    assert!(markdown.contains("list"));
    assert!(markdown.contains("List history entries"));
    assert!(!markdown.contains("clear"));
    assert!(!markdown.contains("| name"));
}

#[test]
fn guide_payload_value_extraction_degrades_and_renders_as_plain_values() {
    let output = run_guide_pipeline(
        sample_guide(),
        "P commands[].name | VALUE name | S value | L 2",
    );
    assert!(GuideView::try_from_output_result(&output).is_none());

    let rendered = render_output(&output, &RenderSettings::test_plain(OutputFormat::Value));
    assert!(rendered.contains("clear"));
    assert!(rendered.contains("list"));
    assert!(!rendered.contains("Usage"));
    assert!(!rendered.contains("Commands"));
}

#[test]
fn command_metadata_builders_flow_into_generated_guide_and_rendering_contracts() {
    assert!(CommandPolicyDef::default().is_empty());

    let profile_choice = ValueChoice::new("prod")
        .help("Production profile")
        .display("Production")
        .sort("010");
    let stage_choice = ValueChoice::new("stage")
        .help("Staging profile")
        .display("Staging")
        .sort("020");
    let format_choice = ValueChoice::new("json")
        .help("Machine output")
        .display("JSON")
        .sort("010");
    let text_choice = ValueChoice::new("text")
        .help("Human output")
        .display("Text")
        .sort("020");
    let policy = CommandPolicyDef {
        visibility: VisibilityMode::Authenticated,
        required_capabilities: vec!["deploy.write".to_string()],
        feature_flags: vec!["beta".to_string()],
    };

    let authored = CommandDef::new("deploy")
        .about("Deploy pending changes")
        .long_about("Long deploy help")
        .usage("osp deploy [OPTIONS] PROFILE [id] <COMMAND>")
        .before_help("Before line 1\nBefore line 2")
        .after_help("After line 1\nAfter line 2")
        .alias("ship")
        .aliases(["rollout", "push"])
        .sort("020")
        .policy(policy.clone())
        .arg(
            ArgDef::new("profile")
                .value_name("PROFILE")
                .help("Target profile")
                .required()
                .multi()
                .value_kind(ValueKind::Enum)
                .choices([profile_choice.clone(), stage_choice.clone()])
                .defaults(["default", "prod"]),
        )
        .args([ArgDef::new("id")
            .help("Optional deployment id")
            .value_kind(ValueKind::FreeText)
            .defaults(["latest"])])
        .flag(
            FlagDef::new("format")
                .short('f')
                .long("format")
                .alias("fmt")
                .aliases(["output-format"])
                .help("Select output format")
                .takes_value("FORMAT")
                .required()
                .multi()
                .value_kind(ValueKind::Enum)
                .choices([format_choice.clone(), text_choice.clone()])
                .defaults(["json"]),
        )
        .flags([
            FlagDef::new("config")
                .long("config")
                .help("Path to config")
                .takes_value("PATH")
                .value_kind(ValueKind::Path)
                .defaults(["./osp.toml"]),
            FlagDef::new("quiet")
                .short('q')
                .long("quiet")
                .help("Reduce output")
                .takes_value("IGNORED")
                .takes_no_value()
                .hidden(),
        ])
        .subcommand(CommandDef::new("apply").about("Apply pending changes"))
        .subcommands([
            CommandDef::new("status").about("Show deployment status"),
            CommandDef::new("secret").about("Hidden command").hidden(),
        ]);

    assert_eq!(authored.aliases, vec!["ship", "rollout", "push"]);
    assert_eq!(authored.long_about.as_deref(), Some("Long deploy help"));
    assert_eq!(authored.sort_key.as_deref(), Some("020"));
    assert_eq!(authored.policy, policy);
    assert_eq!(authored.args[0].defaults, vec!["default", "prod"]);
    let format_flag = authored
        .flags
        .iter()
        .find(|flag| flag.id == "format")
        .expect("format flag should exist");
    assert_eq!(format_flag.aliases, vec!["fmt", "output-format"]);
    assert_eq!(format_flag.choices.len(), 2);
    let quiet_flag = authored
        .flags
        .iter()
        .find(|flag| flag.id == "quiet")
        .expect("quiet flag should exist");
    assert_eq!(quiet_flag.value_name, None);
    assert!(!quiet_flag.takes_value);
    assert!(quiet_flag.hidden);
    assert_eq!(profile_choice.display.as_deref(), Some("Production"));
    assert_eq!(text_choice.sort_key.as_deref(), Some("020"));

    let mut generated = authored.clone();
    generated.usage = None;

    let guide = GuideView::from_command_def(&generated);
    assert_eq!(
        guide.preamble,
        vec!["Before line 1".to_string(), "Before line 2".to_string()]
    );
    assert_eq!(
        guide.epilogue,
        vec!["After line 1".to_string(), "After line 2".to_string()]
    );
    assert_eq!(
        guide.usage,
        vec!["deploy [OPTIONS] PROFILE [id] <COMMAND>".to_string()]
    );
    assert_eq!(
        guide
            .commands
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["apply", "status"]
    );
    assert_eq!(
        guide
            .arguments
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["PROFILE", "id"]
    );
    assert!(guide.options.iter().any(|entry| {
        entry.name == "-f, --format, <FORMAT>" && entry.short_help == "Select output format"
    }));
    assert!(
        guide
            .options
            .iter()
            .any(|entry| entry.name == "--config, <PATH>" && entry.short_help == "Path to config")
    );
    assert!(
        !guide
            .options
            .iter()
            .any(|entry| entry.name.contains("quiet"))
    );

    let output = guide.to_output_result();
    let restored = GuideView::try_from_output_result(&output).expect("guide should restore");
    assert_eq!(restored.usage, guide.usage);
    assert_eq!(restored.commands.len(), 2);
    assert_eq!(restored.options.len(), 2);

    let markdown = render_output(&output, &RenderSettings::test_plain(OutputFormat::Markdown));
    assert!(markdown.contains("Usage"));
    assert!(markdown.contains("deploy [OPTIONS] PROFILE [id] <COMMAND>"));
    assert!(markdown.contains("Apply pending changes"));
    assert!(markdown.contains("After line 2"));

    let rendered_values = render_output(&output, &RenderSettings::test_plain(OutputFormat::Value));
    assert!(rendered_values.contains("Before line 1"));
    assert!(rendered_values.contains("Select output format"));
    assert!(rendered_values.contains("After line 2"));
}

#[cfg(feature = "clap")]
#[test]
fn clap_command_metadata_projects_into_command_defs_and_guides() {
    use clap::{Arg, ArgAction, Command, ValueHint, builder::PossibleValue};

    let command = Command::new("plugins")
        .about("Manage plugin providers")
        .long_about("Long plugin management help")
        .before_help("Plugin preamble")
        .after_help("Plugin epilogue")
        .visible_alias("ext")
        .arg(
            Arg::new("roots")
                .long_help(" Roots to scan ")
                .help_heading("Input")
                .num_args(1..)
                .value_name("ROOT")
                .value_hint(ValueHint::DirPath)
                .default_values(["./plugins"])
                .value_parser([
                    PossibleValue::new("builtin").help("Built-in plugins"),
                    PossibleValue::new("custom").help("Custom plugins"),
                    PossibleValue::new("hidden").help("Hidden").hide(true),
                ]),
        )
        .arg(
            Arg::new("format")
                .long("format")
                .visible_alias("fmt")
                .short('f')
                .visible_short_alias('o')
                .help_heading("Output")
                .long_help(" Output format ")
                .action(ArgAction::Append)
                .value_name("FORMAT")
                .default_values(["json"])
                .value_parser([
                    PossibleValue::new("json").help("Machine readable"),
                    PossibleValue::new("text").help("Human readable"),
                    PossibleValue::new("trace").help("Hidden").hide(true),
                ]),
        )
        .arg(Arg::new("quiet").long("quiet").hide(true))
        .subcommand(Command::new("list").about("List plugins"))
        .subcommand(Command::new("doctor").about("Hidden doctor").hide(true));

    let def = CommandDef::from_clap(command);
    assert_eq!(def.aliases, vec!["ext".to_string()]);
    assert_eq!(def.before_help.as_deref(), Some("Plugin preamble"));
    assert_eq!(def.after_help.as_deref(), Some("Plugin epilogue"));
    assert_eq!(
        def.long_about.as_deref(),
        Some("Long plugin management help")
    );
    assert_eq!(def.args.len(), 1);
    assert_eq!(def.args[0].help_heading.as_deref(), Some("Input"));
    assert_eq!(def.args[0].value_name.as_deref(), Some("ROOT"));
    assert!(def.args[0].multi);
    assert_eq!(def.args[0].value_kind, Some(ValueKind::Path));
    assert_eq!(def.args[0].choices.len(), 2);
    assert_eq!(def.args[0].defaults, vec!["./plugins"]);
    assert_eq!(def.flags.len(), 1);
    assert_eq!(def.flags[0].help_heading.as_deref(), Some("Output"));
    assert!(def.flags[0].multi);
    assert_eq!(def.flags[0].defaults, vec!["json"]);
    assert!(def.flags[0].aliases.contains(&"--fmt".to_string()));
    assert!(def.flags[0].aliases.contains(&"-o".to_string()));
    assert_eq!(def.flags[0].choices.len(), 2);
    assert_eq!(def.subcommands.len(), 1);
    assert_eq!(def.subcommands[0].name, "list");

    let guide = GuideView::from_command_def(&def);
    assert_eq!(guide.preamble, vec!["Plugin preamble".to_string()]);
    assert_eq!(guide.epilogue, vec!["Plugin epilogue".to_string()]);
    assert_eq!(guide.commands[0].name, "list");
    assert!(
        guide
            .options
            .iter()
            .any(|entry| entry.name.contains("--format"))
    );
    assert!(guide.arguments.iter().any(|entry| entry.name == "ROOT"));

    let output = guide.to_output_result();
    let markdown = render_output(&output, &RenderSettings::test_plain(OutputFormat::Markdown));
    assert!(markdown.contains("List plugins"));
    assert!(markdown.contains("Plugin epilogue"));
}

#[test]
fn parsed_and_authored_guides_merge_round_trip_and_render_through_semantic_output() {
    let mut parsed = GuideView::from_text(
        "Deploy overview\n\nUsage: osp deploy <COMMAND>\n\nCommands:\n  status  Show deployment status\n  apply   Apply pending changes\n\nOptions:\n  --json  Render machine output\n  --wait  Wait for completion\nHint: use doctor before prod.\n\nSession:\ncurrent profile: prod\n",
    );
    let authored = GuideView {
        sections: vec![
            GuideSection::new("Notes", GuideSectionKind::Notes)
                .paragraph("Run doctor before deploy"),
            GuideSection::new("Runtime", GuideSectionKind::Custom).data(json!({
                "profile": "prod",
                "theme": "rose-pine-moon",
                "count": 2,
            })),
        ],
        ..GuideView::default()
    };

    parsed.merge(authored);
    let output = parsed.to_output_result();
    let restored = GuideView::try_from_output_result(&output).expect("guide should restore");

    assert_eq!(restored.preamble, vec!["Deploy overview".to_string()]);
    assert_eq!(restored.usage, vec!["osp deploy <COMMAND>".to_string()]);
    assert_eq!(
        restored
            .commands
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["status", "apply"]
    );
    assert_eq!(
        restored
            .options
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["--json", "--wait"]
    );
    assert_eq!(
        restored.epilogue,
        vec!["Hint: use doctor before prod.".to_string()]
    );
    assert_eq!(restored.notes, vec!["Run doctor before deploy".to_string()]);
    assert_eq!(
        restored
            .sections
            .iter()
            .map(|section| section.title.as_str())
            .collect::<Vec<_>>(),
        vec!["Session", "Notes", "Runtime"]
    );

    let markdown = restored.to_markdown_with_width(Some(80));
    assert!(markdown.contains("## Usage"));
    assert!(markdown.contains("## Session"));
    assert!(markdown.contains("current profile: prod"));

    let value_lines = restored.to_value_lines();
    assert!(value_lines.contains(&"Show deployment status".to_string()));
    assert!(value_lines.contains(&"Run doctor before deploy".to_string()));
    assert!(value_lines.contains(&"prod".to_string()));
    assert!(value_lines.contains(&"rose-pine-moon".to_string()));
    assert!(value_lines.contains(&"2".to_string()));
}
