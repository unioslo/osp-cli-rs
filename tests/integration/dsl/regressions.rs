use osp_cli::core::output_model::{OutputDocument, OutputDocumentKind, OutputItems, OutputResult};
use osp_cli::dsl::apply_output_pipeline;
use osp_cli::row;
use serde_json::json;

fn pipeline(output: OutputResult, stages: &[&str]) -> OutputResult {
    apply_output_pipeline(
        output,
        &stages.iter().map(|s| (*s).to_string()).collect::<Vec<_>>(),
    )
    .unwrap()
}

#[test]
fn collection_operations_preserve_unrelated_nulls_and_empty_containers() {
    let input = json!({
        "note": null, "empty_map": {}, "empty_list": [],
        "metadata": {"label": "", "optional": null},
        "hosts": [{"name": "b", "team": "ops"}, {"name": "a", "team": "ops"}]
    });
    for stages in [
        vec!["L 1"],
        vec!["L 0"],
        vec!["S name"],
        vec!["G ?hosts"],
        vec!["A count()"],
    ] {
        let output = pipeline(
            OutputResult::from_rows(vec![])
                .with_document(OutputDocument::new(OutputDocumentKind::Json, input.clone())),
            &stages,
        );
        assert!(output.document.is_none());
        match stages[0] {
            "L 0" => assert!(output.as_rows().unwrap().is_empty()),
            "A count()" => assert_eq!(output.as_rows().unwrap(), &[row! {"count" => 1}]),
            "G ?hosts" => {
                let OutputItems::Groups(groups) = output.items else {
                    panic!("groups");
                };
                assert_eq!(groups[0].rows, vec![input.as_object().unwrap().clone()]);
            }
            _ => assert_eq!(
                output.as_rows().unwrap(),
                &[input.as_object().unwrap().clone()]
            ),
        }
    }
}

#[test]
fn group_shaped_service_records_do_not_lose_extra_fields() {
    let record =
        json!({"groups": {}, "aggregates": {}, "rows": [], "name": "host-a", "note": null});
    let input = OutputResult::from_rows(vec![]).with_document(OutputDocument::new(
        OutputDocumentKind::Json,
        json!([record.clone()]),
    ));
    let output = pipeline(input, &["S name"]);
    assert_eq!(
        serde_json::to_value(output.as_rows().unwrap()).unwrap(),
        json!([record])
    );
}

#[test]
fn only_explicit_grouping_creates_groups_and_continuations_keep_them() {
    let grouped = pipeline(
        OutputResult::from_rows(vec![row! {"team" => "ops"}]),
        &["G team"],
    );
    assert!(!pipeline(grouped, &["Z"]).meta.grouped);
    let record = json!({"groups": {}, "aggregates": {}, "rows": []});
    let input = OutputResult::from_rows(vec![]).with_document(OutputDocument::new(
        OutputDocumentKind::Json,
        json!([record.clone()]),
    ));
    let sorted = pipeline(input.clone(), &["S groups"]);
    assert!(matches!(sorted.items, OutputItems::Rows(_)));
    assert_eq!(
        serde_json::to_value(sorted.as_rows().unwrap()).unwrap(),
        json!([record])
    );
    assert_eq!(
        serde_json::to_value(pipeline(input, &["A count() AS total"]).as_rows().unwrap()).unwrap(),
        json!([{"total": 1}])
    );

    for nested in [false, true] {
        let rows = json!([{"team": "ops", "uid": "alice"}, {"team": "ops", "uid": "bob"}]);
        let value = if nested {
            json!({"hosts": rows, "note": null})
        } else {
            rows
        };
        let input = OutputResult::from_rows(vec![])
            .with_document(OutputDocument::new(OutputDocumentKind::Json, value));
        let input = if nested {
            pipeline(input, &["P hosts[]"])
        } else {
            input
        };
        let grouped = pipeline(input.clone(), &["G team"]);
        let counted = pipeline(grouped, &["A count() AS members"]);
        let collapsed = pipeline(counted, &["Z"]);
        let together = pipeline(input, &["G team", "A count() AS members", "Z"]);
        assert_eq!(collapsed, together);
        let expected = json!([{"team": "ops", "members": 2}]);
        assert_eq!(
            serde_json::to_value(collapsed.as_rows().unwrap()).unwrap(),
            expected
        );
    }
}

#[test]
fn negated_group_filters_use_the_field_owner() {
    let input = OutputResult::from_rows(vec![
        row! {"dept" => "ops", "uid" => "alice"},
        row! {"dept" => "ops", "uid" => "bob"},
        row! {"dept" => "dev", "uid" => "carol"},
    ]);
    for (predicate, expected) in [
        ("F uid!=alice", vec!["bob", "carol"]),
        ("F dept!=ops", vec!["carol"]),
    ] {
        let output = pipeline(input.clone(), &["G dept", predicate]);
        let OutputItems::Groups(groups) = output.items else {
            panic!("groups expected")
        };
        let names = groups
            .iter()
            .flat_map(|group| &group.rows)
            .map(|row| row["uid"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(names, expected, "{predicate}");
    }
}

#[test]
fn filter_rejects_unconsumed_predicate_text() {
    for stage in ["F uid=alice garbage", "F uid=alice AND active=true"] {
        let err =
            apply_output_pipeline(OutputResult::from_rows(vec![]), &[stage.into()]).unwrap_err();
        assert!(err.to_string().contains("unexpected"), "{err}");
    }
}

#[test]
fn aggregates_require_fields_and_consume_the_whole_expression() {
    for stage in [
        "A sum",
        "A avg",
        "A min",
        "A max",
        "A sum amount AS total garbage",
    ] {
        assert!(
            apply_output_pipeline(OutputResult::from_rows(vec![]), &[stage.into()]).is_err(),
            "{stage}"
        );
    }
}

#[test]
fn date_filters_do_not_normalize_invalid_dates_or_malformed_seconds() {
    let input = OutputResult::from_rows(vec![
        row! {"date" => "2024-02-29T00:00:00Z"},
        row! {"date" => "2023-02-29T00:00:00Z"},
        row! {"date" => "2024-04-31T00:00:00Z"},
        row! {"date" => "2024-02-29T00:00:nopeZ"},
    ]);
    let output = pipeline(input, &["F date>=2023-01-01"]);
    assert_eq!(
        output.as_rows().unwrap(),
        &[row! {"date" => "2024-02-29T00:00:00Z"}]
    );
}

#[test]
fn mixed_sort_is_consistent_and_missing_stays_last_in_both_directions() {
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let values = ["2", "10", "1a"];
        let mut rows = vec![row! {"name" => "missing"}];
        rows.extend(order.map(|index| row! {"value" => values[index]}));
        for (stage, expected) in [
            ("S value", vec!["2", "10", "1a"]),
            ("S !value", vec!["1a", "10", "2"]),
        ] {
            let output = pipeline(OutputResult::from_rows(rows.clone()), &[stage]);
            let sorted = output.as_rows().unwrap();
            assert_eq!(sorted.last().unwrap()["name"], "missing");
            assert_eq!(
                sorted[..3]
                    .iter()
                    .map(|row| row["value"].as_str().unwrap())
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }
}

#[test]
fn scalar_document_sort_uses_the_requested_direction_and_cast() {
    let input = OutputResult::from_rows(vec![]).with_document(OutputDocument::new(
        OutputDocumentKind::Json,
        json!(["2", "10", "1"]),
    ));
    assert_eq!(
        serde_json::to_value(pipeline(input, &["S !value AS num"]).as_rows().unwrap()).unwrap(),
        json!([{"value":"10"},{"value":"2"},{"value":"1"}])
    );
}

#[test]
fn values_have_the_same_selector_order_for_rows_and_documents() {
    let rows = vec![row! {"a" => 1, "b" => 2}, row! {"a" => 3, "b" => 4}];
    for document in [
        None,
        Some(OutputDocument::new(OutputDocumentKind::Json, json!(rows))),
    ] {
        let mut input = OutputResult::from_rows(rows.clone());
        input.document = document;
        let output = pipeline(input, &["VALUE a b"]);
        assert_eq!(
            output.as_rows().unwrap(),
            &[
                row! {"value" => 1},
                row! {"value" => 3},
                row! {"value" => 2},
                row! {"value" => 4}
            ]
        );
    }
    let input = OutputResult::from_rows(vec![]).with_document(OutputDocument::new(
        OutputDocumentKind::Json,
        json!({"a":1,"b":2}),
    ));
    assert_eq!(
        pipeline(input, &["VALUE"]).as_rows().unwrap(),
        &[row! {"value" => 1}, row! {"value" => 2}]
    );
}

#[test]
fn jq_evaluates_empty_input_instead_of_skipping_the_expression() {
    for (expression, expected) in [
        ("JQ 'length'", json!([{"value":0}])),
        ("JQ '42'", json!([{"value":42}])),
        ("JQ 'empty'", json!([])),
    ] {
        let output = pipeline(OutputResult::from_rows(vec![]), &[expression]);
        assert_eq!(
            serde_json::to_value(output.as_rows().unwrap()).unwrap(),
            expected
        );
    }
}

#[test]
fn scoped_quick_search_keeps_complete_matching_document_members() {
    let document = json!({"commands":[{"name":"doctor", "about":"diagnostics"}, {"name":"config", "about":"settings"}]});
    for stage in ["doctor", "V doctor"] {
        let input = OutputResult::from_rows(vec![]).with_document(OutputDocument::new(
            OutputDocumentKind::Json,
            document.clone(),
        ));
        let output = pipeline(input, &[stage]);
        assert_eq!(
            serde_json::to_value(output.as_rows().unwrap()).unwrap(),
            json!([document])
        );
    }
}
