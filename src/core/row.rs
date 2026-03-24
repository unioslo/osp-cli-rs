//! Canonical row type used across commands, DSL stages, and rendering.

/// Canonical row representation used across commands, services, and DSL stages.
pub type Row = serde_json::Map<String, serde_json::Value>;
