//! Canonical document-first pipeline DSL.
//!
//! The old split between `dsl` and `dsl2` has been retired. `dsl` owns the
//! implementation again, while `crate::dsl2` remains as a compatibility shim
//! for any lingering internal references.

pub(crate) mod compiled;
pub(crate) mod eval;
pub(crate) mod model;
pub(crate) mod parse;
pub(crate) mod verb_info;

mod engine;
mod rollout;
mod value;
mod verbs;

pub use engine::{
    apply_output_pipeline, apply_pipeline, execute_pipeline, execute_pipeline_streaming,
};
pub use parse::pipeline::{Pipeline, parse_pipeline, parse_stage};
pub use rollout::{Dsl2Mode, apply_output_pipeline_with_mode, configured_mode};
pub use verb_info::{
    VerbInfo, VerbStreaming, is_registered_explicit_verb, registered_verbs, render_streaming_badge,
    verb_info,
};

#[cfg(test)]
mod contract_tests;
#[cfg(test)]
mod tests;
