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
//! - [`execute_pipeline_streaming`] is the iterator-oriented path when callers
//!   want streamable stages to avoid eager materialization

use crate::core::{
    output_model::{
        OutputDocument, OutputItems, OutputMeta, OutputResult, RenderRecommendation,
        output_items_from_value,
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
    apply_output_pipeline(OutputResult::from_rows(rows), stages)
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
        stages,
    )
}

/// Execute a pipeline starting from plain rows.
///
/// This is the lower-level row entrypoint used by tests and internal helpers.
/// Like `apply_pipeline`, it starts with `wants_copy = false`.
///
/// Prefer [`apply_pipeline`] for the common "rows in, output out" path. This
/// entrypoint is more useful when you want the execution wording to match the
/// streaming variant below.
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
    execute_pipeline_streaming(rows, stages)
}

/// Execute a pipeline from any row iterator.
///
/// This keeps flat row stages on an iterator-backed path until a stage
/// requires full materialization (for example sort/group/aggregate/jq).
///
/// Use this when rows come from an iterator and you want streamable stages like
/// `F`, `P`, `U`, and head-only `L` to stay incremental for as long as
/// possible.
///
/// # Examples
///
/// ```
/// use osp_cli::dsl::execute_pipeline_streaming;
/// use osp_cli::row;
///
/// let output = execute_pipeline_streaming(
///     vec![
///         row! { "uid" => "alice" },
///         row! { "uid" => "bob" },
///     ],
///     &["L 1".to_string()],
/// )?;
///
/// assert_eq!(output.as_rows().unwrap()[0]["uid"], "alice");
/// assert_eq!(output.as_rows().unwrap().len(), 1);
/// # Ok::<(), anyhow::Error>(())
/// ```
pub fn execute_pipeline_streaming<I>(rows: I, stages: &[String]) -> Result<OutputResult>
where
    I: IntoIterator<Item = Row>,
    I::IntoIter: 'static,
{
    let parsed = parse_stage_list(stages)?;
    let compiled = CompiledPipeline::from_parsed(parsed)?;
    PipelineExecutor::new_stream(rows.into_iter(), false, compiled).run()
}

fn execute_pipeline_items(
    items: OutputItems,
    initial_document: Option<OutputDocument>,
    initial_wants_copy: bool,
    initial_render_recommendation: Option<RenderRecommendation>,
    stages: &[String],
) -> Result<OutputResult> {
    let parsed = parse_stage_list(stages)?;
    let compiled = CompiledPipeline::from_parsed(parsed)?;
    PipelineExecutor::new(
        items,
        initial_document,
        initial_wants_copy,
        initial_render_recommendation,
        compiled,
    )
    .run()
}

/// Small stateful executor for one parsed pipeline.
///
/// Keeping execution state on a struct makes it easier to read the pipeline
/// flow without carrying `items` / `wants_copy` through every helper.
type RowStream = Box<dyn Iterator<Item = Result<Row>>>;

enum PipelineItems {
    RowStream(RowStream),
    Materialized(OutputItems),
    Semantic(serde_json::Value),
}

struct PipelineExecutor {
    items: PipelineItems,
    document: Option<OutputDocument>,
    wants_copy: bool,
    render_recommendation: Option<RenderRecommendation>,
    compiled: CompiledPipeline,
}

impl PipelineExecutor {
    fn new(
        items: OutputItems,
        document: Option<OutputDocument>,
        wants_copy: bool,
        render_recommendation: Option<RenderRecommendation>,
        compiled: CompiledPipeline,
    ) -> Self {
        let items = if let Some(document) = document.as_ref() {
            // Semantic payloads stay canonical as JSON through the DSL.
            // Generic rows/groups are derived only when the pipeline needs to
            // emit the final `OutputResult`.
            PipelineItems::Semantic(document.value.clone())
        } else {
            match items {
                OutputItems::Rows(rows) => {
                    PipelineItems::RowStream(Box::new(rows.into_iter().map(Ok)))
                }
                OutputItems::Groups(groups) => {
                    PipelineItems::Materialized(OutputItems::Groups(groups))
                }
            }
        };
        Self {
            items,
            document,
            wants_copy,
            render_recommendation,
            compiled,
        }
    }

    fn new_stream<I>(rows: I, wants_copy: bool, compiled: CompiledPipeline) -> Self
    where
        I: Iterator<Item = Row> + 'static,
    {
        Self {
            items: PipelineItems::RowStream(Box::new(rows.map(Ok))),
            document: None,
            wants_copy,
            render_recommendation: None,
            compiled,
        }
    }

    fn run(mut self) -> Result<OutputResult> {
        let stages = self.compiled.stages.clone();
        for stage in &stages {
            self.apply_stage(stage)?;
        }
        self.into_output_result()
    }

    fn apply_stage(&mut self, stage: &CompiledStage) -> Result<()> {
        if !stage.preserves_render_recommendation() {
            self.render_recommendation = None;
        }

        if matches!(self.items, PipelineItems::Semantic(_)) {
            self.apply_semantic_stage(stage)?;
            return Ok(());
        }

        if stage.can_stream()
            && let PipelineItems::RowStream(_) = self.items
        {
            self.apply_stream_stage(stage)?;
            return Ok(());
        }

        let items = self.materialize_items()?;
        self.items = PipelineItems::Materialized(self.apply_flat_stage(items, stage)?);
        self.sync_document_to_items();
        Ok(())
    }

    fn apply_semantic_stage(&mut self, stage: &CompiledStage) -> Result<()> {
        let items = std::mem::replace(
            &mut self.items,
            PipelineItems::Semantic(serde_json::Value::Null),
        );
        let PipelineItems::Semantic(value) = items else {
            self.items = items;
            return Err(anyhow!("semantic stage dispatch requires semantic items"));
        };

        if matches!(stage, CompiledStage::Copy) {
            self.wants_copy = true;
        }

        let transformed = value_stage::apply_stage(value, stage)?;
        self.items = PipelineItems::Semantic(transformed);
        match stage.semantic_effect() {
            // Preserve/transform both keep the semantic payload attached. The
            // renderer decides later whether the transformed JSON still
            // restores as the original semantic kind.
            SemanticEffect::Preserve | SemanticEffect::Transform => {
                self.sync_document_to_items();
            }
            // Destructive stages like `C`, `Z`, and `JQ` intentionally stop
            // claiming the result is still guide/help-shaped semantic output.
            SemanticEffect::Degrade => {
                self.document = None;
            }
        }
        Ok(())
    }

    fn apply_stream_stage(&mut self, stage: &CompiledStage) -> Result<()> {
        let stream = match std::mem::replace(
            &mut self.items,
            PipelineItems::RowStream(Box::new(std::iter::empty())),
        ) {
            PipelineItems::RowStream(stream) => stream,
            PipelineItems::Materialized(items) => {
                debug_assert!(
                    false,
                    "apply_stream_stage called after pipeline had already materialized"
                );
                self.items = PipelineItems::Materialized(items);
                return Ok(());
            }
            PipelineItems::Semantic(value) => {
                debug_assert!(
                    false,
                    "apply_stream_stage called for semantic payload execution"
                );
                self.items = PipelineItems::Semantic(value);
                return Ok(());
            }
        };

        self.items = PipelineItems::RowStream(match stage {
            CompiledStage::Quick(plan) => {
                Box::new(quick::stream_rows_with_plan(stream, plan.clone()))
            }
            CompiledStage::Filter(plan) => {
                let plan = plan.clone();
                Box::new(stream.filter_map(move |row| match row {
                    Ok(row) if plan.matches(&row) => Some(Ok(row)),
                    Ok(_) => None,
                    Err(err) => Some(Err(err)),
                }))
            }
            CompiledStage::Project(plan) => {
                let plan = plan.clone();
                stream_row_fanout_result(stream, move |row| plan.project_row(&row))
            }
            CompiledStage::Unroll(plan) => {
                let plan = plan.clone();
                stream_row_fanout_result(stream, move |row| plan.expand_row(&row))
            }
            CompiledStage::Values(plan) => {
                let plan = plan.clone();
                stream_row_fanout(stream, move |row| plan.extract_row(&row))
            }
            CompiledStage::Limit(spec) => {
                debug_assert!(spec.is_head_only());
                Box::new(
                    stream
                        .skip(spec.offset as usize)
                        .take(spec.count.max(0) as usize),
                )
            }
            CompiledStage::Copy => {
                self.wants_copy = true;
                stream
            }
            CompiledStage::ValueQuick(plan)
            | CompiledStage::KeyQuick(plan)
            | CompiledStage::Question(plan) => {
                Box::new(quick::stream_rows_with_plan(stream, plan.clone()))
            }
            CompiledStage::Clean => Box::new(stream.filter_map(|row| match row {
                Ok(row) => question::clean_row(row).map(Ok),
                Err(err) => Some(Err(err)),
            })),
            other => {
                return Err(anyhow!(
                    "stream stage not implemented for compiled stage: {:?}",
                    other
                ));
            }
        });
        Ok(())
    }

    fn apply_flat_stage(
        &mut self,
        items: OutputItems,
        stage: &CompiledStage,
    ) -> Result<OutputItems> {
        match stage {
            CompiledStage::Quick(plan) => match items {
                OutputItems::Rows(rows) => {
                    quick::apply_with_plan(rows, plan).map(OutputItems::Rows)
                }
                OutputItems::Groups(groups) => {
                    quick::apply_groups_with_plan(groups, plan).map(OutputItems::Groups)
                }
            },
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
            CompiledStage::ValueQuick(plan)
            | CompiledStage::KeyQuick(plan)
            | CompiledStage::Question(plan) => match items {
                OutputItems::Rows(rows) => {
                    quick::apply_with_plan(rows, plan).map(OutputItems::Rows)
                }
                OutputItems::Groups(groups) => {
                    quick::apply_groups_with_plan(groups, plan).map(OutputItems::Groups)
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
            CompiledStage::Copy => {
                self.wants_copy = true;
                Ok(match items {
                    OutputItems::Rows(rows) => OutputItems::Rows(copy::apply(rows)),
                    OutputItems::Groups(groups) => OutputItems::Groups(groups),
                })
            }
            CompiledStage::Clean => Ok(question::clean_items(items)),
            CompiledStage::Jq(expr) => jq::apply_with_expr(items, expr),
        }
    }

    fn materialize_items(&mut self) -> Result<OutputItems> {
        match std::mem::replace(
            &mut self.items,
            PipelineItems::Materialized(OutputItems::Rows(Vec::new())),
        ) {
            PipelineItems::RowStream(stream) => {
                let rows = materialize_row_stream(stream)?;
                Ok(OutputItems::Rows(rows))
            }
            PipelineItems::Materialized(items) => Ok(items),
            PipelineItems::Semantic(value) => Ok(output_items_from_value(value)),
        }
    }

    fn finish_items(&mut self) -> Result<OutputItems> {
        self.materialize_items()
    }

    fn into_output_result(mut self) -> Result<OutputResult> {
        // Capture the semantic value before finish_items replaces self.items via
        // mem::replace — after that call self.items is always Materialized.
        let semantic_value = if let PipelineItems::Semantic(ref v) = self.items {
            Some(v.clone())
        } else {
            None
        };
        let items = self.finish_items()?;
        let meta = self.build_output_meta(&items);
        let document = match semantic_value {
            Some(value) => self.document.map(|document| OutputDocument {
                kind: document.kind,
                value,
            }),
            None => self.document,
        };

        Ok(OutputResult {
            items,
            document,
            meta,
        })
    }

    fn sync_document_to_items(&mut self) {
        let Some(document) = self.document.as_mut() else {
            return;
        };
        match &self.items {
            PipelineItems::Materialized(items) => {
                *document = document.project_over_items(items);
            }
            PipelineItems::Semantic(value) => {
                document.value = value.clone();
            }
            PipelineItems::RowStream(_) => {}
        }
    }

    fn build_output_meta(&self, items: &OutputItems) -> OutputMeta {
        let key_index = match items {
            OutputItems::Rows(rows) => RowContext::from_rows(rows).key_index().to_vec(),
            OutputItems::Groups(groups) => {
                let headers = groups.iter().map(merged_group_header).collect::<Vec<_>>();
                RowContext::from_rows(&headers).key_index().to_vec()
            }
        };

        OutputMeta {
            key_index,
            column_align: Vec::new(),
            wants_copy: self.wants_copy,
            grouped: matches!(items, OutputItems::Groups(_)),
            render_recommendation: self.render_recommendation,
        }
    }
}

fn materialize_row_stream(stream: RowStream) -> Result<Vec<Row>> {
    stream.collect()
}

fn stream_row_fanout<I, F>(stream: RowStream, fanout: F) -> RowStream
where
    I: IntoIterator<Item = Row>,
    F: Fn(Row) -> I + 'static,
{
    Box::new(stream.flat_map(move |row| {
        match row {
            Ok(row) => fanout(row)
                .into_iter()
                .map(Ok)
                .collect::<Vec<_>>()
                .into_iter(),
            Err(err) => vec![Err(err)].into_iter(),
        }
    }))
}

fn stream_row_fanout_result<I, F>(stream: RowStream, fanout: F) -> RowStream
where
    I: IntoIterator<Item = Row>,
    F: Fn(Row) -> Result<I> + 'static,
{
    Box::new(stream.flat_map(move |row| match row {
        Ok(row) => match fanout(row) {
            Ok(rows) => rows.into_iter().map(Ok).collect::<Vec<_>>().into_iter(),
            Err(err) => vec![Err(err)].into_iter(),
        },
        Err(err) => vec![Err(err)].into_iter(),
    }))
}

fn merged_group_header(group: &crate::core::output_model::Group) -> Row {
    let mut row = group.groups.clone();
    row.extend(group.aggregates.clone());
    row
}

#[cfg(test)]
#[path = "tests/engine.rs"]
mod tests;
