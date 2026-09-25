//! JSON fixtures enter through the same command-output adapter as plugins.
use super::compiled::{CompiledPipeline, CompiledStage};
use anyhow::Result;
use serde_json::Value;

pub(crate) fn apply_stage(value: Value, stage: &CompiledStage) -> Result<Value> {
    let output = crate::cli::rows::output::plugin_data_to_output_result(value, None);
    let output = super::engine::run_compiled(
        output,
        &CompiledPipeline {
            stages: vec![stage.clone()],
        },
    )?;
    let mut settings =
        crate::ui::RenderSettings::test_plain(crate::core::output::OutputFormat::Json);
    settings.format_explicit = true;
    Ok(serde_json::from_str(&crate::ui::render_output(
        &output, &settings,
    ))?)
}

#[path = "tests/value_semantics.rs"]
mod tests;
