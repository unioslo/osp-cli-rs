use std::fmt::{Display, Formatter};

use crate::SchemaValueType;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    FileRead {
        path: String,
        reason: String,
    },
    FileWrite {
        path: String,
        reason: String,
    },
    LayerLoad {
        path: String,
        source: Box<ConfigError>,
    },
    InsecureSecretsPermissions {
        path: String,
        mode: u32,
    },
    TomlParse(String),
    TomlRootMustBeTable,
    UnknownTopLevelSection(String),
    InvalidSection {
        section: String,
        expected: String,
    },
    UnsupportedTomlValue {
        path: String,
        kind: String,
    },
    InvalidEnvOverride {
        key: String,
        reason: String,
    },
    InvalidConfigKey {
        key: String,
        reason: String,
    },
    InvalidBootstrapScope {
        key: String,
        profile: Option<String>,
        terminal: Option<String>,
    },
    MissingDefaultProfile,
    InvalidDefaultProfileType(String),
    InvalidDefaultProfileValue(String),
    UnknownProfile {
        profile: String,
        known: Vec<String>,
    },
    InvalidPlaceholderSyntax {
        key: String,
        template: String,
    },
    UnresolvedPlaceholder {
        key: String,
        placeholder: String,
    },
    PlaceholderCycle {
        cycle: Vec<String>,
    },
    NonScalarPlaceholder {
        key: String,
        placeholder: String,
    },
    UnknownConfigKeys {
        keys: Vec<String>,
    },
    MissingRequiredKey {
        key: String,
    },
    InvalidValueType {
        key: String,
        expected: SchemaValueType,
        actual: String,
    },
    InvalidEnumValue {
        key: String,
        value: String,
        allowed: Vec<String>,
    },
}

impl Display for ConfigError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::FileRead { path, reason } => {
                write!(f, "failed to read config file {path}: {reason}")
            }
            ConfigError::FileWrite { path, reason } => {
                write!(f, "failed to write config file {path}: {reason}")
            }
            ConfigError::LayerLoad { path, source } => {
                write!(f, "{source} (path: {path})")
            }
            ConfigError::InsecureSecretsPermissions { path, mode } => {
                write!(
                    f,
                    "insecure permissions on secrets file {path}: mode {:o}, expected 600",
                    mode
                )
            }
            ConfigError::TomlParse(message) => write!(f, "failed to parse TOML: {message}"),
            ConfigError::TomlRootMustBeTable => {
                write!(f, "config root must be a TOML table")
            }
            ConfigError::UnknownTopLevelSection(section) => {
                write!(f, "unknown top-level config section: {section}")
            }
            ConfigError::InvalidSection { section, expected } => {
                write!(f, "invalid section {section}: expected {expected}")
            }
            ConfigError::UnsupportedTomlValue { path, kind } => {
                write!(f, "unsupported TOML value at {path}: {kind}")
            }
            ConfigError::InvalidEnvOverride { key, reason } => {
                write!(f, "invalid env override {key}: {reason}")
            }
            ConfigError::InvalidConfigKey { key, reason } => {
                write!(f, "invalid config key {key}: {reason}")
            }
            ConfigError::InvalidBootstrapScope {
                key,
                profile,
                terminal,
            } => {
                let scope = match (profile.as_deref(), terminal.as_deref()) {
                    (Some(profile), Some(terminal)) => {
                        format!("profile={profile}, terminal={terminal}")
                    }
                    (Some(profile), None) => format!("profile={profile}"),
                    (None, Some(terminal)) => format!("terminal={terminal}"),
                    (None, None) => "global".to_string(),
                };
                write!(
                    f,
                    "bootstrap-only key {key} is not allowed in scope {scope}; allowed scopes: global or terminal-only"
                )
            }
            ConfigError::MissingDefaultProfile => {
                write!(f, "missing profile.default and no fallback profile")
            }
            ConfigError::InvalidDefaultProfileType(actual) => {
                write!(f, "profile.default must be string, got {actual}")
            }
            ConfigError::InvalidDefaultProfileValue(actual) => {
                write!(
                    f,
                    "profile.default must be a non-empty string, got {actual}"
                )
            }
            ConfigError::UnknownProfile { profile, known } => {
                write!(
                    f,
                    "unknown profile '{profile}'. known profiles: {}",
                    known.join(",")
                )
            }
            ConfigError::InvalidPlaceholderSyntax { key, template } => {
                write!(f, "invalid placeholder syntax in key {key}: {template}")
            }
            ConfigError::UnresolvedPlaceholder { key, placeholder } => {
                write!(f, "unresolved placeholder in key {key}: {placeholder}")
            }
            ConfigError::PlaceholderCycle { cycle } => {
                write!(f, "placeholder cycle detected: {}", cycle.join(" -> "))
            }
            ConfigError::NonScalarPlaceholder { key, placeholder } => {
                write!(
                    f,
                    "placeholder {placeholder} in key {key} points to a non-scalar value"
                )
            }
            ConfigError::UnknownConfigKeys { keys } => {
                write!(f, "unknown config keys: {}", keys.join(", "))
            }
            ConfigError::MissingRequiredKey { key } => {
                write!(f, "missing required config key: {key}")
            }
            ConfigError::InvalidValueType {
                key,
                expected,
                actual,
            } => {
                write!(
                    f,
                    "invalid type for key {key}: expected {expected}, got {actual}"
                )
            }
            ConfigError::InvalidEnumValue {
                key,
                value,
                allowed,
            } => {
                write!(
                    f,
                    "invalid value for key {key}: {value}. allowed: {}",
                    allowed.join(", ")
                )
            }
        }
    }
}

impl std::error::Error for ConfigError {}

pub(crate) fn with_path_context(path: String, error: ConfigError) -> ConfigError {
    ConfigError::LayerLoad {
        path,
        source: Box::new(error),
    }
}
