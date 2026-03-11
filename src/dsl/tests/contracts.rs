use serde_json::json;

use crate::{
    core::{
        output_model::{Group, OutputItems, OutputResult},
        row::Row,
    },
    dsl::{apply_output_pipeline, apply_pipeline},
};

fn row(value: serde_json::Value) -> Row {
    value
        .as_object()
        .cloned()
        .expect("fixture should be object")
}

fn grouped_output() -> OutputResult {
    OutputResult {
        items: OutputItems::Groups(vec![Group {
            groups: row(json!({"team": "ops"})),
            aggregates: row(json!({"count": 2})),
            rows: vec![
                row(json!({"uid": "alice", "roles": ["eng", "ops"], "city": "Ålesund"})),
                row(json!({"uid": "bob", "roles": ["sales"], "city": "Oslo"})),
            ],
        }]),
        document: None,
        meta: Default::default(),
    }
}

#[test]
fn quoted_term_contract_is_shared_by_project_and_value_stages() {
    let rows = vec![row(
        json!({"display,name": "Alice", "team ops": "platform"}),
    )];

    let projected = apply_pipeline(rows.clone(), &["P \"display,name\"".to_string()])
        .expect("quoted project term should work");
    let OutputItems::Rows(projected_rows) = projected.items else {
        panic!("expected row output");
    };
    assert_eq!(projected_rows, vec![row(json!({"display,name": "Alice"}))]);

    let values = apply_pipeline(rows, &["VALUE \"display,name\"".to_string()])
        .expect("quoted value term should work");
    let OutputItems::Rows(value_rows) = values.items else {
        panic!("expected row output");
    };
    assert_eq!(value_rows, vec![row(json!({"value": "Alice"}))]);
}

#[test]
fn nested_path_contract_is_shared_by_filter_project_and_value_stages() {
    let rows = vec![row(json!({
        "metadata": {"owner": "Åse"},
        "members": [{"uid": "alice"}, {"uid": "bob"}]
    }))];

    let filtered = apply_pipeline(rows.clone(), &["F metadata.owner=åse".to_string()])
        .expect("nested filter path should work");
    let OutputItems::Rows(filtered_rows) = filtered.items else {
        panic!("expected row output");
    };
    assert_eq!(filtered_rows.len(), 1);

    let projected = apply_pipeline(rows.clone(), &["P metadata.owner".to_string()])
        .expect("nested project path should work");
    let OutputItems::Rows(projected_rows) = projected.items else {
        panic!("expected row output");
    };
    assert_eq!(
        projected_rows,
        vec![row(json!({"metadata": {"owner": "Åse"}}))]
    );

    let owner_values = apply_pipeline(rows.clone(), &["VALUE metadata.owner".to_string()])
        .expect("nested value path should work");
    let OutputItems::Rows(owner_rows) = owner_values.items else {
        panic!("expected row output");
    };
    assert_eq!(owner_rows, vec![row(json!({"value": "Åse"}))]);

    let member_values = apply_pipeline(rows, &["VALUE members[].uid".to_string()])
        .expect("selector value path should work");
    let OutputItems::Rows(member_rows) = member_values.items else {
        panic!("expected row output");
    };
    assert_eq!(
        member_rows,
        vec![row(json!({"value": "alice"})), row(json!({"value": "bob"}))]
    );
}

#[test]
fn grouped_row_stage_contract_preserves_metadata_and_transforms_rows() {
    fn assert_group_metadata(output: &OutputResult, expected_rows: usize) {
        let OutputItems::Groups(groups) = &output.items else {
            panic!("expected grouped output");
        };
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0]
                .groups
                .get("team")
                .and_then(|value| value.as_str()),
            Some("ops")
        );
        assert_eq!(
            groups[0]
                .aggregates
                .get("count")
                .and_then(|value| value.as_i64()),
            Some(2)
        );
        assert_eq!(groups[0].rows.len(), expected_rows);
    }

    let projected = apply_output_pipeline(grouped_output(), &["P uid".to_string()])
        .expect("grouped project should work");
    assert_group_metadata(&projected, 2);

    let filtered = apply_output_pipeline(grouped_output(), &["F uid=alice".to_string()])
        .expect("grouped filter should work");
    assert_group_metadata(&filtered, 1);

    let quick = apply_output_pipeline(grouped_output(), &["ops".to_string()])
        .expect("grouped quick should work");
    assert_group_metadata(&quick, 1);

    let value_only = apply_output_pipeline(grouped_output(), &["V åle".to_string()])
        .expect("grouped value quick should work");
    assert_group_metadata(&value_only, 1);

    let key_only = apply_output_pipeline(grouped_output(), &["K uid".to_string()])
        .expect("grouped key quick should work");
    assert_group_metadata(&key_only, 2);

    let values = apply_output_pipeline(grouped_output(), &["VALUE uid".to_string()])
        .expect("grouped values should work");
    assert_group_metadata(&values, 2);

    let unrolled = apply_output_pipeline(grouped_output(), &["U roles".to_string()])
        .expect("grouped unroll should work");
    assert_group_metadata(&unrolled, 3);

    let cleaned = apply_output_pipeline(grouped_output(), &["? uid".to_string()])
        .expect("grouped clean should work");
    assert_group_metadata(&cleaned, 2);
}

#[test]
fn unsupported_group_only_stage_fails_loudly() {
    let rows = vec![row(json!({"uid": "alice"}))];
    let err = apply_pipeline(rows, &["Z".to_string()]).expect_err("flat collapse should fail");
    assert!(err.to_string().contains("Z requires grouped output"));
}

#[test]
fn unicode_case_insensitive_matching_is_shared_by_quick_and_filter() {
    let rows = vec![
        row(json!({"city": "Ålesund", "Grønn": true})),
        row(json!({"city": "Oslo", "status": true})),
    ];

    let value_quick = apply_pipeline(rows.clone(), &["V åle".to_string()])
        .expect("unicode quick value match should work");
    let OutputItems::Rows(value_rows) = value_quick.items else {
        panic!("expected row output");
    };
    assert_eq!(value_rows.len(), 1);

    let key_quick = apply_pipeline(rows.clone(), &["K grønn".to_string()])
        .expect("unicode quick key match should work");
    let OutputItems::Rows(key_rows) = key_quick.items else {
        panic!("expected row output");
    };
    assert_eq!(key_rows.len(), 1);

    let filtered = apply_pipeline(rows, &["F city=ålesund".to_string()])
        .expect("unicode filter match should work");
    let OutputItems::Rows(filtered_rows) = filtered.items else {
        panic!("expected row output");
    };
    assert_eq!(filtered_rows.len(), 1);
}

#[test]
fn quick_contract_preserves_matching_parent_objects_in_object_arrays() {
    let rows = vec![row(json!({
        "commands": [
            {
                "name": "alpha",
                "short_help": "shared help text"
            },
            {
                "name": "beta",
                "short_help": "shared help text"
            }
        ]
    }))];

    let output = apply_pipeline(rows, &["shared".to_string()])
        .expect("quick should preserve full matching command entries");
    let OutputItems::Rows(result_rows) = output.items else {
        panic!("expected row output");
    };
    assert_eq!(
        result_rows,
        vec![row(json!({
            "commands": [
                {
                    "name": "alpha",
                    "short_help": "shared help text"
                },
                {
                    "name": "beta",
                    "short_help": "shared help text"
                }
            ]
        }))]
    );
}
