use std::collections::HashMap;

use crate::output::{ColorMode, OutputFormat, UnicodeMode};

pub const ENV_OSP_UI_VERBOSITY: &str = "OSP_UI_VERBOSITY";
pub const ENV_OSP_DEBUG_LEVEL: &str = "OSP_DEBUG_LEVEL";
pub const ENV_OSP_FORMAT: &str = "OSP_FORMAT";
pub const ENV_OSP_COLOR: &str = "OSP_COLOR";
pub const ENV_OSP_UNICODE: &str = "OSP_UNICODE";
pub const ENV_OSP_PROFILE: &str = "OSP_PROFILE";
pub const ENV_OSP_TERMINAL: &str = "OSP_TERMINAL";
pub const ENV_OSP_TERMINAL_KIND: &str = "OSP_TERMINAL_KIND";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum UiVerbosity {
    Error,
    Warning,
    Success,
    Info,
    Trace,
}

impl Default for UiVerbosity {
    fn default() -> Self {
        Self::Success
    }
}

impl UiVerbosity {
    pub fn as_str(self) -> &'static str {
        match self {
            UiVerbosity::Error => "error",
            UiVerbosity::Warning => "warning",
            UiVerbosity::Success => "success",
            UiVerbosity::Info => "info",
            UiVerbosity::Trace => "trace",
        }
    }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeTerminalKind {
    Cli,
    Repl,
    Unknown,
}

impl Default for RuntimeTerminalKind {
    fn default() -> Self {
        Self::Unknown
    }
}

impl RuntimeTerminalKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RuntimeTerminalKind::Cli => "cli",
            RuntimeTerminalKind::Repl => "repl",
            RuntimeTerminalKind::Unknown => "unknown",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "cli" => Some(RuntimeTerminalKind::Cli),
            "repl" => Some(RuntimeTerminalKind::Repl),
            "unknown" => Some(RuntimeTerminalKind::Unknown),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeHints {
    pub ui_verbosity: UiVerbosity,
    pub debug_level: u8,
    pub format: OutputFormat,
    pub color: ColorMode,
    pub unicode: UnicodeMode,
    pub profile: Option<String>,
    pub terminal: Option<String>,
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
    pub fn from_env() -> Self {
        Self::from_env_iter(std::env::vars())
    }

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

        Self {
            ui_verbosity,
            debug_level,
            format,
            color,
            unicode,
            profile,
            terminal,
            terminal_kind,
        }
    }

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
    use crate::output::{ColorMode, OutputFormat, UnicodeMode};

    #[test]
    fn env_roundtrip_keeps_runtime_hints() {
        let hints = RuntimeHints {
            ui_verbosity: UiVerbosity::Trace,
            debug_level: 7,
            format: OutputFormat::Json,
            color: ColorMode::Never,
            unicode: UnicodeMode::Always,
            profile: Some("uio".to_string()),
            terminal: Some("xterm-256color".to_string()),
            terminal_kind: RuntimeTerminalKind::Repl,
        };

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
