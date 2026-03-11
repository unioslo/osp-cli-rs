use super::*;
use serde_json::json;

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
