use anyhow::{Result, anyhow};
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
    if let Some(plan) = stage.quick_plan() {
        return quick::apply_value_with_plan(value, plan);
    }

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
