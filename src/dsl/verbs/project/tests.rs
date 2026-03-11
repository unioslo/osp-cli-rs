use crate::core::output_model::Group;
use serde_json::json;

use super::{apply, apply_groups, compile};

#[test]
fn keeps_requested_columns() {
    let rows = vec![
        json!({"uid": "oistes", "cn": "Oistein", "mail": "o@uio.no"})
            .as_object()
            .cloned()
            .expect("object"),
    ];

    let projected = apply(rows, "uid cn").expect("project should work");
    assert!(projected[0].contains_key("uid"));
    assert!(projected[0].contains_key("cn"));
    assert!(!projected[0].contains_key("mail"));
}

#[test]
fn drops_column_with_prefix() {
    let rows = vec![
        json!({"uid": "oistes", "status": "active"})
            .as_object()
            .cloned()
            .expect("object"),
    ];

    let projected = apply(rows, "!status").expect("project should work");
    assert!(projected[0].contains_key("uid"));
    assert!(!projected[0].contains_key("status"));
}

#[test]
fn supports_selector_fanout() {
    let rows = vec![
        json!({
            "interfaces": [
                {"mac": "aa:bb"},
                {"mac": "cc:dd"}
            ]
        })
        .as_object()
        .cloned()
        .expect("object"),
    ];

    let projected = apply(rows, "interfaces[].mac").expect("project should work");
    assert_eq!(projected.len(), 2);
    assert_eq!(projected[0].get("mac"), Some(&json!("aa:bb")));
    assert_eq!(projected[1].get("mac"), Some(&json!("cc:dd")));
}

#[test]
fn quoted_commas_remain_single_projection_terms() {
    let rows = vec![
        json!({"display,name": "Alice", "display": "wrong", "name": "wrong"})
            .as_object()
            .cloned()
            .expect("object"),
    ];

    let projected = apply(rows, "\"display,name\"").expect("project should work");
    assert_eq!(
        projected,
        vec![
            json!({"display,name": "Alice"})
                .as_object()
                .cloned()
                .expect("object")
        ]
    );
}

#[test]
fn keeps_all_exact_nested_matches() {
    let rows = vec![
        json!({
            "id": 55753,
            "txts": {"id": 27994},
            "ipaddresses": [{"id": 57171}, {"id": 57172}],
            "metadata": {"asset": {"id": 42}}
        })
        .as_object()
        .cloned()
        .expect("object"),
    ];

    let projected = apply(rows, "id").expect("project should work");
    assert_eq!(
        projected,
        vec![
            json!({
                "id": 55753,
                "txts": {"id": 27994},
                "ipaddresses": [{"id": 57171}, {"id": 57172}],
                "metadata": {"asset": {"id": 42}}
            })
            .as_object()
            .cloned()
            .expect("object")
        ]
    );
}

#[test]
fn absolute_path_projection_keeps_only_exact_nested_key() {
    let rows = vec![
        json!({"id": 1, "nested": {"id": 2}, "other": {"id": 3}})
            .as_object()
            .cloned()
            .expect("object"),
    ];

    let projected = apply(rows, ".nested.id").expect("project should work");
    assert_eq!(
        projected,
        vec![
            json!({"nested": {"id": 2}})
                .as_object()
                .cloned()
                .expect("object")
        ]
    );
}

#[test]
fn relative_path_projection_requires_exact_flat_path_match() {
    let rows = vec![
        json!({"metadata": {"asset": {"id": 42}}})
            .as_object()
            .cloned()
            .expect("object"),
    ];

    let projected = apply(rows, "asset.id").expect("project should work");
    assert!(projected.is_empty());
}

#[test]
fn apply_groups_keeps_aggregate_only_groups_even_when_rows_drop_out() {
    let groups = vec![Group {
        groups: json!({"dept": "eng"}).as_object().cloned().expect("object"),
        aggregates: json!({"count": 2}).as_object().cloned().expect("object"),
        rows: vec![
            json!({"uid": "alice"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"uid": "bob"}).as_object().cloned().expect("object"),
        ],
    }];

    let projected = apply_groups(groups, "missing").expect("group project should work");
    assert_eq!(projected.len(), 1);
    assert!(projected[0].rows.is_empty());
    assert_eq!(projected[0].aggregates.get("count"), Some(&json!(2)));
}

#[test]
fn empty_project_spec_is_rejected() {
    let err = apply(
        vec![
            json!({"uid": "alice"})
                .as_object()
                .cloned()
                .expect("object"),
        ],
        "   ",
    )
    .expect_err("empty spec should fail");

    assert!(err.to_string().contains("requires one or more keys"));
}

#[test]
fn dropping_dynamic_projection_label_removes_fanout_column() {
    let rows = vec![
        json!({
            "uid": "alice",
            "interfaces": [{"mac": "aa:bb"}, {"mac": "cc:dd"}]
        })
        .as_object()
        .cloned()
        .expect("object"),
    ];

    let projected = apply(rows, "uid interfaces[].mac !mac").expect("project should work");
    assert_eq!(
        projected,
        vec![
            json!({"uid": "alice"})
                .as_object()
                .cloned()
                .expect("object")
        ]
    );
}

#[test]
fn rejects_ambiguous_dynamic_projection_labels() {
    let rows = vec![
        json!({
            "users": [{"name": "alice"}],
            "groups": [{"name": "ops"}]
        })
        .as_object()
        .cloned()
        .expect("object"),
    ];

    let err = apply(rows, "users[].name groups[].name").expect_err("project should fail");
    assert!(
        err.to_string()
            .contains("ambiguous dynamic projection label `name`")
    );
    assert!(err.to_string().contains("users[].name"));
    assert!(err.to_string().contains("groups[].name"));
}

#[test]
fn compile_treats_prefixed_path_droppers_as_structural_selectors() {
    let plan = compile("!sections[0].entries[0]").expect("project should compile");

    assert_eq!(plan.droppers.len(), 1);
    assert!(plan.droppers[0].is_structural());
}
