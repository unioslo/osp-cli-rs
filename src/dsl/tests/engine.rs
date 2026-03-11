use crate::core::output::OutputFormat;
use crate::core::output_model::{
    OutputDocument, OutputDocumentKind, OutputItems, OutputResult, RenderRecommendation,
};
use crate::guide::GuideView;
use serde_json::json;

use super::{apply_output_pipeline, apply_pipeline, execute_pipeline, execute_pipeline_streaming};

fn output_rows(output: &OutputResult) -> &[crate::core::row::Row] {
    output.as_rows().expect("expected row output")
}

#[test]
fn project_then_filter_pipeline_works() {
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

    let stages = vec!["P uid,cn".to_string(), "F uid=oistes".to_string()];
    let output = apply_pipeline(rows, &stages).expect("pipeline should pass");

    assert_eq!(output_rows(&output).len(), 1);
    assert_eq!(
        output_rows(&output)[0]
            .get("uid")
            .and_then(|value| value.as_str()),
        Some("oistes")
    );
}

#[test]
fn bare_quick_stage_without_verb_still_works() {
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

    let stages = vec!["oist".to_string()];
    let output = apply_pipeline(rows, &stages).expect("pipeline should pass");
    assert_eq!(output_rows(&output).len(), 1);
}

#[test]
fn unknown_single_letter_verb_errors() {
    let rows = vec![
        json!({"uid": "oistes"})
            .as_object()
            .cloned()
            .expect("object"),
    ];

    let err = apply_pipeline(rows, &["R oist".to_string()]).expect_err("unknown verb should fail");
    assert!(err.to_string().contains("unknown DSL verb"));
}

#[test]
fn copy_stage_sets_meta_flag() {
    let rows = vec![
        json!({"uid": "oistes"})
            .as_object()
            .cloned()
            .expect("object"),
    ];

    let stages = vec!["Y".to_string()];
    let output = execute_pipeline(rows, &stages).expect("pipeline should pass");

    assert!(output.meta.wants_copy);
}

#[test]
fn value_scope_alias_filters_by_value() {
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

    let stages = vec!["V oist".to_string()];
    let output = apply_pipeline(rows, &stages).expect("pipeline should pass");
    assert_eq!(output_rows(&output).len(), 1);
    assert_eq!(
        output_rows(&output)[0]
            .get("uid")
            .and_then(|value| value.as_str()),
        Some("oistes")
    );
}

#[test]
fn question_stage_cleans_empty_fields() {
    let rows = vec![
        json!({"uid": "oistes", "note": "", "tags": []})
            .as_object()
            .cloned()
            .expect("object"),
        json!({"uid": "andreasd", "note": "ok", "extra": null})
            .as_object()
            .cloned()
            .expect("object"),
    ];

    let output = apply_pipeline(rows, &["?".to_string()]).expect("pipeline should pass");
    assert_eq!(output_rows(&output).len(), 2);
    assert!(output_rows(&output)[0].contains_key("uid"));
    assert!(!output_rows(&output)[0].contains_key("note"));
    assert!(!output_rows(&output)[0].contains_key("tags"));
    assert!(output_rows(&output)[1].contains_key("note"));
    assert!(!output_rows(&output)[1].contains_key("extra"));
}

#[test]
fn question_stage_with_spec_filters_existence() {
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

    let output = apply_pipeline(rows, &["? uid".to_string()]).expect("pipeline should pass");
    assert_eq!(output_rows(&output).len(), 1);
    assert!(output_rows(&output)[0].contains_key("uid"));
}

#[test]
fn streaming_executor_matches_eager_for_streamable_row_pipeline() {
    let rows = vec![
        json!({"uid": "alice", "active": true, "members": ["a", "b"]})
            .as_object()
            .cloned()
            .expect("object"),
        json!({"uid": "bob", "active": false, "members": ["c"]})
            .as_object()
            .cloned()
            .expect("object"),
    ];
    let stages = vec![
        "F active=true".to_string(),
        "P uid,members[]".to_string(),
        "L 2".to_string(),
    ];

    let eager = apply_pipeline(rows.clone(), &stages).expect("eager pipeline should pass");
    let streaming =
        execute_pipeline_streaming(rows, &stages).expect("streaming pipeline should pass");

    assert_eq!(streaming, eager);
}

#[test]
fn streaming_executor_matches_eager_for_quick_hot_path() {
    let rows = vec![
        json!({"uid": "alice", "mail": "alice@example.org"})
            .as_object()
            .cloned()
            .expect("object"),
        json!({"uid": "bob", "mail": "bob@example.org"})
            .as_object()
            .cloned()
            .expect("object"),
        json!({"uid": "carol", "mail": "carol@example.org"})
            .as_object()
            .cloned()
            .expect("object"),
    ];
    let stages = vec!["alice".to_string()];

    let eager = apply_pipeline(rows.clone(), &stages).expect("eager pipeline should pass");
    let streaming =
        execute_pipeline_streaming(rows, &stages).expect("streaming pipeline should pass");

    assert_eq!(streaming, eager);
}

#[test]
fn streaming_executor_preserves_single_row_quick_magic() {
    let rows = vec![
        json!({"uid": "alice", "members": ["eng", "ops"]})
            .as_object()
            .cloned()
            .expect("object"),
    ];
    let stages = vec!["members".to_string()];

    let eager = apply_pipeline(rows.clone(), &stages).expect("eager pipeline should pass");
    let streaming =
        execute_pipeline_streaming(rows, &stages).expect("streaming pipeline should pass");

    assert_eq!(streaming, eager);
    assert_eq!(output_rows(&streaming).len(), 1);
}

#[test]
fn streaming_executor_preserves_copy_flag_and_value_fanout() {
    let rows = vec![
        json!({"uid": "alice", "roles": ["eng", "ops"]})
            .as_object()
            .cloned()
            .expect("object"),
    ];

    let output = execute_pipeline_streaming(rows, &["Y".to_string(), "VALUE roles".to_string()])
        .expect("streaming pipeline should pass");

    assert!(output.meta.wants_copy);
    assert_eq!(output_rows(&output).len(), 2);
}

#[test]
fn unroll_stage_expands_list_field() {
    let rows = vec![
        json!({"members": ["a", "b"], "cn": "grp"})
            .as_object()
            .cloned()
            .expect("object"),
    ];

    let output = apply_pipeline(rows, &["U members".to_string()]).expect("pipeline should pass");

    assert_eq!(output_rows(&output).len(), 2);
    assert_eq!(
        output_rows(&output)
            .iter()
            .map(|row| row.get("members").cloned().expect("member"))
            .collect::<Vec<_>>(),
        vec![json!("a"), json!("b")]
    );
}

#[test]
fn unroll_requires_field_name() {
    let rows = vec![
        json!({"members": ["a", "b"]})
            .as_object()
            .cloned()
            .expect("object"),
    ];

    let err = apply_pipeline(rows, &["U".to_string()]).expect_err("pipeline should fail");
    assert!(err.to_string().contains("missing field name"));
}

#[test]
fn grouped_output_meta_uses_group_headers() {
    let output = apply_output_pipeline(
        OutputResult {
            items: OutputItems::Groups(vec![crate::core::output_model::Group {
                groups: json!({"dept": "sales"})
                    .as_object()
                    .cloned()
                    .expect("object"),
                aggregates: json!({"total": 2}).as_object().cloned().expect("object"),
                rows: vec![],
            }]),
            document: None,
            meta: Default::default(),
        },
        &[],
    )
    .expect("pipeline should pass");

    assert_eq!(output.meta.key_index, vec!["dept", "total"]);
    assert!(output.meta.grouped);
}

#[test]
fn grouped_rows_ignore_flat_row_only_projection_and_copy_preserves_flag() {
    let grouped = OutputResult {
        items: OutputItems::Groups(vec![crate::core::output_model::Group {
            groups: json!({"dept": "sales"})
                .as_object()
                .cloned()
                .expect("object"),
            aggregates: json!({"total": 2}).as_object().cloned().expect("object"),
            rows: vec![
                json!({"uid": "alice"})
                    .as_object()
                    .cloned()
                    .expect("object"),
            ],
        }]),
        document: None,
        meta: Default::default(),
    };

    let projected =
        apply_output_pipeline(grouped.clone(), &["P uid".to_string()]).expect("pipeline works");
    assert_eq!(projected.items, grouped.items);

    let copied = apply_output_pipeline(grouped, &["Y".to_string()]).expect("copy works");
    assert!(copied.meta.wants_copy);
    assert!(copied.meta.grouped);
}

#[test]
fn streaming_materializes_cleanly_at_sort_barrier() {
    let rows = vec![
        json!({"uid": "bob"}).as_object().cloned().expect("object"),
        json!({"uid": "alice"})
            .as_object()
            .cloned()
            .expect("object"),
    ];

    let output = execute_pipeline_streaming(rows, &["S uid".to_string()])
        .expect("streaming pipeline should pass");

    assert_eq!(
        output_rows(&output)
            .iter()
            .map(|row| row
                .get("uid")
                .and_then(|value| value.as_str())
                .unwrap_or_default())
            .collect::<Vec<_>>(),
        vec!["alice", "bob"]
    );
}

#[test]
fn grouped_output_pipeline_applies_quick_and_value_stages_to_group_rows_unit() {
    let grouped = OutputResult {
        items: OutputItems::Groups(vec![crate::core::output_model::Group {
            groups: json!({"team": "ops"}).as_object().cloned().expect("object"),
            aggregates: json!({"count": 2}).as_object().cloned().expect("object"),
            rows: vec![
                json!({"uid": "alice", "roles": ["eng", "ops"]})
                    .as_object()
                    .cloned()
                    .expect("object"),
                json!({"uid": "bob", "roles": ["sales"]})
                    .as_object()
                    .cloned()
                    .expect("object"),
            ],
        }]),
        document: None,
        meta: Default::default(),
    };

    let value_only = apply_output_pipeline(grouped.clone(), &["V ops".to_string()])
        .expect("grouped quick should succeed");
    let OutputItems::Groups(value_groups) = value_only.items else {
        panic!("expected grouped output");
    };
    assert_eq!(value_groups[0].rows.len(), 1);
    assert_eq!(
        value_groups[0].rows[0].get("roles"),
        Some(&json!(["eng", "ops"]))
    );

    let key_only = apply_output_pipeline(grouped.clone(), &["K uid".to_string()])
        .expect("grouped key quick should succeed");
    let OutputItems::Groups(key_groups) = key_only.items else {
        panic!("expected grouped output");
    };
    assert_eq!(key_groups[0].rows.len(), 2);
    assert!(key_groups[0].rows.iter().all(|row| row.contains_key("uid")));

    let values = apply_output_pipeline(grouped.clone(), &["VALUE uid".to_string()])
        .expect("grouped values should succeed");
    let OutputItems::Groups(value_rows) = values.items else {
        panic!("expected grouped output");
    };
    assert_eq!(value_rows[0].rows.len(), 2);
    assert_eq!(
        value_rows[0]
            .rows
            .iter()
            .map(|row| row.get("value").cloned().expect("value"))
            .collect::<Vec<_>>(),
        vec![json!("alice"), json!("bob")]
    );

    let bare_quick = apply_output_pipeline(grouped.clone(), &["ops".to_string()])
        .expect("grouped bare quick should succeed");
    let OutputItems::Groups(bare_groups) = bare_quick.items else {
        panic!("expected grouped output");
    };
    assert_eq!(bare_groups[0].rows.len(), 1);

    let filtered = apply_output_pipeline(grouped.clone(), &["F uid=alice".to_string()])
        .expect("grouped filter should succeed");
    let OutputItems::Groups(filtered_groups) = filtered.items else {
        panic!("expected grouped output");
    };
    assert_eq!(filtered_groups[0].rows.len(), 1);

    let cleaned = apply_output_pipeline(grouped.clone(), &["? uid".to_string()])
        .expect("grouped clean should succeed");
    let OutputItems::Groups(cleaned_groups) = cleaned.items else {
        panic!("expected grouped output");
    };
    assert_eq!(cleaned_groups[0].rows.len(), 2);

    let copied =
        apply_output_pipeline(grouped, &["Y".to_string()]).expect("grouped copy should succeed");
    assert!(copied.meta.wants_copy);
}

#[test]
fn grouped_output_pipeline_covers_group_limit_and_unroll_paths_unit() {
    let grouped = OutputResult {
        items: OutputItems::Groups(vec![
            crate::core::output_model::Group {
                groups: json!({"team": "ops"}).as_object().cloned().expect("object"),
                aggregates: json!({"count": 2}).as_object().cloned().expect("object"),
                rows: vec![
                    json!({"uid": "alice", "roles": ["eng", "ops"]})
                        .as_object()
                        .cloned()
                        .expect("object"),
                ],
            },
            crate::core::output_model::Group {
                groups: json!({"team": "eng"}).as_object().cloned().expect("object"),
                aggregates: json!({"count": 1}).as_object().cloned().expect("object"),
                rows: vec![
                    json!({"uid": "bob", "roles": ["ops"]})
                        .as_object()
                        .cloned()
                        .expect("object"),
                ],
            },
        ]),
        document: None,
        meta: Default::default(),
    };

    let regrouped = apply_output_pipeline(grouped.clone(), &["G team".to_string()])
        .expect("group regroup should succeed");
    assert!(matches!(regrouped.items, OutputItems::Groups(_)));

    let limited = apply_output_pipeline(grouped.clone(), &["L 1".to_string()])
        .expect("group limit should succeed");
    let OutputItems::Groups(limited_groups) = limited.items else {
        panic!("expected grouped output");
    };
    assert_eq!(limited_groups.len(), 1);

    let unrolled = apply_output_pipeline(grouped, &["U roles".to_string()])
        .expect("group unroll should succeed");
    assert!(matches!(unrolled.items, OutputItems::Groups(_)));
}

#[test]
fn streaming_pipeline_covers_stream_stage_variants_and_errors_unit() {
    let rows = vec![
        json!({"uid": "alice", "active": true, "roles": ["eng", "ops"]})
            .as_object()
            .cloned()
            .expect("object"),
        json!({"uid": "bob", "active": false, "roles": ["ops"]})
            .as_object()
            .cloned()
            .expect("object"),
    ];

    let value_output = execute_pipeline_streaming(rows.clone(), &["VALUE uid".to_string()])
        .expect("streaming values should succeed");
    assert_eq!(output_rows(&value_output).len(), 2);

    let filtered = execute_pipeline_streaming(rows.clone(), &["? uid".to_string()])
        .expect("question filter should stream");
    assert_eq!(output_rows(&filtered).len(), 2);

    let cleaned = execute_pipeline_streaming(rows.clone(), &["?".to_string()])
        .expect("question clean should stream");
    assert_eq!(output_rows(&cleaned).len(), 2);

    let limited = execute_pipeline_streaming(rows.clone(), &["L 1".to_string()])
        .expect("head limit should stream");
    assert_eq!(output_rows(&limited).len(), 1);

    let unrolled = execute_pipeline_streaming(rows.clone(), &["U roles".to_string()])
        .expect("unroll should stream");
    assert_eq!(output_rows(&unrolled).len(), 3);

    let err = execute_pipeline_streaming(rows, &["U".to_string()])
        .expect_err("missing unroll field should fail");
    assert!(err.to_string().contains("missing field name"));
}

#[test]
fn apply_output_pipeline_covers_explicit_materializing_row_stages_unit() {
    let rows = vec![
        json!({"uid": "bob", "dept": "ops"})
            .as_object()
            .cloned()
            .expect("object"),
        json!({"uid": "alice", "dept": "ops"})
            .as_object()
            .cloned()
            .expect("object"),
        json!({"uid": "carol", "dept": "eng"})
            .as_object()
            .cloned()
            .expect("object"),
    ];

    let sorted = apply_pipeline(rows.clone(), &["S uid".to_string()]).expect("sort works");
    assert_eq!(
        output_rows(&sorted)[0]
            .get("uid")
            .and_then(|value| value.as_str()),
        Some("alice")
    );

    let grouped = apply_pipeline(rows.clone(), &["G dept".to_string()]).expect("group works");
    assert!(grouped.meta.grouped);

    let aggregated =
        apply_pipeline(rows.clone(), &["A count total".to_string()]).expect("aggregate works");
    assert!(!output_rows(&aggregated).is_empty());

    let counted = apply_pipeline(rows.clone(), &["C".to_string()]).expect("count works");
    assert_eq!(output_rows(&counted).len(), 1);

    let collapsed = apply_pipeline(rows.clone(), &["G dept".to_string(), "Z".to_string()])
        .expect("collapse works");
    assert!(matches!(collapsed.items, OutputItems::Rows(_)));

    let err = apply_pipeline(rows, &["R nope".to_string()])
        .expect_err("unknown explicit stage should fail");
    assert!(err.to_string().contains("unknown DSL verb"));
}

#[test]
fn render_recommendation_survives_narrowing_stages_unit() {
    let rows = vec![
        json!({"uid": "alice", "dept": "ops"})
            .as_object()
            .cloned()
            .expect("object"),
        json!({"uid": "bob", "dept": "eng"})
            .as_object()
            .cloned()
            .expect("object"),
    ];
    let mut output = OutputResult::from_rows(rows);
    output.meta.render_recommendation = Some(RenderRecommendation::Guide);

    let quick =
        apply_output_pipeline(output.clone(), &["alice".to_string()]).expect("quick should work");
    assert_eq!(
        quick.meta.render_recommendation,
        Some(RenderRecommendation::Guide)
    );

    let filtered = apply_output_pipeline(output.clone(), &["F dept=ops".to_string()])
        .expect("filter should work");
    assert_eq!(
        filtered.meta.render_recommendation,
        Some(RenderRecommendation::Guide)
    );

    let sorted = apply_output_pipeline(output, &["S uid".to_string()]).expect("sort works");
    assert_eq!(
        sorted.meta.render_recommendation,
        Some(RenderRecommendation::Guide)
    );
}

#[test]
fn render_recommendation_survives_limit_and_copy_unit() {
    let rows = vec![
        json!({"uid": "alice"})
            .as_object()
            .cloned()
            .expect("object"),
        json!({"uid": "bob"}).as_object().cloned().expect("object"),
    ];
    let mut output = OutputResult::from_rows(rows);
    output.meta.render_recommendation = Some(RenderRecommendation::Format(OutputFormat::Value));

    let limited =
        apply_output_pipeline(output.clone(), &["L 1".to_string()]).expect("limit should work");
    assert_eq!(
        limited.meta.render_recommendation,
        Some(RenderRecommendation::Format(OutputFormat::Value))
    );

    let copied = apply_output_pipeline(output, &["Y".to_string()]).expect("copy should work");
    assert_eq!(
        copied.meta.render_recommendation,
        Some(RenderRecommendation::Format(OutputFormat::Value))
    );
}

#[test]
fn semantic_document_tracks_transformed_output_unit() {
    let output = GuideView::from_text("Usage: osp history <COMMAND>\n\nCommands:\n  list  Show\n")
        .to_output_result();

    let copied =
        apply_output_pipeline(output.clone(), &["Y".to_string()]).expect("copy should work");
    assert!(matches!(
        copied.document,
        Some(OutputDocument {
            kind: OutputDocumentKind::Guide,
            ..
        })
    ));

    let filtered = apply_output_pipeline(output, &["list".to_string()]).expect("quick should work");
    let filtered_guide =
        GuideView::try_from_output_result(&filtered).expect("guide should still restore");
    assert_eq!(filtered_guide.commands.len(), 1);
    assert_eq!(filtered_guide.commands[0].name, "list");
}

#[test]
fn semantic_document_is_source_of_truth_for_initial_items_unit() {
    let mut output = OutputResult::from_rows(vec![
        json!({"value": "stale"})
            .as_object()
            .cloned()
            .expect("object"),
    ]);
    output.document = GuideView::from_text("Commands:\n  list  Show\n")
        .to_output_result()
        .document;
    output.meta.render_recommendation = Some(RenderRecommendation::Guide);

    let rebuilt = apply_output_pipeline(output, &[]).expect("pipeline should succeed");
    let guide = GuideView::try_from_output_result(&rebuilt).expect("guide should restore");
    assert_eq!(guide.commands.len(), 1);
    assert_eq!(guide.commands[0].name, "list");
    assert!(
        rebuilt
            .as_rows()
            .expect("rows")
            .first()
            .expect("row")
            .contains_key("commands")
    );
}

#[test]
fn render_recommendation_clears_on_structural_row_reshapes_unit() {
    let rows = vec![
        json!({"uid": "alice", "roles": ["eng", "ops"]})
            .as_object()
            .cloned()
            .expect("object"),
        json!({"uid": "bob", "roles": ["ops"]})
            .as_object()
            .cloned()
            .expect("object"),
    ];
    let mut output = OutputResult::from_rows(rows);
    output.meta.render_recommendation = Some(RenderRecommendation::Guide);

    let projected =
        apply_output_pipeline(output.clone(), &["P uid".to_string()]).expect("project should work");
    assert_eq!(projected.meta.render_recommendation, None);

    let unrolled = apply_output_pipeline(output.clone(), &["U roles".to_string()])
        .expect("unroll should work");
    assert_eq!(unrolled.meta.render_recommendation, None);

    let values =
        apply_output_pipeline(output, &["VALUE uid".to_string()]).expect("values should work");
    assert_eq!(values.meta.render_recommendation, None);
}

#[test]
fn render_recommendation_clears_on_grouping_and_aggregate_stages_unit() {
    let rows = vec![
        json!({"uid": "alice", "dept": "ops"})
            .as_object()
            .cloned()
            .expect("object"),
        json!({"uid": "bob", "dept": "ops"})
            .as_object()
            .cloned()
            .expect("object"),
    ];
    let mut output = OutputResult::from_rows(rows);
    output.meta.render_recommendation = Some(RenderRecommendation::Guide);

    let grouped =
        apply_output_pipeline(output.clone(), &["G dept".to_string()]).expect("group should work");
    assert_eq!(grouped.meta.render_recommendation, None);

    let aggregated = apply_output_pipeline(output.clone(), &["A count total".to_string()])
        .expect("aggregate should work");
    assert_eq!(aggregated.meta.render_recommendation, None);

    let counted = apply_output_pipeline(output, &["C".to_string()]).expect("count works");
    assert_eq!(counted.meta.render_recommendation, None);
}
