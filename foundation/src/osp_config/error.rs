use std::fmt::{Display, Formatter};

use crate::osp_config::SchemaValueType;

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
    InvalidBootstrapValue {
        key: String,
        reason: String,
    },
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
            ConfigError::InvalidBootstrapValue { key, reason } => {
                write!(f, "invalid bootstrap value for {key}: {reason}")
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

#[cfg(test)]
mod tests {
    use super::{ConfigError, with_path_context};
    use crate::osp_config::SchemaValueType;

    #[test]
    fn config_error_display_covers_user_facing_variants() {
        let cases = [
            (
                ConfigError::FileRead {
                    path: "/tmp/config.toml".to_string(),
                    reason: "permission denied".to_string(),
                },
                "failed to read config file /tmp/config.toml: permission denied",
            ),
            (
                ConfigError::FileWrite {
                    path: "/tmp/config.toml".to_string(),
                    reason: "disk full".to_string(),
                },
                "failed to write config file /tmp/config.toml: disk full",
            ),
            (
                ConfigError::InsecureSecretsPermissions {
                    path: "/tmp/secrets.toml".to_string(),
                    mode: 0o644,
                },
                "expected 600",
            ),
            (
                ConfigError::TomlParse("unexpected token".to_string()),
                "failed to parse TOML: unexpected token",
            ),
            (
                ConfigError::TomlRootMustBeTable,
                "config root must be a TOML table",
            ),
            (
                ConfigError::UnknownTopLevelSection("wat".to_string()),
                "unknown top-level config section: wat",
            ),
            (
                ConfigError::InvalidSection {
                    section: "profile.default".to_string(),
                    expected: "table".to_string(),
                },
                "invalid section profile.default: expected table",
            ),
            (
                ConfigError::UnsupportedTomlValue {
                    path: "ui.format".to_string(),
                    kind: "array".to_string(),
                },
                "unsupported TOML value at ui.format: array",
            ),
            (
                ConfigError::InvalidEnvOverride {
                    key: "OSP_UI_FORMAT".to_string(),
                    reason: "unknown enum".to_string(),
                },
                "invalid env override OSP_UI_FORMAT: unknown enum",
            ),
            (
                ConfigError::InvalidConfigKey {
                    key: "ui.wat".to_string(),
                    reason: "unknown key".to_string(),
                },
                "invalid config key ui.wat: unknown key",
            ),
            (
                ConfigError::InvalidBootstrapScope {
                    key: "profile.default".to_string(),
                    profile: Some("prod".to_string()),
                    terminal: Some("repl".to_string()),
                },
                "profile=prod, terminal=repl",
            ),
            (
                ConfigError::InvalidBootstrapScope {
                    key: "profile.default".to_string(),
                    profile: Some("prod".to_string()),
                    terminal: None,
                },
                "scope profile=prod",
            ),
            (
                ConfigError::InvalidBootstrapScope {
                    key: "profile.default".to_string(),
                    profile: None,
                    terminal: Some("repl".to_string()),
                },
                "scope terminal=repl",
            ),
            (
                ConfigError::InvalidBootstrapScope {
                    key: "profile.default".to_string(),
                    profile: None,
                    terminal: None,
                },
                "scope global",
            ),
            (
                ConfigError::MissingDefaultProfile,
                "missing profile.default and no fallback profile",
            ),
            (
                ConfigError::InvalidBootstrapValue {
                    key: "profile.default".to_string(),
                    reason: "cannot be empty".to_string(),
                },
                "invalid bootstrap value for profile.default: cannot be empty",
            ),
            (
                ConfigError::UnknownProfile {
                    profile: "prod".to_string(),
                    known: vec!["default".to_string(), "dev".to_string()],
                },
                "unknown profile 'prod'. known profiles: default,dev",
            ),
            (
                ConfigError::InvalidPlaceholderSyntax {
                    key: "ui.format".to_string(),
                    template: "${oops".to_string(),
                },
                "invalid placeholder syntax in key ui.format: ${oops",
            ),
            (
                ConfigError::UnresolvedPlaceholder {
                    key: "ldap.uri".to_string(),
                    placeholder: "profile.current".to_string(),
                },
                "unresolved placeholder in key ldap.uri: profile.current",
            ),
            (
                ConfigError::PlaceholderCycle {
                    cycle: vec!["a".to_string(), "b".to_string(), "a".to_string()],
                },
                "placeholder cycle detected: a -> b -> a",
            ),
            (
                ConfigError::NonScalarPlaceholder {
                    key: "ldap.uri".to_string(),
                    placeholder: "profiles".to_string(),
                },
                "placeholder profiles in key ldap.uri points to a non-scalar value",
            ),
            (
                ConfigError::UnknownConfigKeys {
                    keys: vec!["ui.wat".to_string(), "ldap.nope".to_string()],
                },
                "unknown config keys: ui.wat, ldap.nope",
            ),
            (
                ConfigError::MissingRequiredKey {
                    key: "ldap.uri".to_string(),
                },
                "missing required config key: ldap.uri",
            ),
            (
                ConfigError::InvalidValueType {
                    key: "ui.debug".to_string(),
                    expected: SchemaValueType::Bool,
                    actual: "string".to_string(),
                },
                "invalid type for key ui.debug: expected bool, got string",
            ),
            (
                ConfigError::InvalidEnumValue {
                    key: "ui.format".to_string(),
                    value: "yaml".to_string(),
                    allowed: vec!["json".to_string(), "table".to_string()],
                },
                "invalid value for key ui.format: yaml. allowed: json, table",
            ),
        ];

        assert!(
            cases
                .into_iter()
                .all(|(error, expected)| error.to_string().contains(expected))
        );
    }

    #[test]
    fn with_path_context_wraps_source_error() {
        let wrapped = with_path_context(
            "/tmp/config.toml".to_string(),
            ConfigError::TomlParse("bad value".to_string()),
        );

        assert_eq!(
            wrapped.to_string(),
            "failed to parse TOML: bad value (path: /tmp/config.toml)"
        );

        if let ConfigError::LayerLoad { path, source } = wrapped {
            assert_eq!(path, "/tmp/config.toml");
            assert!(matches!(*source, ConfigError::TomlParse(_)));
        }
    }
}
