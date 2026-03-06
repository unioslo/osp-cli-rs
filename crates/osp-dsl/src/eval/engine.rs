use anyhow::{Result, anyhow};
use osp_core::{
    output_model::{OutputItems, OutputMeta, OutputResult},
    row::Row,
};

use crate::{
    eval::context::RowContext,
    parse::pipeline::parse_stage_list,
    stages::{
        aggregate, collapse, copy, filter, group, jq, limit, project, question, quick, sort, values,
    },
};

pub fn apply_pipeline(rows: Vec<Row>, stages: &[String]) -> Result<OutputResult> {
    apply_output_pipeline(OutputResult::from_rows(rows), stages)
}

pub fn apply_output_pipeline(output: OutputResult, stages: &[String]) -> Result<OutputResult> {
    execute_pipeline_items(output.items, output.meta.wants_copy, stages)
}

pub fn execute_pipeline(mut rows: Vec<Row>, stages: &[String]) -> Result<OutputResult> {
    execute_pipeline_items(OutputItems::Rows(std::mem::take(&mut rows)), false, stages)
}

fn execute_pipeline_items(
    mut items: OutputItems,
    initial_wants_copy: bool,
    stages: &[String],
) -> Result<OutputResult> {
    let parsed = parse_stage_list(stages)?;
    let mut wants_copy = initial_wants_copy;

    for stage in parsed.stages {
        if stage.verb.is_empty() {
            continue;
        }

        items = match stage.kind {
            crate::model::ParsedStageKind::Quick => {
                map_rows(items, |rows| quick::apply(rows, &stage.raw))?
            }
            crate::model::ParsedStageKind::UnknownExplicit => {
                return Err(anyhow!("unknown DSL verb: {}", stage.verb));
            }
            crate::model::ParsedStageKind::Explicit => match stage.verb.as_str() {
                "P" => match items {
                    OutputItems::Rows(rows) => {
                        OutputItems::Rows(project::apply(rows, &stage.spec)?)
                    }
                    OutputItems::Groups(groups) => {
                        OutputItems::Groups(project::apply_groups(groups, &stage.spec)?)
                    }
                },
                // `V` is a quick-search scope alias ("value-only"), not a
                // value-output stage.
                "V" => map_rows(items, |rows| {
                    let quick_spec = if stage.spec.is_empty() {
                        "V".to_string()
                    } else {
                        format!("V {}", stage.spec)
                    };
                    quick::apply(rows, &quick_spec)
                })?,
                "K" => map_rows(items, |rows| {
                    let quick_spec = if stage.spec.is_empty() {
                        "K".to_string()
                    } else {
                        format!("K {}", stage.spec)
                    };
                    quick::apply(rows, &quick_spec)
                })?,
                // Explicit extraction stage producing `{\"value\": ...}` rows.
                // Kept separate from `V` so verbs do not imply output format mode.
                "VAL" | "VALUE" => map_rows(items, |rows| values::apply(rows, &stage.spec))?,
                "F" => match items {
                    OutputItems::Rows(rows) => OutputItems::Rows(filter::apply(rows, &stage.spec)?),
                    OutputItems::Groups(groups) => {
                        OutputItems::Groups(filter::apply_groups(groups, &stage.spec)?)
                    }
                },
                "G" => match items {
                    OutputItems::Rows(rows) => {
                        OutputItems::Groups(group::group_rows(rows, &stage.spec)?)
                    }
                    OutputItems::Groups(groups) => {
                        OutputItems::Groups(group::regroup_groups(groups, &stage.spec)?)
                    }
                },
                "A" => aggregate::apply(items, &stage.spec)?,
                "S" => sort::apply(items, &stage.spec)?,
                "L" => match items {
                    OutputItems::Rows(rows) => OutputItems::Rows(limit::apply(rows, &stage.spec)?),
                    OutputItems::Groups(groups) => {
                        OutputItems::Groups(limit::apply(groups, &stage.spec)?)
                    }
                },
                "Z" => collapse::apply(items),
                "C" => aggregate::count_macro(items, &stage.spec)?,
                "Y" => {
                    wants_copy = true;
                    map_rows(items, |rows| Ok(copy::apply(rows)))?
                }
                "U" => {
                    let field = stage.spec.trim();
                    if field.is_empty() {
                        return Err(anyhow!("U: missing field name to unroll"));
                    }
                    let selector = format!("{field}[]");
                    match items {
                        OutputItems::Rows(rows) => {
                            OutputItems::Rows(project::apply(rows, &selector)?)
                        }
                        OutputItems::Groups(groups) => {
                            OutputItems::Groups(project::apply_groups(groups, &selector)?)
                        }
                    }
                }
                "?" => question::apply(items, &stage.spec)?,
                "JQ" => jq::apply(items, &stage.spec)?,
                _ => return Err(anyhow!("unknown DSL verb: {}", stage.verb)),
            },
        };
    }

    let key_index = match &items {
        OutputItems::Rows(rows) => RowContext::from_rows(rows).key_index().to_vec(),
        OutputItems::Groups(groups) => {
            let collapsed = groups
                .iter()
                .map(|group| {
                    let mut row = Row::new();
                    row.extend(group.groups.clone());
                    row.extend(group.aggregates.clone());
                    row
                })
                .collect::<Vec<_>>();
            RowContext::from_rows(&collapsed).key_index().to_vec()
        }
    };
    let grouped = matches!(items, OutputItems::Groups(_));

    Ok(OutputResult {
        items,
        meta: OutputMeta {
            key_index,
            wants_copy,
            grouped,
        },
    })
}

fn map_rows(
    items: OutputItems,
    map_fn: impl FnOnce(Vec<Row>) -> Result<Vec<Row>>,
) -> Result<OutputItems> {
    match items {
        OutputItems::Rows(rows) => map_fn(rows).map(OutputItems::Rows),
        OutputItems::Groups(_) => Ok(items),
    }
}

#[cfg(test)]
mod tests {
    use osp_core::output_model::{OutputItems, OutputResult};
    use serde_json::json;

    use super::{apply_output_pipeline, apply_pipeline, execute_pipeline};

    fn output_rows(output: &OutputResult) -> &[osp_core::row::Row] {
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

        let err =
            apply_pipeline(rows, &["R oist".to_string()]).expect_err("unknown verb should fail");
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
    fn unroll_stage_expands_list_field() {
        let rows = vec![
            json!({"members": ["a", "b"], "cn": "grp"})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let output =
            apply_pipeline(rows, &["U members".to_string()]).expect("pipeline should pass");
        assert_eq!(output_rows(&output).len(), 2);
        let values = output_rows(&output)
            .iter()
            .filter_map(|row| row.get("members").and_then(|v| v.as_str()))
            .collect::<Vec<_>>();
        assert!(values.contains(&"a"));
        assert!(values.contains(&"b"));
    }

    #[test]
    fn explicit_val_stage_extracts_values() {
        let rows = vec![
            json!({"members": ["oistes", "andreasd"]})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let stages = vec!["VAL members".to_string()];
        let output = apply_pipeline(rows, &stages).expect("pipeline should pass");
        assert_eq!(output_rows(&output).len(), 2);
        assert_eq!(
            output_rows(&output)[0]
                .get("value")
                .and_then(|value| value.as_str()),
            Some("oistes")
        );
    }

    #[test]
    fn limit_stage_takes_head() {
        let rows = vec![
            json!({"uid": "a"}).as_object().cloned().expect("object"),
            json!({"uid": "b"}).as_object().cloned().expect("object"),
            json!({"uid": "c"}).as_object().cloned().expect("object"),
        ];

        let output = apply_pipeline(rows, &["L 2".to_string()]).expect("pipeline should pass");
        assert_eq!(output_rows(&output).len(), 2);
        assert_eq!(
            output_rows(&output)[0]
                .get("uid")
                .and_then(|value| value.as_str()),
            Some("a")
        );
    }

    #[test]
    fn collapse_stage_is_noop_for_rows() {
        let rows = vec![
            json!({"uid": "oistes"})
                .as_object()
                .cloned()
                .expect("object"),
        ];
        let output =
            apply_pipeline(rows.clone(), &["Z".to_string()]).expect("pipeline should pass");
        assert_eq!(output_rows(&output), rows.as_slice());
    }

    #[test]
    fn group_stage_sets_grouped_meta() {
        let rows = vec![
            json!({"dept": "sales", "id": 1})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"dept": "eng", "id": 2})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let output = execute_pipeline(rows, &["G dept".to_string()]).expect("pipeline should pass");
        assert!(output.meta.grouped);
        match output.items {
            OutputItems::Groups(groups) => assert_eq!(groups.len(), 2),
            OutputItems::Rows(_) => panic!("expected grouped output"),
        }
    }

    #[test]
    fn count_macro_collapses_to_rows() {
        let rows = vec![
            json!({"id": 1}).as_object().cloned().expect("object"),
            json!({"id": 2}).as_object().cloned().expect("object"),
            json!({"id": 3}).as_object().cloned().expect("object"),
        ];

        let output = execute_pipeline(rows, &["C".to_string()]).expect("pipeline should pass");
        assert!(!output.meta.grouped);
        match output.items {
            OutputItems::Rows(rows) => {
                assert_eq!(rows.len(), 1);
                assert_eq!(
                    rows[0].get("count").and_then(|value| value.as_i64()),
                    Some(3)
                );
            }
            OutputItems::Groups(_) => panic!("expected rows"),
        }
    }

    #[test]
    fn grouped_output_survives_apply_output_pipeline() {
        let grouped = execute_pipeline(
            vec![
                json!({"dept": "sales", "host": "alpha"})
                    .as_object()
                    .cloned()
                    .expect("object"),
                json!({"dept": "sales", "host": "beta"})
                    .as_object()
                    .cloned()
                    .expect("object"),
            ],
            &["G dept".to_string()],
        )
        .expect("grouping should pass");

        let output = apply_output_pipeline(grouped, &[]).expect("pipeline should preserve groups");
        assert!(matches!(output.items, OutputItems::Groups(_)));
    }
}
