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

// traverse_matching contract: when a filter matches entries in only one section,
// the other section must be gone entirely — not an empty container with headers.
#[test]
fn structural_semantic_filter_drops_section_entirely_when_no_entries_survive() {
    let output = run_guide_pipeline(guide_with_sections(), "F name=start");

    let guide = GuideView::try_from_output_result(&output).expect("document must survive filter");

    assert_eq!(guide.sections.len(), 1, "Utilities section must be pruned");
    assert_eq!(guide.sections[0].title, "Actions");
    assert_eq!(guide.sections[0].entries.len(), 1);
    assert_eq!(guide.sections[0].entries[0].name, "start");
}

// traverse_matching contract: envelope scalars (title, kind) must be kept in a
// section when its descendant entries survive, but must not appear in isolation
// if no entries survive (tested by checking the surviving section is complete).
#[test]
fn structural_semantic_filter_preserves_section_envelope_fields_with_surviving_entries() {
    let output = run_guide_pipeline(guide_with_sections(), "F name=version");

    let guide = GuideView::try_from_output_result(&output).expect("document must survive");

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

// Semantic sort must reorder guide commands without losing the document.
#[test]
fn structural_semantic_sort_reorders_guide_commands_preserving_document() {
    let output = run_guide_pipeline(guide_with_sections(), "S name");

    let guide = GuideView::try_from_output_result(&output).expect("document must survive sort");

    let names: Vec<&str> = guide.commands.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["delete", "deploy", "list"]);
    assert!(output.document.is_some());
}

// Semantic limit trims top-level collections.
#[test]
fn structural_semantic_limit_trims_top_level_collections_per_array() {
    let output = run_guide_pipeline(guide_with_sections(), "L 2");

    let guide = GuideView::try_from_output_result(&output).expect("document must survive limit");

    assert_eq!(guide.commands.len(), 2, "commands trimmed to 2");
    assert_eq!(guide.sections.len(), 2, "sections also trimmed to 2");
    assert_eq!(
        guide.sections[0].entries.len(),
        3,
        "entries within surviving sections are not re-trimmed"
    );
}

// Regex filter selects rows whose field value matches a pattern.
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

// Existence grouping (`?field`) creates true/false buckets.
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

    assert_eq!(true_bucket.rows.len(), 2);
    assert_eq!(false_bucket.rows.len(), 2);
}

// Collapse (`Z`) must flatten grouped output into ordinary rows.
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

// Realistic four-stage pipeline across substrate transitions.
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

    assert_eq!(by_mac("aa:bb").aggregates["n"], json!(2));
    assert_eq!(by_mac("cc:dd").aggregates["n"], json!(1));
    assert_eq!(by_mac("ee:ff").aggregates["n"], json!(1));
}

// Six-stage realistic pipeline across row verbs.
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
