use super::*;

#[test]
fn intro_command_dispatches_as_repl_scoped_builtin_unit() {
    let mut cli = Cli::try_parse_from(["osp", "intro"]).expect("cli parse");
    let plan = build_dispatch_plan(&mut cli, &profiles(&["default"])).expect("dispatch plan");

    assert!(matches!(plan.action, RunAction::Intro(_)));
    assert_eq!(plan.action.terminal_kind(), TerminalKind::Repl);
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
