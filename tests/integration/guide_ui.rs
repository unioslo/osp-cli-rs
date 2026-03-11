use osp_cli::core::output::OutputFormat;
use osp_cli::dsl::{apply_output_pipeline, parse_pipeline};
use osp_cli::guide::GuideView;
use osp_cli::ui::{RenderSettings, render_output};

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
