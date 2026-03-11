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
    assert!(
        rebuilt
            .commands
            .iter()
            .all(|entry| entry.short_help.is_empty())
    );
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
