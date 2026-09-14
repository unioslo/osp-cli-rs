//! Self-contained executor body for the canonical DSL.
//!
//! The document-first executor keeps semantic JSON canonical through the
//! pipeline and lowers to rows/groups only where verb semantics need that
//! substrate.
//!
//! Rule of thumb:
//! - selector verbs narrow or rewrite addressed structure
//! - collection verbs operate on row/group collections
//! - the semantic payload stays canonical JSON until a stage intentionally
//!   degrades it
//!
//! Example:
//! - `help | P commands[].name` stays on the semantic path and rebuilds
//!   `{"commands": [{"name": ...}, ...]}`
//! - `... | VALUE name` then transforms that narrowed structure into value rows
//! - `... | G value` crosses onto the row/group substrate on purpose
//!
//! Keep that boundary explicit. If a selector verb starts looking like a custom
//! row/group traversal, it usually belongs in `verbs::selector` or `verbs::json`
//! instead of growing new engine-side special cases.
//!
//! Caller rule of thumb:
//!
//! - [`apply_pipeline`] is the friendly "I already have rows" entrypoint
//! - [`apply_output_pipeline`] is the continuation path when output already has
//!   semantic-document or metadata state attached

use crate::core::{
    output_model::{
        OutputDocument, OutputDocumentKind, OutputItems, OutputMeta, OutputResult,
        RenderRecommendation, output_items_from_value, rows_from_value,
    },
    row::Row,
};
use anyhow::{Result, anyhow};

use super::value as value_stage;
use crate::dsl::verbs::{
    aggregate, collapse, copy, filter, group, jq, limit, project, question, quick, sort, unroll,
    values,
};
use crate::dsl::{
    compiled::{CompiledPipeline, CompiledStage, SemanticEffect},
    eval::context::RowContext,
    parse::pipeline::parse_stage_list,
};

/// Apply a pipeline to plain row output.
///
/// Use this when a command has already produced `Vec<Row>` and you want the
/// ordinary `osp` pipeline behavior without thinking about existing output
/// metadata.
///
/// This starts with `wants_copy = false` because there is no prior output meta
/// to preserve.
///
/// # Examples
///
/// ```
/// use osp_cli::dsl::apply_pipeline;
/// use osp_cli::row;
///
/// let output = apply_pipeline(
///     vec![
///         row! { "uid" => "alice", "team" => "ops" },
///         row! { "uid" => "bob", "team" => "infra" },
///     ],
///     &["F team=ops".to_string(), "P uid".to_string()],
/// )?;
///
/// let rows = output.as_rows().unwrap();
/// assert_eq!(rows.len(), 1);
/// assert_eq!(rows[0]["uid"], "alice");
/// assert!(!rows[0].contains_key("team"));
/// # Ok::<(), anyhow::Error>(())
/// ```
pub fn apply_pipeline(rows: Vec<Row>, stages: &[String]) -> Result<OutputResult> {
    execute_pipeline(rows, stages)
}

/// Apply a pipeline to existing output without flattening grouped data first.
///
/// Unlike `apply_pipeline`, this preserves the incoming `OutputMeta.wants_copy`
/// bit when continuing an existing output flow.
///
/// Use this when the command already produced an [`OutputResult`] and later
/// stages should inherit its render/document metadata instead of starting from
/// scratch.
///
/// # Examples
///
/// ```
/// use osp_cli::core::output_model::OutputResult;
/// use osp_cli::dsl::apply_output_pipeline;
/// use osp_cli::row;
///
/// let mut output = OutputResult::from_rows(vec![
///     row! { "uid" => "alice" },
///     row! { "uid" => "bob" },
/// ]);
/// output.meta.wants_copy = true;
///
/// let limited = apply_output_pipeline(output, &["L 1".to_string()])?;
///
/// assert!(limited.meta.wants_copy);
/// assert_eq!(limited.as_rows().unwrap().len(), 1);
/// # Ok::<(), anyhow::Error>(())
/// ```
pub fn apply_output_pipeline(output: OutputResult, stages: &[String]) -> Result<OutputResult> {
    execute_pipeline_items(
        output.items,
        output.document,
        output.meta.wants_copy,
        output.meta.render_recommendation,
        output.meta.grouped,
        stages,
    )
}

/// Execute a pipeline starting from plain rows.
///
/// This is the lower-level row entrypoint used by tests and internal helpers.
/// Like `apply_pipeline`, it starts with `wants_copy = false`.
///
/// Prefer [`apply_pipeline`] for the common "rows in, output out" path. This
/// entrypoint is useful when you want the execution wording to distinguish it
/// from the metadata-preserving [`apply_output_pipeline`] path.
///
/// # Examples
///
/// ```
/// use osp_cli::dsl::execute_pipeline;
/// use osp_cli::row;
///
/// let output = execute_pipeline(
///     vec![
///         row! { "uid" => "bob" },
///         row! { "uid" => "alice" },
///     ],
///     &["S uid".to_string()],
/// )?;
///
/// assert_eq!(output.as_rows().unwrap()[0]["uid"], "alice");
/// # Ok::<(), anyhow::Error>(())
/// ```
pub fn execute_pipeline(rows: Vec<Row>, stages: &[String]) -> Result<OutputResult> {
    execute_pipeline_items(OutputItems::Rows(rows), None, false, None, false, stages)
}

fn execute_pipeline_items(
    items: OutputItems,
    initial_document: Option<OutputDocument>,
    initial_wants_copy: bool,
    initial_render_recommendation: Option<RenderRecommendation>,
    initial_grouped: bool,
    stages: &[String],
) -> Result<OutputResult> {
    let parsed = parse_stage_list(stages)?;
    let compiled = CompiledPipeline::from_parsed(parsed)?;
    PipelineExecutor::new(
        items,
        initial_document,
        initial_wants_copy,
        initial_render_recommendation,
        initial_grouped,
    )
    .run(compiled)
}

/// Small stateful executor for one parsed pipeline.
///
/// Keeping execution state on a struct makes it easier to read the pipeline
/// flow without carrying `items` / `wants_copy` through every helper.
enum PipelineItems {
    Materialized(OutputItems),
    Semantic(serde_json::Value),
}

struct PipelineExecutor {
    items: PipelineItems,
    document_kind: Option<OutputDocumentKind>,
    wants_copy: bool,
    render_recommendation: Option<RenderRecommendation>,
    semantic_grouped: bool,
}

impl PipelineExecutor {
    fn new(
        items: OutputItems,
        document: Option<OutputDocument>,
        wants_copy: bool,
        render_recommendation: Option<RenderRecommendation>,
        semantic_grouped: bool,
    ) -> Self {
        let semantic_grouped = semantic_grouped && document.is_some();
        // Document identity is explicit; a preferred renderer never changes
        // whether selectors operate on rows or on a nested document.
        let (items, document_kind) = match document {
            Some(document) => (PipelineItems::Semantic(document.value), Some(document.kind)),
            None => (PipelineItems::Materialized(items), None),
        };
        Self {
            items,
            document_kind,
            wants_copy,
            render_recommendation,
            semantic_grouped,
        }
    }

    fn run(mut self, compiled: CompiledPipeline) -> Result<OutputResult> {
        for stage in &compiled.stages {
            self.apply_stage(stage)?;
        }
        Ok(self.into_output_result())
    }

    fn apply_stage(&mut self, stage: &CompiledStage) -> Result<()> {
        let behavior = stage.behavior();
        self.apply_stage_side_effects(stage);
        if !behavior.preserves_render_recommendation {
            self.render_recommendation = None;
        }

        if matches!(self.items, PipelineItems::Semantic(_)) {
            self.apply_semantic_stage(stage, behavior.semantic_effect)
        } else {
            let items = self.materialize_items();
            self.items = PipelineItems::Materialized(self.apply_flat_stage(items, stage)?);
            Ok(())
        }
    }

    fn apply_stage_side_effects(&mut self, stage: &CompiledStage) {
        if matches!(stage, CompiledStage::Copy) {
            self.wants_copy = true;
        }
    }

    fn apply_semantic_stage(
        &mut self,
        stage: &CompiledStage,
        semantic_effect: SemanticEffect,
    ) -> Result<()> {
        let items = std::mem::replace(
            &mut self.items,
            PipelineItems::Semantic(serde_json::Value::Null),
        );
        let PipelineItems::Semantic(value) = items else {
            self.items = items;
            return Err(anyhow!("semantic stage dispatch requires semantic items"));
        };

        let transformed =
            value_stage::apply_stage_preserving_matching_rows(value, stage, self.semantic_grouped)?;
        match stage {
            CompiledStage::Group(_) => self.semantic_grouped = true,
            CompiledStage::Collapse
            | CompiledStage::CountMacro
            | CompiledStage::Jq(_)
            | CompiledStage::Values(_) => self.semantic_grouped = false,
            _ => {}
        }
        self.items = PipelineItems::Semantic(transformed);
        match semantic_effect {
            // Preserve/transform both keep the semantic payload attached. The
            // renderer decides later whether the transformed JSON still
            // restores as the original semantic kind.
            SemanticEffect::Preserve | SemanticEffect::Transform => {}
            // Destructive stages like `C`, `Z`, and `JQ` intentionally stop
            // claiming the result is still guide/help-shaped semantic output.
            SemanticEffect::Degrade => {
                self.document_kind = None;
            }
        }
        Ok(())
    }

    fn apply_flat_stage(
        &mut self,
        items: OutputItems,
        stage: &CompiledStage,
    ) -> Result<OutputItems> {
        if let Some(plan) = stage.quick_plan() {
            return match items {
                OutputItems::Rows(rows) => {
                    quick::apply_with_plan(rows, plan).map(OutputItems::Rows)
                }
                OutputItems::Groups(groups) => {
                    quick::apply_groups_with_plan(groups, plan).map(OutputItems::Groups)
                }
            };
        }

        match stage {
            CompiledStage::Filter(plan) => match items {
                OutputItems::Rows(rows) => {
                    filter::apply_with_plan(rows, plan).map(OutputItems::Rows)
                }
                OutputItems::Groups(groups) => {
                    filter::apply_groups_with_plan(groups, plan).map(OutputItems::Groups)
                }
            },
            CompiledStage::Project(plan) => match items {
                OutputItems::Rows(rows) => {
                    project::apply_with_plan(rows, plan).map(OutputItems::Rows)
                }
                OutputItems::Groups(groups) => {
                    project::apply_groups_with_plan(groups, plan).map(OutputItems::Groups)
                }
            },
            CompiledStage::Unroll(plan) => match items {
                OutputItems::Rows(rows) => {
                    unroll::apply_with_plan(rows, plan).map(OutputItems::Rows)
                }
                OutputItems::Groups(groups) => {
                    unroll::apply_groups_with_plan(groups, plan).map(OutputItems::Groups)
                }
            },
            CompiledStage::Values(plan) => match items {
                OutputItems::Rows(rows) => {
                    values::apply_with_plan(rows, plan).map(OutputItems::Rows)
                }
                OutputItems::Groups(groups) => {
                    values::apply_groups_with_plan(groups, plan).map(OutputItems::Groups)
                }
            },
            CompiledStage::Limit(spec) => match items {
                OutputItems::Rows(rows) => {
                    Ok(OutputItems::Rows(limit::apply_with_spec(rows, *spec)))
                }
                OutputItems::Groups(groups) => {
                    Ok(OutputItems::Groups(limit::apply_with_spec(groups, *spec)))
                }
            },
            CompiledStage::Sort(plan) => sort::apply_with_plan(items, plan),
            CompiledStage::Group(spec) => match items {
                OutputItems::Rows(rows) => Ok(OutputItems::Groups(group::group_rows_with_plan(
                    rows, spec,
                )?)),
                OutputItems::Groups(groups) => Ok(OutputItems::Groups(
                    group::regroup_groups_with_plan(groups, spec)?,
                )),
            },
            CompiledStage::Aggregate(plan) => aggregate::apply_with_plan(items, plan),
            CompiledStage::Collapse => collapse::apply(items),
            CompiledStage::CountMacro => aggregate::count_macro(items, ""),
            CompiledStage::Copy => Ok(match items {
                OutputItems::Rows(rows) => OutputItems::Rows(copy::apply(rows)),
                OutputItems::Groups(groups) => OutputItems::Groups(groups),
            }),
            CompiledStage::Clean => Ok(question::clean_items(items)),
            CompiledStage::Jq(expr) => jq::apply_with_expr(items, expr),
            CompiledStage::Quick(_)
            | CompiledStage::Question(_)
            | CompiledStage::ValueQuick(_)
            | CompiledStage::KeyQuick(_) => Err(anyhow!(
                "quick family should have been handled before flat-stage dispatch"
            )),
        }
    }

    fn materialize_items(&mut self) -> OutputItems {
        match std::mem::replace(
            &mut self.items,
            PipelineItems::Materialized(OutputItems::Rows(Vec::new())),
        ) {
            PipelineItems::Materialized(items) => items,
            PipelineItems::Semantic(value) => decode_semantic_items(value, self.semantic_grouped),
        }
    }

    fn into_output_result(self) -> OutputResult {
        let (items, document) = match self.items {
            PipelineItems::Materialized(items) => (items, None),
            PipelineItems::Semantic(value) => match self.document_kind {
                Some(kind) => (
                    decode_semantic_items(value.clone(), self.semantic_grouped),
                    Some(OutputDocument::new(kind, value)),
                ),
                None => (decode_semantic_items(value, self.semantic_grouped), None),
            },
        };
        let key_index = match &items {
            OutputItems::Rows(rows) => RowContext::from_rows(rows).key_index().to_vec(),
            OutputItems::Groups(groups) => {
                let headers = groups.iter().map(merged_group_header).collect::<Vec<_>>();
                RowContext::from_rows(&headers).key_index().to_vec()
            }
        };

        let meta = OutputMeta {
            key_index,
            column_align: Vec::new(),
            wants_copy: self.wants_copy,
            grouped: self.semantic_grouped || matches!(&items, OutputItems::Groups(_)),
            render_recommendation: self.render_recommendation,
        };
        OutputResult {
            items,
            document,
            meta,
        }
    }
}

fn decode_semantic_items(value: serde_json::Value, grouped: bool) -> OutputItems {
    if grouped {
        output_items_from_value(value)
    } else {
        OutputItems::Rows(rows_from_value(value))
    }
}

fn merged_group_header(group: &crate::core::output_model::Group) -> Row {
    let mut row = group.groups.clone();
    row.extend(group.aggregates.clone());
    row
}

#[cfg(test)]
#[path = "tests/engine.rs"]
mod tests;
