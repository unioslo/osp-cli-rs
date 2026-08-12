//! Built-in command handler implementations.
//!
//! This module exists to map already-parsed CLI arguments onto concrete built-in
//! command behavior. The public command grammar lives in [`crate::cli`]; these
//! handlers are the next step after parsing, not a second place that defines
//! syntax.
//!
//! Keep the split boring:
//!
//! - argument shapes and command names belong in [`crate::cli`]
//! - built-in workflows belong here
//! - host orchestration still belongs in [`crate::app`]
//! - output shaping should flow back as rows/documents, not pre-rendered text
//!
//! The modules here are grouped by built-in command namespace, not by transport
//! or rendering concerns.

pub(crate) mod config;
pub(crate) mod doctor;
pub(crate) mod history;
pub(crate) mod intro;
pub(crate) mod plugins;
pub(crate) mod theme;

fn resolve_terminal_selector(current: &str, selector: Option<&str>) -> Option<String> {
    let value = selector?;
    if value == crate::app::CURRENT_TERMINAL_SENTINEL {
        return Some(current.to_string());
    }
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_ascii_lowercase())
}
