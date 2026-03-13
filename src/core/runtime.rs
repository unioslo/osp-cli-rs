//! Runtime hint model shared across host, native commands, and plugin
//! boundaries.
//!
//! This module exists so runtime-facing behavior like verbosity, output mode,
//! and terminal identity can travel through the system as normalized data
//! instead of ad hoc environment parsing in each caller.
//!
//! High-level flow:
//!
//! - parse runtime-oriented environment hints into
//!   [`crate::core::runtime::RuntimeHints`]
//! - normalize those values into enums and bounded scalar settings
//! - export the same normalized hints back into environment pairs when needed
//!
//! Contract:
//!
//! - environment parsing rules for runtime hints live here
//! - callers should consume [`crate::core::runtime::RuntimeHints`] instead of
//!   reparsing raw env vars
//!
//! Public API shape:
//!
//! - [`crate::core::runtime::RuntimeHints::new`] is the exact constructor for
//!   already-resolved runtime settings
//! - [`crate::core::runtime::RuntimeHints::from_env`] and
//!   [`crate::core::runtime::RuntimeHints::from_env_iter`] are the probing and
//!   adaptation factories
//! - optional metadata such as profile and terminal identity uses `with_*`
//!   refinements instead of raw ad hoc assembly

use std::collections::HashMap;

use crate::core::output::{ColorMode, OutputFormat, UnicodeMode};

/// Environment variable carrying the UI verbosity hint.
pub const ENV_OSP_UI_VERBOSITY: &str = "OSP_UI_VERBOSITY";
/// Environment variable carrying the debug level hint.
pub const ENV_OSP_DEBUG_LEVEL: &str = "OSP_DEBUG_LEVEL";
/// Environment variable carrying the preferred output format.
pub const ENV_OSP_FORMAT: &str = "OSP_FORMAT";
/// Environment variable carrying the color-mode hint.
pub const ENV_OSP_COLOR: &str = "OSP_COLOR";
/// Environment variable carrying the Unicode-mode hint.
pub const ENV_OSP_UNICODE: &str = "OSP_UNICODE";
/// Environment variable carrying the active profile name.
pub const ENV_OSP_PROFILE: &str = "OSP_PROFILE";
/// Environment variable carrying the active terminal identifier.
pub const ENV_OSP_TERMINAL: &str = "OSP_TERMINAL";
/// Environment variable carrying the terminal kind hint.
pub const ENV_OSP_TERMINAL_KIND: &str = "OSP_TERMINAL_KIND";

/// UI message verbosity derived from runtime hints and environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum UiVerbosity {
    /// Show only errors.
    Error,
    /// Show errors and warnings.
    Warning,
    /// Show success messages in addition to warnings and errors.
    #[default]
    Success,
    /// Show normal informational output.
    Info,
    /// Show trace-level output.
    Trace,
}

impl UiVerbosity {
    /// Returns the canonical string representation for this verbosity level.
    pub fn as_str(self) -> &'static str {
        match self {
            UiVerbosity::Error => "error",
            UiVerbosity::Warning => "warning",
            UiVerbosity::Success => "success",
            UiVerbosity::Info => "info",
            UiVerbosity::Trace => "trace",
        }
    }

    /// Parses a case-insensitive verbosity level or supported alias.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::runtime::UiVerbosity;
    ///
    /// assert_eq!(UiVerbosity::parse("warn"), Some(UiVerbosity::Warning));
    /// assert_eq!(UiVerbosity::parse("TRACE"), Some(UiVerbosity::Trace));
    /// assert_eq!(UiVerbosity::parse("loud"), None);
    /// ```
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "error" => Some(UiVerbosity::Error),
            "warning" | "warn" => Some(UiVerbosity::Warning),
            "success" => Some(UiVerbosity::Success),
            "info" => Some(UiVerbosity::Info),
            "trace" => Some(UiVerbosity::Trace),
            _ => None,
        }
    }
}

/// Runtime terminal mode exposed through environment hints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RuntimeTerminalKind {
    /// Invocation is running as a one-shot CLI command.
    Cli,
    /// Invocation is running inside the interactive REPL.
    Repl,
    /// Terminal kind is unknown or unspecified.
    #[default]
    Unknown,
}

impl RuntimeTerminalKind {
    /// Returns the canonical string representation for this terminal kind.
    pub fn as_str(self) -> &'static str {
        match self {
            RuntimeTerminalKind::Cli => "cli",
            RuntimeTerminalKind::Repl => "repl",
            RuntimeTerminalKind::Unknown => "unknown",
        }
    }

    /// Parses a case-insensitive terminal kind name.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::runtime::RuntimeTerminalKind;
    ///
    /// assert_eq!(RuntimeTerminalKind::parse("cli"), Some(RuntimeTerminalKind::Cli));
    /// assert_eq!(RuntimeTerminalKind::parse("REPL"), Some(RuntimeTerminalKind::Repl));
    /// assert_eq!(RuntimeTerminalKind::parse("tty"), None);
    /// ```
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "cli" => Some(RuntimeTerminalKind::Cli),
            "repl" => Some(RuntimeTerminalKind::Repl),
            "unknown" => Some(RuntimeTerminalKind::Unknown),
            _ => None,
        }
    }
}

/// Normalized runtime settings loaded from environment variables.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct RuntimeHints {
    /// Effective UI message verbosity.
    pub ui_verbosity: UiVerbosity,
    /// Effective debug level capped to the supported range.
    pub debug_level: u8,
    /// Effective output format preference.
    pub format: OutputFormat,
    /// Effective color-mode preference.
    pub color: ColorMode,
    /// Effective Unicode-mode preference.
    pub unicode: UnicodeMode,
    /// Active profile identifier, when set.
    pub profile: Option<String>,
    /// Active terminal identifier, when set.
    pub terminal: Option<String>,
    /// Effective terminal kind hint.
    pub terminal_kind: RuntimeTerminalKind,
}

impl Default for RuntimeHints {
    fn default() -> Self {
        Self {
            ui_verbosity: UiVerbosity::Success,
            debug_level: 0,
            format: OutputFormat::Auto,
            color: ColorMode::Auto,
            unicode: UnicodeMode::Auto,
            profile: None,
            terminal: None,
            terminal_kind: RuntimeTerminalKind::Unknown,
        }
    }
}

impl RuntimeHints {
    /// Creates a fully specified runtime-hint payload with no profile or
    /// terminal metadata attached yet.
    ///
    /// `debug_level` is clamped to the supported `0..=3` range.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::output::{ColorMode, OutputFormat, UnicodeMode};
    /// use osp_cli::core::runtime::{RuntimeHints, UiVerbosity};
    ///
    /// let hints = RuntimeHints::new(
    ///     UiVerbosity::Info,
    ///     7,
    ///     OutputFormat::Json,
    ///     ColorMode::Always,
    ///     UnicodeMode::Never,
    /// );
    ///
    /// assert_eq!(hints.ui_verbosity, UiVerbosity::Info);
    /// assert_eq!(hints.debug_level, 3);
    /// assert_eq!(hints.profile, None);
    /// ```
    pub fn new(
        ui_verbosity: UiVerbosity,
        debug_level: u8,
        format: OutputFormat,
        color: ColorMode,
        unicode: UnicodeMode,
    ) -> Self {
        Self {
            ui_verbosity,
            debug_level: debug_level.min(3),
            format,
            color,
            unicode,
            profile: None,
            terminal: None,
            terminal_kind: RuntimeTerminalKind::Unknown,
        }
    }

    /// Reads runtime hints from the current process environment.
    pub fn from_env() -> Self {
        Self::from_env_iter(std::env::vars())
    }

    /// Replaces the optional active-profile label.
    pub fn with_profile(mut self, profile: Option<String>) -> Self {
        self.profile = profile
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        self
    }

    /// Replaces the optional terminal identifier.
    pub fn with_terminal(mut self, terminal: Option<String>) -> Self {
        self.terminal = terminal
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        self
    }

    /// Replaces the terminal-kind hint.
    pub fn with_terminal_kind(mut self, terminal_kind: RuntimeTerminalKind) -> Self {
        self.terminal_kind = terminal_kind;
        self
    }

    /// Builds runtime hints from arbitrary key-value environment pairs.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::output::{ColorMode, OutputFormat, UnicodeMode};
    /// use osp_cli::core::runtime::{
    ///     ENV_OSP_COLOR, ENV_OSP_DEBUG_LEVEL, ENV_OSP_FORMAT, ENV_OSP_PROFILE,
    ///     ENV_OSP_TERMINAL_KIND, ENV_OSP_UI_VERBOSITY, RuntimeHints,
    ///     RuntimeTerminalKind, UiVerbosity,
    /// };
    ///
    /// let hints = RuntimeHints::from_env_iter([
    ///     (ENV_OSP_UI_VERBOSITY, "trace"),
    ///     (ENV_OSP_DEBUG_LEVEL, "7"),
    ///     (ENV_OSP_FORMAT, "json"),
    ///     (ENV_OSP_COLOR, "never"),
    ///     (ENV_OSP_PROFILE, "uio"),
    ///     (ENV_OSP_TERMINAL_KIND, "repl"),
    /// ]);
    ///
    /// assert_eq!(hints.ui_verbosity, UiVerbosity::Trace);
    /// assert_eq!(hints.debug_level, 3);
    /// assert_eq!(hints.format, OutputFormat::Json);
    /// assert_eq!(hints.color, ColorMode::Never);
    /// assert_eq!(hints.unicode, UnicodeMode::Auto);
    /// assert_eq!(hints.profile.as_deref(), Some("uio"));
    /// assert_eq!(hints.terminal_kind, RuntimeTerminalKind::Repl);
    /// ```
    pub fn from_env_iter<I, K, V>(vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let values = vars
            .into_iter()
            .map(|(k, v)| (k.as_ref().to_string(), v.as_ref().to_string()))
            .collect::<HashMap<String, String>>();

        let ui_verbosity = values
            .get(ENV_OSP_UI_VERBOSITY)
            .and_then(|value| UiVerbosity::parse(value))
            .unwrap_or(UiVerbosity::Success);
        let debug_level = values
            .get(ENV_OSP_DEBUG_LEVEL)
            .and_then(|value| value.trim().parse::<u8>().ok())
            .unwrap_or(0)
            .min(3);
        let format = values
            .get(ENV_OSP_FORMAT)
            .and_then(|value| OutputFormat::parse(value))
            .unwrap_or(OutputFormat::Auto);
        let color = values
            .get(ENV_OSP_COLOR)
            .and_then(|value| ColorMode::parse(value))
            .unwrap_or(ColorMode::Auto);
        let unicode = values
            .get(ENV_OSP_UNICODE)
            .and_then(|value| UnicodeMode::parse(value))
            .unwrap_or(UnicodeMode::Auto);
        let profile = values
            .get(ENV_OSP_PROFILE)
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let terminal = values
            .get(ENV_OSP_TERMINAL)
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let terminal_kind = values
            .get(ENV_OSP_TERMINAL_KIND)
            .and_then(|value| RuntimeTerminalKind::parse(value))
            .or_else(|| {
                values
                    .get(ENV_OSP_TERMINAL)
                    .and_then(|value| RuntimeTerminalKind::parse(value))
            })
            .unwrap_or(RuntimeTerminalKind::Unknown);

        Self::new(ui_verbosity, debug_level, format, color, unicode)
            .with_profile(profile)
            .with_terminal(terminal)
            .with_terminal_kind(terminal_kind)
    }

    /// Returns this hint set as environment variable pairs suitable for export.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::core::output::{ColorMode, OutputFormat, UnicodeMode};
    /// use osp_cli::core::runtime::{RuntimeHints, RuntimeTerminalKind, UiVerbosity};
    ///
    /// let hints = RuntimeHints::new(
    ///     UiVerbosity::Info,
    ///     2,
    ///     OutputFormat::Json,
    ///     ColorMode::Always,
    ///     UnicodeMode::Never,
    /// )
    /// .with_profile(Some("uio".to_string()))
    /// .with_terminal(Some("xterm-256color".to_string()))
    /// .with_terminal_kind(RuntimeTerminalKind::Cli);
    /// let pairs = hints.env_pairs();
    ///
    /// assert!(pairs.iter().any(|(key, value)| *key == "OSP_FORMAT" && value == "json"));
    /// assert!(pairs.iter().any(|(key, value)| *key == "OSP_PROFILE" && value == "uio"));
    /// ```
    pub fn env_pairs(&self) -> Vec<(&'static str, String)> {
        let mut out = vec![
            (ENV_OSP_UI_VERBOSITY, self.ui_verbosity.as_str().to_string()),
            (ENV_OSP_DEBUG_LEVEL, self.debug_level.min(3).to_string()),
            (ENV_OSP_FORMAT, self.format.as_str().to_string()),
            (ENV_OSP_COLOR, self.color.as_str().to_string()),
            (ENV_OSP_UNICODE, self.unicode.as_str().to_string()),
            (
                ENV_OSP_TERMINAL_KIND,
                self.terminal_kind.as_str().to_string(),
            ),
        ];

        if let Some(profile) = &self.profile {
            out.push((ENV_OSP_PROFILE, profile.clone()));
        }
        if let Some(terminal) = &self.terminal {
            out.push((ENV_OSP_TERMINAL, terminal.clone()));
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ENV_OSP_COLOR, ENV_OSP_DEBUG_LEVEL, ENV_OSP_FORMAT, ENV_OSP_PROFILE, ENV_OSP_TERMINAL,
        ENV_OSP_UI_VERBOSITY, ENV_OSP_UNICODE, RuntimeHints, RuntimeTerminalKind, UiVerbosity,
    };
    use crate::core::output::{ColorMode, OutputFormat, UnicodeMode};

    #[test]
    fn env_roundtrip_keeps_runtime_hints() {
        let hints = RuntimeHints::new(
            UiVerbosity::Trace,
            7,
            OutputFormat::Json,
            ColorMode::Never,
            UnicodeMode::Always,
        )
        .with_profile(Some("uio".to_string()))
        .with_terminal(Some("xterm-256color".to_string()))
        .with_terminal_kind(RuntimeTerminalKind::Repl);

        let parsed = RuntimeHints::from_env_iter(hints.env_pairs());
        assert_eq!(parsed.ui_verbosity, UiVerbosity::Trace);
        assert_eq!(parsed.debug_level, 3);
        assert_eq!(parsed.format, OutputFormat::Json);
        assert_eq!(parsed.color, ColorMode::Never);
        assert_eq!(parsed.unicode, UnicodeMode::Always);
        assert_eq!(parsed.profile.as_deref(), Some("uio"));
        assert_eq!(parsed.terminal.as_deref(), Some("xterm-256color"));
        assert_eq!(parsed.terminal_kind, RuntimeTerminalKind::Repl);
    }

    #[test]
    fn new_and_with_helpers_build_runtime_hints_unit() {
        let hints = RuntimeHints::new(
            UiVerbosity::Info,
            9,
            OutputFormat::Table,
            ColorMode::Always,
            UnicodeMode::Never,
        )
        .with_profile(Some("  dev  ".to_string()))
        .with_terminal(Some(" xterm-256color ".to_string()))
        .with_terminal_kind(RuntimeTerminalKind::Cli);

        assert_eq!(hints.ui_verbosity, UiVerbosity::Info);
        assert_eq!(hints.debug_level, 3);
        assert_eq!(hints.profile.as_deref(), Some("dev"));
        assert_eq!(hints.terminal.as_deref(), Some("xterm-256color"));
        assert_eq!(hints.terminal_kind, RuntimeTerminalKind::Cli);
    }

    #[test]
    fn from_env_defaults_when_vars_missing_or_invalid() {
        let parsed = RuntimeHints::from_env_iter(vec![
            (ENV_OSP_UI_VERBOSITY, "loud"),
            (ENV_OSP_DEBUG_LEVEL, "NaN"),
            (ENV_OSP_FORMAT, "???"),
            (ENV_OSP_COLOR, "blue"),
            (ENV_OSP_UNICODE, "emoji"),
        ]);

        assert_eq!(parsed.ui_verbosity, UiVerbosity::Success);
        assert_eq!(parsed.debug_level, 0);
        assert_eq!(parsed.format, OutputFormat::Auto);
        assert_eq!(parsed.color, ColorMode::Auto);
        assert_eq!(parsed.unicode, UnicodeMode::Auto);
        assert_eq!(parsed.profile, None);
        assert_eq!(parsed.terminal, None);
        assert_eq!(parsed.terminal_kind, RuntimeTerminalKind::Unknown);
    }

    #[test]
    fn terminal_kind_falls_back_to_terminal_env() {
        let parsed =
            RuntimeHints::from_env_iter(vec![(ENV_OSP_TERMINAL, "repl"), (ENV_OSP_PROFILE, "tsd")]);

        assert_eq!(parsed.profile.as_deref(), Some("tsd"));
        assert_eq!(parsed.terminal.as_deref(), Some("repl"));
        assert_eq!(parsed.terminal_kind, RuntimeTerminalKind::Repl);
    }
}
