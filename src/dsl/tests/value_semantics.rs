use serde_json::json;

use crate::dsl::{
    compiled::CompiledStage,
    model::{ParsedStage, ParsedStageKind},
};

use super::apply_stage;

fn stage(kind: ParsedStageKind, verb: &str, spec: &str, raw: &str) -> CompiledStage {
    let parsed = ParsedStage::new(kind, verb, spec, raw);
    CompiledStage::from_parsed(&parsed).expect("stage should compile")
}

fn guide_like_value() -> serde_json::Value {
    json!({
        "usage": ["osp intro"],
        "commands": [
            {"name": "help", "short_help": "Show overview"},
            {"name": "exit", "short_help": "Leave shell"}
        ],
        "sections": [
            {
                "title": "Commands",
                "kind": "commands",
                "paragraphs": [],
                "entries": [
                    {"name": "help", "short_help": "Show overview"},
                    {"name": "exit", "short_help": "Leave shell"}
                ]
            }
        ]
    })
}

fn two_section_value() -> serde_json::Value {
    json!({
        "sections": [
            {
                "title": "Commands",
                "kind": "commands",
                "entries": [
                    {"name": "apply", "short_help": "Apply pending changes"}
                ]
            },
            {
                "title": "Options",
                "kind": "options",
                "entries": [
                    {"name": "--verbose", "short_help": "Show additional context"}
                ]
            }
        ]
    })
}

#[test]
fn quick_keeps_only_matching_root_fields_and_matching_array_elements_unit() {
    let value = json!({
        "usage": ["osp intro"],
        "notes": ["Read this first"],
        "commands": [
            {"name": "help", "short_help": "Show overview"},
            {"name": "exit", "short_help": "Leave shell"}
        ]
    });

    let filtered = apply_stage(value, &stage(ParsedStageKind::Quick, "", "", "show"))
        .expect("quick stage should succeed");

    assert!(filtered.get("commands").is_some());
    assert!(filtered.get("usage").is_none());
    assert!(filtered.get("notes").is_none());
    let commands = filtered["commands"].as_array().expect("commands array");
    assert_eq!(commands.len(), 1);
    assert_eq!(
        commands[0],
        json!({"name": "help", "short_help": "Show overview"})
    );
}

#[test]
fn quick_keeps_matching_array_object_elements_whole_without_inventing_values_unit() {
    let value = json!({
        "k": [
            "a",
            "b",
            {"a": "d", "k": "c"}
        ]
    });

    let filtered = apply_stage(value, &stage(ParsedStageKind::Quick, "", "", "c"))
        .expect("quick stage should succeed");

    assert_eq!(
        filtered,
        json!({
            "k": [
                {"a": "d", "k": "c"}
            ]
        })
    );
}

#[test]
fn quick_narrows_singleton_matching_array_element_when_whole_element_would_be_noop_unit() {
    let value = json!({
        "k": [
            {"c2": "d2", "e1": "e2"}
        ]
    });

    let filtered = apply_stage(value, &stage(ParsedStageKind::Quick, "", "", "d2"))
        .expect("quick stage should succeed");

    assert_eq!(
        filtered,
        json!({
            "k": [
                {"c2": "d2"}
            ]
        })
    );
}

#[test]
fn limit_trims_nested_semantic_collections_unit() {
    let value = json!({
        "commands": [
            {"name": "help", "short_help": "Show overview"},
            {"name": "exit", "short_help": "Leave shell"}
        ]
    });

    let limited = apply_stage(value, &stage(ParsedStageKind::Explicit, "L", "1", "L 1"))
        .expect("limit stage should succeed");

    let commands = limited["commands"].as_array().expect("commands array");
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0]["name"], json!("help"));
}

#[test]
fn quick_preserves_container_metadata_when_descendants_match_unit() {
    let value = json!({
        "sections": [
            {
                "title": "Commands",
                "kind": "commands",
                "paragraphs": [],
                "entries": [
                    {"name": "help", "short_help": "Show overview"},
                    {"name": "exit", "short_help": "Leave shell"}
                ]
            }
        ]
    });

    let filtered = apply_stage(value, &stage(ParsedStageKind::Quick, "", "", "show"))
        .expect("quick stage should succeed");

    let section = &filtered["sections"].as_array().expect("sections")[0];
    assert_eq!(section["title"], json!("Commands"));
    assert_eq!(section["kind"], json!("commands"));
    assert_eq!(section["entries"].as_array().expect("entries").len(), 1);
}

#[test]
fn filter_preserves_section_metadata_when_descendants_match_unit() {
    let filtered = apply_stage(
        guide_like_value(),
        &stage(ParsedStageKind::Explicit, "F", "name=help", "F name=help"),
    )
    .expect("filter stage should succeed");

    let section = &filtered["sections"].as_array().expect("sections")[0];
    assert_eq!(section["title"], json!("Commands"));
    assert_eq!(section["kind"], json!("commands"));
    assert_eq!(section["entries"].as_array().expect("entries").len(), 1);
    assert_eq!(section["entries"][0]["name"], json!("help"));
}

#[test]
fn filter_addressed_path_failures_collapse_to_null_unit() {
    for spec in [
        "sections[0].entries[1].name!=exit",
        "sections[5].entries[0].name=help",
    ] {
        let raw = format!("F {spec}");
        let filtered = apply_stage(
            guide_like_value(),
            &stage(ParsedStageKind::Explicit, "F", spec, &raw),
        )
        .expect("filter stage should succeed");

        assert_eq!(filtered, json!(null), "spec={spec}");
    }
}

#[test]
fn project_path_dropper_only_removes_selected_branch_unit() {
    let projected = apply_stage(
        guide_like_value(),
        &stage(
            ParsedStageKind::Explicit,
            "P",
            "!sections[0].entries[0]",
            "P !sections[0].entries[0]",
        ),
    )
    .expect("project stage should succeed");

    let section = &projected["sections"].as_array().expect("sections")[0];
    assert_eq!(section["entries"].as_array().expect("entries").len(), 1);
    assert_eq!(section["entries"][0]["name"], json!("exit"));
    assert_eq!(projected["commands"].as_array().expect("commands").len(), 2);
}

#[test]
fn project_structural_selection_preserves_selected_null_array_items_unit() {
    let projected = apply_stage(
        json!({"items": [0, null, 2]}),
        &stage(ParsedStageKind::Explicit, "P", "items[1]", "P items[1]"),
    )
    .expect("project stage should succeed");

    assert_eq!(projected, json!({"items": [null]}));
}

#[test]
fn project_structural_selection_preserves_user_strings_that_look_like_old_hole_markers_unit() {
    let projected = apply_stage(
        json!({"items": ["\u{0}__osp_sparse_hole__"]}),
        &stage(ParsedStageKind::Explicit, "P", "items[0]", "P items[0]"),
    )
    .expect("project stage should succeed");

    assert_eq!(projected, json!({"items": ["\u{0}__osp_sparse_hole__"]}));
}

#[test]
fn project_relative_path_requires_real_path_matches_unit() {
    let projected = apply_stage(
        json!({"x": {"a.b": 1}}),
        &stage(ParsedStageKind::Explicit, "P", "a.b", "P a.b"),
    )
    .expect("project stage should succeed");

    assert_eq!(projected, json!(null));
}

#[test]
fn project_structural_keeper_and_dropper_variants_preserve_section_alignment_unit() {
    let expected = json!({
        "sections": [
            {
                "title": "Options",
                "kind": "options",
                "entries": [{"name": "--verbose"}]
            }
        ]
    });

    for spec in [
        "title sections[1].entries[0].name",
        "sections[1].entries[0].name !sections[0]",
    ] {
        let raw = format!("P {spec}");
        let projected = apply_stage(
            two_section_value(),
            &stage(ParsedStageKind::Explicit, "P", spec, &raw),
        )
        .expect("project stage should succeed");

        assert_eq!(projected, expected, "spec={spec}");
    }
}

#[test]
fn negated_path_quick_preserves_user_strings_that_look_like_old_remove_markers_unit() {
    let projected = apply_stage(
        json!({"items": ["\u{0}__osp_removed_value__", "drop"]}),
        &stage(ParsedStageKind::Quick, "", "", "!items[1]"),
    )
    .expect("quick stage should succeed");

    assert_eq!(projected, json!({"items": ["\u{0}__osp_removed_value__"]}));
}

#[test]
fn value_scope_alias_filters_semantic_entries_by_value_unit() {
    let filtered = apply_stage(
        guide_like_value(),
        &stage(ParsedStageKind::Explicit, "V", "show", "V show"),
    )
    .expect("value-scope quick stage should succeed");

    let section = &filtered["sections"].as_array().expect("sections")[0];
    assert_eq!(section["title"], json!("Commands"));
    assert_eq!(section["entries"].as_array().expect("entries").len(), 1);
    assert_eq!(section["entries"][0]["name"], json!("help"));
}

#[test]
fn key_scope_alias_filters_semantic_entries_by_key_unit() {
    let filtered = apply_stage(
        guide_like_value(),
        &stage(ParsedStageKind::Explicit, "K", "name", "K name"),
    )
    .expect("key-scope quick stage should succeed");

    let section = &filtered["sections"].as_array().expect("sections")[0];
    assert_eq!(section["title"], json!("Commands"));
    assert_eq!(section["entries"].as_array().expect("entries").len(), 2);
}

#[test]
fn fuzzy_quick_filters_semantic_entries_with_typo_tolerance_unit() {
    let filtered = apply_stage(
        guide_like_value(),
        &stage(ParsedStageKind::Quick, "", "", "% exti"),
    )
    .expect("fuzzy quick stage should succeed");

    let section = &filtered["sections"].as_array().expect("sections")[0];
    assert_eq!(section["title"], json!("Commands"));
    assert_eq!(section["entries"].as_array().expect("entries").len(), 1);
    assert_eq!(section["entries"][0]["name"], json!("exit"));
}

#[test]
fn fuzzy_quick_rejects_structural_path_tokens_unit() {
    let parsed = ParsedStage::new(
        ParsedStageKind::Quick,
        "",
        "",
        "% sections[0].entries[0].name",
    );
    let err = CompiledStage::from_parsed(&parsed).expect_err("fuzzy path quick should fail");
    assert!(
        err.to_string().contains("does not support path selectors"),
        "unexpected error: {err}"
    );
}

#[test]
fn sort_orders_nested_semantic_entry_collections_unit() {
    let sorted = apply_stage(
        json!({
            "commands": [
                {"name": "help", "short_help": "Show overview"},
                {"name": "exit", "short_help": "Leave shell"}
            ]
        }),
        &stage(ParsedStageKind::Explicit, "S", "name", "S name"),
    )
    .expect("sort stage should succeed");

    let commands = sorted["commands"].as_array().expect("commands");
    assert_eq!(commands[0]["name"], json!("exit"));
    assert_eq!(commands[1]["name"], json!("help"));
}

#[test]
fn group_groups_nested_semantic_entry_collections_unit() {
    let grouped = apply_stage(
        json!({
            "commands": [
                {"name": "help", "short_help": "Show overview"},
                {"name": "help", "short_help": "More help"},
                {"name": "exit", "short_help": "Leave shell"}
            ]
        }),
        &stage(ParsedStageKind::Explicit, "G", "name", "G name"),
    )
    .expect("group stage should succeed");

    let commands = grouped["commands"].as_array().expect("commands");
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[0]["groups"]["name"], json!("help"));
    assert_eq!(commands[0]["rows"].as_array().expect("rows").len(), 2);
}

#[test]
fn aggregate_and_count_aliases_produce_same_nested_semantic_count_unit() {
    for (verb, spec, raw) in [("A", "count AS count", "A count AS count"), ("C", "", "C")] {
        let counted = apply_stage(
            json!({
                "commands": [
                    {"name": "help", "short_help": "Show overview"},
                    {"name": "exit", "short_help": "Leave shell"}
                ]
            }),
            &stage(ParsedStageKind::Explicit, verb, spec, raw),
        )
        .expect("counting stage should succeed");

        let commands = counted["commands"].as_array().expect("commands array");
        assert_eq!(commands.len(), 1, "raw={raw}");
        assert_eq!(commands[0]["count"], json!(2), "raw={raw}");
    }
}

#[test]
fn unroll_expands_nested_semantic_entry_collections_unit() {
    let unrolled = apply_stage(
        json!({
            "sections": [
                {
                    "title": "Commands",
                    "kind": "commands",
                    "entries": [
                        {"name": "help", "short_help": "Show overview"},
                        {"name": "exit", "short_help": "Leave shell"}
                    ]
                }
            ]
        }),
        &stage(ParsedStageKind::Explicit, "U", "entries", "U entries"),
    )
    .expect("unroll stage should succeed");

    let sections = unrolled["sections"].as_array().expect("sections");
    assert_eq!(sections.len(), 2);
    assert_eq!(sections[0]["title"], json!("Commands"));
    assert_eq!(sections[0]["entries"]["name"], json!("help"));
    assert_eq!(sections[1]["title"], json!("Commands"));
    assert_eq!(sections[1]["entries"]["name"], json!("exit"));
}

#[test]
fn question_cleans_nested_semantic_values_unit() {
    let cleaned = apply_stage(
        json!({
            "usage": [""],
            "commands": [
                {"name": "help", "short_help": ""},
                {"name": "exit", "short_help": "Leave shell"}
            ],
            "notes": []
        }),
        &stage(ParsedStageKind::Explicit, "?", "", "?"),
    )
    .expect("question stage should succeed");

    assert!(cleaned.get("usage").is_none());
    assert!(cleaned.get("notes").is_none());
    let commands = cleaned["commands"].as_array().expect("commands");
    assert_eq!(commands.len(), 2);
    assert!(commands[0].get("short_help").is_none());
}

#[test]
fn value_stage_extracts_nested_semantic_values_unit() {
    let values = apply_stage(
        json!({
            "commands": [
                {"name": "help", "short_help": "Show overview"},
                {"name": "exit", "short_help": "Leave shell"}
            ]
        }),
        &stage(ParsedStageKind::Explicit, "VALUE", "name", "VALUE name"),
    )
    .expect("value stage should succeed");

    let rows = values.as_array().expect("value rows");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["value"], json!("help"));
    assert_eq!(rows[1]["value"], json!("exit"));
}
