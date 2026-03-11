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
fn compile_treats_prefixed_path_quick_as_structural_selector() {
    let plan = compile("!sections[0].entries[0]").expect("quick should compile");

    assert!(plan.spec.is_structural());
}

#[test]
fn compile_rejects_empty_and_unsupported_fuzzy_forms_unit() {
    assert!(compile("   ").is_err());
    assert!(compile("% ?uid").is_err());
    assert!(compile("% ==uid").is_err());
    assert!(compile("% sections[0].uid").is_err());
}

#[test]
fn negated_single_row_quick_removes_matching_array_members_unit() {
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
}

#[test]
fn key_only_multi_row_quick_filters_without_projecting_rows_unit() {
    let rows = vec![
        row(json!({"uid": "alice", "team": "ops"})),
        row(json!({"name": "bob", "team": "eng"})),
    ];

    let filtered = apply_with_plan(rows, &compile("K uid").unwrap()).expect("quick should work");
    assert_eq!(filtered, vec![row(json!({"uid": "alice"}))]);
}

#[test]
fn key_not_equals_matches_rows_with_nonmatching_keys_unit() {
    let row = row(json!({"uid": "alice", "team": "ops"}));
    let flat = flatten_row(&row);
    let plan = compile("K !=uid").expect("quick should compile");
    let (pairs, _) = resolve_pairs(&flat, plan.spec.token());
    let synthetic = build_synthetic_map(&pairs, &flat);
    let result = match_row(&flat, &pairs, synthetic, &plan.spec);

    assert!(plan.spec.key_not_equals);
    assert!(result.matched);
}

#[test]
fn stream_rows_with_plan_preserves_errors_and_uses_multi_row_mode_unit() {
    let plan = compile("uid").expect("quick should compile");
    let mut iter = stream_rows_with_plan(
        vec![
            Err(anyhow!("boom")),
            Ok(row(json!({"uid": "alice"}))),
            Err(anyhow!("later")),
        ],
        plan,
    );

    assert!(iter.next().expect("first item").is_err());
    assert_eq!(
        iter.next().expect("second item").expect("row"),
        row(json!({"uid": "alice"}))
    );
    assert!(iter.next().expect("third item").is_err());
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
fn positive_single_row_quick_filters_matching_array_values_unit() {
    let rows = vec![row(json!({
        "uid": "alice",
        "roles": ["eng", "ops"],
        "city": "Oslo"
    }))];

    let filtered = apply_with_plan(rows, &compile("ops").unwrap()).expect("quick should work");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].get("uid"), Some(&json!("alice")));
    assert_eq!(filtered[0].get("city"), Some(&json!("Oslo")));
    let roles = filtered[0]
        .get("roles")
        .and_then(|value| value.as_array())
        .expect("roles should remain an array");
    assert!(roles.iter().any(|value| value == "ops"));
}

#[test]
fn existence_quick_respects_positive_and_negated_matches_unit() {
    let rows = vec![row(json!({"uid": "alice", "team": "ops"}))];

    assert_eq!(
        apply_with_plan(rows.clone(), &compile("?uid").unwrap()).unwrap(),
        rows
    );
    assert_eq!(
        apply_with_plan(rows, &compile("!?missing").unwrap()).unwrap(),
        vec![row(json!({"uid": "alice", "team": "ops"}))]
    );
}

#[test]
fn stream_rows_with_plan_preserves_second_seed_error_unit() {
    let plan = compile("uid").expect("quick should compile");
    let mut iter = stream_rows_with_plan(
        vec![Ok(row(json!({"uid": "alice"}))), Err(anyhow!("boom"))],
        plan,
    );

    assert_eq!(
        iter.next().expect("first item").expect("row"),
        row(json!({"uid": "alice"}))
    );
    assert!(iter.next().expect("second item").is_err());
}

#[test]
fn path_scoped_row_quick_preserves_row_value_divergence_unit() {
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
}

#[test]
fn path_scoped_value_quick_preserves_document_envelope_unit() {
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
fn transform_row_projection_prefers_synthetic_rows_and_squeezes_single_entries_unit() {
    let mut synthetic = Row::new();
    synthetic.insert("alpha".to_string(), json!([{"name": "bob"}]));
    synthetic.insert("beta".to_string(), json!(["ops"]));
    let mut result = MatchResult {
        matched: true,
        key_hits: vec!["alpha".to_string()],
        value_hits: Vec::new(),
        is_projection: true,
        synthetic,
    };

    let projected =
        transform_row(&Row::new(), &mut result, &compile("alpha").unwrap().spec).unwrap();
    assert_eq!(
        projected,
        vec![row(json!({"name": "bob"})), row(json!({"beta": ["ops"]})),]
    );

    let squeezed = squeeze_single_entry(row(json!({"only": []})));
    assert!(squeezed.is_empty());
}

#[test]
fn transform_row_negated_and_positive_filter_paths_trim_arrays_and_synthetic_hits_unit() {
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

    let negated_rows = transform_row(&flat, &mut negated, &compile("!ops").unwrap().spec)
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
    let positive_rows = transform_row(&flat, &mut positive, &compile("ops").unwrap().spec)
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
