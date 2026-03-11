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

fn sample_guide() -> GuideView {
    GuideView {
        usage: vec!["osp intro".to_string()],
        commands: sample_commands(),
        ..GuideView::default()
    }
}

fn sample_commands() -> Vec<GuideEntry> {
    vec![
        GuideEntry {
            name: "help".to_string(),
            short_help: "Show overview".to_string(),
            display_indent: None,
            display_gap: None,
        },
        GuideEntry {
            name: "config".to_string(),
            short_help: "Show config values".to_string(),
            display_indent: None,
            display_gap: None,
        },
        GuideEntry {
            name: "exit".to_string(),
            short_help: "Leave shell".to_string(),
            display_indent: None,
            display_gap: None,
        },
    ]
}

fn guide_with_sections() -> GuideView {
    GuideView {
        commands: vec![
            GuideEntry {
                name: "deploy".to_string(),
                short_help: "Deploy a VM".to_string(),
                ..Default::default()
            },
            GuideEntry {
                name: "list".to_string(),
                short_help: "List all VMs".to_string(),
                ..Default::default()
            },
            GuideEntry {
                name: "delete".to_string(),
                short_help: "Delete a VM".to_string(),
                ..Default::default()
            },
        ],
        sections: vec![
            GuideSection {
                title: "Actions".to_string(),
                kind: GuideSectionKind::Commands,
                paragraphs: vec![],
                entries: vec![
                    GuideEntry {
                        name: "start".to_string(),
                        short_help: "Start the VM".to_string(),
                        ..Default::default()
                    },
                    GuideEntry {
                        name: "stop".to_string(),
                        short_help: "Stop the VM".to_string(),
                        ..Default::default()
                    },
                    GuideEntry {
                        name: "restart".to_string(),
                        short_help: "Restart the VM".to_string(),
                        ..Default::default()
                    },
                ],
            },
            GuideSection {
                title: "Utilities".to_string(),
                kind: GuideSectionKind::Custom,
                paragraphs: vec![],
                entries: vec![
                    GuideEntry {
                        name: "version".to_string(),
                        short_help: "Show version info".to_string(),
                        ..Default::default()
                    },
                    GuideEntry {
                        name: "doctor".to_string(),
                        short_help: "Run diagnostics".to_string(),
                        ..Default::default()
                    },
                ],
            },
        ],
        ..GuideView::default()
    }
}
