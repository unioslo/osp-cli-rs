//! Main host-facing entrypoints plus bootstrap/runtime state.

pub(crate) mod bootstrap;
pub mod runtime;
pub mod session;
pub mod sink;

pub use crate::osp_cli::{
    App, AppBuilder, AppRunner, run_from, run_process, run_process_with_sink,
};
pub use runtime::{
    AppClients, AppRuntime, AppState, AuthState, ConfigState, LaunchContext,
    RuntimeContext, TerminalKind, UiState,
};
pub use session::{
    AppSession, DebugTimingBadge, DebugTimingState, LastFailure, ReplScopeFrame,
    ReplScopeStack,
};
pub use sink::{BufferedUiSink, StdIoUiSink, UiSink};
