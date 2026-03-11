use anyhow::Result;

use crate::core::output_model::OutputResult;

use super::engine;

/// Execution mode for the document-first DSL.
///
/// The legacy/compare rollout has been retired. We keep the enum so existing
/// callers and tests have a stable type to reference, but there is now a
/// single production mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dsl2Mode {
    /// Use the canonical document-first DSL engine.
    Enabled,
}

impl Dsl2Mode {
    #[cfg(test)]
    pub(super) fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" | "0" | "off" | "false" | "legacy" | "1" | "on" | "true" | "enabled" | "dsl2"
            | "ab" | "compare" | "shadow" => Some(Self::Enabled),
            _ => None,
        }
    }
}

/// Returns the configured DSL mode.
///
/// The unified DSL is the only engine now, so this is intentionally
/// unconditional.
pub fn configured_mode() -> Dsl2Mode {
    Dsl2Mode::Enabled
}

/// Applies a pipeline using the canonical DSL engine.
pub fn apply_output_pipeline_with_mode(
    output: OutputResult,
    stages: &[String],
) -> Result<OutputResult> {
    engine::apply_output_pipeline(output, stages)
}
