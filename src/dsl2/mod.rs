//! Compatibility shim for the retired `crate::dsl2` path.
//!
//! The canonical implementation lives in [`crate::dsl`]. This module exists so
//! older internal references can be migrated incrementally without reintroducing
//! a second engine.
#![allow(unused_imports)]

pub use crate::dsl::{
    Dsl2Mode, Pipeline, VerbInfo, VerbStreaming, apply_output_pipeline,
    apply_output_pipeline_with_mode, apply_pipeline, configured_mode, execute_pipeline,
    execute_pipeline_streaming, is_registered_explicit_verb, parse_pipeline, parse_stage,
    registered_verbs, render_streaming_badge, verb_info,
};
