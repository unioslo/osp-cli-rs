use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};
use std::sync::OnceLock;

use crate::config::ConfigError;

/// Result details for an in-place TOML edit operation.
#[derive(Debug, Clone, PartialEq)]
pub struct TomlEditResult {
    /// Previous value removed or replaced by the edit, if one existed.
    pub previous: Option<ConfigValue>,
}

/// Origin of a resolved configuration value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ConfigSource {
    /// Built-in defaults compiled into the CLI.
    BuiltinDefaults,
    /// Presentation defaults derived from the active UI preset.
    PresentationDefaults,
    /// Values loaded from user configuration files.
    ConfigFile,
    /// Values loaded from the secrets layer, including secrets files and
    /// secret-specific environment overrides.
    Secrets,
    /// Values supplied through `OSP__...` environment variables.
    Environment,
    /// Values supplied on the current command line.
    Cli,
    /// Values recorded for the current interactive session.
    Session,
    /// Values derived internally during resolution.
    Derived,
}

impl Display for ConfigSource {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            ConfigSource::BuiltinDefaults => "defaults",
            ConfigSource::PresentationDefaults => "presentation",
            ConfigSource::ConfigFile => "file",
            ConfigSource::Secrets => "secrets",
            ConfigSource::Environment => "env",
            ConfigSource::Cli => "cli",
            ConfigSource::Session => "session",
            ConfigSource::Derived => "derived",
        };
        write!(f, "{value}")
    }
}

/// Typed value stored in config layers and resolved output.
#[derive(Debug, Clone, PartialEq)]
pub enum ConfigValue {
    /// UTF-8 string value.
    String(String),
    /// Boolean value.
    Bool(bool),
    /// Signed 64-bit integer value.
    Integer(i64),
    /// 64-bit floating-point value.
    Float(f64),
    /// Ordered list of nested config values.
    List(Vec<ConfigValue>),
    /// Value wrapped for redacted display and debug output.
    Secret(SecretValue),
}

impl ConfigValue {
    /// Returns `true` when the value is wrapped as a secret.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::config::ConfigValue;
    ///
    /// assert!(!ConfigValue::String("alice".to_string()).is_secret());
    /// assert!(ConfigValue::String("alice".to_string()).into_secret().is_secret());
    /// ```
    pub fn is_secret(&self) -> bool {
        matches!(self, ConfigValue::Secret(_))
    }

    /// Returns the underlying value, unwrapping one secret layer if present.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::config::ConfigValue;
    ///
    /// let secret = ConfigValue::String("alice".to_string()).into_secret();
    /// assert_eq!(secret.reveal(), &ConfigValue::String("alice".to_string()));
    /// ```
    pub fn reveal(&self) -> &ConfigValue {
        match self {
            ConfigValue::Secret(secret) => secret.expose(),
            other => other,
        }
    }

    /// Wraps the value as a secret unless it is already secret.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::config::ConfigValue;
    ///
    /// let wrapped = ConfigValue::String("token".to_string()).into_secret();
    /// assert!(wrapped.is_secret());
    /// ```
    pub fn into_secret(self) -> ConfigValue {
        match self {
            ConfigValue::Secret(_) => self,
            other => ConfigValue::Secret(SecretValue::new(other)),
        }
    }

    pub(crate) fn from_toml(path: &str, value: &toml::Value) -> Result<Self, ConfigError> {
        match value {
            toml::Value::String(v) => Ok(Self::String(v.clone())),
            toml::Value::Integer(v) => Ok(Self::Integer(*v)),
            toml::Value::Float(v) => Ok(Self::Float(*v)),
            toml::Value::Boolean(v) => Ok(Self::Bool(*v)),
            toml::Value::Datetime(v) => Ok(Self::String(v.to_string())),
            toml::Value::Array(values) => {
                let mut out = Vec::with_capacity(values.len());
                for item in values {
                    out.push(Self::from_toml(path, item)?);
                }
                Ok(Self::List(out))
            }
            toml::Value::Table(_) => Err(ConfigError::UnsupportedTomlValue {
                path: path.to_string(),
                kind: "table".to_string(),
            }),
        }
    }

    pub(crate) fn as_interpolation_string(
        &self,
        key: &str,
        placeholder: &str,
    ) -> Result<String, ConfigError> {
        match self.reveal() {
            ConfigValue::String(value) => Ok(value.clone()),
            ConfigValue::Bool(value) => Ok(value.to_string()),
            ConfigValue::Integer(value) => Ok(value.to_string()),
            ConfigValue::Float(value) => Ok(value.to_string()),
            ConfigValue::List(_) => Err(ConfigError::NonScalarPlaceholder {
                key: key.to_string(),
                placeholder: placeholder.to_string(),
            }),
            ConfigValue::Secret(_) => Err(ConfigError::NonScalarPlaceholder {
                key: key.to_string(),
                placeholder: placeholder.to_string(),
            }),
        }
    }
}

/// Secret config value that redacts its display and debug output.
#[derive(Clone, PartialEq)]
pub struct SecretValue(Box<ConfigValue>);

impl SecretValue {
    /// Wraps a config value in a secret container.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::config::{ConfigValue, SecretValue};
    ///
    /// let secret = SecretValue::new(ConfigValue::String("hidden".to_string()));
    /// assert_eq!(secret.expose(), &ConfigValue::String("hidden".to_string()));
    /// ```
    pub fn new(value: ConfigValue) -> Self {
        Self(Box::new(value))
    }

    /// Returns the underlying unredacted value.
    pub fn expose(&self) -> &ConfigValue {
        &self.0
    }

    /// Consumes the wrapper and returns the inner value.
    pub fn into_inner(self) -> ConfigValue {
        *self.0
    }
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "[REDACTED]")
    }
}

impl Display for SecretValue {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "[REDACTED]")
    }
}

/// Schema-level type used for parsing and validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaValueType {
    /// Scalar string value.
    String,
    /// Scalar boolean value.
    Bool,
    /// Scalar signed integer value.
    Integer,
    /// Scalar floating-point value.
    Float,
    /// List of string values.
    StringList,
}

impl Display for SchemaValueType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            SchemaValueType::String => "string",
            SchemaValueType::Bool => "bool",
            SchemaValueType::Integer => "integer",
            SchemaValueType::Float => "float",
            SchemaValueType::StringList => "list",
        };
        write!(f, "{value}")
    }
}

/// Bootstrap stage in which a key must be resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapPhase {
    /// The key is needed before path-dependent config can be loaded.
    Path,
    /// The key is needed before the active profile can be finalized.
    Profile,
}

/// Scope restriction for bootstrap-only keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapScopeRule {
    /// The key is valid only in the global scope.
    GlobalOnly,
    /// The key is valid globally or in a terminal-only scope.
    GlobalOrTerminal,
}

/// Additional validation rule for bootstrap values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapValueRule {
    /// The value must be a string containing at least one non-whitespace character.
    NonEmptyString,
}

/// Bootstrap metadata derived from a schema entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapKeySpec {
    /// Canonical dotted config key.
    pub key: &'static str,
    /// Bootstrap phase in which the key is consulted.
    pub phase: BootstrapPhase,
    /// Whether the key also appears in the runtime-resolved config.
    pub runtime_visible: bool,
    /// Scope restriction enforced for the key.
    pub scope_rule: BootstrapScopeRule,
}

impl BootstrapKeySpec {
    fn allows_scope(&self, scope: &Scope) -> bool {
        match self.scope_rule {
            BootstrapScopeRule::GlobalOnly => scope.profile.is_none() && scope.terminal.is_none(),
            BootstrapScopeRule::GlobalOrTerminal => scope.profile.is_none(),
        }
    }
}

/// Schema definition for a single config key.
#[derive(Debug, Clone)]
#[must_use]
pub struct SchemaEntry {
    canonical_key: Option<&'static str>,
    value_type: SchemaValueType,
    required: bool,
    writable: bool,
    allowed_values: Option<Vec<String>>,
    runtime_visible: bool,
    bootstrap_phase: Option<BootstrapPhase>,
    bootstrap_scope_rule: Option<BootstrapScopeRule>,
    bootstrap_value_rule: Option<BootstrapValueRule>,
}

impl SchemaEntry {
    /// Starts a schema entry for string values.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::config::{SchemaEntry, SchemaValueType};
    ///
    /// let entry = SchemaEntry::string().required();
    /// assert_eq!(entry.value_type(), SchemaValueType::String);
    /// assert!(entry.runtime_visible());
    /// ```
    pub fn string() -> Self {
        Self {
            canonical_key: None,
            value_type: SchemaValueType::String,
            required: false,
            writable: true,
            allowed_values: None,
            runtime_visible: true,
            bootstrap_phase: None,
            bootstrap_scope_rule: None,
            bootstrap_value_rule: None,
        }
    }

    /// Starts a schema entry for boolean values.
    pub fn boolean() -> Self {
        Self {
            canonical_key: None,
            value_type: SchemaValueType::Bool,
            required: false,
            writable: true,
            allowed_values: None,
            runtime_visible: true,
            bootstrap_phase: None,
            bootstrap_scope_rule: None,
            bootstrap_value_rule: None,
        }
    }

    /// Starts a schema entry for integer values.
    pub fn integer() -> Self {
        Self {
            canonical_key: None,
            value_type: SchemaValueType::Integer,
            required: false,
            writable: true,
            allowed_values: None,
            runtime_visible: true,
            bootstrap_phase: None,
            bootstrap_scope_rule: None,
            bootstrap_value_rule: None,
        }
    }

    /// Starts a schema entry for floating-point values.
    pub fn float() -> Self {
        Self {
            canonical_key: None,
            value_type: SchemaValueType::Float,
            required: false,
            writable: true,
            allowed_values: None,
            runtime_visible: true,
            bootstrap_phase: None,
            bootstrap_scope_rule: None,
            bootstrap_value_rule: None,
        }
    }

    /// Starts a schema entry for lists of strings.
    pub fn string_list() -> Self {
        Self {
            canonical_key: None,
            value_type: SchemaValueType::StringList,
            required: false,
            writable: true,
            allowed_values: None,
            runtime_visible: true,
            bootstrap_phase: None,
            bootstrap_scope_rule: None,
            bootstrap_value_rule: None,
        }
    }

    /// Marks the key as required in the resolved runtime view.
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Marks the key as read-only for user-provided config sources.
    pub fn read_only(mut self) -> Self {
        self.writable = false;
        self
    }

    /// Marks the key as bootstrap-only with the given phase and scope rule.
    pub fn bootstrap_only(mut self, phase: BootstrapPhase, scope_rule: BootstrapScopeRule) -> Self {
        self.runtime_visible = false;
        self.bootstrap_phase = Some(phase);
        self.bootstrap_scope_rule = Some(scope_rule);
        self
    }

    /// Adds a bootstrap-only value validation rule.
    pub fn with_bootstrap_value_rule(mut self, rule: BootstrapValueRule) -> Self {
        self.bootstrap_value_rule = Some(rule);
        self
    }

    /// Restricts accepted values using a case-insensitive allow-list.
    pub fn with_allowed_values<I, S>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.allowed_values = Some(
            values
                .into_iter()
                .map(|value| value.as_ref().to_ascii_lowercase())
                .collect(),
        );
        self
    }

    /// Returns the declared schema type for the key.
    pub fn value_type(&self) -> SchemaValueType {
        self.value_type
    }

    /// Returns the normalized allow-list, if the key is enumerated.
    pub fn allowed_values(&self) -> Option<&[String]> {
        self.allowed_values.as_deref()
    }

    /// Returns whether the key is visible in resolved runtime config.
    pub fn runtime_visible(&self) -> bool {
        self.runtime_visible
    }

    /// Returns whether the key can be written by user-controlled sources.
    pub fn writable(&self) -> bool {
        self.writable
    }

    fn with_canonical_key(mut self, key: &'static str) -> Self {
        self.canonical_key = Some(key);
        self
    }

    fn bootstrap_spec(&self) -> Option<BootstrapKeySpec> {
        Some(BootstrapKeySpec {
            key: self.canonical_key?,
            phase: self.bootstrap_phase?,
            runtime_visible: self.runtime_visible,
            scope_rule: self.bootstrap_scope_rule?,
        })
    }

    fn validate_bootstrap_value(&self, key: &str, value: &ConfigValue) -> Result<(), ConfigError> {
        match self.bootstrap_value_rule {
            Some(BootstrapValueRule::NonEmptyString) => match value.reveal() {
                ConfigValue::String(current) if !current.trim().is_empty() => Ok(()),
                ConfigValue::String(current) => Err(ConfigError::InvalidBootstrapValue {
                    key: key.to_string(),
                    reason: format!("expected a non-empty string, got {current:?}"),
                }),
                other => Err(ConfigError::InvalidBootstrapValue {
                    key: key.to_string(),
                    reason: format!("expected string, got {other:?}"),
                }),
            },
            None => Ok(()),
        }
    }
}

/// Config schema used for validation, parsing, and runtime filtering.
#[derive(Debug, Clone)]
pub struct ConfigSchema {
    entries: BTreeMap<String, SchemaEntry>,
    allow_extensions_namespace: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DynamicSchemaKeyKind {
    PluginCommandState,
    PluginCommandProvider,
}

impl Default for ConfigSchema {
    fn default() -> Self {
        builtin_config_schema().clone()
    }
}

impl ConfigSchema {
    fn builtin() -> Self {
        let mut schema = Self {
            entries: BTreeMap::new(),
            allow_extensions_namespace: true,
        };

        schema.insert(
            "profile.default",
            SchemaEntry::string()
                .bootstrap_only(
                    BootstrapPhase::Profile,
                    BootstrapScopeRule::GlobalOrTerminal,
                )
                .with_bootstrap_value_rule(BootstrapValueRule::NonEmptyString),
        );
        schema.insert(
            "profile.active",
            SchemaEntry::string().required().read_only(),
        );
        schema.insert("theme.name", SchemaEntry::string());
        schema.insert("theme.path", SchemaEntry::string_list());
        schema.insert("user.name", SchemaEntry::string());
        schema.insert("user.display_name", SchemaEntry::string());
        schema.insert("user.full_name", SchemaEntry::string());
        schema.insert("domain", SchemaEntry::string());

        schema.insert(
            "ui.format",
            SchemaEntry::string()
                .with_allowed_values(["auto", "guide", "json", "table", "md", "mreg", "value"]),
        );
        schema.insert(
            "ui.mode",
            SchemaEntry::string().with_allowed_values(["auto", "plain", "rich"]),
        );
        schema.insert(
            "ui.presentation",
            SchemaEntry::string().with_allowed_values([
                "expressive",
                "compact",
                "austere",
                "gammel-og-bitter",
            ]),
        );
        schema.insert(
            "ui.color.mode",
            SchemaEntry::string().with_allowed_values(["auto", "always", "never"]),
        );
        schema.insert(
            "ui.unicode.mode",
            SchemaEntry::string().with_allowed_values(["auto", "always", "never"]),
        );
        schema.insert("ui.width", SchemaEntry::integer());
        schema.insert("ui.margin", SchemaEntry::integer());
        schema.insert("ui.indent", SchemaEntry::integer());
        schema.insert(
            "ui.help.level",
            SchemaEntry::string()
                .with_allowed_values(["inherit", "none", "tiny", "normal", "verbose"]),
        );
        schema.insert(
            "ui.guide.default_format",
            SchemaEntry::string().with_allowed_values(["guide", "inherit", "none"]),
        );
        schema.insert(
            "ui.messages.layout",
            SchemaEntry::string().with_allowed_values(["grouped", "minimal"]),
        );
        schema.insert(
            "ui.chrome.frame",
            SchemaEntry::string().with_allowed_values([
                "none",
                "top",
                "bottom",
                "top-bottom",
                "square",
                "round",
            ]),
        );
        schema.insert(
            "ui.chrome.rule_policy",
            SchemaEntry::string().with_allowed_values([
                "per-section",
                "independent",
                "separate",
                "shared",
                "stacked",
                "list",
            ]),
        );
        schema.insert(
            "ui.table.overflow",
            SchemaEntry::string().with_allowed_values([
                "clip", "hidden", "crop", "ellipsis", "truncate", "wrap", "none", "visible",
            ]),
        );
        schema.insert(
            "ui.table.border",
            SchemaEntry::string().with_allowed_values(["none", "square", "round"]),
        );
        schema.insert(
            "ui.help.table_chrome",
            SchemaEntry::string().with_allowed_values(["inherit", "none", "square", "round"]),
        );
        schema.insert("ui.help.entry_indent", SchemaEntry::string());
        schema.insert("ui.help.entry_gap", SchemaEntry::string());
        schema.insert("ui.help.section_spacing", SchemaEntry::string());
        schema.insert("ui.short_list_max", SchemaEntry::integer());
        schema.insert("ui.medium_list_max", SchemaEntry::integer());
        schema.insert("ui.grid_padding", SchemaEntry::integer());
        schema.insert("ui.grid_columns", SchemaEntry::integer());
        schema.insert("ui.column_weight", SchemaEntry::integer());
        schema.insert("ui.mreg.stack_min_col_width", SchemaEntry::integer());
        schema.insert("ui.mreg.stack_overflow_ratio", SchemaEntry::integer());
        schema.insert(
            "ui.message.verbosity",
            SchemaEntry::string()
                .with_allowed_values(["error", "warning", "success", "info", "trace"]),
        );
        schema.insert("ui.prompt", SchemaEntry::string());
        schema.insert("ui.prompt.secrets", SchemaEntry::boolean());
        schema.insert("extensions.plugins.timeout_ms", SchemaEntry::integer());
        schema.insert("extensions.plugins.discovery.path", SchemaEntry::boolean());
        schema.insert("repl.prompt", SchemaEntry::string());
        schema.insert(
            "repl.input_mode",
            SchemaEntry::string().with_allowed_values(["auto", "interactive", "basic"]),
        );
        schema.insert("repl.simple_prompt", SchemaEntry::boolean());
        schema.insert("repl.shell_indicator", SchemaEntry::string());
        schema.insert(
            "repl.intro",
            SchemaEntry::string().with_allowed_values(["none", "minimal", "compact", "full"]),
        );
        schema.insert("repl.intro_template.minimal", SchemaEntry::string());
        schema.insert("repl.intro_template.compact", SchemaEntry::string());
        schema.insert("repl.intro_template.full", SchemaEntry::string());
        schema.insert("repl.history.path", SchemaEntry::string());
        schema.insert("repl.history.max_entries", SchemaEntry::integer());
        schema.insert("repl.history.enabled", SchemaEntry::boolean());
        schema.insert("repl.history.dedupe", SchemaEntry::boolean());
        schema.insert("repl.history.profile_scoped", SchemaEntry::boolean());
        schema.insert("repl.history.menu_rows", SchemaEntry::integer());
        schema.insert("repl.history.exclude", SchemaEntry::string_list());
        schema.insert("session.cache.max_results", SchemaEntry::integer());
        schema.insert("color.prompt.text", SchemaEntry::string());
        schema.insert("color.prompt.command", SchemaEntry::string());
        schema.insert("color.prompt.completion.text", SchemaEntry::string());
        schema.insert("color.prompt.completion.background", SchemaEntry::string());
        schema.insert("color.prompt.completion.highlight", SchemaEntry::string());
        schema.insert("color.text", SchemaEntry::string());
        schema.insert("color.text.muted", SchemaEntry::string());
        schema.insert("color.key", SchemaEntry::string());
        schema.insert("color.border", SchemaEntry::string());
        schema.insert("color.table.header", SchemaEntry::string());
        schema.insert("color.mreg.key", SchemaEntry::string());
        schema.insert("color.value", SchemaEntry::string());
        schema.insert("color.value.number", SchemaEntry::string());
        schema.insert("color.value.bool_true", SchemaEntry::string());
        schema.insert("color.value.bool_false", SchemaEntry::string());
        schema.insert("color.value.null", SchemaEntry::string());
        schema.insert("color.value.ipv4", SchemaEntry::string());
        schema.insert("color.value.ipv6", SchemaEntry::string());
        schema.insert("color.panel.border", SchemaEntry::string());
        schema.insert("color.panel.title", SchemaEntry::string());
        schema.insert("color.code", SchemaEntry::string());
        schema.insert("color.json.key", SchemaEntry::string());
        schema.insert("color.message.error", SchemaEntry::string());
        schema.insert("color.message.warning", SchemaEntry::string());
        schema.insert("color.message.success", SchemaEntry::string());
        schema.insert("color.message.info", SchemaEntry::string());
        schema.insert("color.message.trace", SchemaEntry::string());
        schema.insert("auth.visible.builtins", SchemaEntry::string());
        schema.insert("auth.visible.plugins", SchemaEntry::string());
        schema.insert("debug.level", SchemaEntry::integer());
        schema.insert("log.file.enabled", SchemaEntry::boolean());
        schema.insert("log.file.path", SchemaEntry::string());
        schema.insert(
            "log.file.level",
            SchemaEntry::string().with_allowed_values(["error", "warn", "info", "debug", "trace"]),
        );

        schema.insert("base.dir", SchemaEntry::string());

        schema
    }
}

impl ConfigSchema {
    /// Registers or replaces a schema entry for a canonical key.
    pub fn insert(&mut self, key: &'static str, entry: SchemaEntry) {
        self.entries
            .insert(key.to_string(), entry.with_canonical_key(key));
    }

    /// Enables or disables the `extensions.*` namespace shortcut.
    pub fn set_allow_extensions_namespace(&mut self, value: bool) {
        self.allow_extensions_namespace = value;
    }

    /// Returns whether the key is recognized by the schema.
    pub fn is_known_key(&self, key: &str) -> bool {
        self.entries.contains_key(key)
            || self.is_extension_key(key)
            || self.is_alias_key(key)
            || dynamic_schema_key_kind(key).is_some()
    }

    /// Returns whether the key can appear in resolved runtime output.
    pub fn is_runtime_visible_key(&self, key: &str) -> bool {
        self.entries
            .get(key)
            .is_some_and(SchemaEntry::runtime_visible)
            || self.is_extension_key(key)
            || dynamic_schema_key_kind(key).is_some()
    }

    /// Rejects read-only keys for user-supplied config input.
    pub fn validate_writable_key(&self, key: &str) -> Result<(), ConfigError> {
        let normalized = key.trim().to_ascii_lowercase();
        if let Some(entry) = self.entries.get(&normalized)
            && !entry.writable()
        {
            return Err(ConfigError::ReadOnlyConfigKey {
                key: normalized,
                reason: "derived at runtime".to_string(),
            });
        }
        Ok(())
    }

    /// Returns bootstrap metadata for the key, if it has bootstrap semantics.
    pub fn bootstrap_key_spec(&self, key: &str) -> Option<BootstrapKeySpec> {
        let normalized = key.trim().to_ascii_lowercase();
        self.entries
            .get(&normalized)
            .and_then(SchemaEntry::bootstrap_spec)
    }

    /// Iterates over canonical schema entries.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &SchemaEntry)> {
        self.entries
            .iter()
            .map(|(key, entry)| (key.as_str(), entry))
    }

    /// Returns the expected runtime type for a key.
    pub fn expected_type(&self, key: &str) -> Option<SchemaValueType> {
        self.entries
            .get(key)
            .map(|entry| entry.value_type)
            .or_else(|| dynamic_schema_key_kind(key).map(|_| SchemaValueType::String))
    }

    /// Parses a raw string into the schema's typed config representation.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::config::{ConfigSchema, ConfigValue};
    ///
    /// let schema = ConfigSchema::default();
    /// assert_eq!(
    ///     schema.parse_input_value("repl.history.enabled", "true").unwrap(),
    ///     ConfigValue::Bool(true)
    /// );
    /// assert_eq!(
    ///     schema.parse_input_value("theme.name", "dracula").unwrap(),
    ///     ConfigValue::String("dracula".to_string())
    /// );
    /// ```
    pub fn parse_input_value(&self, key: &str, raw: &str) -> Result<ConfigValue, ConfigError> {
        if !self.is_known_key(key) {
            return Err(ConfigError::UnknownConfigKeys {
                keys: vec![key.to_string()],
            });
        }
        self.validate_writable_key(key)?;

        let value = match self.expected_type(key) {
            Some(SchemaValueType::String) | None => ConfigValue::String(raw.to_string()),
            Some(SchemaValueType::Bool) => {
                ConfigValue::Bool(
                    parse_bool(raw).ok_or_else(|| ConfigError::InvalidValueType {
                        key: key.to_string(),
                        expected: SchemaValueType::Bool,
                        actual: "string".to_string(),
                    })?,
                )
            }
            Some(SchemaValueType::Integer) => {
                let parsed =
                    raw.trim()
                        .parse::<i64>()
                        .map_err(|_| ConfigError::InvalidValueType {
                            key: key.to_string(),
                            expected: SchemaValueType::Integer,
                            actual: "string".to_string(),
                        })?;
                ConfigValue::Integer(parsed)
            }
            Some(SchemaValueType::Float) => {
                let parsed =
                    raw.trim()
                        .parse::<f64>()
                        .map_err(|_| ConfigError::InvalidValueType {
                            key: key.to_string(),
                            expected: SchemaValueType::Float,
                            actual: "string".to_string(),
                        })?;
                ConfigValue::Float(parsed)
            }
            Some(SchemaValueType::StringList) => {
                let items = parse_string_list(raw);
                ConfigValue::List(items.into_iter().map(ConfigValue::String).collect())
            }
        };

        if let Some(entry) = self.entries.get(key) {
            validate_allowed_values(
                key,
                &value,
                entry
                    .allowed_values()
                    .map(|values| values.iter().map(String::as_str).collect::<Vec<_>>())
                    .as_deref(),
            )?;
        } else if let Some(DynamicSchemaKeyKind::PluginCommandState) = dynamic_schema_key_kind(key)
        {
            validate_allowed_values(key, &value, Some(&["enabled", "disabled"]))?;
        }

        Ok(value)
    }

    pub(crate) fn validate_and_adapt(
        &self,
        values: &mut BTreeMap<String, ResolvedValue>,
    ) -> Result<(), ConfigError> {
        let mut unknown = Vec::new();
        for key in values.keys() {
            if self.is_runtime_visible_key(key) {
                continue;
            }
            unknown.push(key.clone());
        }
        if !unknown.is_empty() {
            unknown.sort();
            return Err(ConfigError::UnknownConfigKeys { keys: unknown });
        }

        for (key, entry) in &self.entries {
            if entry.runtime_visible && entry.required && !values.contains_key(key) {
                return Err(ConfigError::MissingRequiredKey { key: key.clone() });
            }
        }

        for (key, resolved) in values.iter_mut() {
            if let Some(kind) = dynamic_schema_key_kind(key) {
                resolved.value = adapt_dynamic_value_for_schema(key, &resolved.value, kind)?;
                continue;
            }
            let Some(schema_entry) = self.entries.get(key) else {
                continue;
            };
            if !schema_entry.runtime_visible {
                continue;
            }
            resolved.value = adapt_value_for_schema(key, &resolved.value, schema_entry)?;
        }

        Ok(())
    }

    fn is_extension_key(&self, key: &str) -> bool {
        self.allow_extensions_namespace && key.starts_with("extensions.")
    }

    fn is_alias_key(&self, key: &str) -> bool {
        key.starts_with("alias.")
    }

    /// Validates that a key is allowed in the provided scope.
    pub fn validate_key_scope(&self, key: &str, scope: &Scope) -> Result<(), ConfigError> {
        let normalized_scope = normalize_scope(scope.clone());
        if let Some(spec) = self.bootstrap_key_spec(key)
            && !spec.allows_scope(&normalized_scope)
        {
            return Err(ConfigError::InvalidBootstrapScope {
                key: spec.key.to_string(),
                profile: normalized_scope.profile,
                terminal: normalized_scope.terminal,
            });
        }

        Ok(())
    }

    /// Validates bootstrap-only value rules for a key.
    pub fn validate_bootstrap_value(
        &self,
        key: &str,
        value: &ConfigValue,
    ) -> Result<(), ConfigError> {
        let normalized = key.trim().to_ascii_lowercase();
        let Some(entry) = self.entries.get(&normalized) else {
            return Ok(());
        };
        entry.validate_bootstrap_value(&normalized, value)
    }
}

fn builtin_config_schema() -> &'static ConfigSchema {
    static BUILTIN_SCHEMA: OnceLock<ConfigSchema> = OnceLock::new();
    BUILTIN_SCHEMA.get_or_init(ConfigSchema::builtin)
}

impl Display for ConfigValue {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigValue::String(v) => write!(f, "{v}"),
            ConfigValue::Bool(v) => write!(f, "{v}"),
            ConfigValue::Integer(v) => write!(f, "{v}"),
            ConfigValue::Float(v) => write!(f, "{v}"),
            ConfigValue::List(v) => {
                let joined = v
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<String>>()
                    .join(",");
                write!(f, "[{joined}]")
            }
            ConfigValue::Secret(secret) => write!(f, "{secret}"),
        }
    }
}

impl From<&str> for ConfigValue {
    fn from(value: &str) -> Self {
        ConfigValue::String(value.to_string())
    }
}

impl From<String> for ConfigValue {
    fn from(value: String) -> Self {
        ConfigValue::String(value)
    }
}

impl From<bool> for ConfigValue {
    fn from(value: bool) -> Self {
        ConfigValue::Bool(value)
    }
}

impl From<i64> for ConfigValue {
    fn from(value: i64) -> Self {
        ConfigValue::Integer(value)
    }
}

impl From<f64> for ConfigValue {
    fn from(value: f64) -> Self {
        ConfigValue::Float(value)
    }
}

impl From<Vec<String>> for ConfigValue {
    fn from(values: Vec<String>) -> Self {
        ConfigValue::List(values.into_iter().map(ConfigValue::String).collect())
    }
}

/// Scope selector used when storing or resolving config entries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    /// Profile selector, normalized to the canonical profile identifier.
    pub profile: Option<String>,
    /// Terminal selector, normalized to the canonical terminal identifier.
    pub terminal: Option<String>,
}

impl Scope {
    /// Creates an unscoped selector.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::config::Scope;
    ///
    /// assert_eq!(Scope::global(), Scope::default());
    /// ```
    pub fn global() -> Self {
        Self::default()
    }

    /// Creates a selector scoped to one profile.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::config::Scope;
    ///
    /// let scope = Scope::profile("TSD");
    /// assert_eq!(scope.profile.as_deref(), Some("tsd"));
    /// assert_eq!(scope.terminal, None);
    /// ```
    pub fn profile(profile: &str) -> Self {
        Self {
            profile: Some(normalize_identifier(profile)),
            terminal: None,
        }
    }

    /// Creates a selector scoped to one terminal kind.
    pub fn terminal(terminal: &str) -> Self {
        Self {
            profile: None,
            terminal: Some(normalize_identifier(terminal)),
        }
    }

    /// Creates a selector scoped to both profile and terminal.
    pub fn profile_terminal(profile: &str, terminal: &str) -> Self {
        Self {
            profile: Some(normalize_identifier(profile)),
            terminal: Some(normalize_identifier(terminal)),
        }
    }
}

/// Single entry stored inside a config layer.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerEntry {
    /// Canonical config key.
    pub key: String,
    /// Stored value for the key in this layer.
    pub value: ConfigValue,
    /// Scope attached to the entry.
    pub scope: Scope,
    /// External origin label such as an environment variable name.
    pub origin: Option<String>,
}

/// Ordered collection of config entries from one source layer.
#[derive(Debug, Clone, Default)]
pub struct ConfigLayer {
    pub(crate) entries: Vec<LayerEntry>,
}

impl ConfigLayer {
    /// Returns the entries in insertion order.
    pub fn entries(&self) -> &[LayerEntry] {
        &self.entries
    }

    /// Appends every entry from another layer in insertion order.
    ///
    /// Later entries from `other` win over earlier entries in this layer when
    /// the resolver evaluates duplicate keys from the same source layer.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::config::ConfigLayer;
    ///
    /// let mut base = ConfigLayer::default();
    /// base.set("theme.name", "dracula");
    ///
    /// let mut site = ConfigLayer::default();
    /// site.set("extensions.site.enabled", true);
    /// site.set("theme.name", "nord");
    ///
    /// base.extend_from_layer(&site);
    ///
    /// assert_eq!(base.entries().len(), 3);
    /// assert_eq!(base.entries()[2].key, "theme.name");
    /// assert_eq!(base.entries()[2].value.to_string(), "nord");
    /// ```
    pub fn extend_from_layer(&mut self, other: &ConfigLayer) {
        self.entries.extend(other.entries().iter().cloned());
    }

    /// Inserts a global entry.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::config::{ConfigLayer, Scope};
    ///
    /// let mut layer = ConfigLayer::default();
    /// layer.set("theme.name", "dracula");
    ///
    /// let entry = &layer.entries()[0];
    /// assert_eq!(entry.key, "theme.name");
    /// assert_eq!(entry.scope, Scope::global());
    /// ```
    pub fn set<K, V>(&mut self, key: K, value: V)
    where
        K: Into<String>,
        V: Into<ConfigValue>,
    {
        self.insert(key, value, Scope::global());
    }

    /// Inserts an entry scoped to a profile.
    pub fn set_for_profile<K, V>(&mut self, profile: &str, key: K, value: V)
    where
        K: Into<String>,
        V: Into<ConfigValue>,
    {
        self.insert(key, value, Scope::profile(profile));
    }

    /// Inserts an entry scoped to a terminal.
    pub fn set_for_terminal<K, V>(&mut self, terminal: &str, key: K, value: V)
    where
        K: Into<String>,
        V: Into<ConfigValue>,
    {
        self.insert(key, value, Scope::terminal(terminal));
    }

    /// Inserts an entry scoped to both profile and terminal.
    pub fn set_for_profile_terminal<K, V>(
        &mut self,
        profile: &str,
        terminal: &str,
        key: K,
        value: V,
    ) where
        K: Into<String>,
        V: Into<ConfigValue>,
    {
        self.insert(key, value, Scope::profile_terminal(profile, terminal));
    }

    /// Inserts an entry with an explicit scope.
    pub fn insert<K, V>(&mut self, key: K, value: V, scope: Scope)
    where
        K: Into<String>,
        V: Into<ConfigValue>,
    {
        self.entries.push(LayerEntry {
            key: key.into(),
            value: value.into(),
            scope: normalize_scope(scope),
            origin: None,
        });
    }

    /// Inserts an entry and records its external origin.
    pub fn insert_with_origin<K, V, O>(&mut self, key: K, value: V, scope: Scope, origin: Option<O>)
    where
        K: Into<String>,
        V: Into<ConfigValue>,
        O: Into<String>,
    {
        self.entries.push(LayerEntry {
            key: key.into(),
            value: value.into(),
            scope: normalize_scope(scope),
            origin: origin.map(Into::into),
        });
    }

    /// Marks every entry in the layer as secret.
    pub fn mark_all_secret(&mut self) {
        for entry in &mut self.entries {
            if !entry.value.is_secret() {
                entry.value = entry.value.clone().into_secret();
            }
        }
    }

    /// Removes the last matching entry for a key and scope.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::config::{ConfigLayer, ConfigValue, Scope};
    ///
    /// let mut layer = ConfigLayer::default();
    /// layer.set("theme.name", "catppuccin");
    /// layer.set("theme.name", "dracula");
    ///
    /// let removed = layer.remove_scoped("theme.name", &Scope::global());
    /// assert_eq!(removed, Some(ConfigValue::String("dracula".to_string())));
    /// ```
    pub fn remove_scoped(&mut self, key: &str, scope: &Scope) -> Option<ConfigValue> {
        let normalized_scope = normalize_scope(scope.clone());
        let index = self
            .entries
            .iter()
            .rposition(|entry| entry.key == key && entry.scope == normalized_scope)?;
        Some(self.entries.remove(index).value)
    }

    /// Parses a config layer from the project's TOML layout.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::config::ConfigLayer;
    ///
    /// let layer = ConfigLayer::from_toml_str(r#"
    /// [default]
    /// theme.name = "dracula"
    ///
    /// [profile.tsd]
    /// ui.format = "json"
    /// "#).unwrap();
    ///
    /// assert_eq!(layer.entries().len(), 2);
    /// ```
    pub fn from_toml_str(raw: &str) -> Result<Self, ConfigError> {
        let parsed = raw
            .parse::<toml::Value>()
            .map_err(|err| ConfigError::TomlParse(err.to_string()))?;

        let root = parsed.as_table().ok_or(ConfigError::TomlRootMustBeTable)?;
        let mut layer = ConfigLayer::default();

        for (section, value) in root {
            match section.as_str() {
                "default" => {
                    let table = value
                        .as_table()
                        .ok_or_else(|| ConfigError::InvalidSection {
                            section: "default".to_string(),
                            expected: "table".to_string(),
                        })?;
                    flatten_table(&mut layer, table, "", &Scope::global())?;
                }
                "profile" => {
                    let profiles = value
                        .as_table()
                        .ok_or_else(|| ConfigError::InvalidSection {
                            section: "profile".to_string(),
                            expected: "table".to_string(),
                        })?;
                    for (profile, profile_table_value) in profiles {
                        let profile_table = profile_table_value.as_table().ok_or_else(|| {
                            ConfigError::InvalidSection {
                                section: format!("profile.{profile}"),
                                expected: "table".to_string(),
                            }
                        })?;
                        flatten_table(&mut layer, profile_table, "", &Scope::profile(profile))?;
                    }
                }
                "terminal" => {
                    let terminals =
                        value
                            .as_table()
                            .ok_or_else(|| ConfigError::InvalidSection {
                                section: "terminal".to_string(),
                                expected: "table".to_string(),
                            })?;

                    for (terminal, terminal_table_value) in terminals {
                        let terminal_table = terminal_table_value.as_table().ok_or_else(|| {
                            ConfigError::InvalidSection {
                                section: format!("terminal.{terminal}"),
                                expected: "table".to_string(),
                            }
                        })?;

                        for (key, terminal_value) in terminal_table {
                            if key == "profile" {
                                continue;
                            }

                            flatten_key_value(
                                &mut layer,
                                key,
                                terminal_value,
                                &Scope::terminal(terminal),
                            )?;
                        }

                        if let Some(profile_section) = terminal_table.get("profile") {
                            let profile_tables = profile_section.as_table().ok_or_else(|| {
                                ConfigError::InvalidSection {
                                    section: format!("terminal.{terminal}.profile"),
                                    expected: "table".to_string(),
                                }
                            })?;

                            for (profile_key, profile_value) in profile_tables {
                                if let Some(profile_table) = profile_value.as_table() {
                                    flatten_table(
                                        &mut layer,
                                        profile_table,
                                        "",
                                        &Scope::profile_terminal(profile_key, terminal),
                                    )?;
                                } else {
                                    flatten_key_value(
                                        &mut layer,
                                        &format!("profile.{profile_key}"),
                                        profile_value,
                                        &Scope::terminal(terminal),
                                    )?;
                                }
                            }
                        }
                    }
                }
                unknown => {
                    return Err(ConfigError::UnknownTopLevelSection(unknown.to_string()));
                }
            }
        }

        Ok(layer)
    }

    /// Builds a config layer from `OSP__...` environment variables.
    pub fn from_env_iter<I, K, V>(vars: I) -> Result<Self, ConfigError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut layer = ConfigLayer::default();

        for (name, value) in vars {
            let key = name.as_ref();
            if !key.starts_with("OSP__") {
                continue;
            }

            let spec = parse_env_key(key)?;
            builtin_config_schema().validate_writable_key(&spec.key)?;
            validate_key_scope(&spec.key, &spec.scope)?;
            let converted = ConfigValue::String(value.as_ref().to_string());
            validate_bootstrap_value(&spec.key, &converted)?;
            layer.insert_with_origin(spec.key, converted, spec.scope, Some(key.to_string()));
        }

        Ok(layer)
    }

    pub(crate) fn validate_entries(&self) -> Result<(), ConfigError> {
        for entry in &self.entries {
            builtin_config_schema().validate_writable_key(&entry.key)?;
            validate_key_scope(&entry.key, &entry.scope)?;
            validate_bootstrap_value(&entry.key, &entry.value)?;
        }

        Ok(())
    }
}

pub(crate) struct EnvKeySpec {
    pub(crate) key: String,
    pub(crate) scope: Scope,
}

/// Options that affect profile and terminal selection during resolution.
#[derive(Debug, Clone, Default)]
#[must_use]
pub struct ResolveOptions {
    /// Explicit profile to use instead of the configured default profile.
    pub profile_override: Option<String>,
    /// Terminal selector used to include terminal-scoped entries.
    pub terminal: Option<String>,
}

impl ResolveOptions {
    /// Creates empty resolution options with no explicit profile or terminal.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::config::ResolveOptions;
    ///
    /// let options = ResolveOptions::new();
    /// assert_eq!(options.profile_override, None);
    /// assert_eq!(options.terminal, None);
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the optional normalized profile override.
    pub fn with_profile_override(mut self, profile_override: Option<String>) -> Self {
        self.profile_override = profile_override
            .map(|value| normalize_identifier(&value))
            .filter(|value| !value.is_empty());
        self
    }

    /// Forces resolution to use the provided profile.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::config::ResolveOptions;
    ///
    /// let options = ResolveOptions::new().with_profile("TSD");
    /// assert_eq!(options.profile_override.as_deref(), Some("tsd"));
    /// ```
    pub fn with_profile(mut self, profile: &str) -> Self {
        self.profile_override = Some(normalize_identifier(profile));
        self
    }

    /// Resolves values for the provided terminal selector.
    pub fn with_terminal(mut self, terminal: &str) -> Self {
        self.terminal = Some(normalize_identifier(terminal));
        self
    }

    /// Replaces the optional normalized terminal selector.
    pub fn with_terminal_override(mut self, terminal: Option<String>) -> Self {
        self.terminal = terminal
            .map(|value| normalize_identifier(&value))
            .filter(|value| !value.is_empty());
        self
    }
}

/// Fully resolved value together with selection metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedValue {
    /// Value before schema adaptation or interpolation.
    pub raw_value: ConfigValue,
    /// Final runtime value after adaptation and interpolation.
    pub value: ConfigValue,
    /// Source layer that contributed the selected value.
    pub source: ConfigSource,
    /// Scope of the selected entry.
    pub scope: Scope,
    /// External origin label for the selected entry, if tracked.
    pub origin: Option<String>,
}

/// Candidate entry considered while explaining a key.
#[derive(Debug, Clone, PartialEq)]
pub struct ExplainCandidate {
    /// Zero-based index of the entry within its layer.
    pub entry_index: usize,
    /// Candidate value before final selection.
    pub value: ConfigValue,
    /// Scope attached to the candidate entry.
    pub scope: Scope,
    /// External origin label for the candidate entry, if tracked.
    pub origin: Option<String>,
    /// Selection rank used by resolution, if one was assigned.
    pub rank: Option<u8>,
    /// Whether this candidate won selection within its layer.
    pub selected_in_layer: bool,
}

/// Per-layer explanation for a resolved or bootstrap key.
#[derive(Debug, Clone, PartialEq)]
pub struct ExplainLayer {
    /// Source represented by this explanation layer.
    pub source: ConfigSource,
    /// Index of the selected candidate within `candidates`, if any.
    pub selected_entry_index: Option<usize>,
    /// Candidate entries contributed by the layer.
    pub candidates: Vec<ExplainCandidate>,
}

/// Single placeholder expansion step captured by `config explain`.
#[derive(Debug, Clone, PartialEq)]
pub struct ExplainInterpolationStep {
    /// Placeholder name referenced by the template.
    pub placeholder: String,
    /// Placeholder value before schema adaptation or interpolation.
    pub raw_value: ConfigValue,
    /// Placeholder value after schema adaptation and interpolation.
    pub value: ConfigValue,
    /// Source layer that provided the placeholder value.
    pub source: ConfigSource,
    /// Scope of the entry that supplied the placeholder.
    pub scope: Scope,
    /// External origin label for the placeholder entry, if tracked.
    pub origin: Option<String>,
}

/// Interpolation trace for a resolved string value.
#[derive(Debug, Clone, PartialEq)]
pub struct ExplainInterpolation {
    /// Original string template before placeholder substitution.
    pub template: String,
    /// Placeholder expansion steps applied to the template.
    pub steps: Vec<ExplainInterpolationStep>,
}

/// Source used to determine the active profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveProfileSource {
    /// The active profile came from an explicit override.
    Override,
    /// The active profile came from `profile.default`.
    DefaultProfile,
}

impl ActiveProfileSource {
    /// Returns the stable string label used in explain output.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Override => "override",
            Self::DefaultProfile => "profile.default",
        }
    }
}

/// Human-readable explanation of runtime resolution for a single key.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigExplain {
    /// Canonical key being explained.
    pub key: String,
    /// Profile used during resolution.
    pub active_profile: String,
    /// Source used to determine `active_profile`.
    pub active_profile_source: ActiveProfileSource,
    /// Terminal selector used during resolution, if any.
    pub terminal: Option<String>,
    /// Profiles discovered across the evaluated layers.
    pub known_profiles: BTreeSet<String>,
    /// Per-layer candidate and selection details.
    pub layers: Vec<ExplainLayer>,
    /// Final resolved entry, if the key resolved successfully.
    pub final_entry: Option<ResolvedValue>,
    /// Interpolation trace for string results, if interpolation occurred.
    pub interpolation: Option<ExplainInterpolation>,
}

/// Human-readable explanation of bootstrap resolution for a single key.
#[derive(Debug, Clone, PartialEq)]
pub struct BootstrapConfigExplain {
    /// Canonical key being explained.
    pub key: String,
    /// Profile used during bootstrap resolution.
    pub active_profile: String,
    /// Source used to determine `active_profile`.
    pub active_profile_source: ActiveProfileSource,
    /// Terminal selector used during bootstrap resolution, if any.
    pub terminal: Option<String>,
    /// Profiles discovered across the evaluated layers.
    pub known_profiles: BTreeSet<String>,
    /// Per-layer candidate and selection details.
    pub layers: Vec<ExplainLayer>,
    /// Final bootstrap-resolved entry, if one was selected.
    pub final_entry: Option<ResolvedValue>,
}

/// Final resolved configuration view used at runtime.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedConfig {
    pub(crate) active_profile: String,
    pub(crate) terminal: Option<String>,
    pub(crate) known_profiles: BTreeSet<String>,
    pub(crate) values: BTreeMap<String, ResolvedValue>,
    pub(crate) aliases: BTreeMap<String, ResolvedValue>,
}

impl ResolvedConfig {
    /// Returns the profile selected for resolution.
    pub fn active_profile(&self) -> &str {
        &self.active_profile
    }

    /// Returns the terminal selector used during resolution, if any.
    pub fn terminal(&self) -> Option<&str> {
        self.terminal.as_deref()
    }

    /// Returns the set of profiles discovered across config layers.
    pub fn known_profiles(&self) -> &BTreeSet<String> {
        &self.known_profiles
    }

    /// Returns all resolved runtime-visible values.
    pub fn values(&self) -> &BTreeMap<String, ResolvedValue> {
        &self.values
    }

    /// Returns resolved alias entries excluded from normal runtime values.
    pub fn aliases(&self) -> &BTreeMap<String, ResolvedValue> {
        &self.aliases
    }

    /// Returns the resolved value for a key.
    pub fn get(&self, key: &str) -> Option<&ConfigValue> {
        self.values.get(key).map(|entry| &entry.value)
    }

    /// Returns the resolved string value for a key.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::config::{ConfigLayer, ConfigResolver, ResolveOptions};
    ///
    /// let mut defaults = ConfigLayer::default();
    /// defaults.set("profile.default", "default");
    /// defaults.set("theme.name", "dracula");
    ///
    /// let mut resolver = ConfigResolver::default();
    /// resolver.set_defaults(defaults);
    /// let resolved = resolver.resolve(ResolveOptions::default()).unwrap();
    ///
    /// assert_eq!(resolved.get_string("theme.name"), Some("dracula"));
    /// ```
    pub fn get_string(&self, key: &str) -> Option<&str> {
        match self.get(key).map(ConfigValue::reveal) {
            Some(ConfigValue::String(value)) => Some(value),
            _ => None,
        }
    }

    /// Returns the resolved boolean value for a key.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::config::{ConfigLayer, ConfigResolver, ResolveOptions};
    ///
    /// let mut defaults = ConfigLayer::default();
    /// defaults.set("profile.default", "default");
    /// defaults.set("repl.history.enabled", true);
    ///
    /// let mut resolver = ConfigResolver::default();
    /// resolver.set_defaults(defaults);
    /// let resolved = resolver.resolve(ResolveOptions::default()).unwrap();
    ///
    /// assert_eq!(resolved.get_bool("repl.history.enabled"), Some(true));
    /// ```
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        match self.get(key).map(ConfigValue::reveal) {
            Some(ConfigValue::Bool(value)) => Some(*value),
            _ => None,
        }
    }

    /// Returns the resolved string list for a key.
    ///
    /// If the resolved value is a scalar string instead of a list, it is
    /// promoted to a single-element vector.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::config::{ConfigLayer, ConfigResolver, ResolveOptions};
    ///
    /// let mut defaults = ConfigLayer::default();
    /// defaults.set("profile.default", "default");
    /// defaults.set("theme.path", vec!["/tmp/themes".to_string()]);
    ///
    /// let mut resolver = ConfigResolver::default();
    /// resolver.set_defaults(defaults);
    /// let resolved = resolver.resolve(ResolveOptions::default()).unwrap();
    ///
    /// assert_eq!(
    ///     resolved.get_string_list("theme.path"),
    ///     Some(vec!["/tmp/themes".to_string()])
    /// );
    ///
    /// let mut defaults = ConfigLayer::default();
    /// defaults.set("profile.default", "default");
    /// defaults.set("theme.path", "/tmp/themes");
    ///
    /// let mut resolver = ConfigResolver::default();
    /// resolver.set_defaults(defaults);
    /// let resolved = resolver.resolve(ResolveOptions::default()).unwrap();
    ///
    /// assert_eq!(
    ///     resolved.get_string_list("theme.path"),
    ///     Some(vec!["/tmp/themes".to_string()])
    /// );
    /// ```
    pub fn get_string_list(&self, key: &str) -> Option<Vec<String>> {
        match self.get(key).map(ConfigValue::reveal) {
            Some(ConfigValue::List(values)) => Some(
                values
                    .iter()
                    .filter_map(|value| match value {
                        ConfigValue::String(text) => Some(text.clone()),
                        ConfigValue::Secret(secret) => match secret.expose() {
                            ConfigValue::String(text) => Some(text.clone()),
                            _ => None,
                        },
                        _ => None,
                    })
                    .collect(),
            ),
            Some(ConfigValue::String(value)) => Some(vec![value.clone()]),
            Some(ConfigValue::Secret(secret)) => match secret.expose() {
                ConfigValue::String(value) => Some(vec![value.clone()]),
                _ => None,
            },
            _ => None,
        }
    }

    /// Returns the full resolved entry for a runtime-visible key.
    pub fn get_value_entry(&self, key: &str) -> Option<&ResolvedValue> {
        self.values.get(key)
    }

    /// Returns the resolved alias entry for a key.
    pub fn get_alias_entry(&self, key: &str) -> Option<&ResolvedValue> {
        let normalized = if key.trim().to_ascii_lowercase().starts_with("alias.") {
            key.trim().to_ascii_lowercase()
        } else {
            format!("alias.{}", key.trim().to_ascii_lowercase())
        };
        self.aliases.get(&normalized)
    }
}

fn flatten_table(
    layer: &mut ConfigLayer,
    table: &toml::value::Table,
    prefix: &str,
    scope: &Scope,
) -> Result<(), ConfigError> {
    for (key, value) in table {
        let full_key = if prefix.is_empty() {
            key.to_string()
        } else {
            format!("{prefix}.{key}")
        };

        flatten_key_value(layer, &full_key, value, scope)?;
    }

    Ok(())
}

fn flatten_key_value(
    layer: &mut ConfigLayer,
    key: &str,
    value: &toml::Value,
    scope: &Scope,
) -> Result<(), ConfigError> {
    match value {
        toml::Value::Table(table) => flatten_table(layer, table, key, scope),
        _ => {
            let converted = ConfigValue::from_toml(key, value)?;
            builtin_config_schema().validate_writable_key(key)?;
            validate_key_scope(key, scope)?;
            validate_bootstrap_value(key, &converted)?;
            layer.insert(key.to_string(), converted, scope.clone());
            Ok(())
        }
    }
}

/// Looks up bootstrap-time metadata for a canonical config key.
pub fn bootstrap_key_spec(key: &str) -> Option<BootstrapKeySpec> {
    builtin_config_schema().bootstrap_key_spec(key)
}

/// Reports whether `key` is consumed during bootstrap but not exposed as a
/// normal runtime-resolved config key.
pub fn is_bootstrap_only_key(key: &str) -> bool {
    bootstrap_key_spec(key).is_some_and(|spec| !spec.runtime_visible)
}

/// Reports whether `key` belongs to the `alias.*` namespace.
///
/// # Examples
///
/// ```
/// use osp_cli::config::is_alias_key;
///
/// assert!(is_alias_key("alias.prod"));
/// assert!(is_alias_key(" Alias.User "));
/// assert!(!is_alias_key("ui.format"));
/// ```
pub fn is_alias_key(key: &str) -> bool {
    key.trim().to_ascii_lowercase().starts_with("alias.")
}

/// Validates that a key can be written in the provided scope.
pub fn validate_key_scope(key: &str, scope: &Scope) -> Result<(), ConfigError> {
    builtin_config_schema().validate_key_scope(key, scope)
}

/// Validates bootstrap-only value constraints for a key.
pub fn validate_bootstrap_value(key: &str, value: &ConfigValue) -> Result<(), ConfigError> {
    builtin_config_schema().validate_bootstrap_value(key, value)
}

fn adapt_value_for_schema(
    key: &str,
    value: &ConfigValue,
    schema: &SchemaEntry,
) -> Result<ConfigValue, ConfigError> {
    let (is_secret, value) = match value {
        ConfigValue::Secret(secret) => (true, secret.expose()),
        other => (false, other),
    };

    let adapted = match schema.value_type {
        SchemaValueType::String => match value {
            ConfigValue::String(value) => ConfigValue::String(value.clone()),
            other => {
                return Err(ConfigError::InvalidValueType {
                    key: key.to_string(),
                    expected: SchemaValueType::String,
                    actual: value_type_name(other).to_string(),
                });
            }
        },
        SchemaValueType::Bool => match value {
            ConfigValue::Bool(value) => ConfigValue::Bool(*value),
            ConfigValue::String(value) => {
                ConfigValue::Bool(parse_bool(value).ok_or_else(|| {
                    ConfigError::InvalidValueType {
                        key: key.to_string(),
                        expected: SchemaValueType::Bool,
                        actual: "string".to_string(),
                    }
                })?)
            }
            other => {
                return Err(ConfigError::InvalidValueType {
                    key: key.to_string(),
                    expected: SchemaValueType::Bool,
                    actual: value_type_name(other).to_string(),
                });
            }
        },
        SchemaValueType::Integer => match value {
            ConfigValue::Integer(value) => ConfigValue::Integer(*value),
            ConfigValue::String(value) => {
                let parsed =
                    value
                        .trim()
                        .parse::<i64>()
                        .map_err(|_| ConfigError::InvalidValueType {
                            key: key.to_string(),
                            expected: SchemaValueType::Integer,
                            actual: "string".to_string(),
                        })?;
                ConfigValue::Integer(parsed)
            }
            other => {
                return Err(ConfigError::InvalidValueType {
                    key: key.to_string(),
                    expected: SchemaValueType::Integer,
                    actual: value_type_name(other).to_string(),
                });
            }
        },
        SchemaValueType::Float => match value {
            ConfigValue::Float(value) => ConfigValue::Float(*value),
            ConfigValue::Integer(value) => ConfigValue::Float(*value as f64),
            ConfigValue::String(value) => {
                let parsed =
                    value
                        .trim()
                        .parse::<f64>()
                        .map_err(|_| ConfigError::InvalidValueType {
                            key: key.to_string(),
                            expected: SchemaValueType::Float,
                            actual: "string".to_string(),
                        })?;
                ConfigValue::Float(parsed)
            }
            other => {
                return Err(ConfigError::InvalidValueType {
                    key: key.to_string(),
                    expected: SchemaValueType::Float,
                    actual: value_type_name(other).to_string(),
                });
            }
        },
        SchemaValueType::StringList => match value {
            ConfigValue::List(values) => {
                let mut out = Vec::with_capacity(values.len());
                for value in values {
                    match value {
                        ConfigValue::String(value) => out.push(ConfigValue::String(value.clone())),
                        ConfigValue::Secret(secret) => match secret.expose() {
                            ConfigValue::String(value) => {
                                out.push(ConfigValue::String(value.clone()))
                            }
                            other => {
                                return Err(ConfigError::InvalidValueType {
                                    key: key.to_string(),
                                    expected: SchemaValueType::StringList,
                                    actual: value_type_name(other).to_string(),
                                });
                            }
                        },
                        other => {
                            return Err(ConfigError::InvalidValueType {
                                key: key.to_string(),
                                expected: SchemaValueType::StringList,
                                actual: value_type_name(other).to_string(),
                            });
                        }
                    }
                }
                ConfigValue::List(out)
            }
            ConfigValue::String(value) => {
                let items = parse_string_list(value);
                ConfigValue::List(items.into_iter().map(ConfigValue::String).collect())
            }
            ConfigValue::Secret(secret) => match secret.expose() {
                ConfigValue::String(value) => {
                    let items = parse_string_list(value);
                    ConfigValue::List(items.into_iter().map(ConfigValue::String).collect())
                }
                other => {
                    return Err(ConfigError::InvalidValueType {
                        key: key.to_string(),
                        expected: SchemaValueType::StringList,
                        actual: value_type_name(other).to_string(),
                    });
                }
            },
            other => {
                return Err(ConfigError::InvalidValueType {
                    key: key.to_string(),
                    expected: SchemaValueType::StringList,
                    actual: value_type_name(other).to_string(),
                });
            }
        },
    };

    let adapted = if is_secret {
        adapted.into_secret()
    } else {
        adapted
    };

    if let Some(allowed_values) = &schema.allowed_values
        && let ConfigValue::String(value) = adapted.reveal()
    {
        let normalized = value.to_ascii_lowercase();
        if !allowed_values.contains(&normalized) {
            return Err(ConfigError::InvalidEnumValue {
                key: key.to_string(),
                value: value.clone(),
                allowed: allowed_values.clone(),
            });
        }
    }

    Ok(adapted)
}

fn adapt_dynamic_value_for_schema(
    key: &str,
    value: &ConfigValue,
    kind: DynamicSchemaKeyKind,
) -> Result<ConfigValue, ConfigError> {
    let adapted = match kind {
        DynamicSchemaKeyKind::PluginCommandState | DynamicSchemaKeyKind::PluginCommandProvider => {
            adapt_value_for_schema(key, value, &SchemaEntry::string())?
        }
    };

    if matches!(kind, DynamicSchemaKeyKind::PluginCommandState) {
        validate_allowed_values(key, &adapted, Some(&["enabled", "disabled"]))?;
    }

    Ok(adapted)
}

fn validate_allowed_values(
    key: &str,
    value: &ConfigValue,
    allowed: Option<&[&str]>,
) -> Result<(), ConfigError> {
    let Some(allowed) = allowed else {
        return Ok(());
    };
    if let ConfigValue::String(current) = value {
        let normalized = current.to_ascii_lowercase();
        if !allowed.iter().any(|candidate| *candidate == normalized) {
            return Err(ConfigError::InvalidEnumValue {
                key: key.to_string(),
                value: current.clone(),
                allowed: allowed.iter().map(|value| (*value).to_string()).collect(),
            });
        }
    }
    Ok(())
}

fn dynamic_schema_key_kind(key: &str) -> Option<DynamicSchemaKeyKind> {
    let normalized = key.trim().to_ascii_lowercase();
    let remainder = normalized.strip_prefix("plugins.")?;
    let (command, field) = remainder.rsplit_once('.')?;
    if command.trim().is_empty() {
        return None;
    }
    match field {
        "state" => Some(DynamicSchemaKeyKind::PluginCommandState),
        "provider" => Some(DynamicSchemaKeyKind::PluginCommandProvider),
        _ => None,
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn parse_string_list(value: &str) -> Vec<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let inner = trimmed
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(trimmed);

    inner
        .split(',')
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .or_else(|| {
                    value
                        .strip_prefix('\'')
                        .and_then(|value| value.strip_suffix('\''))
                })
                .unwrap_or(value)
                .to_string()
        })
        .collect()
}

fn value_type_name(value: &ConfigValue) -> &'static str {
    match value.reveal() {
        ConfigValue::String(_) => "string",
        ConfigValue::Bool(_) => "bool",
        ConfigValue::Integer(_) => "integer",
        ConfigValue::Float(_) => "float",
        ConfigValue::List(_) => "list",
        ConfigValue::Secret(_) => "string",
    }
}

pub(crate) fn parse_env_key(key: &str) -> Result<EnvKeySpec, ConfigError> {
    let Some(raw) = key.strip_prefix("OSP__") else {
        return Err(ConfigError::InvalidEnvOverride {
            key: key.to_string(),
            reason: "missing OSP__ prefix".to_string(),
        });
    };

    let parts = raw
        .split("__")
        .filter(|part| !part.is_empty())
        .collect::<Vec<&str>>();

    if parts.is_empty() {
        return Err(ConfigError::InvalidEnvOverride {
            key: key.to_string(),
            reason: "missing key path".to_string(),
        });
    }

    let mut cursor = 0usize;
    let mut terminal: Option<String> = None;
    let mut profile: Option<String> = None;

    while cursor < parts.len() {
        let part = parts[cursor];
        if part.eq_ignore_ascii_case("TERM") {
            if terminal.is_some() {
                return Err(ConfigError::InvalidEnvOverride {
                    key: key.to_string(),
                    reason: "TERM scope specified more than once".to_string(),
                });
            }
            let term = parts
                .get(cursor + 1)
                .ok_or_else(|| ConfigError::InvalidEnvOverride {
                    key: key.to_string(),
                    reason: "TERM requires a terminal name".to_string(),
                })?;
            terminal = Some(normalize_identifier(term));
            cursor += 2;
            continue;
        }

        if part.eq_ignore_ascii_case("PROFILE") {
            // `profile.default` is a bootstrap key, not a profile scope. Keep
            // the exception isolated here so the scope parser stays readable.
            if remaining_parts_are_bootstrap_profile_default(&parts[cursor..]) {
                break;
            }
            if profile.is_some() {
                return Err(ConfigError::InvalidEnvOverride {
                    key: key.to_string(),
                    reason: "PROFILE scope specified more than once".to_string(),
                });
            }
            let profile_name =
                parts
                    .get(cursor + 1)
                    .ok_or_else(|| ConfigError::InvalidEnvOverride {
                        key: key.to_string(),
                        reason: "PROFILE requires a profile name".to_string(),
                    })?;
            profile = Some(normalize_identifier(profile_name));
            cursor += 2;
            continue;
        }

        break;
    }

    let key_parts = &parts[cursor..];
    if key_parts.is_empty() {
        return Err(ConfigError::InvalidEnvOverride {
            key: key.to_string(),
            reason: "missing final config key".to_string(),
        });
    }

    let dotted_key = key_parts
        .iter()
        .map(|part| part.to_ascii_lowercase())
        .collect::<Vec<String>>()
        .join(".");

    Ok(EnvKeySpec {
        key: dotted_key,
        scope: Scope { profile, terminal },
    })
}

fn remaining_parts_are_bootstrap_profile_default(parts: &[&str]) -> bool {
    matches!(parts, [profile, default]
        if profile.eq_ignore_ascii_case("PROFILE")
            && default.eq_ignore_ascii_case("DEFAULT"))
}

pub(crate) fn normalize_scope(scope: Scope) -> Scope {
    Scope {
        profile: scope
            .profile
            .as_deref()
            .map(normalize_identifier)
            .filter(|value| !value.is_empty()),
        terminal: scope
            .terminal
            .as_deref()
            .map(normalize_identifier)
            .filter(|value| !value.is_empty()),
    }
}

pub(crate) fn normalize_identifier(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests;
