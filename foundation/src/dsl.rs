//! DSL parsing, stage metadata, and pipeline execution.

pub mod eval {
    pub use crate::osp_dsl::eval::*;
}

pub mod model {
    pub use crate::osp_dsl::model::*;
}

pub mod parse {
    pub use crate::osp_dsl::parse::*;
}

pub mod stages {
    pub use crate::osp_dsl::stages::*;
}

pub mod verbs {
    pub use crate::osp_dsl::verbs::*;
}

pub use crate::osp_dsl::{
    Pipeline, VerbInfo, VerbStreaming, apply_output_pipeline, apply_pipeline,
    execute_pipeline, execute_pipeline_streaming, is_registered_explicit_verb,
    parse_pipeline, registered_verbs, render_streaming_badge, verb_info,
};
