use std::fmt::{Display, Formatter};

use crate::config::SchemaValueType;

/// Error type returned by config parsing, validation, and resolution code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// Reading a config file from disk failed.
    FileRead {
        /// Path that could not be read.
        path: String,
        /// Lower-level failure description.
        reason: String,
    },
    /// Writing a config file to disk failed.
    FileWrite {
        /// Path that could not be written.
        path: String,
        /// Lower-level failure description.
        reason: String,
    },
    /// Adds path context while propagating another config error.
    LayerLoad {
        /// Path of the layer being loaded.
        path: String,
        /// Underlying config error.
        source: Box<ConfigError>,
    },
    /// Secrets file permissions are broader than allowed.
    InsecureSecretsPermissions {
        /// Path of the secrets file.
        path: String,
        /// Observed Unix file mode.
        mode: u32,
    },
    /// TOML parsing failed before semantic validation.
    TomlParse(String),
    /// The parsed TOML document root was not a table.
    TomlRootMustBeTable,
    /// Encountered an unsupported top-level section name.
    UnknownTopLevelSection(String),
    /// A named section had the wrong TOML type.
    InvalidSection {
        /// Fully qualified section name.
        section: String,
        /// Expected TOML kind for the section.
        expected: String,
    },
    /// Encountered a TOML value kind that the config model does not accept.
    UnsupportedTomlValue {
        /// Dotted config path at which the value was found.
        path: String,
        /// TOML value kind that was rejected.
        kind: String,
    },
    /// An `OSP__...` environment override used invalid syntax or scope.
    InvalidEnvOverride {
        /// Environment variable name or derived config key.
        key: String,
        /// Validation failure description.
        reason: String,
    },
    /// A config key failed syntactic or semantic validation.
    InvalidConfigKey {
        /// Key that failed validation.
        key: String,
        /// Validation failure description.
        reason: String,
    },
    /// A caller attempted to write a read-only config key.
    ReadOnlyConfigKey {
        /// Key that is read-only.
        key: String,
        /// Explanation of why the key is immutable.
        reason: String,
    },
    /// A bootstrap-only key was used in a disallowed scope.
    InvalidBootstrapScope {
        /// Bootstrap-only key being validated.
        key: String,
        /// Profile selector present on the rejected scope, if any.
        profile: Option<String>,
        /// Terminal selector present on the rejected scope, if any.
        terminal: Option<String>,
    },
    /// No default profile could be determined during bootstrap.
    MissingDefaultProfile,
    /// A bootstrap-only key failed its value validation rule.
    InvalidBootstrapValue {
        /// Bootstrap-only key being validated.
        key: String,
        /// Validation failure description.
        reason: String,
    },
    /// Resolution requested a profile that is not known.
    UnknownProfile {
        /// Requested profile name.
        profile: String,
        /// Known profile names at the time of resolution.
        known: Vec<String>,
    },
    /// Placeholder syntax inside a template string is malformed.
    InvalidPlaceholderSyntax {
        /// Key containing the invalid template.
        key: String,
        /// Original template string.
        template: String,
    },
    /// Placeholder expansion referenced an unknown key.
    UnresolvedPlaceholder {
        /// Key whose template is being resolved.
        key: String,
        /// Placeholder that could not be resolved.
        placeholder: String,
    },
    /// Placeholder expansion formed a dependency cycle.
    PlaceholderCycle {
        /// Ordered key path describing the detected cycle.
        cycle: Vec<String>,
    },
    /// Placeholder expansion referenced a non-scalar value.
    NonScalarPlaceholder {
        /// Key whose template is being resolved.
        key: String,
        /// Placeholder that resolved to a non-scalar value.
        placeholder: String,
    },
    /// One or more config keys are unknown to the schema.
    UnknownConfigKeys {
        /// Unknown canonical keys.
        keys: Vec<String>,
    },
    /// A required runtime-visible key was missing after resolution.
    MissingRequiredKey {
        /// Missing canonical key.
        key: String,
    },
    /// A value could not be adapted to the schema's declared type.
    InvalidValueType {
        /// Key whose value had the wrong type.
        key: String,
        /// Schema type expected for the key.
        expected: SchemaValueType,
        /// Actual type name observed during adaptation.
        actual: String,
    },
    /// A string value was outside the schema allow-list.
    InvalidEnumValue {
        /// Key whose value was rejected.
        key: String,
        /// Rejected value.
        value: String,
        /// Allowed normalized values.
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
            ConfigError::ReadOnlyConfigKey { key, reason } => {
                write!(f, "config key {key} is read-only: {reason}")
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
    use crate::config::SchemaValueType;

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
                ConfigError::ReadOnlyConfigKey {
                    key: "profile.active".to_string(),
                    reason: "derived at runtime".to_string(),
                },
                "config key profile.active is read-only: derived at runtime",
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
