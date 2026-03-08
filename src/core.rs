//! Shared output, row, runtime, and plugin protocol types.

pub mod output {
    pub use crate::osp_core::output::*;
}

pub mod output_model {
    pub use crate::osp_core::output_model::*;
}

pub mod plugin {
    pub use crate::osp_core::plugin::*;
}

pub mod row {
    pub use crate::osp_core::row::*;
}

pub mod runtime {
    pub use crate::osp_core::runtime::*;
}

pub use crate::osp_core::row::Row;
pub use crate::osp_core::runtime::{RuntimeHints, RuntimeTerminalKind, UiVerbosity};
