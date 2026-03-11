use super::*;
use serde_json::json;

// Protects the happy semantic path: a narrowed guide payload should still round
// trip through the DSL and restore as a guide rather than degrading to generic
// rows.
#[test]
fn help_like_payload_restores_after_narrowing_multistage_pipeline() {
    let output = run_guide_pipeline(help_like_guide(), "status | ? | S name | L 1");
    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should still restore");

    assert_eq!(rebuilt.commands.len(), 1);
    assert_eq!(rebuilt.commands[0].name, "status");
    assert_eq!(rebuilt.commands[0].short_help, "Show deployment status");
    assert_eq!(rebuilt.preamble, vec!["Deploy commands"]);
    assert_eq!(rebuilt.usage, vec!["osp deploy <COMMAND>"]);
    assert_eq!(
        rebuilt.notes,
        vec!["Run `doctor` before applying production changes."]
    );
    assert_eq!(rebuilt.epilogue, vec!["footer text"]);
    assert!(
        rebuilt.options.is_empty(),
        "unmatched option entries should prune"
    );
    assert_eq!(rebuilt.sections.len(), 0);
}

// Protects semantic envelope preservation specifically for guide-shaped data:
// a descendant match should keep the surviving section title and its entry shell.
#[test]
fn help_like_payload_keeps_section_envelope_for_partial_match() {
    let output = run_guide_pipeline(help_like_guide(), "status | ? | L 1");
    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should still restore");

    assert_eq!(rebuilt.preamble, vec!["Deploy commands"]);
    assert_eq!(rebuilt.usage, vec!["osp deploy <COMMAND>"]);
    assert_eq!(rebuilt.commands.len(), 1);
    assert_eq!(rebuilt.commands[0].name, "status");
    assert_eq!(rebuilt.sections.len(), 0);
}

// Protects the degrade path: once a semantic payload is structurally reshaped
// into generic value rows, restore must stop rather than fabricating guide
// semantics from the new shape.
#[test]
fn help_like_payload_does_not_restore_after_value_extraction_pipeline() {
    let output = run_guide_pipeline(
        help_like_guide(),
        "P commands[].name | VALUE name | S value | L 2",
    );

    assert!(GuideView::try_from_output_result(&output).is_none());
    let OutputItems::Rows(rows) = output.items else {
        panic!("expected flat value rows");
    };
    assert_eq!(
        rows,
        vec![
            row(json!({"value": "apply"})),
            row(json!({"value": "doctor"})),
        ]
    );
}

// Protects the new semantic unroll path: nested entry arrays should duplicate
// their parent section shell per entry instead of flattening into anonymous
// row-like fragments.
#[test]
fn help_like_payload_unroll_preserves_parent_section_shell() {
    let output = run_guide_pipeline(help_like_guide(), "U entries");

    assert!(GuideView::try_from_output_result(&output).is_none());
    let document = output
        .document
        .expect("semantic document should remain attached");
    assert_eq!(
        document.value,
        json!({
            "preamble": ["Deploy commands"],
            "usage": ["osp deploy <COMMAND>"],
            "commands": [
                {"name": "apply", "short_help": "Apply pending changes"},
                {"name": "doctor", "short_help": "Inspect runtime health"},
                {"name": "status", "short_help": "Show deployment status"}
            ],
            "options": [
                {"name": "--verbose", "short_help": "Show additional context"},
                {"name": "--json", "short_help": "Render machine-readable output"}
            ],
            "notes": ["Run `doctor` before applying production changes."],
            "sections": [
                {
                    "title": "Commands",
                    "kind": "commands",
                    "paragraphs": ["pick one"],
                    "entries": {"name": "apply", "short_help": "Apply pending changes"}
                },
                {
                    "title": "Commands",
                    "kind": "commands",
                    "paragraphs": ["pick one"],
                    "entries": {"name": "doctor", "short_help": "Inspect runtime health"}
                },
                {
                    "title": "Commands",
                    "kind": "commands",
                    "paragraphs": ["pick one"],
                    "entries": {"name": "status", "short_help": "Show deployment status"}
                },
                {
                    "title": "Options",
                    "kind": "options",
                    "paragraphs": ["rendering"],
                    "entries": {"name": "--verbose", "short_help": "Show additional context"}
                },
                {
                    "title": "Options",
                    "kind": "options",
                    "paragraphs": ["rendering"],
                    "entries": {"name": "--json", "short_help": "Render machine-readable output"}
                }
            ],
            "epilogue": ["footer text"]
        })
    );
}

// Protects fuzzy quick as a permissive end-to-end semantic feature:
// typo-tolerant guide narrowing should still restore cleanly, keep the useful
// envelope, and retain the intended near-hit without requiring a single exact
// survivor.
#[test]
fn help_like_payload_fuzzy_quick_restores_typo_matched_command() {
    let output = run_guide_pipeline(help_like_guide(), "%docter | ? | L 1");

    let rebuilt = GuideView::try_from_output_result(&output).expect("guide should still restore");
    assert!(
        rebuilt.commands.iter().any(|entry| entry.name == "doctor"),
        "doctor should survive typo-tolerant fuzzy narrowing"
    );
    assert!(
        rebuilt.commands.iter().all(|entry| entry.name != "apply"),
        "unrelated commands should not survive the narrowed guide"
    );
    assert_eq!(rebuilt.preamble, vec!["Deploy commands"]);
    assert_eq!(rebuilt.options.len(), 0);
}
