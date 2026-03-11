use osp_cli::core::output_model::{OutputItems, OutputResult};
use osp_cli::core::row::Row;
use osp_cli::dsl::{apply_output_pipeline, parse_pipeline};
use osp_cli::guide::{GuideEntry, GuideSection, GuideSectionKind, GuideView};
use serde_json::Value;

mod addressed;
mod rows;
mod semantics;
mod values;

fn row(value: Value) -> Row {
    value
        .as_object()
        .cloned()
        .expect("fixture should be an object")
}

fn run_rows_pipeline(rows: Vec<Row>, pipeline: &str) -> OutputResult {
    let parsed = parse_pipeline(&format!("fixture | {pipeline}")).expect("pipeline should parse");
    apply_output_pipeline(OutputResult::from_rows(rows), &parsed.stages)
        .expect("pipeline should succeed")
}

fn run_guide_pipeline(view: GuideView, pipeline: &str) -> OutputResult {
    let parsed = parse_pipeline(&format!("fixture | {pipeline}")).expect("pipeline should parse");
    apply_output_pipeline(view.to_output_result(), &parsed.stages).expect("pipeline should succeed")
}

fn help_like_guide() -> GuideView {
    let commands = vec![
        GuideEntry {
            name: "apply".to_string(),
            short_help: "Apply pending changes".to_string(),
            display_indent: None,
            display_gap: None,
        },
        GuideEntry {
            name: "doctor".to_string(),
            short_help: "Inspect runtime health".to_string(),
            display_indent: None,
            display_gap: None,
        },
        GuideEntry {
            name: "status".to_string(),
            short_help: "Show deployment status".to_string(),
            display_indent: None,
            display_gap: None,
        },
    ];
    let options = vec![
        GuideEntry {
            name: "--verbose".to_string(),
            short_help: "Show additional context".to_string(),
            display_indent: None,
            display_gap: None,
        },
        GuideEntry {
            name: "--json".to_string(),
            short_help: "Render machine-readable output".to_string(),
            display_indent: None,
            display_gap: None,
        },
    ];

    GuideView {
        preamble: vec!["Deploy commands".to_string()],
        usage: vec!["osp deploy <COMMAND>".to_string()],
        commands: commands.clone(),
        options: options.clone(),
        notes: vec!["Run `doctor` before applying production changes.".to_string()],
        epilogue: vec!["footer text".to_string()],
        sections: vec![
            GuideSection {
                title: "Commands".to_string(),
                kind: GuideSectionKind::Commands,
                paragraphs: vec!["pick one".to_string()],
                entries: commands,
            },
            GuideSection {
                title: "Options".to_string(),
                kind: GuideSectionKind::Options,
                paragraphs: vec!["rendering".to_string()],
                entries: options,
            },
        ],
        ..GuideView::default()
    }
}
