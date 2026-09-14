use anyhow::{Result, anyhow};
use serde_json::Value;

use crate::dsl::compiled::CompiledStage;
use crate::dsl::verbs::{
    aggregate, collapse, filter, group, jq, json, limit, project, question, quick, sort, unroll,
    values,
};

/// Applies one parsed stage directly to canonical JSON.
///
/// The semantic/document path keeps `Value` as the source of truth.
/// Stages may still reuse existing row/group operators for local tabular
/// collections, but the executor itself no longer treats those projections as
/// the canonical substrate.
#[cfg(test)]
pub(crate) fn apply_stage(value: Value, stage: &CompiledStage) -> Result<Value> {
    if let Some(plan) = stage.quick_plan() {
        return quick::apply_value_with_plan(value, plan);
    }

    apply_non_quick_stage(value, stage)
}

pub(crate) fn apply_stage_preserving_matching_rows(
    value: Value,
    stage: &CompiledStage,
    grouped: bool,
) -> Result<Value> {
    if let Some(plan) = stage.quick_plan() {
        return quick::apply_value_with_plan_preserving_matching_rows(value, plan);
    }

    if grouped {
        use crate::core::output_model::OutputItems;
        match stage {
            CompiledStage::Sort(plan) => {
                return json::traverse_group_collections(value, |items| {
                    sort::apply_with_plan(items, plan)
                });
            }
            CompiledStage::Aggregate(plan) => {
                return json::traverse_group_collections(value, |items| {
                    aggregate::apply_with_plan(items, plan)
                });
            }
            CompiledStage::Group(plan) => {
                return json::traverse_group_collections(value, |items| match items {
                    OutputItems::Rows(rows) => {
                        group::group_rows_with_plan(rows, plan).map(OutputItems::Groups)
                    }
                    OutputItems::Groups(groups) => {
                        group::regroup_groups_with_plan(groups, plan).map(OutputItems::Groups)
                    }
                });
            }
            CompiledStage::Collapse => {
                return json::traverse_group_collections(value, collapse::apply);
            }
            CompiledStage::CountMacro => {
                return json::traverse_group_collections(value, |items| {
                    aggregate::count_macro(items, "")
                });
            }
            _ => {}
        }
    }
    apply_non_quick_stage(value, stage)
}

fn apply_non_quick_stage(value: Value, stage: &CompiledStage) -> Result<Value> {
    match stage {
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
        CompiledStage::Jq(expr) => jq::apply_value_with_expr(value, expr),
        CompiledStage::Values(plan) => values::apply_value_with_plan(value, plan),
        CompiledStage::Quick(_)
        | CompiledStage::Question(_)
        | CompiledStage::ValueQuick(_)
        | CompiledStage::KeyQuick(_) => Err(anyhow!(
            "quick family should have been handled before value-stage dispatch"
        )),
    }
}

#[cfg(test)]
#[path = "tests/value_semantics.rs"]
mod tests;
