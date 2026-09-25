//! One executor over canonical rows and explicit group metadata.
//! Commands unwrap service collections before entering the DSL; raw response
//! documents are retained only for unstaged rendering.

use crate::core::{
    output_model::{OutputItems, OutputResult, compute_key_index},
    row::Row,
};
use anyhow::Result;

use crate::dsl::verbs::{
    aggregate, collapse, filter, group, jq, limit, project, question, quick, sort, unroll, values,
};
use crate::dsl::{
    compiled::{CompiledPipeline, CompiledStage},
    model::RowSet,
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
    let compiled = CompiledPipeline::from_parsed(parse_stage_list(stages)?)?;
    run_compiled(output, &compiled)
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
    apply_output_pipeline(OutputResult::from_rows(rows), stages)
}

pub(crate) fn run_compiled(
    mut output: OutputResult,
    compiled: &CompiledPipeline,
) -> Result<OutputResult> {
    if compiled.stages.is_empty() {
        return Ok(output);
    }
    // A transformed result describes the selected rows, never a stale service
    // envelope (whose total/cursors describe a different collection).
    let guide_view = output
        .document
        .as_ref()
        .and_then(crate::guide::GuideView::pipeline_document);
    output.document = None;
    let mut preserve_guide = guide_view.is_some();
    let mut rows = match &guide_view {
        Some(guide) => RowSet::rows(guide.pipeline_rows()),
        None => RowSet::from(output.items),
    };
    for stage in &compiled.stages {
        // Cleanup is structural only when it changes rows. In particular, a
        // no-op `?` must not discard a narrowed guide's layout and render hint.
        if matches!(stage, CompiledStage::Clean)
            && rows.partitions.iter().all(|partition| {
                partition
                    .rows
                    .iter()
                    .all(|row| !row.is_empty() && !row.values().any(question::is_empty_value))
            })
        {
            continue;
        }
        output.meta.wants_copy |= matches!(stage, CompiledStage::Copy);
        if !stage.behavior().preserves_render_recommendation {
            output.meta.render_recommendation = None;
            output.meta.display_columns = None;
            output.meta.column_align.clear();
            output.meta.unix_timestamp_columns.clear();
            output.meta.display_rules.clear();
        }
        preserve_guide &= matches!(
            stage.behavior().semantic_effect,
            crate::dsl::compiled::SemanticEffect::Preserve
        );
        rows = apply_stage(rows, stage)?;
    }
    output.meta.grouped = rows.grouped;
    output.items = rows.into();
    output.meta.key_index = match &output.items {
        OutputItems::Rows(rows) => compute_key_index(rows),
        OutputItems::Groups(groups) => {
            compute_key_index(&groups.iter().map(merged_group_header).collect::<Vec<_>>())
        }
    };
    if preserve_guide && let (Some(view), OutputItems::Rows(rows)) = (&guide_view, &output.items) {
        output.document = Some(crate::core::output_model::OutputDocument::new(
            crate::core::output_model::OutputDocumentKind::Guide,
            serde_json::to_value(view.select_pipeline_rows(rows))?,
        ));
    }
    Ok(output)
}

pub(crate) fn apply_stage(items: RowSet, stage: &CompiledStage) -> Result<RowSet> {
    if let Some(plan) = stage.quick_plan() {
        return items.map_rows(|rows| quick::apply_with_plan(rows, plan));
    }
    match stage {
        CompiledStage::Filter(plan) => filter::apply_set(items, plan),
        CompiledStage::Project(plan) => items.map_rows(|rows| project::apply_with_plan(rows, plan)),
        CompiledStage::Unroll(plan) => items.map_rows(|rows| unroll::apply_with_plan(rows, plan)),
        CompiledStage::Values(plan) => items.map_rows(|rows| values::apply_with_plan(rows, plan)),
        CompiledStage::Limit(spec) => limit::apply_set(items, *spec),
        CompiledStage::Sort(plan) => sort::apply_set(items, plan),
        CompiledStage::Group(plan) => group::apply_set(items, plan),
        CompiledStage::Aggregate(plan) => aggregate::apply_set(items, plan),
        CompiledStage::Collapse => collapse::apply_set(items),
        CompiledStage::CountMacro => aggregate::count_set(items),
        CompiledStage::Copy => Ok(items),
        CompiledStage::Clean => items.map_rows(|rows| Ok(question::clean_rows(rows))),
        CompiledStage::Jq(expr) => jq::apply_set(items, expr),
        CompiledStage::Quick(_)
        | CompiledStage::Question(_)
        | CompiledStage::ValueQuick(_)
        | CompiledStage::KeyQuick(_) => unreachable!("quick family dispatched above"),
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
