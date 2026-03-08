use crate::core::{
    output_model::{OutputItems, OutputMeta, OutputResult},
    row::Row,
};
use anyhow::{Result, anyhow};

use crate::dsl::{
    eval::context::RowContext,
    model::{ParsedPipeline, ParsedStage, ParsedStageKind},
    parse::pipeline::parse_stage_list,
    stages::{
        aggregate, collapse, copy, filter, group, jq, limit, project, question, quick, sort, values,
    },
    verbs::stage_can_stream_rows,
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
    execute_pipeline_items(output.items, output.meta.wants_copy, stages)
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
    PipelineExecutor::new_stream(rows.into_iter(), false, parsed).run()
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
type RowStream = Box<dyn Iterator<Item = Result<Row>>>;

enum PipelineItems {
    RowStream(RowStream),
    Materialized(OutputItems),
}

struct PipelineExecutor {
    items: PipelineItems,
    wants_copy: bool,
    parsed: ParsedPipeline,
}

impl PipelineExecutor {
    fn new(items: OutputItems, wants_copy: bool, parsed: ParsedPipeline) -> Self {
        Self {
            items: match items {
                OutputItems::Rows(rows) => {
                    PipelineItems::RowStream(Box::new(rows.into_iter().map(Ok)))
                }
                OutputItems::Groups(groups) => {
                    PipelineItems::Materialized(OutputItems::Groups(groups))
                }
            },
            wants_copy,
            parsed,
        }
    }

    fn new_stream<I>(rows: I, wants_copy: bool, parsed: ParsedPipeline) -> Self
    where
        I: Iterator<Item = Row> + 'static,
    {
        Self {
            items: PipelineItems::RowStream(Box::new(rows.map(Ok))),
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

        let items = self.finish_items()?;
        let meta = self.build_output_meta(&items);

        Ok(OutputResult { meta, items })
    }

    fn apply_stage(&mut self, stage: &ParsedStage) -> Result<()> {
        if stage_can_stream_rows(stage)
            && let PipelineItems::RowStream(_) = self.items
        {
            self.apply_stream_stage(stage)?;
            return Ok(());
        }

        let items = self.materialize_items()?;
        self.items = PipelineItems::Materialized(match stage.kind {
            ParsedStageKind::Quick => self.apply_quick_stage(items, stage)?,
            ParsedStageKind::UnknownExplicit => {
                return Err(anyhow!("unknown DSL verb: {}", stage.verb));
            }
            ParsedStageKind::Explicit => self.apply_explicit_stage(items, stage)?,
        });
        Ok(())
    }

    fn apply_stream_stage(&mut self, stage: &ParsedStage) -> Result<()> {
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
        };

        self.items = PipelineItems::RowStream(match stage.verb.as_str() {
            _ if matches!(stage.kind, ParsedStageKind::Quick) => {
                let plan = quick::compile(&stage.raw)?;
                Box::new(quick::stream_rows_with_plan(stream, plan))
            }
            "F" => {
                let plan = filter::compile(&stage.spec)?;
                Box::new(stream.filter_map(move |row| match row {
                    Ok(row) if plan.matches(&row) => Some(Ok(row)),
                    Ok(_) => None,
                    Err(err) => Some(Err(err)),
                }))
            }
            "P" => {
                let plan = project::compile(&stage.spec)?;
                stream_row_fanout(stream, move |row| plan.project_row(&row))
            }
            "VAL" | "VALUE" => {
                let plan = values::compile(&stage.spec);
                stream_row_fanout(stream, move |row| plan.extract_row(&row))
            }
            "L" => {
                let spec = limit::parse_limit_spec(&stage.spec)?;
                debug_assert!(spec.is_head_only());
                Box::new(
                    stream
                        .skip(spec.offset as usize)
                        .take(spec.count.max(0) as usize),
                )
            }
            "Y" => {
                self.wants_copy = true;
                stream
            }
            "V" | "K" => {
                let plan = quick::compile(&format!(
                    "{}{}{}",
                    stage.verb,
                    if stage.spec.is_empty() { "" } else { " " },
                    stage.spec
                ))?;
                Box::new(quick::stream_rows_with_plan(stream, plan))
            }
            "U" => {
                let field = stage.spec.trim();
                if field.is_empty() {
                    return Err(anyhow!("U: missing field name to unroll"));
                }
                let plan = project::compile(&format!("{field}[]"))?;
                stream_row_fanout(stream, move |row| plan.project_row(&row))
            }
            "?" => {
                if stage.spec.trim().is_empty() {
                    Box::new(stream.filter_map(|row| match row {
                        Ok(row) => question::clean_row(row).map(Ok),
                        Err(err) => Some(Err(err)),
                    }))
                } else {
                    let plan = quick::compile(&format!("? {}", stage.spec))?;
                    Box::new(quick::stream_rows_with_plan(stream, plan))
                }
            }
            other => return Err(anyhow!("stream stage not implemented for verb: {other}")),
        });
        Ok(())
    }

    fn apply_quick_stage(&self, items: OutputItems, stage: &ParsedStage) -> Result<OutputItems> {
        map_rows(items, |rows| quick::apply(rows, &stage.raw))
    }

    fn apply_explicit_stage(
        &mut self,
        items: OutputItems,
        stage: &ParsedStage,
    ) -> Result<OutputItems> {
        match stage.verb.as_str() {
            "P" => self.project(items, stage),
            // `V` is a quick-search scope alias ("value-only"), not a values stage.
            "V" => self.apply_quick_alias(items, stage, "V"),
            "K" => self.apply_quick_alias(items, stage, "K"),
            // `VAL` / `VALUE` produce explicit `{"value": ...}` rows.
            "VAL" | "VALUE" => map_rows(items, |rows| values::apply(rows, &stage.spec)),
            "F" => self.filter(items, stage),
            "G" => self.group(items, stage),
            "A" => aggregate::apply(items, &stage.spec),
            "S" => sort::apply(items, &stage.spec),
            "L" => self.limit(items, stage),
            "Z" => Ok(collapse::apply(items)),
            "C" => aggregate::count_macro(items, &stage.spec),
            "Y" => self.copy(items, stage),
            "U" => self.unroll(items, stage),
            "?" => question::apply(items, &stage.spec),
            "JQ" => jq::apply(items, &stage.spec),
            _ => Err(anyhow!("unknown DSL verb: {}", stage.verb)),
        }
    }

    fn project(&self, items: OutputItems, stage: &ParsedStage) -> Result<OutputItems> {
        match items {
            OutputItems::Rows(rows) => Ok(OutputItems::Rows(project::apply(rows, &stage.spec)?)),
            OutputItems::Groups(groups) => Ok(OutputItems::Groups(project::apply_groups(
                groups,
                &stage.spec,
            )?)),
        }
    }

    fn filter(&self, items: OutputItems, stage: &ParsedStage) -> Result<OutputItems> {
        match items {
            OutputItems::Rows(rows) => Ok(OutputItems::Rows(filter::apply(rows, &stage.spec)?)),
            OutputItems::Groups(groups) => Ok(OutputItems::Groups(filter::apply_groups(
                groups,
                &stage.spec,
            )?)),
        }
    }

    fn group(&self, items: OutputItems, stage: &ParsedStage) -> Result<OutputItems> {
        match items {
            OutputItems::Rows(rows) => {
                Ok(OutputItems::Groups(group::group_rows(rows, &stage.spec)?))
            }
            OutputItems::Groups(groups) => Ok(OutputItems::Groups(group::regroup_groups(
                groups,
                &stage.spec,
            )?)),
        }
    }

    fn limit(&self, items: OutputItems, stage: &ParsedStage) -> Result<OutputItems> {
        match items {
            OutputItems::Rows(rows) => Ok(OutputItems::Rows(limit::apply(rows, &stage.spec)?)),
            OutputItems::Groups(groups) => {
                Ok(OutputItems::Groups(limit::apply(groups, &stage.spec)?))
            }
        }
    }

    fn copy(&mut self, items: OutputItems, _stage: &ParsedStage) -> Result<OutputItems> {
        self.wants_copy = true;
        map_rows(items, |rows| Ok(copy::apply(rows)))
    }

    fn unroll(&self, items: OutputItems, stage: &ParsedStage) -> Result<OutputItems> {
        let field = stage.spec.trim();
        if field.is_empty() {
            return Err(anyhow!("U: missing field name to unroll"));
        }

        let selector = format!("{field}[]");
        match items {
            OutputItems::Rows(rows) => Ok(OutputItems::Rows(project::apply(rows, &selector)?)),
            OutputItems::Groups(groups) => Ok(OutputItems::Groups(project::apply_groups(
                groups, &selector,
            )?)),
        }
    }

    fn apply_quick_alias(
        &self,
        items: OutputItems,
        stage: &ParsedStage,
        alias: &str,
    ) -> Result<OutputItems> {
        let quick_spec = if stage.spec.is_empty() {
            alias.to_string()
        } else {
            format!("{alias} {}", stage.spec)
        };
        map_rows(items, |rows| quick::apply(rows, &quick_spec))
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
        }
    }

    fn finish_items(&mut self) -> Result<OutputItems> {
        self.materialize_items()
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

fn merged_group_header(group: &crate::core::output_model::Group) -> Row {
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
        // These stages only make sense on flat rows. When the pipeline is
        // already grouped, leave groups unchanged instead of flattening them
        // implicitly behind the caller's back.
        OutputItems::Groups(groups) => Ok(OutputItems::Groups(groups)),
    }
}

#[cfg(test)]
mod tests {
    use crate::core::output_model::{OutputItems, OutputResult};
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
}
