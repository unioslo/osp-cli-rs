use super::*;
use serde_json::json;

// These selector checks use a structured service record. Declared guides expose
// entry rows instead; their rendering contract is exercised in semantics.rs.

#[test]
fn document_exact_index_projection_emits_selected_value_row() {
    let output = run_document_pipeline(help_like_guide(), "P sections[1].entries[0].name");
    assert_eq!(result_rows(&output), json!([{"name":"--verbose"}]));
    assert!(output.document.is_none());
}

#[test]
fn document_negative_index_projection_emits_last_entry_name() {
    let output = run_document_pipeline(help_like_guide(), "P sections[-1].entries[-1].name");
    assert_eq!(result_rows(&output), json!([{"name":"--json"}]));
    assert!(output.document.is_none());
}

#[test]
fn document_exact_index_filter_keeps_complete_matching_row() {
    let output =
        run_document_pipeline(help_like_guide(), "F sections[1].entries[0].name=--verbose");
    assert_eq!(
        result_rows(&output),
        json!([help_like_guide().to_json_value()])
    );
}

#[test]
fn document_negated_index_filter_keeps_complete_matching_row() {
    let output = run_document_pipeline(help_like_guide(), "F sections[1].entries[0].name!=--json");
    assert_eq!(
        result_rows(&output),
        json!([help_like_guide().to_json_value()])
    );
}

#[test]
fn document_fanout_filter_keeps_all_members_of_matching_row() {
    let output = run_document_pipeline(help_like_guide(), "F sections[].entries[].name=--json");
    assert_eq!(
        result_rows(&output),
        json!([help_like_guide().to_json_value()])
    );
}

#[test]
fn document_slice_projection_emits_selected_names_in_order() {
    let output = run_document_pipeline(help_like_guide(), "P sections[0].entries[1:3].name");
    assert_eq!(
        result_rows(&output),
        json!([{"name":"doctor"},{"name":"status"}])
    );
}

#[test]
fn document_projection_preserves_explicit_parent_fields_and_selected_names() {
    for (spec, expected) in [
        (
            "usage sections[0].entries[1].name",
            json!([{"usage":["osp deploy <COMMAND>"],"name":"doctor"}]),
        ),
        (
            "sections[0].entries[1].name !short_help",
            json!([{"name":"doctor"}]),
        ),
        (
            "sections[].entries[].name",
            json!([{"name":"apply"},{"name":"doctor"},{"name":"status"},{"name":"--verbose"},{"name":"--json"}]),
        ),
    ] {
        let output = run_document_pipeline(help_like_guide(), &format!("P {spec}"));
        assert_eq!(result_rows(&output), expected, "{spec}");
    }
    let output = run_document_pipeline(help_like_guide(), "P name");
    assert_eq!(
        result_rows(&output),
        json!([{
            "commands": [{"name":"apply"},{"name":"doctor"},{"name":"status"}],
            "options": [{"name":"--verbose"},{"name":"--json"}],
            "sections": [
                {"entries": [{"name":"apply"},{"name":"doctor"},{"name":"status"}]},
                {"entries": [{"name":"--verbose"},{"name":"--json"}]}
            ]
        }])
    );
}

#[test]
fn document_fanout_projection_drops_only_addressed_names() {
    let output = run_document_pipeline(help_like_guide(), "P !sections[].entries[].name");
    let mut expected = help_like_guide().to_json_value();
    for section in expected["sections"].as_array_mut().unwrap() {
        for entry in section["entries"].as_array_mut().unwrap() {
            entry.as_object_mut().unwrap().remove("name");
        }
    }
    assert_eq!(result_rows(&output), json!([expected]));
}

#[test]
fn document_path_search_keeps_complete_matching_row() {
    let output = run_document_pipeline(help_like_guide(), "sections[1].entries[0].name");
    assert_eq!(
        result_rows(&output),
        json!([help_like_guide().to_json_value()])
    );
}

#[test]
fn document_projection_drops_one_entry_and_preserves_siblings() {
    let output = run_document_pipeline(help_like_guide(), "P !sections[1].entries[0]");
    let mut expected = help_like_guide().to_json_value();
    expected["sections"][1]["entries"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    assert_eq!(result_rows(&output), json!([expected]));
}

#[test]
fn document_path_existence_keeps_complete_matching_row() {
    let output = run_document_pipeline(help_like_guide(), "?sections[1].entries[0].name");
    assert_eq!(
        result_rows(&output),
        json!([help_like_guide().to_json_value()])
    );
}

#[test]
fn document_projection_rejects_duplicate_labels_and_accepts_slice() {
    let input = OutputResult::from_rows(vec![row(help_like_guide().to_json_value())]);
    let err = apply_output_pipeline(
        input,
        &["P sections[0].entries[0].name sections[0].entries[1].name".into()],
    )
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("ambiguous dynamic projection label")
    );
    let output = run_document_pipeline(help_like_guide(), "P sections[0].entries[0:2].name");
    assert_eq!(
        result_rows(&output),
        json!([{"name":"apply"},{"name":"doctor"}])
    );
}
