//! Host runtime, session, and launch-state types used to embed the CLI/REPL.

pub use crate::osp_cli::state::{
    AppClients, AppRuntime, AppSession, AppState, AuthState, ConfigState,
    DebugTimingBadge, DebugTimingState, LastFailure, LaunchContext, ReplScopeFrame,
    ReplScopeStack, RuntimeContext, TerminalKind, UiState,
};
