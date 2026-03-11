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

// Protects addressed VALUE extraction on semantic payloads: extracting one
// nested entry field should keep the surviving section shell while degrading
// the targeted leaf into `{value: ...}` rows.
#[test]
fn help_like_payload_value_extracts_nested_entry_field_with_section_envelope() {
    let output = run_guide_pipeline(help_like_guide(), "VALUE sections[1].entries[0].name");

    assert!(GuideView::try_from_output_result(&output).is_none());
    let document = output
        .document
        .expect("semantic document should remain attached");
    assert_eq!(
        document.value,
        json!({
            "sections": [
                {
                    "title": "Options",
                    "kind": "options",
                    "paragraphs": ["rendering"],
                    "entries": [
                        {"value": "--verbose"}
                    ]
                }
            ]
        })
    );
}

// Protects mixed-depth VALUE extraction: combining a top-level scalar-array
// selector with a nested structural selector should preserve each selected
// branch in-place instead of collapsing them into one synthetic wrapper.
#[test]
fn help_like_payload_value_mixed_depth_selectors_keep_structural_branches() {
    let output = run_guide_pipeline(help_like_guide(), "VALUE usage sections[0].entries[].name");

    assert!(GuideView::try_from_output_result(&output).is_none());
    let document = output
        .document
        .expect("semantic document should remain attached");
    assert_eq!(
        document.value,
        json!({
            "usage": [
                {"value": "osp deploy <COMMAND>"}
            ],
            "sections": [
                {
                    "title": "Commands",
                    "kind": "commands",
                    "paragraphs": ["pick one"],
                    "entries": [
                        {"value": "apply"},
                        {"value": "doctor"},
                        {"value": "status"}
                    ]
                }
            ]
        })
    );
}

// Protects VALUE extraction for sibling fields on the same addressed object:
// once one entry survives, both selected leaves should stay attached to the
// same structural entry instead of being split into unrelated wrappers.
#[test]
fn help_like_payload_value_keeps_sibling_field_identity_for_same_object() {
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
        json!({
            "sections": [
                {
                    "title": "Commands",
                    "kind": "commands",
                    "paragraphs": ["pick one"],
                    "entries": [
                        {
                            "name": {"value": "apply"},
                            "short_help": {"value": "Apply pending changes"}
                        }
                    ]
                }
            ]
        })
    );
}
