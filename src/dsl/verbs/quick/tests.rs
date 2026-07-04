use anyhow::anyhow;
use serde_json::json;

use super::{
    MatchResult, apply_groups_with_plan, apply_value, apply_value_with_plan, apply_with_plan,
    build_synthetic_map, compile, flatten_row, last_segment_name, match_row,
    object_array_parent_prefix, resolve_pairs, squeeze_single_entry, stream_rows_with_plan,
    transform_row, value_matches_token,
};
use crate::core::{output_model::Group, row::Row};
use crate::dsl::parse::key_spec::ExactMode;

fn row(value: serde_json::Value) -> Row {
    value
        .as_object()
        .cloned()
        .expect("fixture should be an object")
}

#[test]
fn compile_classifies_structural_selectors_and_rejects_invalid_fuzzy_forms_unit() {
    let plan = compile("!sections[0].entries[0]").expect("quick should compile");
    assert!(plan.spec.is_structural());

    assert!(compile("   ").is_err());
    assert!(compile("% ?uid").is_err());
    assert!(compile("% ==uid").is_err());
    assert!(compile("% sections[0].uid").is_err());
}

#[test]
fn single_row_and_key_scoped_quick_modes_cover_positive_negated_existence_and_key_matching_unit() {
    let rows = vec![row(json!({
        "uid": "alice",
        "roles": ["eng", "ops"],
        "city": "Oslo"
    }))];
    let filtered = apply_with_plan(rows, &compile("!ops").unwrap()).expect("quick should work");
    assert_eq!(
        filtered,
        vec![row(json!({
            "uid": "alice",
            "roles": ["eng"],
            "city": "Oslo"
        }))]
    );

    let rows = vec![
        row(json!({"uid": "alice", "team": "ops"})),
        row(json!({"name": "bob", "team": "eng"})),
    ];
    let filtered = apply_with_plan(rows.clone(), &compile("K uid").unwrap())
        .expect("key-only quick should work");
    assert_eq!(filtered, vec![row(json!({"uid": "alice"}))]);
    assert_eq!(
        apply_with_plan(rows, &compile("?uid").unwrap()).unwrap(),
        vec![row(json!({"uid": "alice", "team": "ops"}))]
    );

    let key_match_row = row(json!({"uid": "alice", "team": "ops"}));
    let flat = flatten_row(&key_match_row);
    let plan = compile("K !=uid").expect("quick should compile");
    let (pairs, _) = resolve_pairs(&flat, plan.spec.token());
    let synthetic = build_synthetic_map(&pairs, &flat);
    let result = match_row(&flat, &pairs, synthetic, &plan.spec);
    assert!(plan.spec.key_not_equals);
    assert!(result.matched);

    let all_matching_rows = vec![row(json!({
        "id": 55753,
        "txts": {"id": 27994},
        "ipaddresses": [{"id": 57171}, {"id": 57172}],
        "metadata": {"asset": {"id": 42}}
    }))];
    let filtered =
        apply_with_plan(all_matching_rows, &compile("id").unwrap()).expect("quick should work");
    assert_eq!(
        filtered,
        vec![row(json!({
            "id": 55753,
            "txts": {"id": 27994},
            "ipaddresses": [{"id": 57171}, {"id": 57172}],
            "metadata": {"asset": {"id": 42}}
        }))]
    );

    let rows = vec![row(json!({"uid": "alice", "team": "ops"}))];
    assert_eq!(
        apply_with_plan(rows, &compile("!?missing").unwrap()).unwrap(),
        vec![row(json!({"uid": "alice", "team": "ops"}))]
    );

    let rows = vec![row(json!({
        "uid": "alice",
        "roles": ["eng", "ops"],
        "city": "Oslo"
    }))];
    let filtered = apply_with_plan(rows, &compile("ops").unwrap()).expect("quick should work");
    assert_eq!(filtered, vec![row(json!({"roles": ["ops"]}))]);
}

#[test]
fn stream_rows_with_plan_preserves_rows_and_errors_across_seed_orders_unit() {
    for seeds in [
        vec![
            Err(anyhow!("boom")),
            Ok(row(json!({"uid": "alice"}))),
            Err(anyhow!("later")),
        ],
        vec![Ok(row(json!({"uid": "alice"}))), Err(anyhow!("boom"))],
    ] {
        let plan = compile("uid").expect("quick should compile");
        let results = stream_rows_with_plan(seeds, plan).collect::<Vec<_>>();
        assert!(results.iter().any(|item| {
            item.as_ref()
                .is_ok_and(|row| row.get("uid") == Some(&json!("alice")))
        }));
        assert!(results.iter().any(|item| item.is_err()));
    }
}

#[test]
fn grouped_quick_uses_single_row_mode_per_group_unit() {
    let groups = vec![Group {
        groups: row(json!({"team": "ops"})),
        aggregates: row(json!({"count": 1})),
        rows: vec![row(json!({"uid": "alice", "city": "Oslo"}))],
    }];

    let filtered =
        apply_groups_with_plan(groups, &compile("uid").unwrap()).expect("group quick should work");
    assert_eq!(filtered[0].rows, vec![row(json!({"uid": "alice"}))]);
}

#[test]
fn single_row_quick_matches_semantic_descendant_narrowing_and_compacts_array_hits_unit() {
    let row_input = row(json!({
        "cn": "Oistein Sovik",
        "netgroups": [
            "ansatt-373034",
            "ansatt-tekadm-373034",
            "vcs-cfengine",
            "vcs-dhcp",
            "vcs-it-org",
            "vcs-it-osprov",
            "vcs-iti",
            "vcs-ops",
            "vcs-radius",
            "vcs-ssd",
            "vcs-usit",
            "vcs-virtprov-admins",
            "zabbix-iti-ops"
        ],
        "filegroups": ["oistes", "ucore", "usit", "vortex-opptak"]
    }));

    let filtered = apply_with_plan(vec![row_input], &compile("vcs").unwrap())
        .expect("single-row quick should narrow like the semantic path");

    assert_eq!(
        filtered,
        vec![row(json!({
            "netgroups": [
                "vcs-cfengine",
                "vcs-dhcp",
                "vcs-it-org",
                "vcs-it-osprov",
                "vcs-iti",
                "vcs-ops",
                "vcs-radius",
                "vcs-ssd",
                "vcs-usit",
                "vcs-virtprov-admins"
            ]
        }))]
    );
}

#[test]
fn single_row_row_quick_drops_root_siblings_but_preserves_matching_array_objects_unit() {
    let input = json!({
        "commands": [
            {"name": "doctor", "short_help": "Run diagnostics checks"},
            {"name": "history", "short_help": "Show command history"}
        ]
    });

    let row_filtered = apply_with_plan(vec![row(input)], &compile("doctor").unwrap())
        .expect("row quick should succeed");

    assert_eq!(
        row_filtered,
        vec![row(json!({
            "commands": [
                {"name": "doctor", "short_help": "Run diagnostics checks"}
            ]
        }))]
    );
}

#[test]
fn single_row_quick_preserves_scalar_envelope_around_nested_collection_matches_unit() {
    let input = row(json!({
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
    }));

    let filtered = apply_with_plan(vec![input], &compile("deploy").unwrap())
        .expect("row quick should preserve document envelope");

    assert_eq!(
        filtered,
        vec![row(json!({
            "title": "Deploy Reference",
            "footer": ["Generated from prod metadata"],
            "commands": [
                {
                    "name": "deploy",
                    "summary": "Roll out service",
                    "owner": "platform"
                }
            ]
        }))]
    );
}

#[test]
fn single_row_negated_quick_preserves_original_root_key_order_unit() {
    let rows = vec![row(json!({
        "cn": "Oistein Sovik",
        "uid": "oistes",
        "netgroups": ["ansatt-373034", "vcs-dhcp", "vcs-ops"],
        "filegroups": ["oistes"],
        "dn": "uid=oistes,dc=uio,dc=no"
    }))];

    let filtered = apply_with_plan(rows, &compile("!vcs").unwrap()).expect("quick should work");
    let keys = filtered[0].keys().cloned().collect::<Vec<_>>();

    assert_eq!(
        keys,
        vec![
            "cn".to_string(),
            "uid".to_string(),
            "netgroups".to_string(),
            "filegroups".to_string(),
            "dn".to_string(),
        ]
    );
}

#[test]
fn dotted_token_search_falls_back_to_visible_row_text_for_exact_partial_and_escaped_queries_unit() {
    let rows = vec![
        row(json!({"key": "theme.name", "value": "dracula"})),
        row(json!({"key": "theme.path", "value": "/tmp/themes"})),
    ];

    let exact = apply_with_plan(rows.clone(), &compile("theme.name").unwrap())
        .expect("dotted quick should fall back to visible row search");
    assert_eq!(
        exact,
        vec![row(json!({"key": "theme.name", "value": "dracula"}))]
    );

    let partial = apply_with_plan(rows.clone(), &compile("theme.n").unwrap())
        .expect("partial dotted quick should still search row values");
    assert_eq!(
        partial,
        vec![row(json!({"key": "theme.name", "value": "dracula"}))]
    );

    let escaped = apply_with_plan(
        vec![row(json!({"key": "theme.name", "value": "dracula"}))],
        &compile(r#"theme\.name"#).unwrap(),
    )
    .expect("escaped dotted quick should behave like a literal text search");
    assert_eq!(escaped, vec![row(json!({"key": "theme.name"}))]);
}

#[test]
fn path_scoped_quick_preserves_row_and_value_envelopes_unit() {
    let source = row(json!({
        "meta": "people",
        "users": [
            {"uid": "alice", "team": "ops"},
            {"uid": "bob", "team": "eng"}
        ]
    }));
    let rows = vec![source.clone()];

    let projected = apply_with_plan(rows.clone(), &compile("users[1]").unwrap())
        .expect("path quick should project addressed row matches");
    assert_eq!(projected, vec![row(json!({"uid": "bob", "team": "eng"}))]);

    let kept = apply_with_plan(rows.clone(), &compile("?users[0].uid").unwrap())
        .expect("truthy existence should keep the original row");
    assert_eq!(kept, vec![source.clone()]);

    let negated_missing = apply_with_plan(rows.clone(), &compile("!?users[9].uid").unwrap())
        .expect("negated missing existence should keep the original row");
    assert_eq!(negated_missing, vec![source.clone()]);

    let removed = apply_with_plan(rows.clone(), &compile("!users[0].uid").unwrap())
        .expect("negated path quick should remove only the addressed branch");
    assert_eq!(
        removed,
        vec![row(json!({
            "meta": "people",
            "users": [
                {"team": "ops"},
                {"uid": "bob", "team": "eng"}
            ]
        }))]
    );

    let no_match = apply_with_plan(rows, &compile("users[9]").unwrap())
        .expect("missing structural path should simply drop the row");
    assert!(no_match.is_empty());

    let source = json!({
        "meta": "people",
        "users": [
            {"uid": "alice", "team": "ops"},
            {"uid": "bob", "team": "eng"}
        ]
    });
    let projected = apply_value_with_plan(source.clone(), &compile("users[1]").unwrap())
        .expect("path quick should preserve the document envelope for value output");
    assert_eq!(
        projected,
        json!({
            "meta": "people",
            "users": [
                {"uid": "bob", "team": "eng"}
            ]
        })
    );
    let kept = apply_value_with_plan(source.clone(), &compile("?users[0].uid").unwrap())
        .expect("truthy existence should keep the root payload");
    assert_eq!(kept, source);
    let missing = apply_value_with_plan(
        json!({
            "meta": "people",
            "users": [
                {"uid": "alice", "team": "ops"},
                {"uid": "bob", "team": "eng"}
            ]
        }),
        &compile("users[9]").unwrap(),
    )
    .expect("missing structural path should become null in value mode");
    assert_eq!(missing, serde_json::Value::Null);
}

#[test]
fn transform_row_variants_trim_arrays_synthetic_hits_and_squeezed_entries_unit() {
    let mut synthetic = Row::new();
    synthetic.insert("alpha".to_string(), json!([{"name": "bob"}]));
    synthetic.insert("beta".to_string(), json!(["ops"]));
    let mut projection = MatchResult {
        matched: true,
        key_hits: vec!["alpha".to_string()],
        value_hits: Vec::new(),
        is_projection: true,
        synthetic,
    };
    let projected = transform_row(
        &Row::new(),
        &mut projection,
        &compile("alpha").unwrap().spec,
        true,
    )
    .unwrap();
    assert_eq!(
        projected,
        vec![row(json!({"name": "bob"})), row(json!({"beta": ["ops"]}))]
    );
    assert!(squeeze_single_entry(row(json!({"only": []}))).is_empty());

    let flat = row(json!({
        "roles": ["ops", "eng"],
        "title": "ops",
        "city": "Oslo"
    }));
    let mut negated = MatchResult {
        matched: false,
        key_hits: vec!["title".to_string()],
        value_hits: vec!["roles".to_string(), "aliases".to_string()],
        is_projection: false,
        synthetic: row(json!({"aliases": ["ops", "backup"]})),
    };
    let negated_rows = transform_row(&flat, &mut negated, &compile("!ops").unwrap().spec, true)
        .expect("negated quick should keep surviving siblings");
    assert_eq!(
        negated_rows,
        vec![row(json!({
            "roles": ["eng"],
            "aliases": ["backup"],
            "title": "ops",
            "city": "Oslo"
        }))]
    );

    let mut positive = MatchResult {
        matched: true,
        key_hits: vec!["city".to_string()],
        value_hits: vec!["roles".to_string()],
        is_projection: false,
        synthetic: Row::new(),
    };
    let positive_rows = transform_row(&flat, &mut positive, &compile("ops").unwrap().spec, true)
        .expect("positive quick should keep only matching array values");
    assert_eq!(
        positive_rows,
        vec![row(json!({
            "roles": ["ops"],
            "title": "ops",
            "city": "Oslo"
        }))]
    );
}

#[test]
fn quick_helper_functions_cover_parent_prefix_and_value_matching_unit() {
    assert_eq!(
        object_array_parent_prefix("users[1].name"),
        Some("users[1]".to_string())
    );
    assert_eq!(object_array_parent_prefix("users[-1].name"), None);
    assert_eq!(object_array_parent_prefix("users[*].name"), None);

    assert_eq!(last_segment_name("users[1].name"), Some("name".to_string()));
    assert_eq!(last_segment_name("users[0]"), Some("users".to_string()));

    assert!(value_matches_token(
        &json!(["OPS", "eng"]),
        "OPS",
        ExactMode::CaseSensitive,
        false
    ));
    assert!(value_matches_token(
        &json!(["OPS", "eng"]),
        "ops",
        ExactMode::CaseInsensitive,
        false
    ));
    assert!(value_matches_token(
        &json!(["operations"]),
        "oprtns",
        ExactMode::None,
        true
    ));

    let filtered = apply_value(
        json!({
            "team": "ops",
            "meta": {"owner": "alice"}
        }),
        "ops",
    )
    .expect("non-structural quick should use descendant filtering for value payloads");
    assert_eq!(
        filtered,
        json!({
            "team": "ops"
        })
    );
}
