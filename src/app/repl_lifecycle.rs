//! App-level REPL state rebuild helpers.
//!
//! REPL reloads need to reconstruct runtime, session, and client state in a
//! coordinated way after config/theme changes. This module keeps that rebuild
//! path in one place so the host loop can request a reload without knowing how
//! the individual app parts are reassembled.

use miette::Result;

use super::rebuild::ReplStateRebuilder;
use crate::app::session::AppStateParts;
use crate::app::{AppClients, AppRuntime, AppSession, AppState};

pub(crate) fn rebuild_repl_parts(
    runtime: &AppRuntime,
    session: &AppSession,
    clients: &AppClients,
) -> Result<(AppRuntime, AppSession, AppClients)> {
    let (runtime, session, clients) =
        ReplStateRebuilder::capture(runtime, session, clients).rebuild()?;
    super::assembly::apply_runtime_side_effects(
        runtime.config.resolved(),
        runtime.ui.debug_verbosity,
        &runtime.themes,
    );
    Ok((runtime, session, clients))
}

pub(crate) fn rebuild_repl_in_place(state: &mut AppState) -> Result<()> {
    let (runtime, session, clients) =
        rebuild_repl_parts(&state.runtime, &state.session, &state.clients)?;
    state.replace_parts(AppStateParts {
        runtime,
        session,
        clients,
    });
    Ok(())
}

#[cfg(test)]
pub(crate) fn rebuild_repl_state(current: &AppState) -> Result<AppState> {
    let (runtime, session, clients) =
        rebuild_repl_parts(&current.runtime, &current.session, &current.clients)?;
    Ok(AppState::from_parts(AppStateParts {
        runtime,
        session,
        clients,
    }))
}
