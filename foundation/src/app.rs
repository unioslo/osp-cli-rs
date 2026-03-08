//! Main host-facing entrypoints and stateful runtime surfaces.

pub use crate::osp_cli::{
    App, AppBuilder, AppRunner, BufferedUiSink, StdIoUiSink, UiSink, run_from,
    run_process, run_process_with_sink,
};
