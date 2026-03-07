use anyhow::{Result, anyhow};
use osp_core::{
    output_model::{OutputItems, OutputMeta, OutputResult},
    row::Row,
};

use crate::{
    eval::context::RowContext,
    model::{ParsedPipeline, ParsedStage, ParsedStageKind},
    parse::pipeline::parse_stage_list,
    stages::{
        aggregate, collapse, copy, filter, group, jq, limit, project, question, quick, sort, values,
    },
};

/// Apply a pipeline to row output and keep the richer `OutputResult` shape.
pub fn apply_pipeline(rows: Vec<Row>, stages: &[String]) -> Result<OutputResult> {
    apply_output_pipeline(OutputResult::from_rows(rows), stages)
}

/// Apply a pipeline to existing output without flattening grouped data first.
pub fn apply_output_pipeline(output: OutputResult, stages: &[String]) -> Result<OutputResult> {
    execute_pipeline_items(output.items, output.meta.wants_copy, stages)
}

/// Execute a pipeline starting from plain rows.
pub fn execute_pipeline(mut rows: Vec<Row>, stages: &[String]) -> Result<OutputResult> {
    execute_pipeline_items(OutputItems::Rows(std::mem::take(&mut rows)), false, stages)
}

fn execute_pipeline_items(
    items: OutputItems,
    initial_wants_copy: bool,
    stages: &[String],
) -> Result<OutputResult> {
    let parsed = parse_stage_list(stages)?;
    PipelineExecutor::new(items, initial_wants_copy, parsed).run()
}

/// Small stateful executor for one parsed pipeline.
///
/// Keeping execution state on a struct makes it easier to read the pipeline
/// flow without carrying `items` / `wants_copy` through every helper.
struct PipelineExecutor {
    items: OutputItems,
    wants_copy: bool,
    parsed: ParsedPipeline,
}

impl PipelineExecutor {
    fn new(items: OutputItems, wants_copy: bool, parsed: ParsedPipeline) -> Self {
        Self {
            items,
            wants_copy,
            parsed,
        }
    }

    fn run(mut self) -> Result<OutputResult> {
        let stages = self.parsed.stages.clone();
        for stage in &stages {
            if stage.verb.is_empty() {
                continue;
            }
            self.apply_stage(stage)?;
        }

        Ok(OutputResult {
            meta: self.build_output_meta(),
            items: self.items,
        })
    }

    fn apply_stage(&mut self, stage: &ParsedStage) -> Result<()> {
        self.items = match stage.kind {
            ParsedStageKind::Quick => self.apply_quick_stage(stage)?,
            ParsedStageKind::UnknownExplicit => {
                return Err(anyhow!("unknown DSL verb: {}", stage.verb));
            }
            ParsedStageKind::Explicit => self.apply_explicit_stage(stage)?,
        };
        Ok(())
    }

    fn apply_quick_stage(&self, stage: &ParsedStage) -> Result<OutputItems> {
        map_rows(self.items.clone(), |rows| quick::apply(rows, &stage.raw))
    }

    fn apply_explicit_stage(&mut self, stage: &ParsedStage) -> Result<OutputItems> {
        match stage.verb.as_str() {
            "P" => self.project(stage),
            // `V` is a quick-search scope alias ("value-only"), not a values stage.
            "V" => self.apply_quick_alias(stage, "V"),
            "K" => self.apply_quick_alias(stage, "K"),
            // `VAL` / `VALUE` produce explicit `{"value": ...}` rows.
            "VAL" | "VALUE" => {
                map_rows(self.items.clone(), |rows| values::apply(rows, &stage.spec))
            }
            "F" => self.filter(stage),
            "G" => self.group(stage),
            "A" => aggregate::apply(self.items.clone(), &stage.spec),
            "S" => sort::apply(self.items.clone(), &stage.spec),
            "L" => self.limit(stage),
            "Z" => Ok(collapse::apply(self.items.clone())),
            "C" => aggregate::count_macro(self.items.clone(), &stage.spec),
            "Y" => self.copy(stage),
            "U" => self.unroll(stage),
            "?" => question::apply(self.items.clone(), &stage.spec),
            "JQ" => jq::apply(self.items.clone(), &stage.spec),
            _ => Err(anyhow!("unknown DSL verb: {}", stage.verb)),
        }
    }

    fn project(&self, stage: &ParsedStage) -> Result<OutputItems> {
        match &self.items {
            OutputItems::Rows(rows) => Ok(OutputItems::Rows(project::apply(
                rows.clone(),
                &stage.spec,
            )?)),
            OutputItems::Groups(groups) => Ok(OutputItems::Groups(project::apply_groups(
                groups.clone(),
                &stage.spec,
            )?)),
        }
    }

    fn filter(&self, stage: &ParsedStage) -> Result<OutputItems> {
        match &self.items {
            OutputItems::Rows(rows) => {
                Ok(OutputItems::Rows(filter::apply(rows.clone(), &stage.spec)?))
            }
            OutputItems::Groups(groups) => Ok(OutputItems::Groups(filter::apply_groups(
                groups.clone(),
                &stage.spec,
            )?)),
        }
    }

    fn group(&self, stage: &ParsedStage) -> Result<OutputItems> {
        match &self.items {
            OutputItems::Rows(rows) => Ok(OutputItems::Groups(group::group_rows(
                rows.clone(),
                &stage.spec,
            )?)),
            OutputItems::Groups(groups) => Ok(OutputItems::Groups(group::regroup_groups(
                groups.clone(),
                &stage.spec,
            )?)),
        }
    }

    fn limit(&self, stage: &ParsedStage) -> Result<OutputItems> {
        match &self.items {
            OutputItems::Rows(rows) => {
                Ok(OutputItems::Rows(limit::apply(rows.clone(), &stage.spec)?))
            }
            OutputItems::Groups(groups) => Ok(OutputItems::Groups(limit::apply(
                groups.clone(),
                &stage.spec,
            )?)),
        }
    }

    fn copy(&mut self, _stage: &ParsedStage) -> Result<OutputItems> {
        self.wants_copy = true;
        map_rows(self.items.clone(), |rows| Ok(copy::apply(rows)))
    }

    fn unroll(&self, stage: &ParsedStage) -> Result<OutputItems> {
        let field = stage.spec.trim();
        if field.is_empty() {
            return Err(anyhow!("U: missing field name to unroll"));
        }

        let selector = format!("{field}[]");
        match &self.items {
            OutputItems::Rows(rows) => {
                Ok(OutputItems::Rows(project::apply(rows.clone(), &selector)?))
            }
            OutputItems::Groups(groups) => Ok(OutputItems::Groups(project::apply_groups(
                groups.clone(),
                &selector,
            )?)),
        }
    }

    fn apply_quick_alias(&self, stage: &ParsedStage, alias: &str) -> Result<OutputItems> {
        let quick_spec = if stage.spec.is_empty() {
            alias.to_string()
        } else {
            format!("{alias} {}", stage.spec)
        };
        map_rows(self.items.clone(), |rows| quick::apply(rows, &quick_spec))
    }

    fn build_output_meta(&self) -> OutputMeta {
        let key_index = match &self.items {
            OutputItems::Rows(rows) => RowContext::from_rows(rows).key_index().to_vec(),
            OutputItems::Groups(groups) => {
                let headers = groups.iter().map(merged_group_header).collect::<Vec<_>>();
                RowContext::from_rows(&headers).key_index().to_vec()
            }
        };

        OutputMeta {
            key_index,
            wants_copy: self.wants_copy,
            grouped: matches!(self.items, OutputItems::Groups(_)),
        }
    }
}

fn merged_group_header(group: &osp_core::output_model::Group) -> Row {
    let mut row = group.groups.clone();
    row.extend(group.aggregates.clone());
    row
}

fn map_rows(
    items: OutputItems,
    map_fn: impl FnOnce(Vec<Row>) -> Result<Vec<Row>>,
) -> Result<OutputItems> {
    match items {
        OutputItems::Rows(rows) => map_fn(rows).map(OutputItems::Rows),
        OutputItems::Groups(groups) => Ok(OutputItems::Groups(groups)),
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
        assert_eq!(
            output_rows(&output)
                .iter()
                .map(|row| row.get("members").cloned().expect("member"))
                .collect::<Vec<_>>(),
            vec![json!("a"), json!("b")]
        );
    }

    #[test]
    fn unroll_requires_field_name() {
        let rows = vec![
            json!({"members": ["a", "b"]})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let err = apply_pipeline(rows, &["U".to_string()]).expect_err("pipeline should fail");
        assert!(err.to_string().contains("missing field name"));
    }

    #[test]
    fn grouped_output_meta_uses_group_headers() {
        let output = apply_output_pipeline(
            OutputResult {
                items: OutputItems::Groups(vec![osp_core::output_model::Group {
                    groups: json!({"dept": "sales"})
                        .as_object()
                        .cloned()
                        .expect("object"),
                    aggregates: json!({"total": 2}).as_object().cloned().expect("object"),
                    rows: vec![],
                }]),
                meta: Default::default(),
            },
            &[],
        )
        .expect("pipeline should pass");

        assert_eq!(output.meta.key_index, vec!["dept", "total"]);
        assert!(output.meta.grouped);
    }
}
