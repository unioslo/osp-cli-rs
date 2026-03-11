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
