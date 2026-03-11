use super::*;

#[test]
fn intro_command_dispatches_as_repl_scoped_builtin_unit() {
    let mut cli = Cli::try_parse_from(["osp", "intro"]).expect("cli parse");
    let plan = build_dispatch_plan(&mut cli, &profiles(&["default"])).expect("dispatch plan");

    assert!(matches!(plan.action, RunAction::Intro(_)));
    assert_eq!(plan.action.terminal_kind(), TerminalKind::Repl);
}

#[test]
fn intro_command_emits_semantic_json_with_explicit_format_unit() {
    let mut sink = BufferedUiSink::default();
    let code = crate::app::App::new()
        .run_with_sink(
            ["osp", "--no-env", "--no-config-file", "--json", "intro"],
            &mut sink,
        )
        .expect("intro command should succeed");

    assert_eq!(code, 0);
    let json: serde_json::Value =
        serde_json::from_str(&sink.stdout).expect("intro JSON should parse");
    let rows = json.as_array().expect("guide output should be row array");
    assert_eq!(rows.len(), 1);
    assert!(
        rows[0].get("sections").is_some() || rows[0].get("preamble").is_some(),
        "expected semantic intro payload, got: {}",
        sink.stdout
    );
}

#[test]
fn top_level_help_supports_all_explicit_output_formats_unit() {
    let json = run_app_stdout(&["osp", "--no-env", "--no-config-file", "--json", "--help"]);
    let json_value: serde_json::Value =
        serde_json::from_str(&json).expect("help json should parse");
    let json_rows = json_value
        .as_array()
        .expect("help json should be row array");
    assert_eq!(json_rows.len(), 1);
    assert!(json_rows[0].get("usage").is_some());

    let guide = run_app_stdout(&["osp", "--no-env", "--no-config-file", "--guide", "--help"]);
    assert!(guide.contains("Usage"));
    assert!(guide.contains("Commands"));

    let markdown = run_app_stdout(&["osp", "--no-env", "--no-config-file", "--md", "--help"]);
    assert!(markdown.contains("## Usage"));
    assert!(markdown.contains("## Commands"));
    assert!(markdown.contains("- `help` "));
    assert!(!markdown.contains("| name"));

    let table = run_app_stdout(&["osp", "--no-env", "--no-config-file", "--table", "--help"]);
    assert!(table.contains("preamble"));
    assert!(table.contains("usage"));
    assert!(table.contains("commands"));

    let mreg = run_app_stdout(&["osp", "--no-env", "--no-config-file", "--mreg", "--help"]);
    assert!(mreg.contains("preamble:"));
    assert!(mreg.contains("commands ("));

    let value = run_app_stdout(&["osp", "--no-env", "--no-config-file", "--value", "--help"]);
    assert!(value.contains("OSP CLI"));
    assert!(value.contains("osp [OPTIONS] [COMMAND]"));
    assert!(value.contains("Inspect and mutate CLI configuration"));
    assert!(!value.contains("Usage"));
    assert!(!value.contains("Commands"));
}

#[test]
fn intro_command_supports_all_explicit_output_formats_unit() {
    let json = run_app_stdout(&["osp", "--no-env", "--no-config-file", "--json", "intro"]);
    let json_value: serde_json::Value =
        serde_json::from_str(&json).expect("intro json should parse");
    let json_rows = json_value
        .as_array()
        .expect("intro json should be row array");
    assert_eq!(json_rows.len(), 1);
    assert!(json_rows[0].get("sections").is_some());

    let guide = run_app_stdout(&["osp", "--no-env", "--no-config-file", "--guide", "intro"]);
    assert!(guide.contains("OSP"));
    assert!(guide.contains("Commands"));
    let osp = guide.find("OSP").expect("OSP section should render");
    let keybindings = guide
        .find("Keybindings")
        .expect("Keybindings section should render");
    let pipes = guide.find("Pipes").expect("Pipes section should render");
    let usage = guide.find("Usage").expect("Usage section should render");
    let commands = guide
        .find("Commands")
        .expect("Commands section should render");
    assert!(osp < keybindings);
    assert!(keybindings < pipes);
    assert!(pipes < usage);
    assert!(usage < commands);

    let markdown = run_app_stdout(&["osp", "--no-env", "--no-config-file", "--md", "intro"]);
    assert!(markdown.contains("## OSP"));
    assert!(markdown.contains("## Commands"));
    assert!(markdown.contains("- `help` Show this command overview."));
    assert!(!markdown.contains("| name"));

    let table = run_app_stdout(&["osp", "--no-env", "--no-config-file", "--table", "intro"]);
    assert!(table.contains("sections"));

    let mreg = run_app_stdout(&["osp", "--no-env", "--no-config-file", "--mreg", "intro"]);
    assert!(mreg.contains("sections ("));
    assert!(mreg.contains("title:"));

    let value = run_app_stdout(&["osp", "--no-env", "--no-config-file", "--value", "intro"]);
    assert!(value.contains("Welcome"));
    assert!(value.contains("Show this command overview."));
    assert!(value.contains("Inspect and edit runtime config"));
}

#[test]
fn staged_semantic_quick_search_preserves_guide_shape_by_default_unit() {
    let guide = GuideView::from_text(
        "Usage: osp [COMMAND]\n\nCommands:\n  help  Show this command overview.\n  theme  Inspect and apply themes\n  config  Inspect and edit runtime config\n",
    );

    let (output, format_hint) =
        apply_output_stages(guide.to_output_result(), &["inspect".to_string()], None)
            .expect("staged guide output should succeed");

    assert!(format_hint.is_none());
    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should still restore");
    assert!(
        rebuilt.usage.is_empty(),
        "quick should prune unmatched root siblings like usage: {rebuilt:?}"
    );
    assert_eq!(rebuilt.commands.len(), 2);

    let rendered = render_output(&output, &RenderSettings::test_plain(OutputFormat::Guide));
    assert!(rendered.contains("Commands"));
    assert!(!rendered.contains("Usage"));
    assert!(!rendered.contains("Sections"));
    assert!(!rendered.contains("Entries"));
}
