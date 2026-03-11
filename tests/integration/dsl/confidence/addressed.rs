use super::*;
use serde_json::json;

// Protects the new addressed-path projection path: selecting one indexed
// descendant should rebuild only that exact branch while keeping stable guide
// envelope metadata around it.
#[test]
fn help_like_payload_exact_index_projection_rebuilds_selected_branch() {
    let output = run_guide_pipeline(help_like_guide(), "P sections[1].entries[0].name");

    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should still restore");
    assert_eq!(rebuilt.preamble, vec!["Deploy commands"]);
    assert_eq!(rebuilt.usage, vec!["osp deploy <COMMAND>"]);
    assert_eq!(
        rebuilt.notes,
        vec!["Run `doctor` before applying production changes."]
    );
    assert_eq!(rebuilt.epilogue, vec!["footer text"]);
    assert_eq!(rebuilt.options.len(), 1);
    assert_eq!(rebuilt.options[0].name, "--verbose");
    assert_eq!(rebuilt.sections.len(), 0);

    let document = output
        .document
        .expect("semantic document should remain attached");
    assert_eq!(
        document.value,
        json!({
            "preamble": ["Deploy commands"],
            "usage": ["osp deploy <COMMAND>"],
            "notes": ["Run `doctor` before applying production changes."],
            "epilogue": ["footer text"],
            "sections": [
                {
                    "title": "Options",
                    "kind": "options",
                    "paragraphs": ["rendering"],
                    "entries": [{"name": "--verbose"}]
                }
            ]
        })
    );
}

// Protects negative-index structural projection: addressed resolution should
// normalize negative indexes before rebuilding, so the last addressed entry is
// selected structurally rather than falling back to flat heuristics.
#[test]
fn help_like_payload_negative_index_projection_rebuilds_last_entry() {
    let output = run_guide_pipeline(help_like_guide(), "P sections[-1].entries[-1].name");

    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should still restore");
    assert_eq!(rebuilt.options.len(), 1);
    assert_eq!(rebuilt.options[0].name, "--json");
    assert_eq!(rebuilt.sections.len(), 0);

    let document = output
        .document
        .expect("semantic document should remain attached");
    assert_eq!(
        document.value,
        json!({
            "preamble": ["Deploy commands"],
            "usage": ["osp deploy <COMMAND>"],
            "notes": ["Run `doctor` before applying production changes."],
            "epilogue": ["footer text"],
            "sections": [
                {
                    "title": "Options",
                    "kind": "options",
                    "paragraphs": ["rendering"],
                    "entries": [{"name": "--json"}]
                }
            ]
        })
    );
}

// Protects the new addressed filter path: an exact indexed predicate should
// rebuild only the matching semantic branch and still restore the guide shell
// around it instead of degrading to flat path fragments.
#[test]
fn help_like_payload_exact_index_filter_rebuilds_selected_branch() {
    let output = run_guide_pipeline(help_like_guide(), "F sections[1].entries[0].name=--verbose");

    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should still restore");
    assert_eq!(rebuilt.preamble, vec!["Deploy commands"]);
    assert_eq!(rebuilt.usage, vec!["osp deploy <COMMAND>"]);
    assert_eq!(
        rebuilt.notes,
        vec!["Run `doctor` before applying production changes."]
    );
    assert_eq!(rebuilt.epilogue, vec!["footer text"]);
    assert_eq!(rebuilt.options.len(), 1);
    assert_eq!(rebuilt.options[0].name, "--verbose");
    assert_eq!(rebuilt.sections.len(), 0);

    let document = output
        .document
        .expect("semantic document should remain attached");
    assert_eq!(
        document.value,
        json!({
            "preamble": ["Deploy commands"],
            "usage": ["osp deploy <COMMAND>"],
            "notes": ["Run `doctor` before applying production changes."],
            "epilogue": ["footer text"],
            "sections": [
                {
                    "title": "Options",
                    "kind": "options",
                    "paragraphs": ["rendering"],
                    "entries": [{"name": "--verbose"}]
                }
            ]
        })
    );
}

// Protects negated exact-address filters: when the addressed predicate passes,
// they should still rebuild the selected branch instead of falling back to a
// whole-document generic match.
#[test]
fn help_like_payload_exact_index_negated_filter_rebuilds_selected_branch() {
    let output = run_guide_pipeline(help_like_guide(), "F sections[1].entries[0].name!=--json");

    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should still restore");
    assert_eq!(rebuilt.options.len(), 1);
    assert_eq!(rebuilt.options[0].name, "--verbose");
    assert_eq!(rebuilt.sections.len(), 0);

    let document = output
        .document
        .expect("semantic document should remain attached");
    assert_eq!(
        document.value,
        json!({
            "preamble": ["Deploy commands"],
            "usage": ["osp deploy <COMMAND>"],
            "notes": ["Run `doctor` before applying production changes."],
            "epilogue": ["footer text"],
            "sections": [
                {
                    "title": "Options",
                    "kind": "options",
                    "paragraphs": ["rendering"],
                    "entries": [{"name": "--verbose"}]
                }
            ]
        })
    );
}

// Protects broader structural filters: fanout path selectors should rebuild
// only the surviving addressed descendants instead of falling back to generic
// descendant traversal over leaf rows.
#[test]
fn help_like_payload_fanout_filter_rebuilds_selected_branch() {
    let output = run_guide_pipeline(help_like_guide(), "F sections[].entries[].name=--json");

    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should still restore");
    assert_eq!(rebuilt.commands.len(), 0);
    assert_eq!(rebuilt.options.len(), 1);
    assert_eq!(rebuilt.options[0].name, "--json");
    assert_eq!(rebuilt.sections.len(), 0);

    let document = output
        .document
        .expect("semantic document should remain attached");
    assert_eq!(
        document.value,
        json!({
            "preamble": ["Deploy commands"],
            "usage": ["osp deploy <COMMAND>"],
            "notes": ["Run `doctor` before applying production changes."],
            "epilogue": ["footer text"],
            "sections": [
                {
                    "title": "Options",
                    "kind": "options",
                    "paragraphs": ["rendering"],
                    "entries": [{"name": "--json"}]
                }
            ]
        })
    );
}

// Protects structural slice projection: slice selectors should rebuild the
// selected addressed range in order and compact away unselected holes.
#[test]
fn help_like_payload_slice_projection_rebuilds_selected_range() {
    let output = run_guide_pipeline(help_like_guide(), "P sections[0].entries[1:3].name");

    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should still restore");
    assert_eq!(rebuilt.commands.len(), 2);
    assert_eq!(rebuilt.commands[0].name, "doctor");
    assert_eq!(rebuilt.commands[1].name, "status");
    assert_eq!(rebuilt.options.len(), 0);
    assert_eq!(rebuilt.sections.len(), 0);

    let document = output
        .document
        .expect("semantic document should remain attached");
    assert_eq!(
        document.value,
        json!({
            "preamble": ["Deploy commands"],
            "usage": ["osp deploy <COMMAND>"],
            "notes": ["Run `doctor` before applying production changes."],
            "epilogue": ["footer text"],
            "sections": [
                {
                    "title": "Commands",
                    "kind": "commands",
                    "paragraphs": ["pick one"],
                    "entries": [{"name": "doctor"}, {"name": "status"}]
                }
            ]
        })
    );
}

// Protects structural fanout negation: removing a matched descendant across a
// fanout path should delete only the addressed leaves, not the containing
// sections or unrelated top-level guide arrays.
#[test]
fn help_like_payload_fanout_negated_path_quick_removes_only_matched_names() {
    let output = run_guide_pipeline(help_like_guide(), "!sections[].entries[].name");

    let document = output
        .document
        .expect("semantic document should remain attached");
    let sections = document.value["sections"]
        .as_array()
        .expect("sections array");
    assert_eq!(sections.len(), 2);
    for section in sections {
        let entries = section["entries"].as_array().expect("entries array");
        assert!(!entries.is_empty(), "entries should remain present");
        for entry in entries {
            assert!(
                entry.get("name").is_none(),
                "fanout negation should remove only the addressed name field"
            );
            assert!(
                entry.get("short_help").is_some(),
                "sibling entry metadata should survive addressed removal"
            );
        }
    }

    let commands = document.value["commands"]
        .as_array()
        .expect("commands array");
    assert_eq!(commands.len(), 3);
    assert_eq!(commands[0]["name"], json!("apply"));
    assert_eq!(commands[1]["name"], json!("doctor"));
    assert_eq!(commands[2]["name"], json!("status"));
}

// Protects structural path quick selection on semantic payloads: path-scoped
// quick should keep the same useful guide envelope as exact structural `P/F`
// instead of dropping the payload shell around the selected branch.
#[test]
fn help_like_payload_path_quick_projects_selected_branch_and_restores() {
    let output = run_guide_pipeline(help_like_guide(), "sections[1].entries[0].name");

    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should restore");
    assert_eq!(rebuilt.preamble, vec!["Deploy commands"]);
    assert_eq!(rebuilt.usage, vec!["osp deploy <COMMAND>"]);
    assert_eq!(
        rebuilt.notes,
        vec!["Run `doctor` before applying production changes."]
    );
    assert_eq!(rebuilt.epilogue, vec!["footer text"]);
    assert_eq!(rebuilt.options.len(), 1);
    assert_eq!(rebuilt.options[0].name, "--verbose");
    assert_eq!(rebuilt.sections.len(), 0);

    let document = output
        .document
        .expect("semantic document should remain attached");
    assert_eq!(
        document.value,
        json!({
            "preamble": ["Deploy commands"],
            "usage": ["osp deploy <COMMAND>"],
            "notes": ["Run `doctor` before applying production changes."],
            "epilogue": ["footer text"],
            "sections": [
                {
                    "title": "Options",
                    "kind": "options",
                    "paragraphs": ["rendering"],
                    "entries": [{"name": "--verbose"}]
                }
            ]
        })
    );
}

// Protects structural path negation on semantic payloads: removing one nested
// addressed branch should keep the remaining guide intact and still restore.
#[test]
fn help_like_payload_negated_path_quick_removes_selected_entry_and_restores() {
    let output = run_guide_pipeline(help_like_guide(), "!sections[1].entries[0]");

    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should still restore");
    assert_eq!(rebuilt.options.len(), 1);
    assert_eq!(rebuilt.options[0].name, "--json");
    assert_eq!(rebuilt.sections.len(), 0);
}

// Protects question-path semantics: `?path` should behave like a structural
// existence filter, keeping the full payload when the addressed path exists.
#[test]
fn help_like_payload_question_path_keeps_full_payload_when_address_exists() {
    let output = run_guide_pipeline(help_like_guide(), "?sections[1].entries[0].name");

    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should still restore");
    assert_eq!(rebuilt.commands.len(), 3);
    assert_eq!(rebuilt.options.len(), 2);
    assert_eq!(rebuilt.sections.len(), 0);
}

// Protects overlapping structural keepers: projecting two exact descendants
// under the same section should merge into one rebuilt branch instead of
// duplicating or dropping siblings during structural union.
#[test]
fn help_like_payload_project_overlapping_structural_keepers_merges_shared_branch() {
    let output = run_guide_pipeline(
        help_like_guide(),
        "P sections[0].entries[0].name sections[0].entries[1].name",
    );

    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should still restore");
    assert_eq!(rebuilt.commands.len(), 2);
    assert_eq!(rebuilt.commands[0].name, "apply");
    assert_eq!(rebuilt.commands[1].name, "doctor");
    assert_eq!(rebuilt.options.len(), 0);
    assert_eq!(rebuilt.sections.len(), 0);

    let document = output
        .document
        .expect("semantic document should remain attached");
    assert_eq!(
        document.value,
        json!({
            "preamble": ["Deploy commands"],
            "usage": ["osp deploy <COMMAND>"],
            "notes": ["Run `doctor` before applying production changes."],
            "epilogue": ["footer text"],
            "sections": [
                {
                    "title": "Commands",
                    "kind": "commands",
                    "paragraphs": ["pick one"],
                    "entries": [{"name": "apply"}, {"name": "doctor"}]
                }
            ]
        })
    );
}
