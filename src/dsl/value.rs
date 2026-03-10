use anyhow::Result;
use serde_json::Value;

use crate::dsl::compiled::CompiledStage;
use crate::dsl::verbs::{
    aggregate, collapse, filter, group, jq, limit, project, question, quick, sort, unroll, values,
};

/// Applies one parsed stage directly to canonical JSON.
///
/// The semantic/document path keeps `Value` as the source of truth.
/// Stages may still reuse existing row/group operators for local tabular
/// collections, but the executor itself no longer treats those projections as
/// the canonical substrate.
pub(crate) fn apply_stage(value: Value, stage: &CompiledStage) -> Result<Value> {
    match stage {
        CompiledStage::Quick(plan) => quick::apply_value_with_plan(value, plan),
        CompiledStage::Filter(plan) => filter::apply_value_with_plan(value, plan),
        CompiledStage::Project(plan) => project::apply_value_with_plan(value, plan),
        CompiledStage::Unroll(plan) => unroll::apply_value_with_plan(value, plan),
        CompiledStage::Sort(plan) => sort::apply_value_with_plan(value, plan),
        CompiledStage::Group(plan) => group::apply_value_with_plan(value, plan),
        CompiledStage::Aggregate(plan) => aggregate::apply_value_with_plan(value, plan),
        CompiledStage::Limit(spec) => limit::apply_value_with_spec(value, *spec),
        CompiledStage::Collapse => collapse::apply_value(value),
        CompiledStage::CountMacro => aggregate::count_macro_value(value, ""),
        CompiledStage::Copy => Ok(value),
        CompiledStage::Clean => question::apply_value(value, ""),
        CompiledStage::Question(plan)
        | CompiledStage::ValueQuick(plan)
        | CompiledStage::KeyQuick(plan) => quick::apply_value_with_plan(value, plan),
        CompiledStage::Jq(expr) => jq::apply_value_with_expr(value, expr),
        CompiledStage::Values(plan) => values::apply_value_with_plan(value, plan),
    }
}

#[cfg(test)]
mod tests {
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
    fn quick_filters_semantic_container_fields_without_flattening_unit() {
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
        assert_eq!(filtered["usage"], json!(["osp intro"]));
        assert_eq!(filtered["notes"], json!(["Read this first"]));
        let commands = filtered["commands"].as_array().expect("commands array");
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0]["name"], json!("help"));
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
    fn filter_exact_index_path_rebuilds_selected_semantic_branch_unit() {
        let filtered = apply_stage(
            guide_like_value(),
            &stage(
                ParsedStageKind::Explicit,
                "F",
                "sections[0].entries[1].name=exit",
                "F sections[0].entries[1].name=exit",
            ),
        )
        .expect("filter stage should succeed");

        assert_eq!(filtered["usage"], json!(["osp intro"]));
        let section = &filtered["sections"].as_array().expect("sections")[0];
        assert_eq!(section["title"], json!("Commands"));
        assert_eq!(section["kind"], json!("commands"));
        assert_eq!(section["entries"].as_array().expect("entries").len(), 1);
        assert_eq!(section["entries"][0]["name"], json!("exit"));
        assert!(filtered.get("commands").is_none());
    }

    #[test]
    fn filter_exact_negated_index_path_rebuilds_selected_branch_when_predicate_passes_unit() {
        let filtered = apply_stage(
            guide_like_value(),
            &stage(
                ParsedStageKind::Explicit,
                "F",
                "sections[0].entries[1].name!=help",
                "F sections[0].entries[1].name!=help",
            ),
        )
        .expect("filter stage should succeed");

        assert_eq!(filtered["usage"], json!(["osp intro"]));
        let section = &filtered["sections"].as_array().expect("sections")[0];
        assert_eq!(section["title"], json!("Commands"));
        assert_eq!(section["kind"], json!("commands"));
        assert_eq!(section["entries"].as_array().expect("entries").len(), 1);
        assert_eq!(section["entries"][0]["name"], json!("exit"));
        assert!(filtered.get("commands").is_none());
    }

    #[test]
    fn filter_exact_negated_index_path_drops_when_predicate_fails_unit() {
        let filtered = apply_stage(
            guide_like_value(),
            &stage(
                ParsedStageKind::Explicit,
                "F",
                "sections[0].entries[1].name!=exit",
                "F sections[0].entries[1].name!=exit",
            ),
        )
        .expect("filter stage should succeed");

        assert_eq!(filtered, json!(null));
    }

    #[test]
    fn filter_fanout_path_rebuilds_only_matching_semantic_branch_unit() {
        let filtered = apply_stage(
            guide_like_value(),
            &stage(
                ParsedStageKind::Explicit,
                "F",
                "sections[].entries[].name=exit",
                "F sections[].entries[].name=exit",
            ),
        )
        .expect("filter stage should succeed");

        assert_eq!(filtered["usage"], json!(["osp intro"]));
        let section = &filtered["sections"].as_array().expect("sections")[0];
        assert_eq!(section["title"], json!("Commands"));
        assert_eq!(section["kind"], json!("commands"));
        assert_eq!(section["entries"].as_array().expect("entries").len(), 1);
        assert_eq!(section["entries"][0]["name"], json!("exit"));
        assert!(filtered.get("commands").is_none());
    }

    #[test]
    fn filter_out_of_bounds_structural_path_returns_null_unit() {
        let filtered = apply_stage(
            guide_like_value(),
            &stage(
                ParsedStageKind::Explicit,
                "F",
                "sections[5].entries[0].name=help",
                "F sections[5].entries[0].name=help",
            ),
        )
        .expect("filter stage should succeed");

        assert_eq!(filtered, json!(null));
    }

    #[test]
    fn project_preserves_section_metadata_when_descendants_are_projected_unit() {
        let projected = apply_stage(
            guide_like_value(),
            &stage(ParsedStageKind::Explicit, "P", "name", "P name"),
        )
        .expect("project stage should succeed");

        let section = &projected["sections"].as_array().expect("sections")[0];
        assert_eq!(section["title"], json!("Commands"));
        assert_eq!(section["kind"], json!("commands"));
        assert_eq!(section["entries"].as_array().expect("entries").len(), 2);
        assert_eq!(section["entries"][0]["name"], json!("help"));
        assert!(section["entries"][0].get("short_help").is_none());
    }

    #[test]
    fn project_mixed_exact_and_generic_keepers_stays_structural_unit() {
        let projected = apply_stage(
            guide_like_value(),
            &stage(
                ParsedStageKind::Explicit,
                "P",
                "usage sections[0].entries[1].name",
                "P usage sections[0].entries[1].name",
            ),
        )
        .expect("project stage should succeed");

        assert_eq!(projected["usage"], json!(["osp intro"]));
        let section = &projected["sections"].as_array().expect("sections")[0];
        assert_eq!(section["title"], json!("Commands"));
        assert_eq!(section["kind"], json!("commands"));
        assert_eq!(section["entries"].as_array().expect("entries").len(), 1);
        assert_eq!(section["entries"][0]["name"], json!("exit"));
        assert!(projected.get("commands").is_none());
    }

    #[test]
    fn project_exact_projection_with_droppers_stays_structural_unit() {
        let projected = apply_stage(
            guide_like_value(),
            &stage(
                ParsedStageKind::Explicit,
                "P",
                "sections[0].entries[1].name !short_help",
                "P sections[0].entries[1].name !short_help",
            ),
        )
        .expect("project stage should succeed");

        let section = &projected["sections"].as_array().expect("sections")[0];
        assert_eq!(section["title"], json!("Commands"));
        assert_eq!(section["kind"], json!("commands"));
        assert_eq!(section["entries"][0]["name"], json!("exit"));
        assert!(section["entries"][0].get("short_help").is_none());
        assert!(projected.get("commands").is_none());
    }

    #[test]
    fn project_fanout_path_rebuilds_selected_descendants_unit() {
        let projected = apply_stage(
            guide_like_value(),
            &stage(
                ParsedStageKind::Explicit,
                "P",
                "sections[].entries[].name",
                "P sections[].entries[].name",
            ),
        )
        .expect("project stage should succeed");

        assert_eq!(projected["usage"], json!(["osp intro"]));
        let section = &projected["sections"].as_array().expect("sections")[0];
        assert_eq!(section["title"], json!("Commands"));
        assert_eq!(section["kind"], json!("commands"));
        let entries = section["entries"].as_array().expect("entries");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["name"], json!("help"));
        assert!(entries[0].get("short_help").is_none());
        assert_eq!(entries[1]["name"], json!("exit"));
        assert!(projected.get("commands").is_none());
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
    fn project_mixed_structural_and_generic_keepers_keep_original_section_alignment_unit() {
        let projected = apply_stage(
            two_section_value(),
            &stage(
                ParsedStageKind::Explicit,
                "P",
                "title sections[1].entries[0].name",
                "P title sections[1].entries[0].name",
            ),
        )
        .expect("project stage should succeed");

        assert_eq!(
            projected,
            json!({
                "sections": [
                    {
                        "title": "Options",
                        "kind": "options",
                        "entries": [{"name": "--verbose"}]
                    }
                ]
            })
        );
    }

    #[test]
    fn project_structural_droppers_use_original_indexes_after_structural_keepers_unit() {
        let projected = apply_stage(
            two_section_value(),
            &stage(
                ParsedStageKind::Explicit,
                "P",
                "sections[1].entries[0].name !sections[0]",
                "P sections[1].entries[0].name !sections[0]",
            ),
        )
        .expect("project stage should succeed");

        assert_eq!(
            projected,
            json!({
                "sections": [
                    {
                        "title": "Options",
                        "kind": "options",
                        "entries": [{"name": "--verbose"}]
                    }
                ]
            })
        );
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
    fn aggregate_counts_nested_semantic_entry_collections_unit() {
        let aggregated = apply_stage(
            json!({
                "commands": [
                    {"name": "help", "short_help": "Show overview"},
                    {"name": "exit", "short_help": "Leave shell"}
                ]
            }),
            &stage(
                ParsedStageKind::Explicit,
                "A",
                "count AS count",
                "A count AS count",
            ),
        )
        .expect("aggregate stage should succeed");

        let commands = aggregated["commands"].as_array().expect("commands array");
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0]["count"], json!(2));
    }

    #[test]
    fn count_macro_counts_nested_semantic_entry_collections_unit() {
        let counted = apply_stage(
            json!({
                "commands": [
                    {"name": "help", "short_help": "Show overview"},
                    {"name": "exit", "short_help": "Leave shell"}
                ]
            }),
            &stage(ParsedStageKind::Explicit, "C", "", "C"),
        )
        .expect("count stage should succeed");

        let commands = counted["commands"].as_array().expect("commands array");
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0]["count"], json!(2));
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

    #[test]
    fn value_keeps_sibling_field_identity_for_same_object_unit() {
        let values = apply_stage(
            guide_like_value(),
            &stage(
                ParsedStageKind::Explicit,
                "VALUE",
                "sections[0].entries[0].name sections[0].entries[0].short_help",
                "VALUE sections[0].entries[0].name sections[0].entries[0].short_help",
            ),
        )
        .expect("value stage should succeed");

        assert_eq!(
            values,
            json!({
                "sections": [
                    {
                        "title": "Commands",
                        "kind": "commands",
                        "paragraphs": [],
                        "entries": [
                            {
                                "name": {"value": "help"},
                                "short_help": {"value": "Show overview"}
                            }
                        ]
                    }
                ]
            })
        );
    }
}
