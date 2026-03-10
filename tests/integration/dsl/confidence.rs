use osp_cli::core::output_model::{OutputItems, OutputResult};
use osp_cli::core::row::Row;
use osp_cli::dsl::{apply_output_pipeline, parse_pipeline};
use osp_cli::guide::{GuideEntry, GuideSection, GuideSectionKind, GuideView};
use serde_json::{Value, json};

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

// Protects the end-to-end interaction of filter, sort, limit, and project on
// ordinary flat rows, which is the bread-and-butter pipeline shape in real use.
#[test]
fn flat_pipeline_filter_sort_limit_project_produces_ranked_subset() {
    let rows = vec![
        row(json!({"host": "alpha", "status": "active", "score": 30, "owner": "ops"})),
        row(json!({"host": "beta", "status": "active", "score": 10, "owner": "db"})),
        row(json!({"host": "gamma", "status": "disabled", "score": 5, "owner": "ops"})),
        row(json!({"host": "delta", "status": "active", "score": 20, "owner": "api"})),
    ];

    let output = run_rows_pipeline(rows, "F status=active | S score | L 2 | P host score owner");
    let OutputItems::Rows(rows) = output.items else {
        panic!("expected flat rows");
    };

    assert_eq!(
        rows,
        vec![
            row(json!({"host": "beta", "score": 10, "owner": "db"})),
            row(json!({"host": "delta", "score": 20, "owner": "api"})),
        ]
    );
}

// Protects composed fanout/filter/project/sort behavior on nested row data so
// nested arrays behave like real datasets instead of toy objects.
#[test]
fn nested_row_pipeline_fanout_filter_project_sort_preserves_selected_records() {
    let rows = vec![
        row(json!({
            "host": "alpha",
            "networks": [
                {"cidr": "10.0.0.0/24", "vlan": 120, "role": "prod"},
                {"cidr": "10.0.1.0/24", "vlan": 220, "role": "db"}
            ]
        })),
        row(json!({
            "host": "beta",
            "networks": [
                {"cidr": "10.0.2.0/24", "vlan": 250, "role": "prod"}
            ]
        })),
        row(json!({"host": "gamma"})),
    ];

    let output = run_rows_pipeline(rows, "P networks[] | F vlan>=200 | P cidr role | S cidr");
    let OutputItems::Rows(rows) = output.items else {
        panic!("expected flat rows");
    };

    assert_eq!(
        rows,
        vec![
            row(json!({"cidr": "10.0.1.0/24", "role": "db"})),
            row(json!({"cidr": "10.0.2.0/24", "role": "prod"})),
        ]
    );
}

// Protects grouped execution as a distinct mode: filtering after grouping must
// operate over member rows and aggregation must reflect the filtered members.
#[test]
fn grouped_pipeline_filter_then_aggregate_uses_member_rows_not_headers() {
    let rows = vec![
        row(json!({"dept": "sales", "env": "prod", "amount": 100})),
        row(json!({"dept": "sales", "env": "dev", "amount": 30})),
        row(json!({"dept": "eng", "env": "prod", "amount": 50})),
        row(json!({"dept": "eng", "env": "dev", "amount": 70})),
        row(json!({"dept": "support", "env": "dev", "amount": 40})),
    ];

    let output = run_rows_pipeline(
        rows,
        "G dept | F env=prod | A sum(amount) AS total | S dept",
    );
    let OutputItems::Groups(groups) = output.items else {
        panic!("expected grouped rows");
    };

    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].groups["dept"], json!("eng"));
    assert_eq!(groups[0].aggregates["total"], json!(50.0));
    assert_eq!(groups[1].groups["dept"], json!("sales"));
    assert_eq!(groups[1].aggregates["total"], json!(100.0));
}

// Protects the "no matches" end state for a composed pipeline so later stages
// do not invent rows or retain stale structure after an empty filter result.
#[test]
fn composed_pipeline_returns_clean_empty_rows_when_nothing_matches() {
    let rows = vec![
        row(json!({"host": "alpha", "status": "active", "score": 30})),
        row(json!({"host": "beta", "status": "active", "score": 10})),
        row(json!({"host": "gamma", "status": "disabled", "score": 5})),
    ];

    let output = run_rows_pipeline(rows, "F status=retired | S score | L 1 | P host");
    let OutputItems::Rows(rows) = output.items else {
        panic!("expected flat rows");
    };

    assert!(rows.is_empty());
}

// Protects the row/group seam by proving grouped summaries can be aggregated,
// collapsed back to rows, and then treated like ordinary row output.
#[test]
fn grouped_pipeline_can_collapse_back_to_ranked_summary_rows() {
    let rows = vec![
        row(json!({"dept": "sales", "amount": 70})),
        row(json!({"dept": "sales", "amount": 60})),
        row(json!({"dept": "eng", "amount": 50})),
        row(json!({"dept": "eng", "amount": 40})),
    ];

    let output = run_rows_pipeline(
        rows,
        "G dept | A sum(amount) AS total | Z | F total>120 | S dept | P dept total",
    );
    let OutputItems::Rows(rows) = output.items else {
        panic!("expected summary rows");
    };

    assert_eq!(rows, vec![row(json!({"dept": "sales", "total": 130.0}))]);
}

// Protects envelope preservation for nested row data: matching inside an object
// array must keep the matching parent object intact instead of flattening it to
// a scalar fragment.
#[test]
fn nested_document_like_rows_keep_parent_envelope_when_descendant_matches() {
    let rows = vec![row(json!({
        "title": "Deploy Reference",
        "footer": ["Generated from prod metadata"],
        "commands": [
            {
                "name": "deploy",
                "summary": "Roll out service",
                "owner": "platform"
            },
            {
                "name": "status",
                "summary": "Inspect rollout",
                "owner": "ops"
            }
        ]
    }))];

    let output = run_rows_pipeline(rows, "deploy | ? | L 1");
    let OutputItems::Rows(rows) = output.items else {
        panic!("expected flat rows");
    };

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["title"], json!("Deploy Reference"));
    assert_eq!(rows[0]["footer"], json!(["Generated from prod metadata"]));
    let commands = rows[0]["commands"].as_array().expect("commands array");
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0]["name"], json!("deploy"));
    assert_eq!(commands[0]["summary"], json!("Roll out service"));
    assert_eq!(commands[0]["owner"], json!("platform"));
}

// Protects the happy semantic path: a narrowed guide payload should still round
// trip through the DSL and restore as a guide rather than degrading to generic
// rows.
#[test]
fn help_like_payload_restores_after_narrowing_multistage_pipeline() {
    let output = run_guide_pipeline(help_like_guide(), "status | ? | S name | L 1");
    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should still restore");

    assert_eq!(rebuilt.commands.len(), 1);
    assert_eq!(rebuilt.commands[0].name, "status");
    assert_eq!(rebuilt.commands[0].short_help, "Show deployment status");
    assert_eq!(rebuilt.preamble, vec!["Deploy commands"]);
    assert_eq!(rebuilt.usage, vec!["osp deploy <COMMAND>"]);
    assert_eq!(
        rebuilt.notes,
        vec!["Run `doctor` before applying production changes."]
    );
    assert_eq!(rebuilt.epilogue, vec!["footer text"]);
    assert!(
        rebuilt.options.is_empty(),
        "unmatched option entries should prune"
    );
    assert_eq!(rebuilt.sections.len(), 0);
}

// Protects semantic envelope preservation specifically for guide-shaped data:
// a descendant match should keep the surviving section title and its entry shell.
#[test]
fn help_like_payload_keeps_section_envelope_for_partial_match() {
    let output = run_guide_pipeline(help_like_guide(), "status | ? | L 1");
    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should still restore");

    assert_eq!(rebuilt.preamble, vec!["Deploy commands"]);
    assert_eq!(rebuilt.usage, vec!["osp deploy <COMMAND>"]);
    assert_eq!(rebuilt.commands.len(), 1);
    assert_eq!(rebuilt.commands[0].name, "status");
    assert_eq!(rebuilt.sections.len(), 0);
}

// Protects the degrade path: once a semantic payload is structurally reshaped
// into generic value rows, restore must stop rather than fabricating guide
// semantics from the new shape.
#[test]
fn help_like_payload_does_not_restore_after_value_extraction_pipeline() {
    let output = run_guide_pipeline(
        help_like_guide(),
        "P commands[].name | VALUE name | S value | L 2",
    );

    assert!(GuideView::try_from_output_result(&output).is_none());
    let OutputItems::Rows(rows) = output.items else {
        panic!("expected flat value rows");
    };
    assert_eq!(
        rows,
        vec![
            row(json!({"value": "apply"})),
            row(json!({"value": "doctor"})),
        ]
    );
}

// Protects mixed-structure handling in a realistic fanout/group pipeline: rows
// with missing or empty collections must not create phantom groups or null junk.
#[test]
fn mixed_structures_do_not_create_phantom_groups_after_fanout_pipeline() {
    let rows = vec![
        row(json!({
            "service": "api",
            "endpoints": [
                {"region": "eu", "enabled": true},
                {"region": "us", "enabled": false}
            ]
        })),
        row(json!({
            "service": "worker",
            "endpoints": [
                {"region": "us", "enabled": true}
            ]
        })),
        row(json!({"service": "cron", "endpoints": []})),
        row(json!({"service": "ops"})),
    ];

    let output = run_rows_pipeline(
        rows,
        "P endpoints[] | F enabled=true | G region | A count AS count | S region",
    );
    let OutputItems::Groups(groups) = output.items else {
        panic!("expected grouped rows");
    };

    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].groups["region"], json!("eu"));
    assert_eq!(groups[0].aggregates["count"], json!(1));
    assert_eq!(groups[1].groups["region"], json!("us"));
    assert_eq!(groups[1].aggregates["count"], json!(1));
}

// Protects the new semantic unroll path: nested entry arrays should duplicate
// their parent section shell per entry instead of flattening into anonymous
// row-like fragments.
#[test]
fn help_like_payload_unroll_preserves_parent_section_shell() {
    let output = run_guide_pipeline(help_like_guide(), "U entries");

    assert!(GuideView::try_from_output_result(&output).is_none());
    let document = output
        .document
        .expect("semantic document should remain attached");
    assert_eq!(
        document.value,
        json!({
            "preamble": ["Deploy commands"],
            "usage": ["osp deploy <COMMAND>"],
            "commands": [
                {"name": "apply", "short_help": "Apply pending changes"},
                {"name": "doctor", "short_help": "Inspect runtime health"},
                {"name": "status", "short_help": "Show deployment status"}
            ],
            "options": [
                {"name": "--verbose", "short_help": "Show additional context"},
                {"name": "--json", "short_help": "Render machine-readable output"}
            ],
            "notes": ["Run `doctor` before applying production changes."],
            "sections": [
                {
                    "title": "Commands",
                    "kind": "commands",
                    "paragraphs": ["pick one"],
                    "entries": {"name": "apply", "short_help": "Apply pending changes"}
                },
                {
                    "title": "Commands",
                    "kind": "commands",
                    "paragraphs": ["pick one"],
                    "entries": {"name": "doctor", "short_help": "Inspect runtime health"}
                },
                {
                    "title": "Commands",
                    "kind": "commands",
                    "paragraphs": ["pick one"],
                    "entries": {"name": "status", "short_help": "Show deployment status"}
                },
                {
                    "title": "Options",
                    "kind": "options",
                    "paragraphs": ["rendering"],
                    "entries": {"name": "--verbose", "short_help": "Show additional context"}
                },
                {
                    "title": "Options",
                    "kind": "options",
                    "paragraphs": ["rendering"],
                    "entries": {"name": "--json", "short_help": "Render machine-readable output"}
                }
            ],
            "epilogue": ["footer text"]
        })
    );
}

// Protects the new addressed-path projection path: selecting one indexed
// descendant should rebuild only that exact branch while keeping stable guide
// envelope metadata around it.
#[test]
fn help_like_payload_exact_index_projection_rebuilds_selected_branch() {
    let output = run_guide_pipeline(help_like_guide(), "P sections[1].entries[0].name");

    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should still restore");
    assert_eq!(rebuilt.preamble, vec!["Deploy commands"]);
    assert_eq!(rebuilt.usage, vec!["osp deploy <COMMAND>"]);
    assert_eq!(
        rebuilt.notes,
        vec!["Run `doctor` before applying production changes."]
    );
    assert_eq!(rebuilt.epilogue, vec!["footer text"]);
    assert_eq!(rebuilt.options.len(), 1);
    assert_eq!(rebuilt.options[0].name, "--verbose");
    assert_eq!(rebuilt.sections.len(), 0);

    let document = output
        .document
        .expect("semantic document should remain attached");
    assert_eq!(
        document.value,
        json!({
            "preamble": ["Deploy commands"],
            "usage": ["osp deploy <COMMAND>"],
            "notes": ["Run `doctor` before applying production changes."],
            "epilogue": ["footer text"],
            "sections": [
                {
                    "title": "Options",
                    "kind": "options",
                    "paragraphs": ["rendering"],
                    "entries": [{"name": "--verbose"}]
                }
            ]
        })
    );
}

// Protects negative-index structural projection: addressed resolution should
// normalize negative indexes before rebuilding, so the last addressed entry is
// selected structurally rather than falling back to flat heuristics.
#[test]
fn help_like_payload_negative_index_projection_rebuilds_last_entry() {
    let output = run_guide_pipeline(help_like_guide(), "P sections[-1].entries[-1].name");

    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should still restore");
    assert_eq!(rebuilt.options.len(), 1);
    assert_eq!(rebuilt.options[0].name, "--json");
    assert_eq!(rebuilt.sections.len(), 0);

    let document = output
        .document
        .expect("semantic document should remain attached");
    assert_eq!(
        document.value,
        json!({
            "preamble": ["Deploy commands"],
            "usage": ["osp deploy <COMMAND>"],
            "notes": ["Run `doctor` before applying production changes."],
            "epilogue": ["footer text"],
            "sections": [
                {
                    "title": "Options",
                    "kind": "options",
                    "paragraphs": ["rendering"],
                    "entries": [{"name": "--json"}]
                }
            ]
        })
    );
}

// Protects the new addressed filter path: an exact indexed predicate should
// rebuild only the matching semantic branch and still restore the guide shell
// around it instead of degrading to flat path fragments.
#[test]
fn help_like_payload_exact_index_filter_rebuilds_selected_branch() {
    let output = run_guide_pipeline(help_like_guide(), "F sections[1].entries[0].name=--verbose");

    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should still restore");
    assert_eq!(rebuilt.preamble, vec!["Deploy commands"]);
    assert_eq!(rebuilt.usage, vec!["osp deploy <COMMAND>"]);
    assert_eq!(
        rebuilt.notes,
        vec!["Run `doctor` before applying production changes."]
    );
    assert_eq!(rebuilt.epilogue, vec!["footer text"]);
    assert_eq!(rebuilt.options.len(), 1);
    assert_eq!(rebuilt.options[0].name, "--verbose");
    assert_eq!(rebuilt.sections.len(), 0);

    let document = output
        .document
        .expect("semantic document should remain attached");
    assert_eq!(
        document.value,
        json!({
            "preamble": ["Deploy commands"],
            "usage": ["osp deploy <COMMAND>"],
            "notes": ["Run `doctor` before applying production changes."],
            "epilogue": ["footer text"],
            "sections": [
                {
                    "title": "Options",
                    "kind": "options",
                    "paragraphs": ["rendering"],
                    "entries": [{"name": "--verbose"}]
                }
            ]
        })
    );
}

// Protects negated exact-address filters: when the addressed predicate passes,
// they should still rebuild the selected branch instead of falling back to a
// whole-document generic match.
#[test]
fn help_like_payload_exact_index_negated_filter_rebuilds_selected_branch() {
    let output = run_guide_pipeline(help_like_guide(), "F sections[1].entries[0].name!=--json");

    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should still restore");
    assert_eq!(rebuilt.options.len(), 1);
    assert_eq!(rebuilt.options[0].name, "--verbose");
    assert_eq!(rebuilt.sections.len(), 0);

    let document = output
        .document
        .expect("semantic document should remain attached");
    assert_eq!(
        document.value,
        json!({
            "preamble": ["Deploy commands"],
            "usage": ["osp deploy <COMMAND>"],
            "notes": ["Run `doctor` before applying production changes."],
            "epilogue": ["footer text"],
            "sections": [
                {
                    "title": "Options",
                    "kind": "options",
                    "paragraphs": ["rendering"],
                    "entries": [{"name": "--verbose"}]
                }
            ]
        })
    );
}

// Protects broader structural filters: fanout path selectors should rebuild
// only the surviving addressed descendants instead of falling back to generic
// descendant traversal over leaf rows.
#[test]
fn help_like_payload_fanout_filter_rebuilds_selected_branch() {
    let output = run_guide_pipeline(help_like_guide(), "F sections[].entries[].name=--json");

    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should still restore");
    assert_eq!(rebuilt.commands.len(), 0);
    assert_eq!(rebuilt.options.len(), 1);
    assert_eq!(rebuilt.options[0].name, "--json");
    assert_eq!(rebuilt.sections.len(), 0);

    let document = output
        .document
        .expect("semantic document should remain attached");
    assert_eq!(
        document.value,
        json!({
            "preamble": ["Deploy commands"],
            "usage": ["osp deploy <COMMAND>"],
            "notes": ["Run `doctor` before applying production changes."],
            "epilogue": ["footer text"],
            "sections": [
                {
                    "title": "Options",
                    "kind": "options",
                    "paragraphs": ["rendering"],
                    "entries": [{"name": "--json"}]
                }
            ]
        })
    );
}

// Protects structural slice projection: slice selectors should rebuild the
// selected addressed range in order and compact away unselected holes.
#[test]
fn help_like_payload_slice_projection_rebuilds_selected_range() {
    let output = run_guide_pipeline(help_like_guide(), "P sections[0].entries[1:3].name");

    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should still restore");
    assert_eq!(rebuilt.commands.len(), 2);
    assert_eq!(rebuilt.commands[0].name, "doctor");
    assert_eq!(rebuilt.commands[1].name, "status");
    assert_eq!(rebuilt.options.len(), 0);
    assert_eq!(rebuilt.sections.len(), 0);

    let document = output
        .document
        .expect("semantic document should remain attached");
    assert_eq!(
        document.value,
        json!({
            "preamble": ["Deploy commands"],
            "usage": ["osp deploy <COMMAND>"],
            "notes": ["Run `doctor` before applying production changes."],
            "epilogue": ["footer text"],
            "sections": [
                {
                    "title": "Commands",
                    "kind": "commands",
                    "paragraphs": ["pick one"],
                    "entries": [{"name": "doctor"}, {"name": "status"}]
                }
            ]
        })
    );
}

// Protects structural fanout negation: removing a matched descendant across a
// fanout path should delete only the addressed leaves, not the containing
// sections or unrelated top-level guide arrays.
#[test]
fn help_like_payload_fanout_negated_path_quick_removes_only_matched_names() {
    let output = run_guide_pipeline(help_like_guide(), "!sections[].entries[].name");

    let document = output
        .document
        .expect("semantic document should remain attached");
    let sections = document.value["sections"]
        .as_array()
        .expect("sections array");
    assert_eq!(sections.len(), 2);
    for section in sections {
        let entries = section["entries"].as_array().expect("entries array");
        assert!(!entries.is_empty(), "entries should remain present");
        for entry in entries {
            assert!(
                entry.get("name").is_none(),
                "fanout negation should remove only the addressed name field"
            );
            assert!(
                entry.get("short_help").is_some(),
                "sibling entry metadata should survive addressed removal"
            );
        }
    }

    let commands = document.value["commands"]
        .as_array()
        .expect("commands array");
    assert_eq!(commands.len(), 3);
    assert_eq!(commands[0]["name"], json!("apply"));
    assert_eq!(commands[1]["name"], json!("doctor"));
    assert_eq!(commands[2]["name"], json!("status"));
}

// Protects structural path quick selection on semantic payloads: path-scoped
// quick should keep the same useful guide envelope as exact structural `P/F`
// instead of dropping the payload shell around the selected branch.
#[test]
fn help_like_payload_path_quick_projects_selected_branch_and_restores() {
    let output = run_guide_pipeline(help_like_guide(), "sections[1].entries[0].name");

    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should restore");
    assert_eq!(rebuilt.preamble, vec!["Deploy commands"]);
    assert_eq!(rebuilt.usage, vec!["osp deploy <COMMAND>"]);
    assert_eq!(
        rebuilt.notes,
        vec!["Run `doctor` before applying production changes."]
    );
    assert_eq!(rebuilt.epilogue, vec!["footer text"]);
    assert_eq!(rebuilt.options.len(), 1);
    assert_eq!(rebuilt.options[0].name, "--verbose");
    assert_eq!(rebuilt.sections.len(), 0);

    let document = output
        .document
        .expect("semantic document should remain attached");
    assert_eq!(
        document.value,
        json!({
            "preamble": ["Deploy commands"],
            "usage": ["osp deploy <COMMAND>"],
            "notes": ["Run `doctor` before applying production changes."],
            "epilogue": ["footer text"],
            "sections": [
                {
                    "title": "Options",
                    "kind": "options",
                    "paragraphs": ["rendering"],
                    "entries": [{"name": "--verbose"}]
                }
            ]
        })
    );
}

// Protects structural path negation on semantic payloads: removing one nested
// addressed branch should keep the remaining guide intact and still restore.
#[test]
fn help_like_payload_negated_path_quick_removes_selected_entry_and_restores() {
    let output = run_guide_pipeline(help_like_guide(), "!sections[1].entries[0]");

    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should still restore");
    assert_eq!(rebuilt.options.len(), 1);
    assert_eq!(rebuilt.options[0].name, "--json");
    assert_eq!(rebuilt.sections.len(), 0);
}

// Protects negated path quick on ordinary rows: deleting one addressed array
// element must not also delete real sibling `null` values when the collection
// is compacted afterward.
#[test]
fn negated_path_quick_preserves_real_null_array_items() {
    let rows = vec![row(json!({
        "items": [null, {"name": "keep"}, {"name": "drop"}]
    }))];

    let output = run_rows_pipeline(rows, "!items[2]");
    let OutputItems::Rows(rows) = output.items else {
        panic!("expected flat rows");
    };

    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0],
        row(json!({
            "items": [null, {"name": "keep"}]
        }))
    );
}

// Protects question-path semantics: `?path` should behave like a structural
// existence filter, keeping the full payload when the addressed path exists.
#[test]
fn help_like_payload_question_path_keeps_full_payload_when_address_exists() {
    let output = run_guide_pipeline(help_like_guide(), "?sections[1].entries[0].name");

    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should still restore");
    assert_eq!(rebuilt.commands.len(), 3);
    assert_eq!(rebuilt.options.len(), 2);
    assert_eq!(rebuilt.sections.len(), 0);
}

// Protects fuzzy quick as a permissive end-to-end semantic feature:
// typo-tolerant guide narrowing should still restore cleanly, keep the useful
// envelope, and retain the intended near-hit without requiring a single exact
// survivor.
#[test]
fn help_like_payload_fuzzy_quick_restores_typo_matched_command() {
    let output = run_guide_pipeline(help_like_guide(), "%docter | ? | L 1");

    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should still restore");
    assert!(
        rebuilt.commands.iter().any(|entry| entry.name == "doctor"),
        "doctor should survive typo-tolerant fuzzy narrowing"
    );
    assert!(
        rebuilt.commands.iter().all(|entry| entry.name != "apply"),
        "unrelated commands should not survive the narrowed guide"
    );
    assert_eq!(rebuilt.preamble, vec!["Deploy commands"]);
    assert_eq!(rebuilt.options.len(), 0);
}

// Protects fuzzy quick as a filter rather than a ranking language: matched rows
// should keep source order even when one row is a "closer" fuzzy hit.
#[test]
fn fuzzy_quick_filters_rows_without_reordering_matches() {
    let rows = vec![
        row(json!({"name": "docter", "kind": "near"})),
        row(json!({"name": "doctor", "kind": "exact-word"})),
        row(json!({"name": "status", "kind": "miss"})),
    ];

    let output = run_rows_pipeline(rows, "%docter");
    let OutputItems::Rows(rows) = output.items else {
        panic!("expected flat rows");
    };

    assert_eq!(
        rows,
        vec![
            row(json!({"name": "docter", "kind": "near"})),
            row(json!({"name": "doctor", "kind": "exact-word"})),
        ]
    );
}

// Protects semantic VALUE extraction on top-level scalar arrays: guide-like
// payload metadata such as usage should transform directly from canonical JSON
// instead of silently depending on row-shaped collections.
#[test]
fn help_like_payload_value_extracts_top_level_usage_array() {
    let output = run_guide_pipeline(help_like_guide(), "VALUE usage");

    assert!(GuideView::try_from_output_result(&output).is_none());
    let document = output
        .document
        .expect("semantic document should remain attached");
    assert_eq!(
        document.value,
        json!([
            {"value": "osp deploy <COMMAND>"}
        ])
    );
}

// Protects addressed VALUE extraction on semantic payloads: extracting one
// nested entry field should keep the surviving section shell while degrading
// the targeted leaf into `{value: ...}` rows.
#[test]
fn help_like_payload_value_extracts_nested_entry_field_with_section_envelope() {
    let output = run_guide_pipeline(help_like_guide(), "VALUE sections[1].entries[0].name");

    assert!(GuideView::try_from_output_result(&output).is_none());
    let document = output
        .document
        .expect("semantic document should remain attached");
    assert_eq!(
        document.value,
        json!({
            "sections": [
                {
                    "title": "Options",
                    "kind": "options",
                    "paragraphs": ["rendering"],
                    "entries": [
                        {"value": "--verbose"}
                    ]
                }
            ]
        })
    );
}

// Protects mixed-depth VALUE extraction: combining a top-level scalar-array
// selector with a nested structural selector should preserve each selected
// branch in-place instead of collapsing them into one synthetic wrapper.
#[test]
fn help_like_payload_value_mixed_depth_selectors_keep_structural_branches() {
    let output = run_guide_pipeline(help_like_guide(), "VALUE usage sections[0].entries[].name");

    assert!(GuideView::try_from_output_result(&output).is_none());
    let document = output
        .document
        .expect("semantic document should remain attached");
    assert_eq!(
        document.value,
        json!({
            "usage": [
                {"value": "osp deploy <COMMAND>"}
            ],
            "sections": [
                {
                    "title": "Commands",
                    "kind": "commands",
                    "paragraphs": ["pick one"],
                    "entries": [
                        {"value": "apply"},
                        {"value": "doctor"},
                        {"value": "status"}
                    ]
                }
            ]
        })
    );
}

// Protects overlapping structural keepers: projecting two exact descendants
// under the same section should merge into one rebuilt branch instead of
// duplicating or dropping siblings during structural union.
#[test]
fn help_like_payload_project_overlapping_structural_keepers_merges_shared_branch() {
    let output = run_guide_pipeline(
        help_like_guide(),
        "P sections[0].entries[0].name sections[0].entries[1].name",
    );

    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should still restore");
    assert_eq!(rebuilt.commands.len(), 2);
    assert_eq!(rebuilt.commands[0].name, "apply");
    assert_eq!(rebuilt.commands[1].name, "doctor");
    assert_eq!(rebuilt.options.len(), 0);
    assert_eq!(rebuilt.sections.len(), 0);

    let document = output
        .document
        .expect("semantic document should remain attached");
    assert_eq!(
        document.value,
        json!({
            "preamble": ["Deploy commands"],
            "usage": ["osp deploy <COMMAND>"],
            "notes": ["Run `doctor` before applying production changes."],
            "epilogue": ["footer text"],
            "sections": [
                {
                    "title": "Commands",
                    "kind": "commands",
                    "paragraphs": ["pick one"],
                    "entries": [{"name": "apply"}, {"name": "doctor"}]
                }
            ]
        })
    );
}
