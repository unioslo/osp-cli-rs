use super::*;
use serde_json::json;

// Protects semantic VALUE extraction on top-level scalar arrays: guide-like
// payload metadata such as usage should transform directly from canonical JSON
// instead of silently depending on row-shaped collections.
#[test]
fn help_like_payload_value_extracts_top_level_usage_array() {
    let output = run_guide_pipeline(help_like_guide(), "VALUE usage");

    assert!(GuideView::try_from_output_result(&output).is_none());
    let document = output
        .document
        .expect("semantic document should remain attached");
    assert_eq!(document.value, json!([{"value": "osp deploy <COMMAND>"}]));
}

// Addressed VALUE extraction is an extractor, so semantic input must produce
// the same flat `{value: ...}` rows as row-shaped input.
#[test]
fn help_like_payload_value_extracts_nested_entry_field_as_flat_rows() {
    let output = run_guide_pipeline(help_like_guide(), "VALUE sections[1].entries[0].name");

    assert!(GuideView::try_from_output_result(&output).is_none());
    let document = output
        .document
        .expect("semantic document should remain attached");
    assert_eq!(document.value, json!([{"value": "--verbose"}]));
}

// Multiple VALUE selectors emit flat rows in selector order and retain each
// selector's document order.
#[test]
fn help_like_payload_value_mixed_depth_selectors_keep_stable_order() {
    let output = run_guide_pipeline(help_like_guide(), "VALUE usage sections[0].entries[].name");

    assert!(GuideView::try_from_output_result(&output).is_none());
    let document = output
        .document
        .expect("semantic document should remain attached");
    assert_eq!(
        document.value,
        json!([
            {"value": "osp deploy <COMMAND>"},
            {"value": "apply"},
            {"value": "doctor"},
            {"value": "status"}
        ])
    );
}

// Sibling selectors remain independent extractions rather than rebuilding the
// owning semantic object.
#[test]
fn help_like_payload_value_extracts_sibling_fields_as_rows() {
    let output = run_guide_pipeline(
        help_like_guide(),
        "VALUE sections[0].entries[0].name sections[0].entries[0].short_help",
    );

    assert!(GuideView::try_from_output_result(&output).is_none());
    let document = output
        .document
        .expect("semantic document should remain attached");
    assert_eq!(
        document.value,
        json!([
            {"value": "apply"},
            {"value": "Apply pending changes"}
        ])
    );
}
