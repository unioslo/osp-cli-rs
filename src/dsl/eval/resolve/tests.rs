use serde_json::{Value, json};

use crate::dsl::parse::{key_spec::ExactMode, path::parse_path};

use super::{
    AddressStep, compact_sparse_arrays, enumerate_path_matches, enumerate_path_values,
    evaluate_path, is_exact_address_path, is_sparse_hole, is_truthy, materialize_path_matches,
    resolve_first_value, resolve_pairs, resolve_path_matches, resolve_values, slice_indices,
};

fn normalize_sparse_holes(value: &mut Value) {
    if is_sparse_hole(value) {
        *value = Value::Null;
        return;
    }

    match value {
        Value::Array(items) => {
            for item in items {
                normalize_sparse_holes(item);
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                normalize_sparse_holes(item);
            }
        }
        _ => {}
    }
}

#[test]
fn resolve_values_uses_direct_path_only_for_structural_tokens() {
    let row = json!({"metadata": {"asset": {"id": 42}}, "id": 7})
        .as_object()
        .cloned()
        .expect("object");

    let values = resolve_values(&row, "asset.id", ExactMode::None);
    assert!(values.is_empty());

    let values = resolve_values(&row, ".asset.id", ExactMode::None);
    assert!(values.is_empty());

    let values = resolve_values(&row, "id", ExactMode::None);
    assert_eq!(values, vec![json!(7)]);
}

#[test]
fn evaluate_path_handles_fanout_and_slice() {
    let root = json!({"items": [{"id": 1}, {"id": 2}, {"id": 3}]});

    let path = parse_path("items[].id").expect("path should parse");
    assert_eq!(
        evaluate_path(&root, &path),
        vec![json!(1), json!(2), json!(3)]
    );

    let path = parse_path("items[:2].id").expect("path should parse");
    assert_eq!(evaluate_path(&root, &path), vec![json!(1), json!(2)]);

    let path = parse_path("items[::-1].id").expect("path should parse");
    assert_eq!(
        evaluate_path(&root, &path),
        vec![json!(3), json!(2), json!(1)]
    );
}

#[test]
fn slice_indices_handles_forward_and_reverse_ranges_consistently() {
    assert_eq!(slice_indices(5, Some(1), Some(4), Some(1)), vec![1, 2, 3]);
    assert_eq!(slice_indices(5, None, None, Some(-1)), vec![4, 3, 2, 1, 0]);
    assert_eq!(slice_indices(5, Some(-3), None, Some(1)), vec![2, 3, 4]);
    assert_eq!(slice_indices(0, None, None, Some(-1)), Vec::<i64>::new());
    assert_eq!(slice_indices(5, None, None, Some(0)), Vec::<i64>::new());
}

#[test]
fn truthy_rules_match_dsl_expectations() {
    assert!(!is_truthy(&json!(null)));
    assert!(!is_truthy(&json!("")));
    assert!(!is_truthy(&json!([])));
    assert!(is_truthy(&json!("x")));
    assert!(is_truthy(&json!([1])));
}

#[test]
fn resolve_first_value_unwraps_arrays_and_dedups_results() {
    let row = json!({
        "items": [{"id": 7}, {"id": 7}],
        "dup": [1, 1]
    })
    .as_object()
    .cloned()
    .expect("object");

    assert_eq!(
        resolve_first_value(&row, "items[].id", ExactMode::None),
        Some(json!(7))
    );
    assert_eq!(
        resolve_values(&row, "dup[]", ExactMode::None),
        vec![json!(1)]
    );
}

#[test]
fn resolve_pairs_handles_path_flat_fallback_and_full_materialization() {
    let flat = json!({
        "items[0].id": 1,
        "items[1].id": 2,
        "flat.value": "x"
    })
    .as_object()
    .cloned()
    .expect("object");

    let (path_pairs, materialized) = resolve_pairs(&flat, "items[].id");
    assert!(!materialized);
    assert_eq!(
        path_pairs,
        vec![
            ("items[0].id".to_string(), json!(1)),
            ("items[1].id".to_string(), json!(2))
        ]
    );

    let (materialized_pairs, materialized) = resolve_pairs(&flat, "items[-1].id");
    assert!(materialized);
    assert_eq!(
        materialized_pairs,
        vec![("items[1].id".to_string(), json!(2))]
    );

    let (flat_pairs, materialized) = resolve_pairs(&flat, "flat.value");
    assert!(!materialized);
    assert_eq!(flat_pairs, vec![("flat.value".to_string(), json!("x"))]);

    let (fallback_pairs, materialized) = resolve_pairs(&flat, "missing");
    assert!(materialized);
    assert_eq!(fallback_pairs.len(), 3);
}

#[test]
fn resolve_pairs_falls_back_after_missing_structural_token_on_flat_rows() {
    let flat = json!({
        "key": "theme.name",
        "value": "dracula"
    })
    .as_object()
    .cloned()
    .expect("object");

    let (pairs, materialized) = resolve_pairs(&flat, "theme.name");
    assert!(materialized);
    assert_eq!(
        pairs,
        vec![
            ("key".to_string(), json!("theme.name")),
            ("value".to_string(), json!("dracula"))
        ]
    );
}

#[test]
fn enumerate_paths_and_selectors_cover_negative_indexes() {
    let root = json!({"items": [{"id": 1}, {"id": 2}, {"id": 3}]});
    let path = parse_path("items[-1].id").expect("path should parse");
    assert_eq!(evaluate_path(&root, &path), vec![json!(3)]);

    let enumerated = enumerate_path_values(
        &root,
        &parse_path("items[:2].id").expect("path should parse"),
    );
    assert_eq!(
        enumerated,
        vec![
            ("items[0].id".to_string(), json!(1)),
            ("items[1].id".to_string(), json!(2))
        ]
    );
}

#[test]
fn enumerate_path_matches_tracks_addresses_for_nested_selectors() {
    let root = json!({"sections": [{"entries": [{"name": "help"}, {"name": "exit"}]}]});
    let path = parse_path("sections[].entries[1].name").expect("path should parse");

    let matches = enumerate_path_matches(&root, &path);
    assert_eq!(matches.len(), 1);
    assert_eq!(
        matches[0].address,
        vec![
            AddressStep::Field("sections".to_string()),
            AddressStep::Index(0),
            AddressStep::Field("entries".to_string()),
            AddressStep::Index(1),
            AddressStep::Field("name".to_string()),
        ]
    );
    assert_eq!(matches[0].flat_key, "sections[0].entries[1].name");
    assert_eq!(matches[0].value, json!("exit"));
}

#[test]
fn materialize_path_matches_rebuilds_exact_nested_projection() {
    let root = json!({
        "title": "Deploy",
        "sections": [
            {"entries": [{"name": "start"}, {"name": "stop"}]},
            {"entries": [{"name": "restart"}]}
        ]
    });
    let path = parse_path("sections[1].entries[0].name").expect("path should parse");

    let mut projected = materialize_path_matches(&enumerate_path_matches(&root, &path));
    normalize_sparse_holes(&mut projected);
    assert_eq!(
        projected,
        json!({
            "sections": [
                null,
                {"entries": [{"name": "restart"}]}
            ]
        })
    );
}

#[test]
fn exact_address_path_accepts_indexed_selectors_and_rejects_fanout() {
    assert!(is_exact_address_path(
        &parse_path("sections[1].entries[0].name").expect("path should parse")
    ));
    assert!(!is_exact_address_path(
        &parse_path("sections[].entries[0].name").expect("path should parse")
    ));
    assert!(!is_exact_address_path(
        &parse_path("sections[:1].entries[0].name").expect("path should parse")
    ));
    assert!(is_exact_address_path(
        &parse_path(".sections.entries.name").expect("path should parse")
    ));
    assert!(!is_exact_address_path(
        &parse_path("sections.entries.name").expect("path should parse")
    ));
}

#[test]
fn resolve_path_matches_do_not_fall_back_to_relative_flattened_key_matching() {
    let root = json!({
        "metadata": {"asset": {"id": 42}},
        "items": [{"asset": {"id": 7}}]
    });

    let matches = resolve_path_matches(&root, "asset.id", ExactMode::None);
    assert!(matches.is_empty());
}

#[test]
fn resolve_path_matches_require_full_relative_path_for_indexed_selectors() {
    let root = json!({
        "sections": [{"entries": [{"name": "help"}]}]
    });

    let matches = resolve_path_matches(&root, "entries[0].name", ExactMode::None);
    assert!(matches.is_empty());
}

#[test]
fn compact_sparse_arrays_preserves_selected_null_values() {
    let root = json!({"items": [0, null, 2]});
    let mut matches =
        enumerate_path_matches(&root, &parse_path("items[1]").expect("path should parse"));
    matches.extend(enumerate_path_matches(
        &root,
        &parse_path("items[2]").expect("path should parse"),
    ));

    let mut projected = materialize_path_matches(&matches);
    compact_sparse_arrays(&mut projected);

    assert_eq!(projected, json!({"items": [null, 2]}));
}
