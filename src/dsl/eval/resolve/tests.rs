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
fn relative_path_resolution_requires_structural_matches_unit() {
    let row = json!({"metadata": {"asset": {"id": 42}}, "id": 7})
        .as_object()
        .cloned()
        .expect("object");
    assert!(resolve_values(&row, "asset.id", ExactMode::None).is_empty());
    assert!(resolve_values(&row, ".asset.id", ExactMode::None).is_empty());
    assert_eq!(resolve_values(&row, "id", ExactMode::None), vec![json!(7)]);

    let root = json!({
        "metadata": {"asset": {"id": 42}},
        "items": [{"asset": {"id": 7}}],
        "sections": [{"entries": [{"name": "help"}]}],
    });
    assert!(resolve_path_matches(&root, "asset.id", ExactMode::None).is_empty());
    assert!(resolve_path_matches(&root, "entries[0].name", ExactMode::None).is_empty());
}

#[test]
fn path_evaluation_and_exact_address_helpers_cover_fanout_slices_and_indexes_unit() {
    let root = json!({"items": [{"id": 1}, {"id": 2}, {"id": 3}]});

    for (spec, expected) in [
        ("items[].id", vec![json!(1), json!(2), json!(3)]),
        ("items[:2].id", vec![json!(1), json!(2)]),
        ("items[::-1].id", vec![json!(3), json!(2), json!(1)]),
        ("items[-1].id", vec![json!(3)]),
    ] {
        let path = parse_path(spec).expect("path should parse");
        assert_eq!(evaluate_path(&root, &path), expected, "spec={spec}");
    }

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

    for (spec, expected) in [
        ("sections[1].entries[0].name", true),
        ("sections[].entries[0].name", false),
        ("sections[:1].entries[0].name", false),
        (".sections.entries.name", true),
        ("sections.entries.name", false),
    ] {
        let path = parse_path(spec).expect("path should parse");
        assert_eq!(is_exact_address_path(&path), expected, "spec={spec}");
    }
}

#[test]
fn slice_and_truthiness_helpers_cover_edge_cases_unit() {
    assert_eq!(slice_indices(5, Some(1), Some(4), Some(1)), vec![1, 2, 3]);
    assert_eq!(slice_indices(5, None, None, Some(-1)), vec![4, 3, 2, 1, 0]);
    assert_eq!(slice_indices(5, Some(-3), None, Some(1)), vec![2, 3, 4]);
    assert_eq!(slice_indices(0, None, None, Some(-1)), Vec::<i64>::new());
    assert_eq!(slice_indices(5, None, None, Some(0)), Vec::<i64>::new());

    assert!(!is_truthy(&json!(null)));
    assert!(!is_truthy(&json!("")));
    assert!(!is_truthy(&json!([])));
    assert!(is_truthy(&json!("x")));
    assert!(is_truthy(&json!([1])));
}

#[test]
fn resolve_pair_helpers_cover_deduplication_flat_fallback_and_materialization_unit() {
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

    let fallback_flat = json!({
        "key": "theme.name",
        "value": "dracula"
    })
    .as_object()
    .cloned()
    .expect("object");
    let (pairs, materialized) = resolve_pairs(&fallback_flat, "theme.name");
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
fn path_match_materialization_tracks_addresses_and_selected_nulls_unit() {
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

    let rebuild_source = json!({
        "title": "Deploy",
        "sections": [
            {"entries": [{"name": "start"}, {"name": "stop"}]},
            {"entries": [{"name": "restart"}]}
        ]
    });
    let exact_path = parse_path("sections[1].entries[0].name").expect("path should parse");
    let mut projected =
        materialize_path_matches(&enumerate_path_matches(&rebuild_source, &exact_path));
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

    let sparse_root = json!({"items": [0, null, 2]});
    let mut sparse_matches = enumerate_path_matches(
        &sparse_root,
        &parse_path("items[1]").expect("path should parse"),
    );
    sparse_matches.extend(enumerate_path_matches(
        &sparse_root,
        &parse_path("items[2]").expect("path should parse"),
    ));
    let mut compacted = materialize_path_matches(&sparse_matches);
    compact_sparse_arrays(&mut compacted);
    assert_eq!(compacted, json!({"items": [null, 2]}));
}
