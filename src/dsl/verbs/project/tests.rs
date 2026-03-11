use crate::core::output_model::Group;
use serde_json::json;

use super::{apply, apply_groups, compile};

#[test]
fn project_basic_selection_variants_cover_columns_droppers_and_quoted_terms_unit() {
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

    let rows = vec![
        json!({"uid": "oistes", "status": "active"})
            .as_object()
            .cloned()
            .expect("object"),
    ];
    let projected = apply(rows, "!status").expect("project should work");
    assert!(projected[0].contains_key("uid"));
    assert!(!projected[0].contains_key("status"));

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
fn project_nested_path_variants_cover_exact_relative_and_absolute_matching_unit() {
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
fn project_fanout_and_dynamic_label_rules_cover_projection_drop_and_ambiguity_unit() {
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
fn project_group_retention_and_compile_guards_cover_empty_specs_and_structural_droppers_unit() {
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

    let plan = compile("!sections[0].entries[0]").expect("project should compile");
    assert_eq!(plan.droppers.len(), 1);
    assert!(plan.droppers[0].is_structural());
}
