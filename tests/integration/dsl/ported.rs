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

#[test]
fn ported_project_then_limit_keeps_reversed_slice_head() {
    let rows = vec![
        row(json!({"netgroups": ["ansatt-373034", "ucore", "usit"]})),
        row(json!({"netgroups": ["ansatt-373034"]})),
        row(json!({"value": "standalone"})),
    ];

    let output = run_rows_pipeline(rows, "P netgroups[::-1] | L 2");
    let OutputItems::Rows(rows) = output.items else {
        panic!("expected flat rows");
    };

    assert_eq!(
        rows,
        vec![
            row(json!({"netgroups": "usit"})),
            row(json!({"netgroups": "ucore"})),
        ]
    );
}

#[test]
fn ported_project_group_networks_fans_out_then_groups_cleanly() {
    let rows = vec![row(json!({
        "networks": [
            {"network": "129.240.130.0/24", "vlan": 200},
            {"network": "2001:700:100:4003::/64", "vlan": 200},
        ]
    }))];

    let output = run_rows_pipeline(rows, "P networks[] | P network | G network");
    let OutputItems::Groups(groups) = output.items else {
        panic!("expected grouped rows");
    };

    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].rows.len(), 1);
    assert_eq!(groups[1].rows.len(), 1);
    assert_eq!(
        groups
            .iter()
            .map(|group| group.groups["network"].clone())
            .collect::<Vec<_>>(),
        vec![json!("129.240.130.0/24"), json!("2001:700:100:4003::/64"),]
    );
    assert_eq!(
        groups[0].rows[0].keys().collect::<Vec<_>>(),
        vec!["network"]
    );
}

#[test]
fn ported_quick_path_scoping_distinguishes_nested_and_root_matches() {
    let rows = vec![row(json!({
        "id": 55753,
        "txts": {"id": 27994},
        "ipaddresses": [{"id": 57171}, {"id": 57172}],
        "metadata": {"asset": {"id": 42}}
    }))];

    let asset = run_rows_pipeline(rows.clone(), "asset.id");
    let OutputItems::Rows(asset_rows) = asset.items else {
        panic!("expected flat rows");
    };
    assert!(asset_rows.is_empty());

    let root_only = run_rows_pipeline(rows.clone(), ".asset.id");
    let OutputItems::Rows(root_rows) = root_only.items else {
        panic!("expected flat rows");
    };
    assert!(root_rows.is_empty());

    let ids = run_rows_pipeline(rows, "id");
    let OutputItems::Rows(id_rows) = ids.items else {
        panic!("expected flat rows");
    };
    assert_eq!(
        id_rows,
        vec![row(json!({
            "id": 55753,
            "txts": {"id": 27994},
            "ipaddresses": [{"id": 57171}, {"id": 57172}],
            "metadata": {"asset": {"id": 42}}
        }))]
    );
}

#[test]
fn ported_quick_collects_all_exact_matches_across_nested_fields() {
    let rows = vec![row(json!({
        "id": 55753,
        "txts": {"id": 27994},
        "ipaddresses": [{"id": 57171}, {"id": 57172}],
        "metadata": {"asset": {"id": 42}}
    }))];

    let output = run_rows_pipeline(rows, "id");
    let OutputItems::Rows(rows) = output.items else {
        panic!("expected flat rows");
    };

    assert_eq!(
        rows,
        vec![row(json!({
            "id": 55753,
            "txts": {"id": 27994},
            "ipaddresses": [{"id": 57171}, {"id": 57172}],
            "metadata": {"asset": {"id": 42}}
        }))]
    );
}

#[test]
fn ported_quick_list_selector_fanout_preserves_selector_order() {
    let rows = vec![row(json!({
        "networks": [
            {"network": "129.240.130.0/24", "vlan": 200},
            {"network": "2001:700:100:4003::/64", "vlan": 200}
        ]
    }))];

    let output = run_rows_pipeline(rows, "networks[]");
    let OutputItems::Rows(rows) = output.items else {
        panic!("expected flat rows");
    };

    assert_eq!(
        rows,
        vec![
            row(json!({"network": "129.240.130.0/24", "vlan": 200})),
            row(json!({"network": "2001:700:100:4003::/64", "vlan": 200})),
        ]
    );
}

#[test]
fn ported_group_nested_slice_chain_uses_selector_result() {
    let rows = vec![row(json!({
        "metadata": {
            "items": [
                {
                    "props": [
                        {"key": "x", "value": "one"},
                        {"key": "y", "value": "two"}
                    ]
                },
                {
                    "props": [
                        {"key": "z", "value": "three"}
                    ]
                }
            ]
        }
    }))];

    let output = run_rows_pipeline(rows, "G metadata.items[:1].props[:1].key");
    let OutputItems::Groups(groups) = output.items else {
        panic!("expected grouped rows");
    };

    assert_eq!(groups.len(), 1);
    assert_eq!(
        groups[0].groups["metadata.items[:1].props[:1].key"],
        json!("x")
    );
}

#[test]
fn ported_filter_datetime_comparison_handles_naive_rhs() {
    let rows = vec![
        row(json!({"ts": "2026-02-13T20:00:00+00:00"})),
        row(json!({"ts": "2026-02-12T08:00:00+00:00"})),
    ];

    let output = run_rows_pipeline(rows, "F ts>2026-02-13 00:00:00");
    let OutputItems::Rows(rows) = output.items else {
        panic!("expected flat rows");
    };

    assert_eq!(rows, vec![row(json!({"ts": "2026-02-13T20:00:00+00:00"}))]);
}

#[test]
fn nasty_semantic_quick_sort_limit_preserves_guide_shape() {
    let output = run_guide_pipeline(sample_guide(), "show | S name | L 1");
    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should still restore");

    assert_eq!(rebuilt.commands.len(), 1);
    assert_eq!(rebuilt.commands[0].name, "config");
    assert!(rebuilt.sections.is_empty());
    assert!(output.document.is_some());
}

#[test]
fn nasty_semantic_project_of_entry_field_preserves_guide_shape_with_narrowed_entries() {
    let output = run_guide_pipeline(sample_guide(), "P commands[].name");
    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should restore");

    assert_eq!(rebuilt.commands.len(), 3);
    assert_eq!(rebuilt.commands[0].name, "help");
    assert_eq!(rebuilt.commands[1].name, "config");
    assert_eq!(rebuilt.commands[2].name, "exit");
    assert!(rebuilt.commands.iter().all(|entry| entry.short_help.is_empty()));
}

#[test]
fn nasty_semantic_group_breaks_restore_but_keeps_grouped_payload() {
    let output = run_guide_pipeline(sample_guide(), "G name");

    assert!(GuideView::try_from_output_result(&output).is_none());
    let OutputItems::Rows(rows) = output.items else {
        panic!("expected semantic projection to remain row-based");
    };
    let commands = rows[0]["commands"].as_array().expect("commands array");
    assert_eq!(commands.len(), 3);
    assert!(commands[0].get("groups").is_some());
    assert!(commands[0].get("rows").is_some());
}

#[test]
fn nasty_group_list_fanout_keeps_none_bucket_for_empty_collections() {
    let rows = vec![row(json!({"tags": []})), row(json!({"tags": ["a", "b"]}))];

    let output = run_rows_pipeline(rows, "G tags[]");
    let OutputItems::Groups(groups) = output.items else {
        panic!("expected grouped rows");
    };

    let observed = groups
        .iter()
        .map(|group| group.groups["tags"].clone())
        .collect::<Vec<_>>();
    assert!(observed.contains(&Value::Null));
    assert!(observed.contains(&json!("a")));
    assert!(observed.contains(&json!("b")));
}

#[test]
fn nasty_regroup_after_list_fanout_preserves_both_headers() {
    let rows = vec![
        row(json!({"dept": "ops", "roles": ["admin", "ssh"], "user": "alice"})),
        row(json!({"dept": "ops", "roles": ["ssh"], "user": "bob"})),
        row(json!({"dept": "eng", "roles": ["deploy"], "user": "cara"})),
    ];

    let output = run_rows_pipeline(rows, "G dept | G roles[]");
    let OutputItems::Groups(groups) = output.items else {
        panic!("expected grouped rows");
    };

    assert_eq!(groups.len(), 3);
    assert!(groups.iter().all(|group| group.groups.contains_key("dept")));
    assert!(
        groups
            .iter()
            .all(|group| group.groups.contains_key("roles"))
    );
    assert!(groups.iter().all(|group| !group.rows.is_empty()));
}

#[test]
fn nasty_group_filter_aggregate_keeps_only_matching_bucket() {
    let rows = vec![
        row(json!({"dept": "sales", "amount": 100})),
        row(json!({"dept": "sales", "amount": 200})),
        row(json!({"dept": "eng", "amount": 50})),
    ];

    let output = run_rows_pipeline(rows, "G dept | F dept=sales | A sum(amount) AS total");
    let OutputItems::Groups(groups) = output.items else {
        panic!("expected grouped rows");
    };

    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].groups["dept"], json!("sales"));
    assert_eq!(groups[0].aggregates["total"], json!(300.0));
}

#[test]
fn nasty_unroll_group_aggregate_counts_fanout_rows() {
    let rows = vec![
        row(json!({"host": "alpha", "interfaces": [{"mac": "aa"}, {"mac": "bb"}]})),
        row(json!({"host": "beta", "interfaces": [{"mac": "aa"}]})),
    ];

    let output = run_rows_pipeline(rows, "U interfaces | G mac | A count AS count");
    let OutputItems::Groups(groups) = output.items else {
        panic!("expected grouped rows");
    };

    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].groups["mac"], json!("aa"));
    assert_eq!(groups[0].aggregates["count"], json!(2));
    assert_eq!(groups[1].groups["mac"], json!("bb"));
    assert_eq!(groups[1].aggregates["count"], json!(1));
}

#[test]
fn nasty_nested_slice_filter_then_project_keeps_selected_branch() {
    let rows = vec![row(json!({
        "metadata": {
            "items": [
                {
                    "props": [
                        {"key": "x", "value": "one"},
                        {"key": "y", "value": "two"}
                    ]
                },
                {
                    "props": [
                        {"key": "z", "value": "three"}
                    ]
                }
            ]
        }
    }))];

    let output = run_rows_pipeline(
        rows,
        "F metadata.items[:1].props[:1].key=x | \
         P metadata.items[:1].props[:1].value",
    );
    let OutputItems::Rows(rows) = output.items else {
        panic!("expected flat rows");
    };

    assert_eq!(rows, vec![row(json!({"value": "one"}))]);
}

#[test]
fn nasty_negated_nested_filter_keeps_missing_rows() {
    let rows = vec![
        row(json!({"id": 1, "meta": {"status": "active"}})),
        row(json!({"id": 2})),
        row(json!({"id": 3, "meta": {"status": "inactive"}})),
    ];

    let output = run_rows_pipeline(rows, "F !meta.status=active | P id");
    let OutputItems::Rows(rows) = output.items else {
        panic!("expected flat rows");
    };

    assert_eq!(rows, vec![row(json!({"id": 2})), row(json!({"id": 3}))]);
}

#[test]
fn nasty_group_alias_aggregate_sort_handles_renamed_headers() {
    let rows = vec![
        row(json!({"dept": "sales", "amount": 100})),
        row(json!({"dept": "sales", "amount": 200})),
        row(json!({"dept": "eng", "amount": 50})),
    ];

    let output = run_rows_pipeline(
        rows,
        "G dept AS department | A sum(amount) AS total | S department",
    );
    let OutputItems::Groups(groups) = output.items else {
        panic!("expected grouped rows");
    };

    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].groups["department"], json!("eng"));
    assert_eq!(groups[0].aggregates["total"], json!(50.0));
    assert_eq!(groups[1].groups["department"], json!("sales"));
    assert_eq!(groups[1].aggregates["total"], json!(300.0));
}

// ── Helpers for the structural integration suite ─────────────────────────────

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

// ── Test 1 ────────────────────────────────────────────────────────────────────
// Five sequential semantic stages on a guide. Document must survive all five
// and the recovered guide must reflect every stage's effect: quick narrows,
// sort reorders, limit trims, clean is a no-op on clean data, and copy keeps
// the semantic document attached. Use the simple command-only guide shape that
// is supposed to remain restorable through these narrowing stages.
#[test]
fn structural_five_stage_semantic_pipeline_preserves_document_and_applies_all_stages() {
    let output = run_guide_pipeline(sample_guide(), "show | S name | L 1 | ? | Y");

    let guide = GuideView::try_from_output_result(&output)
        .expect("document must survive five semantic stages");

    assert_eq!(guide.commands.len(), 1);
    assert_eq!(guide.commands[0].name, "config");
    assert!(output.document.is_some(), "document must still be attached");
}

// ── Test 2 ────────────────────────────────────────────────────────────────────
// traverse_matching contract: when a filter matches entries in only one section,
// the other section must be gone entirely — not an empty container with headers.
#[test]
fn structural_semantic_filter_drops_section_entirely_when_no_entries_survive() {
    // "start" and "stop" exist only in the Actions section.
    // The Utilities section has "version" and "doctor" — neither matches.
    let output = run_guide_pipeline(guide_with_sections(), "F name=start");

    let guide = GuideView::try_from_output_result(&output).expect("document must survive filter");

    assert_eq!(guide.sections.len(), 1, "Utilities section must be pruned");
    assert_eq!(guide.sections[0].title, "Actions");
    assert_eq!(guide.sections[0].entries.len(), 1);
    assert_eq!(guide.sections[0].entries[0].name, "start");
}

// ── Test 3 ────────────────────────────────────────────────────────────────────
// traverse_matching contract: envelope scalars (title, kind) must be kept in a
// section when its descendant entries survive, but must not appear in isolation
// if no entries survive (tested by checking the surviving section is complete).
#[test]
fn structural_semantic_filter_preserves_section_envelope_fields_with_surviving_entries() {
    let output = run_guide_pipeline(guide_with_sections(), "F name=version");

    let guide = GuideView::try_from_output_result(&output).expect("document must survive");

    // Actions section drops (start/stop/restart don't match). Utilities survives.
    assert_eq!(guide.sections.len(), 1);
    let section = &guide.sections[0];
    assert_eq!(
        section.title, "Utilities",
        "section title must be preserved"
    );
    assert_eq!(
        section.kind,
        GuideSectionKind::Custom,
        "section kind must be preserved"
    );
    assert_eq!(section.entries.len(), 1);
    assert_eq!(section.entries[0].name, "version");
}

// ── Test 4 ────────────────────────────────────────────────────────────────────
// Semantic sort must reorder guide commands without losing the document.
// We start with commands in declaration order (deploy, list, delete) and sort
// by name ascending — the recovered guide must show them alphabetically.
#[test]
fn structural_semantic_sort_reorders_guide_commands_preserving_document() {
    let output = run_guide_pipeline(guide_with_sections(), "S name");

    let guide = GuideView::try_from_output_result(&output).expect("document must survive sort");

    let names: Vec<&str> = guide.commands.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["delete", "deploy", "list"]);
    assert!(output.document.is_some());
}

// ── Test 5 ────────────────────────────────────────────────────────────────────
// Semantic limit trims top-level collections. A guide with three commands and
// two sections under L 2 should yield two commands and two sections (both fit),
// with entries within sections untouched.
#[test]
fn structural_semantic_limit_trims_top_level_collections_per_array() {
    let output = run_guide_pipeline(guide_with_sections(), "L 2");

    let guide = GuideView::try_from_output_result(&output).expect("document must survive limit");

    assert_eq!(guide.commands.len(), 2, "commands trimmed to 2");
    // Both sections fit within L 2, entries inside sections are not trimmed
    // (limit applies at each array boundary it encounters, but doesn't re-enter
    // elements already sliced from a parent array).
    assert_eq!(guide.sections.len(), 2, "sections also trimmed to 2");
    assert_eq!(
        guide.sections[0].entries.len(),
        3,
        "entries within surviving sections are not re-trimmed"
    );
}

// ── Test 6 ────────────────────────────────────────────────────────────────────
// Regex filter selects rows whose field value matches a pattern.
// Tests the `~` operator end to end through the full engine.
#[test]
fn structural_regex_filter_selects_matching_rows_by_pattern() {
    let rows = vec![
        row(json!({"uid": "alice", "dept": "eng"})),
        row(json!({"uid": "aaron", "dept": "ops"})),
        row(json!({"uid": "bob", "dept": "sales"})),
        row(json!({"uid": "annika", "dept": "eng"})),
    ];

    let output = run_rows_pipeline(rows, "F uid ~ ^a");
    let OutputItems::Rows(rows) = output.items else {
        panic!("expected flat rows");
    };

    let uids: Vec<&str> = rows
        .iter()
        .map(|row| row["uid"].as_str().expect("uid"))
        .collect();
    assert_eq!(uids.len(), 3);
    assert!(uids.contains(&"alice"));
    assert!(uids.contains(&"aaron"));
    assert!(uids.contains(&"annika"));
    assert!(!uids.contains(&"bob"));
}

// ── Test 7 ────────────────────────────────────────────────────────────────────
// Existence grouping (`?field`) creates true/false buckets based on whether each
// row has the field with a truthy value, rather than grouping by the field value.
#[test]
fn structural_group_by_existence_key_buckets_by_field_presence() {
    let rows = vec![
        row(json!({"uid": "alice", "email": "alice@example.com"})),
        row(json!({"uid": "bob"})),
        row(json!({"uid": "carol", "email": "carol@example.com"})),
        row(json!({"uid": "dave", "email": null})),
    ];

    let output = run_rows_pipeline(rows, "G ?email");
    let OutputItems::Groups(groups) = output.items else {
        panic!("expected grouped rows");
    };

    assert_eq!(
        groups.len(),
        2,
        "must produce exactly true and false buckets"
    );

    let true_bucket = groups
        .iter()
        .find(|g| g.groups["email"] == json!(true))
        .expect("true bucket must exist");
    let false_bucket = groups
        .iter()
        .find(|g| g.groups["email"] == json!(false))
        .expect("false bucket must exist");

    // alice and carol have truthy email; bob has no email, dave has null email
    assert_eq!(true_bucket.rows.len(), 2);
    assert_eq!(false_bucket.rows.len(), 2);
}

// ── Test 8 ────────────────────────────────────────────────────────────────────
// Collapse (`Z`) must flatten grouped output into ordinary rows that include the
// group header fields inline. After collapse there is one summary row per
// group, not one row per original member.
#[test]
fn structural_collapse_after_group_produces_flat_rows_with_header_fields() {
    let rows = vec![
        row(json!({"dept": "eng", "host": "alpha"})),
        row(json!({"dept": "eng", "host": "beta"})),
        row(json!({"dept": "sales", "host": "gamma"})),
    ];

    let output = run_rows_pipeline(rows, "G dept | Z");
    let OutputItems::Rows(flat) = output.items else {
        panic!("expected flat rows after collapse");
    };

    assert_eq!(flat.len(), 2, "collapse produces one summary row per group");
    // Every row must carry the dept header that was the group key
    assert!(
        flat.iter().all(|row| row.contains_key("dept")),
        "group header field must be merged into each collapsed row"
    );
    let depts: Vec<&str> = flat
        .iter()
        .map(|row| row["dept"].as_str().expect("dept"))
        .collect();
    assert!(depts.contains(&"eng"));
    assert!(depts.contains(&"sales"));
}

// ── Test 9 ────────────────────────────────────────────────────────────────────
// Realistic four-stage pipeline: unroll a nested array, project two fields,
// group by one of them, then aggregate. Tests that the substrate transitions
// (materialization, grouping, aggregation) chain correctly across four verbs.
#[test]
fn structural_unroll_project_group_aggregate_pipeline_counts_by_key() {
    let rows = vec![
        row(
            json!({"host": "alpha", "interfaces": [{"mac": "aa:bb", "speed": 1000}, {"mac": "cc:dd", "speed": 100}]}),
        ),
        row(json!({"host": "beta",  "interfaces": [{"mac": "aa:bb", "speed": 1000}]})),
        row(json!({"host": "gamma", "interfaces": [{"mac": "ee:ff", "speed": 10}]})),
    ];

    let output = run_rows_pipeline(rows, "U interfaces | P mac,speed | G mac | A count AS n");
    let OutputItems::Groups(groups) = output.items else {
        panic!("expected grouped rows");
    };

    let by_mac = |mac: &str| {
        groups
            .iter()
            .find(|g| g.groups["mac"] == json!(mac))
            .unwrap_or_else(|| panic!("group for {mac} not found"))
    };

    // "aa:bb" appears in alpha and beta → count 2
    assert_eq!(by_mac("aa:bb").aggregates["n"], json!(2));
    // "cc:dd" appears only in alpha → count 1
    assert_eq!(by_mac("cc:dd").aggregates["n"], json!(1));
    // "ee:ff" appears only in gamma → count 1
    assert_eq!(by_mac("ee:ff").aggregates["n"], json!(1));
}

// ── Test 10 ───────────────────────────────────────────────────────────────────
// Six-stage realistic pipeline: filter active users, project relevant fields,
// group by department, sum revenue, sort ascending, take top-2-by-value result.
// If all six verbs compose correctly the correct two departments emerge in order.
#[test]
fn structural_six_stage_filter_project_group_aggregate_sort_limit_pipeline() {
    let rows = vec![
        row(json!({"user": "alice", "dept": "eng",   "active": true,  "amount": 400})),
        row(json!({"user": "bob",   "dept": "sales", "active": true,  "amount": 100})),
        row(json!({"user": "carol", "dept": "eng",   "active": false, "amount": 999})),
        row(json!({"user": "dave",  "dept": "ops",   "active": true,  "amount": 200})),
        row(json!({"user": "eve",   "dept": "sales", "active": true,  "amount": 150})),
        row(json!({"user": "frank", "dept": "ops",   "active": true,  "amount": 300})),
    ];

    // carol is inactive → filtered out (eng total becomes 400, not 1399)
    // Sort ascending by revenue: sales(250) < ops(500) < eng(400)
    //   → sorted order: sales, eng, ops
    // L 2 → sales and eng
    let output = run_rows_pipeline(
        rows,
        "F active=true | P user,dept,amount | G dept | A sum(amount) AS revenue | S revenue | L 2",
    );
    let OutputItems::Groups(groups) = output.items else {
        panic!("expected grouped rows");
    };

    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].groups["dept"], json!("sales"));
    assert_eq!(groups[0].aggregates["revenue"], json!(250.0));
    assert_eq!(groups[1].groups["dept"], json!("eng"));
    assert_eq!(groups[1].aggregates["revenue"], json!(400.0));
}
