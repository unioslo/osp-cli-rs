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
/// This starts with `wants_copy = false` because there is no prior output meta
/// to preserve.
pub fn apply_pipeline(rows: Vec<Row>, stages: &[String]) -> Result<OutputResult> {
    apply_output_pipeline(OutputResult::from_rows(rows), stages)
}

/// Apply a pipeline to existing output without flattening grouped data first.
///
/// Unlike `apply_pipeline`, this preserves the incoming `OutputMeta.wants_copy`
/// bit when continuing an existing output flow.
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
pub fn execute_pipeline(rows: Vec<Row>, stages: &[String]) -> Result<OutputResult> {
    execute_pipeline_streaming(rows, stages)
}

/// Execute a pipeline from any row iterator.
///
/// This keeps flat row stages on an iterator-backed path until a stage
/// requires full materialization (for example sort/group/aggregate/jq).
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
        let PipelineItems::Semantic(value) = std::mem::replace(
            &mut self.items,
            PipelineItems::Semantic(serde_json::Value::Null),
        ) else {
            unreachable!("semantic stage dispatch requires semantic items");
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
mod tests {
    use crate::core::output::OutputFormat;
    use crate::core::output_model::{
        OutputDocument, OutputDocumentKind, OutputItems, OutputResult, RenderRecommendation,
    };
    use crate::guide::GuideView;
    use serde_json::json;

    use super::{
        apply_output_pipeline, apply_pipeline, execute_pipeline, execute_pipeline_streaming,
    };

    fn output_rows(output: &OutputResult) -> &[crate::core::row::Row] {
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
    fn streaming_executor_matches_eager_for_streamable_row_pipeline() {
        let rows = vec![
            json!({"uid": "alice", "active": true, "members": ["a", "b"]})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"uid": "bob", "active": false, "members": ["c"]})
                .as_object()
                .cloned()
                .expect("object"),
        ];
        let stages = vec![
            "F active=true".to_string(),
            "P uid,members[]".to_string(),
            "L 2".to_string(),
        ];

        let eager = apply_pipeline(rows.clone(), &stages).expect("eager pipeline should pass");
        let streaming =
            execute_pipeline_streaming(rows, &stages).expect("streaming pipeline should pass");

        assert_eq!(streaming, eager);
    }

    #[test]
    fn streaming_executor_matches_eager_for_quick_hot_path() {
        let rows = vec![
            json!({"uid": "alice", "mail": "alice@example.org"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"uid": "bob", "mail": "bob@example.org"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"uid": "carol", "mail": "carol@example.org"})
                .as_object()
                .cloned()
                .expect("object"),
        ];
        let stages = vec!["alice".to_string()];

        let eager = apply_pipeline(rows.clone(), &stages).expect("eager pipeline should pass");
        let streaming =
            execute_pipeline_streaming(rows, &stages).expect("streaming pipeline should pass");

        assert_eq!(streaming, eager);
    }

    #[test]
    fn streaming_executor_preserves_single_row_quick_magic() {
        let rows = vec![
            json!({"uid": "alice", "members": ["eng", "ops"]})
                .as_object()
                .cloned()
                .expect("object"),
        ];
        let stages = vec!["members".to_string()];

        let eager = apply_pipeline(rows.clone(), &stages).expect("eager pipeline should pass");
        let streaming =
            execute_pipeline_streaming(rows, &stages).expect("streaming pipeline should pass");

        assert_eq!(streaming, eager);
        assert_eq!(output_rows(&streaming).len(), 1);
    }

    #[test]
    fn streaming_executor_preserves_copy_flag_and_value_fanout() {
        let rows = vec![
            json!({"uid": "alice", "roles": ["eng", "ops"]})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let output =
            execute_pipeline_streaming(rows, &["Y".to_string(), "VALUE roles".to_string()])
                .expect("streaming pipeline should pass");

        assert!(output.meta.wants_copy);
        assert_eq!(output_rows(&output).len(), 2);
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
                items: OutputItems::Groups(vec![crate::core::output_model::Group {
                    groups: json!({"dept": "sales"})
                        .as_object()
                        .cloned()
                        .expect("object"),
                    aggregates: json!({"total": 2}).as_object().cloned().expect("object"),
                    rows: vec![],
                }]),
                document: None,
                meta: Default::default(),
            },
            &[],
        )
        .expect("pipeline should pass");

        assert_eq!(output.meta.key_index, vec!["dept", "total"]);
        assert!(output.meta.grouped);
    }

    #[test]
    fn grouped_rows_ignore_flat_row_only_projection_and_copy_preserves_flag() {
        let grouped = OutputResult {
            items: OutputItems::Groups(vec![crate::core::output_model::Group {
                groups: json!({"dept": "sales"})
                    .as_object()
                    .cloned()
                    .expect("object"),
                aggregates: json!({"total": 2}).as_object().cloned().expect("object"),
                rows: vec![
                    json!({"uid": "alice"})
                        .as_object()
                        .cloned()
                        .expect("object"),
                ],
            }]),
            document: None,
            meta: Default::default(),
        };

        let projected =
            apply_output_pipeline(grouped.clone(), &["P uid".to_string()]).expect("pipeline works");
        assert_eq!(projected.items, grouped.items);

        let copied = apply_output_pipeline(grouped, &["Y".to_string()]).expect("copy works");
        assert!(copied.meta.wants_copy);
        assert!(copied.meta.grouped);
    }

    #[test]
    fn streaming_materializes_cleanly_at_sort_barrier() {
        let rows = vec![
            json!({"uid": "bob"}).as_object().cloned().expect("object"),
            json!({"uid": "alice"})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let output = execute_pipeline_streaming(rows, &["S uid".to_string()])
            .expect("streaming pipeline should pass");

        assert_eq!(
            output_rows(&output)
                .iter()
                .map(|row| row
                    .get("uid")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default())
                .collect::<Vec<_>>(),
            vec!["alice", "bob"]
        );
    }

    #[test]
    fn grouped_output_pipeline_applies_quick_and_value_stages_to_group_rows_unit() {
        let grouped = OutputResult {
            items: OutputItems::Groups(vec![crate::core::output_model::Group {
                groups: json!({"team": "ops"}).as_object().cloned().expect("object"),
                aggregates: json!({"count": 2}).as_object().cloned().expect("object"),
                rows: vec![
                    json!({"uid": "alice", "roles": ["eng", "ops"]})
                        .as_object()
                        .cloned()
                        .expect("object"),
                    json!({"uid": "bob", "roles": ["sales"]})
                        .as_object()
                        .cloned()
                        .expect("object"),
                ],
            }]),
            document: None,
            meta: Default::default(),
        };

        let value_only = apply_output_pipeline(grouped.clone(), &["V ops".to_string()])
            .expect("grouped quick should succeed");
        let OutputItems::Groups(value_groups) = value_only.items else {
            panic!("expected grouped output");
        };
        assert_eq!(value_groups[0].rows.len(), 1);
        assert_eq!(
            value_groups[0].rows[0].get("roles"),
            Some(&json!(["eng", "ops"]))
        );

        let key_only = apply_output_pipeline(grouped.clone(), &["K uid".to_string()])
            .expect("grouped key quick should succeed");
        let OutputItems::Groups(key_groups) = key_only.items else {
            panic!("expected grouped output");
        };
        assert_eq!(key_groups[0].rows.len(), 2);
        assert!(key_groups[0].rows.iter().all(|row| row.contains_key("uid")));

        let values = apply_output_pipeline(grouped.clone(), &["VALUE uid".to_string()])
            .expect("grouped values should succeed");
        let OutputItems::Groups(value_rows) = values.items else {
            panic!("expected grouped output");
        };
        assert_eq!(value_rows[0].rows.len(), 2);
        assert_eq!(
            value_rows[0]
                .rows
                .iter()
                .map(|row| row.get("value").cloned().expect("value"))
                .collect::<Vec<_>>(),
            vec![json!("alice"), json!("bob")]
        );

        let bare_quick = apply_output_pipeline(grouped.clone(), &["ops".to_string()])
            .expect("grouped bare quick should succeed");
        let OutputItems::Groups(bare_groups) = bare_quick.items else {
            panic!("expected grouped output");
        };
        assert_eq!(bare_groups[0].rows.len(), 1);

        let filtered = apply_output_pipeline(grouped.clone(), &["F uid=alice".to_string()])
            .expect("grouped filter should succeed");
        let OutputItems::Groups(filtered_groups) = filtered.items else {
            panic!("expected grouped output");
        };
        assert_eq!(filtered_groups[0].rows.len(), 1);

        let cleaned = apply_output_pipeline(grouped.clone(), &["? uid".to_string()])
            .expect("grouped clean should succeed");
        let OutputItems::Groups(cleaned_groups) = cleaned.items else {
            panic!("expected grouped output");
        };
        assert_eq!(cleaned_groups[0].rows.len(), 2);

        let copied = apply_output_pipeline(grouped, &["Y".to_string()])
            .expect("grouped copy should succeed");
        assert!(copied.meta.wants_copy);
    }

    #[test]
    fn grouped_output_pipeline_covers_group_limit_and_unroll_paths_unit() {
        let grouped = OutputResult {
            items: OutputItems::Groups(vec![
                crate::core::output_model::Group {
                    groups: json!({"team": "ops"}).as_object().cloned().expect("object"),
                    aggregates: json!({"count": 2}).as_object().cloned().expect("object"),
                    rows: vec![
                        json!({"uid": "alice", "roles": ["eng", "ops"]})
                            .as_object()
                            .cloned()
                            .expect("object"),
                    ],
                },
                crate::core::output_model::Group {
                    groups: json!({"team": "eng"}).as_object().cloned().expect("object"),
                    aggregates: json!({"count": 1}).as_object().cloned().expect("object"),
                    rows: vec![
                        json!({"uid": "bob", "roles": ["ops"]})
                            .as_object()
                            .cloned()
                            .expect("object"),
                    ],
                },
            ]),
            document: None,
            meta: Default::default(),
        };

        let regrouped = apply_output_pipeline(grouped.clone(), &["G team".to_string()])
            .expect("group regroup should succeed");
        assert!(matches!(regrouped.items, OutputItems::Groups(_)));

        let limited = apply_output_pipeline(grouped.clone(), &["L 1".to_string()])
            .expect("group limit should succeed");
        let OutputItems::Groups(limited_groups) = limited.items else {
            panic!("expected grouped output");
        };
        assert_eq!(limited_groups.len(), 1);

        let unrolled = apply_output_pipeline(grouped, &["U roles".to_string()])
            .expect("group unroll should succeed");
        assert!(matches!(unrolled.items, OutputItems::Groups(_)));
    }

    #[test]
    fn streaming_pipeline_covers_stream_stage_variants_and_errors_unit() {
        let rows = vec![
            json!({"uid": "alice", "active": true, "roles": ["eng", "ops"]})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"uid": "bob", "active": false, "roles": ["ops"]})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let value_output = execute_pipeline_streaming(rows.clone(), &["VALUE uid".to_string()])
            .expect("streaming values should succeed");
        assert_eq!(output_rows(&value_output).len(), 2);

        let filtered = execute_pipeline_streaming(rows.clone(), &["? uid".to_string()])
            .expect("question filter should stream");
        assert_eq!(output_rows(&filtered).len(), 2);

        let cleaned = execute_pipeline_streaming(rows.clone(), &["?".to_string()])
            .expect("question clean should stream");
        assert_eq!(output_rows(&cleaned).len(), 2);

        let limited = execute_pipeline_streaming(rows.clone(), &["L 1".to_string()])
            .expect("head limit should stream");
        assert_eq!(output_rows(&limited).len(), 1);

        let unrolled = execute_pipeline_streaming(rows.clone(), &["U roles".to_string()])
            .expect("unroll should stream");
        assert_eq!(output_rows(&unrolled).len(), 3);

        let err = execute_pipeline_streaming(rows, &["U".to_string()])
            .expect_err("missing unroll field should fail");
        assert!(err.to_string().contains("missing field name"));
    }

    #[test]
    fn apply_output_pipeline_covers_explicit_materializing_row_stages_unit() {
        let rows = vec![
            json!({"uid": "bob", "dept": "ops"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"uid": "alice", "dept": "ops"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"uid": "carol", "dept": "eng"})
                .as_object()
                .cloned()
                .expect("object"),
        ];

        let sorted = apply_pipeline(rows.clone(), &["S uid".to_string()]).expect("sort works");
        assert_eq!(
            output_rows(&sorted)[0]
                .get("uid")
                .and_then(|value| value.as_str()),
            Some("alice")
        );

        let grouped = apply_pipeline(rows.clone(), &["G dept".to_string()]).expect("group works");
        assert!(grouped.meta.grouped);

        let aggregated =
            apply_pipeline(rows.clone(), &["A count total".to_string()]).expect("aggregate works");
        assert!(!output_rows(&aggregated).is_empty());

        let counted = apply_pipeline(rows.clone(), &["C".to_string()]).expect("count works");
        assert_eq!(output_rows(&counted).len(), 1);

        let collapsed = apply_pipeline(rows.clone(), &["G dept".to_string(), "Z".to_string()])
            .expect("collapse works");
        assert!(matches!(collapsed.items, OutputItems::Rows(_)));

        let err = apply_pipeline(rows, &["R nope".to_string()])
            .expect_err("unknown explicit stage should fail");
        assert!(err.to_string().contains("unknown DSL verb"));
    }

    #[test]
    fn render_recommendation_survives_narrowing_stages_unit() {
        let rows = vec![
            json!({"uid": "alice", "dept": "ops"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"uid": "bob", "dept": "eng"})
                .as_object()
                .cloned()
                .expect("object"),
        ];
        let mut output = OutputResult::from_rows(rows);
        output.meta.render_recommendation = Some(RenderRecommendation::Guide);

        let quick = apply_output_pipeline(output.clone(), &["alice".to_string()])
            .expect("quick should work");
        assert_eq!(
            quick.meta.render_recommendation,
            Some(RenderRecommendation::Guide)
        );

        let filtered = apply_output_pipeline(output.clone(), &["F dept=ops".to_string()])
            .expect("filter should work");
        assert_eq!(
            filtered.meta.render_recommendation,
            Some(RenderRecommendation::Guide)
        );

        let sorted = apply_output_pipeline(output, &["S uid".to_string()]).expect("sort works");
        assert_eq!(
            sorted.meta.render_recommendation,
            Some(RenderRecommendation::Guide)
        );
    }

    #[test]
    fn render_recommendation_survives_limit_and_copy_unit() {
        let rows = vec![
            json!({"uid": "alice"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"uid": "bob"}).as_object().cloned().expect("object"),
        ];
        let mut output = OutputResult::from_rows(rows);
        output.meta.render_recommendation = Some(RenderRecommendation::Format(OutputFormat::Value));

        let limited =
            apply_output_pipeline(output.clone(), &["L 1".to_string()]).expect("limit should work");
        assert_eq!(
            limited.meta.render_recommendation,
            Some(RenderRecommendation::Format(OutputFormat::Value))
        );

        let copied = apply_output_pipeline(output, &["Y".to_string()]).expect("copy should work");
        assert_eq!(
            copied.meta.render_recommendation,
            Some(RenderRecommendation::Format(OutputFormat::Value))
        );
    }

    #[test]
    fn semantic_document_tracks_transformed_output_unit() {
        let output =
            GuideView::from_text("Usage: osp history <COMMAND>\n\nCommands:\n  list  Show\n")
                .to_output_result();

        let copied =
            apply_output_pipeline(output.clone(), &["Y".to_string()]).expect("copy should work");
        assert!(matches!(
            copied.document,
            Some(OutputDocument {
                kind: OutputDocumentKind::Guide,
                ..
            })
        ));

        let filtered =
            apply_output_pipeline(output, &["list".to_string()]).expect("quick should work");
        let filtered_guide =
            GuideView::try_from_output_result(&filtered).expect("guide should still restore");
        assert_eq!(filtered_guide.commands.len(), 1);
        assert_eq!(filtered_guide.commands[0].name, "list");
    }

    #[test]
    fn semantic_document_is_source_of_truth_for_initial_items_unit() {
        let mut output = OutputResult::from_rows(vec![
            json!({"value": "stale"})
                .as_object()
                .cloned()
                .expect("object"),
        ]);
        output.document = GuideView::from_text("Commands:\n  list  Show\n")
            .to_output_result()
            .document;
        output.meta.render_recommendation = Some(RenderRecommendation::Guide);

        let rebuilt = apply_output_pipeline(output, &[]).expect("pipeline should succeed");
        let guide = GuideView::try_from_output_result(&rebuilt).expect("guide should restore");
        assert_eq!(guide.commands.len(), 1);
        assert_eq!(guide.commands[0].name, "list");
        assert!(
            rebuilt
                .as_rows()
                .expect("rows")
                .first()
                .expect("row")
                .contains_key("commands")
        );
    }

    #[test]
    fn render_recommendation_clears_on_structural_row_reshapes_unit() {
        let rows = vec![
            json!({"uid": "alice", "roles": ["eng", "ops"]})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"uid": "bob", "roles": ["ops"]})
                .as_object()
                .cloned()
                .expect("object"),
        ];
        let mut output = OutputResult::from_rows(rows);
        output.meta.render_recommendation = Some(RenderRecommendation::Guide);

        let projected = apply_output_pipeline(output.clone(), &["P uid".to_string()])
            .expect("project should work");
        assert_eq!(projected.meta.render_recommendation, None);

        let unrolled = apply_output_pipeline(output.clone(), &["U roles".to_string()])
            .expect("unroll should work");
        assert_eq!(unrolled.meta.render_recommendation, None);

        let values =
            apply_output_pipeline(output, &["VALUE uid".to_string()]).expect("values should work");
        assert_eq!(values.meta.render_recommendation, None);
    }

    #[test]
    fn render_recommendation_clears_on_grouping_and_aggregate_stages_unit() {
        let rows = vec![
            json!({"uid": "alice", "dept": "ops"})
                .as_object()
                .cloned()
                .expect("object"),
            json!({"uid": "bob", "dept": "ops"})
                .as_object()
                .cloned()
                .expect("object"),
        ];
        let mut output = OutputResult::from_rows(rows);
        output.meta.render_recommendation = Some(RenderRecommendation::Guide);

        let grouped = apply_output_pipeline(output.clone(), &["G dept".to_string()])
            .expect("group should work");
        assert_eq!(grouped.meta.render_recommendation, None);

        let aggregated = apply_output_pipeline(output.clone(), &["A count total".to_string()])
            .expect("aggregate should work");
        assert_eq!(aggregated.meta.render_recommendation, None);

        let counted = apply_output_pipeline(output, &["C".to_string()]).expect("count works");
        assert_eq!(counted.meta.render_recommendation, None);
    }
}
