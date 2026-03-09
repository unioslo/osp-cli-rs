use serde_json::json;

use super::{
    MatchResult, apply, compact_sparse_arrays, last_segment_name, match_row, parse_quick_spec,
    squeeze_single_entry, transform_row, value_matches_token,
};
use crate::dsl::eval::flatten::flatten_row;
use crate::dsl::parse::key_spec::ExactMode;

fn row(value: serde_json::Value) -> crate::core::row::Row {
    value.as_object().cloned().expect("object")
}

#[test]
fn quick_matches_keys_and_values_by_default() {
    let rows = vec![
        json!({"uid": "oistes"})
            .as_object()
            .cloned()
            .expect("object"),
        json!({"cn": "Andreas"})
            .as_object()
            .cloned()
            .expect("object"),
    ];

    let output = apply(rows, "oist").expect("quick should work");
    assert_eq!(output.len(), 1);
}

#[test]
fn quick_key_scope_not_equals_works() {
    let rows = vec![
        json!({"uid": "oistes"})
            .as_object()
            .cloned()
            .expect("object"),
        json!({"cn": "Andreas"})
            .as_object()
            .cloned()
            .expect("object"),
    ];

    let output = apply(rows, "K !=uid").expect("quick should work");
    assert_eq!(output.len(), 1);
    assert!(output[0].contains_key("cn"));
}

#[test]
fn quick_value_scope_works() {
    let rows = vec![
        json!({"uid": "oistes"})
            .as_object()
            .cloned()
            .expect("object"),
        json!({"uid": "andreasd"})
            .as_object()
            .cloned()
            .expect("object"),
    ];

    let output = apply(rows, "V oist").expect("quick should work");
    assert_eq!(output.len(), 1);
    assert_eq!(
        output[0].get("uid").and_then(|v| v.as_str()),
        Some("oistes")
    );
}

#[test]
fn quick_projects_exact_key_matches() {
    let rows = vec![
        json!({"uid": "oistes", "cn": "Oistein"})
            .as_object()
            .cloned()
            .expect("object"),
        json!({"uid": "andreasd", "cn": "Andreas"})
            .as_object()
            .cloned()
            .expect("object"),
    ];

    let output = apply(rows, "K uid").expect("quick should work");
    assert_eq!(output.len(), 2);
    assert!(output.iter().all(|row| row.contains_key("uid")));
    assert!(output.iter().all(|row| !row.contains_key("cn")));
}

#[test]
fn quick_requires_non_empty_search_token() {
    let err = apply(
        vec![
            json!({"uid": "oistes"})
                .as_object()
                .cloned()
                .expect("object"),
        ],
        "   ",
    )
    .expect_err("empty quick token should fail");

    assert!(
        err.to_string()
            .contains("quick stage requires a search token")
    );
}

#[test]
fn quick_existence_and_negated_existence_filter_multi_rows() {
    let rows = vec![
        json!({"uid": "oistes"})
            .as_object()
            .cloned()
            .expect("object"),
        json!({"cn": "Oistein"})
            .as_object()
            .cloned()
            .expect("object"),
    ];

    let present = apply(rows.clone(), "?uid").expect("existence check should work");
    assert_eq!(present.len(), 1);
    assert!(present[0].contains_key("uid"));

    let missing = apply(rows, "!?uid").expect("negated existence check should work");
    assert_eq!(missing.len(), 1);
    assert!(missing[0].contains_key("cn"));
}

#[test]
fn quick_negated_value_scope_removes_matching_array_items_on_single_row() {
    let rows = vec![
        json!({"uid": "oistes", "groups": ["ops", "admins"]})
            .as_object()
            .cloned()
            .expect("object"),
    ];

    let output = apply(rows, "V !ops").expect("single-row negated value stage should work");
    assert_eq!(output.len(), 1);
    assert_eq!(
        output[0].get("uid").and_then(|value| value.as_str()),
        Some("oistes")
    );
    let groups = output[0]
        .get("groups")
        .and_then(|value| value.as_array())
        .expect("groups array should remain present");
    assert!(groups.iter().any(|value| value == "admins"));
    assert!(groups.iter().all(|value| value != "ops"));
}

#[test]
fn quick_negated_single_row_projection_removes_matching_field() {
    let rows = vec![
        json!({"uid": "alice", "cn": "Alice Example"})
            .as_object()
            .cloned()
            .expect("object"),
    ];

    let output = apply(rows, "!uid").expect("negated quick should work");
    assert_eq!(output.len(), 1);
    assert!(!output[0].contains_key("uid"));
    assert_eq!(
        output[0].get("cn").and_then(|value| value.as_str()),
        Some("Alice Example")
    );
}

#[test]
fn quick_value_scope_filters_matching_items_inside_arrays() {
    let rows = vec![
        json!({"groups": ["ops", "eng", "sales"]})
            .as_object()
            .cloned()
            .expect("object"),
    ];

    let output = apply(rows, "V eng").expect("array quick filter should work");
    assert_eq!(output[0].get("groups"), Some(&json!(["eng"])));
}

#[test]
fn quick_projects_nested_path_matches_from_single_row() {
    let rows = vec![
        json!({"person": {"name": "Alice Example", "mail": "a@example.org"}})
            .as_object()
            .cloned()
            .expect("object"),
    ];

    let output = apply(rows, "name").expect("quick should project nested path matches");
    assert_eq!(
        output,
        vec![
            json!({"person": {"name": "Alice Example"}})
                .as_object()
                .cloned()
                .expect("object")
        ]
    );
}

#[test]
fn quick_exact_case_modes_respect_prefix_rules() {
    let rows = vec![
        json!({"uid": "Alice"})
            .as_object()
            .cloned()
            .expect("object"),
    ];

    let output = apply(rows.clone(), "=alice").expect("case-insensitive exact quick should work");
    assert_eq!(output.len(), 1);

    let output = apply(rows, "==alice").expect("case-sensitive exact quick should work");
    assert!(output.is_empty());
}

#[test]
fn quick_multi_row_negated_value_scope_keeps_only_non_matching_rows() {
    let rows = vec![
        json!({"uid": "alice", "groups": ["ops"]})
            .as_object()
            .cloned()
            .expect("object"),
        json!({"uid": "bob", "groups": ["eng"]})
            .as_object()
            .cloned()
            .expect("object"),
    ];

    let output = apply(rows, "V !ops").expect("negated multi-row value quick should work");
    assert_eq!(output.len(), 1);
    assert_eq!(output[0].get("uid"), Some(&json!("bob")));
}

#[test]
fn quick_single_row_key_scope_keeps_matching_key_without_projection_split() {
    let rows = vec![
        json!({"uid": "alice", "cn": "Alice Example"})
            .as_object()
            .cloned()
            .expect("object"),
    ];

    let output = apply(rows, "K cn").expect("key-only quick should work");
    assert_eq!(output.len(), 1);
    assert_eq!(output[0].get("cn"), Some(&json!("Alice Example")));
    assert!(!output[0].contains_key("uid"));
}

#[test]
fn quick_single_row_value_projection_squeezes_single_object_array() {
    let rows = vec![
        json!({"interfaces": [{"mac": "aa:bb", "name": "eth0"}]})
            .as_object()
            .cloned()
            .expect("object"),
    ];

    let output = apply(rows, "V aa:bb").expect("value projection should work");
    assert_eq!(output.len(), 1);
    assert_eq!(
        output[0],
        json!({"interfaces": [{"mac": "aa:bb"}]})
            .as_object()
            .cloned()
            .expect("object")
    );
}

#[test]
fn quick_value_scope_projects_synthetic_rows_and_drops_empty_results_unit() {
    let original = row(json!({"person": {"name": "Alice", "mail": "a@example.org"}}));
    let flat = flatten_row(&original);
    let spec = parse_quick_spec("V person");
    let mut projected_result = MatchResult {
        matched: true,
        key_hits: Vec::new(),
        value_hits: Vec::new(),
        is_projection: true,
        synthetic: row(json!({
            "person.name": "Alice",
            "person.mail": "a@example.org"
        })),
    };
    let projected =
        transform_row(&flat, &mut projected_result, &spec).expect("synthetic projection works");
    assert_eq!(
        projected,
        vec![
            row(json!({"mail": "a@example.org"})),
            row(json!({"name": "Alice"})),
        ]
    );

    let mut empty_result = MatchResult {
        matched: true,
        key_hits: Vec::new(),
        value_hits: Vec::new(),
        is_projection: true,
        synthetic: crate::core::row::Row::new(),
    };
    assert!(transform_row(&flat, &mut empty_result, &spec).is_none());
}

#[test]
fn quick_negated_value_scope_handles_nested_arrays_and_synthetic_matches_unit() {
    let nested = apply(
        vec![row(json!({"groups": ["ops", "admins"], "uid": "alice"}))],
        "V !ops",
    )
    .expect("negated nested value removal should work");
    assert_eq!(nested[0].get("groups"), Some(&json!([null, "admins"])));

    let synthetic = apply(
        vec![row(json!({"profile": {"groups": ["ops", "eng"]}}))],
        "V !ops",
    )
    .expect("negated synthetic value removal should work");
    assert_eq!(
        synthetic,
        vec![row(json!({"profile": {"groups": [null, "eng"]}}))]
    );

    let removed = apply(vec![row(json!({"profile": {"groups": ["ops"]}}))], "V !ops")
        .expect("fully removed synthetic values should succeed");
    assert!(removed.is_empty());
}

#[test]
fn quick_helper_functions_cover_sparse_arrays_and_segment_fallbacks_unit() {
    assert!(value_matches_token(
        &json!(["ops", "eng"]),
        "eng",
        ExactMode::CaseSensitive
    ));
    assert!(value_matches_token(
        &json!(["OPS"]),
        "ops",
        ExactMode::CaseInsensitive
    ));
    assert!(value_matches_token(
        &json!(["Platform Ops"]),
        "ops",
        ExactMode::None
    ));

    assert_eq!(
        last_segment_name("profile.groups[0]"),
        Some("groups".to_string())
    );
    assert_eq!(last_segment_name("plain"), Some("plain".to_string()));

    assert_eq!(
        squeeze_single_entry(row(json!({"items": [{"uid": "alice"}]}))),
        row(json!({"uid": "alice"}))
    );
    assert_eq!(
        squeeze_single_entry(row(json!({"items": []}))),
        crate::core::row::Row::new()
    );

    let mut sparse = json!({
        "groups": [null, "ops", null],
        "profile": {"members": [null, "alice"]}
    });
    compact_sparse_arrays(&mut sparse);
    assert_eq!(
        sparse,
        json!({"groups": ["ops"], "profile": {"members": ["alice"]}})
    );

    let spec = parse_quick_spec("K uid");
    assert_eq!(spec.key_spec.token, "uid");
}

#[test]
fn quick_matching_is_unicode_case_insensitive_for_keys_and_values() {
    let value_matches = apply(
        vec![
            row(json!({"name": "Øystein"})),
            row(json!({"name": "Alice"})),
        ],
        "V øys",
    )
    .expect("unicode value search should work");
    assert_eq!(value_matches, vec![row(json!({"name": "Øystein"}))]);

    let key_matches = apply(
        vec![row(json!({"Grønn": true})), row(json!({"status": true}))],
        "K grønn",
    )
    .expect("unicode key search should work");
    assert_eq!(key_matches, vec![row(json!({"Grønn": true}))]);
}

#[test]
fn quick_match_and_transform_helpers_cover_scalar_negated_and_filtered_paths_unit() {
    let original = row(json!({
        "uid": "alice",
        "groups": ["ops", "eng"],
        "title": "Platform Ops"
    }));
    let flat = flatten_row(&original);

    let scalar_spec = parse_quick_spec("V ops");
    let scalar_pairs = vec![("title".to_string(), json!("Platform Ops"))];
    let scalar_match = match_row(
        &flat,
        &scalar_pairs,
        crate::core::row::Row::new(),
        &scalar_spec,
    );
    assert!(scalar_match.matched);
    assert_eq!(scalar_match.value_hits, vec!["title".to_string()]);

    let negated_spec = parse_quick_spec("V !ops");
    let mut negated = MatchResult {
        matched: true,
        key_hits: vec!["uid".to_string()],
        value_hits: vec!["groups".to_string()],
        is_projection: false,
        synthetic: row(json!({
            "profile.groups": ["ops", "eng"],
            "profile.name": "ops"
        })),
    };
    let negated_rows =
        transform_row(&flat, &mut negated, &negated_spec).expect("negated row should remain");
    assert_eq!(
        negated_rows,
        vec![row(json!({
            "groups": ["ops", "eng"],
            "profile": {"groups": ["ops", "eng"], "name": "ops"},
            "title": "Platform Ops"
        }))]
    );

    let positive_spec = parse_quick_spec("V eng");
    let mut positive = MatchResult {
        matched: true,
        key_hits: vec!["missing".to_string()],
        value_hits: vec!["groups".to_string()],
        is_projection: false,
        synthetic: row(json!({"groups": ["ops", "eng"]})),
    };
    let filtered =
        transform_row(&flat, &mut positive, &positive_spec).expect("positive filter should work");
    assert_eq!(filtered, vec![row(json!({"groups": ["eng"]}))]);
}

#[test]
fn quick_helper_edge_cases_cover_fallback_names_and_scalar_squeeze_unit() {
    assert_eq!(last_segment_name("broken["), Some("broken".to_string()));

    let multi = row(json!({"left": 1, "right": 2}));
    assert_eq!(squeeze_single_entry(multi.clone()), multi);

    let scalar_only = row(json!({"value": [1, null]}));
    assert_eq!(
        squeeze_single_entry(scalar_only),
        row(json!({"value": [1]}))
    );

    let direct_object = row(json!({"value": {"uid": "alice"}}));
    assert_eq!(
        squeeze_single_entry(direct_object),
        row(json!({"uid": "alice"}))
    );

    let literal = row(json!({"value": "alice"}));
    assert_eq!(squeeze_single_entry(literal.clone()), literal);
}

#[test]
fn quick_projection_and_negation_helpers_cover_empty_synthetic_and_array_removal_unit() {
    let projection_spec = parse_quick_spec("V profile");
    let flat_projection = crate::core::row::Row::new();
    let mut projection = MatchResult {
        matched: true,
        key_hits: Vec::new(),
        value_hits: Vec::new(),
        is_projection: true,
        synthetic: row(json!({"profile.groups": [null, null]})),
    };
    let projected = transform_row(&flat_projection, &mut projection, &projection_spec)
        .expect("empty synthetic rows should fall back to grouped projection");
    assert_eq!(projected, vec![row(json!({"groups": [null, null]}))]);

    let negated_spec = parse_quick_spec("V !ops");
    let flat_array = row(json!({"groups": ["ops", "eng"], "uid": "alice"}));
    let mut negated_flat = MatchResult {
        matched: true,
        key_hits: Vec::new(),
        value_hits: vec!["groups".to_string()],
        is_projection: false,
        synthetic: crate::core::row::Row::new(),
    };
    let flat_removed = transform_row(&flat_array, &mut negated_flat, &negated_spec)
        .expect("negated flat arrays should keep non-matching values");
    assert_eq!(
        flat_removed,
        vec![row(json!({"groups": ["eng"], "uid": "alice"}))]
    );

    let flat_scalar = row(json!({"uid": "alice"}));
    let mut negated_synthetic = MatchResult {
        matched: true,
        key_hits: Vec::new(),
        value_hits: vec!["groups".to_string(), "role".to_string()],
        is_projection: false,
        synthetic: row(json!({"groups": ["ops", "eng"], "role": "ops"})),
    };
    let synthetic_removed = transform_row(&flat_scalar, &mut negated_synthetic, &negated_spec)
        .expect("negated synthetic arrays should merge survivors back into the row");
    assert_eq!(
        synthetic_removed,
        vec![row(json!({"groups": ["eng"], "uid": "alice"}))]
    );
}

#[test]
fn quick_filter_helpers_cover_array_pair_matches_and_empty_results_unit() {
    let flat = row(json!({"uid": "alice"}));
    let spec = parse_quick_spec("V eng");
    let pair_match = match_row(
        &flat,
        &[("groups".to_string(), json!(["ops", "eng"]))],
        crate::core::row::Row::new(),
        &spec,
    );
    assert!(pair_match.matched);
    assert_eq!(pair_match.value_hits, vec!["groups".to_string()]);

    let filtered_flat = row(json!({"groups": ["ops"]}));
    let mut filtered = MatchResult {
        matched: true,
        key_hits: Vec::new(),
        value_hits: vec!["groups".to_string()],
        is_projection: false,
        synthetic: crate::core::row::Row::new(),
    };
    assert!(transform_row(&filtered_flat, &mut filtered, &spec).is_none());
}

#[test]
fn quick_sparse_array_compaction_keeps_all_null_lists_intact_unit() {
    let mut value = json!({
        "groups": [null, null],
        "profile": {"members": [null]}
    });
    compact_sparse_arrays(&mut value);
    assert_eq!(
        value,
        json!({
            "groups": [null, null],
            "profile": {"members": [null]}
        })
    );
}
