use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};
use std::sync::OnceLock;

use crate::config::ConfigError;

#[derive(Debug, Clone, PartialEq)]
pub struct TomlEditResult {
    pub previous: Option<ConfigValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ConfigSource {
    BuiltinDefaults,
    PresentationDefaults,
    ConfigFile,
    Secrets,
    Environment,
    Cli,
    Session,
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

#[derive(Debug, Clone, PartialEq)]
pub enum ConfigValue {
    String(String),
    Bool(bool),
    Integer(i64),
    Float(f64),
    List(Vec<ConfigValue>),
    Secret(SecretValue),
}

impl ConfigValue {
    pub fn is_secret(&self) -> bool {
        matches!(self, ConfigValue::Secret(_))
    }

    pub fn reveal(&self) -> &ConfigValue {
        match self {
            ConfigValue::Secret(secret) => secret.expose(),
            other => other,
        }
    }

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

#[derive(Clone, PartialEq)]
pub struct SecretValue(Box<ConfigValue>);

impl SecretValue {
    pub fn new(value: ConfigValue) -> Self {
        Self(Box::new(value))
    }

    pub fn expose(&self) -> &ConfigValue {
        &self.0
    }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaValueType {
    String,
    Bool,
    Integer,
    Float,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapPhase {
    Path,
    Profile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapScopeRule {
    GlobalOnly,
    GlobalOrTerminal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapValueRule {
    NonEmptyString,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapKeySpec {
    pub key: &'static str,
    pub phase: BootstrapPhase,
    pub runtime_visible: bool,
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

#[derive(Debug, Clone)]
pub struct SchemaEntry {
    canonical_key: Option<&'static str>,
    value_type: SchemaValueType,
    required: bool,
    allowed_values: Option<Vec<String>>,
    runtime_visible: bool,
    bootstrap_phase: Option<BootstrapPhase>,
    bootstrap_scope_rule: Option<BootstrapScopeRule>,
    bootstrap_value_rule: Option<BootstrapValueRule>,
}

impl SchemaEntry {
    pub fn string() -> Self {
        Self {
            canonical_key: None,
            value_type: SchemaValueType::String,
            required: false,
            allowed_values: None,
            runtime_visible: true,
            bootstrap_phase: None,
            bootstrap_scope_rule: None,
            bootstrap_value_rule: None,
        }
    }

    pub fn boolean() -> Self {
        Self {
            canonical_key: None,
            value_type: SchemaValueType::Bool,
            required: false,
            allowed_values: None,
            runtime_visible: true,
            bootstrap_phase: None,
            bootstrap_scope_rule: None,
            bootstrap_value_rule: None,
        }
    }

    pub fn integer() -> Self {
        Self {
            canonical_key: None,
            value_type: SchemaValueType::Integer,
            required: false,
            allowed_values: None,
            runtime_visible: true,
            bootstrap_phase: None,
            bootstrap_scope_rule: None,
            bootstrap_value_rule: None,
        }
    }

    pub fn float() -> Self {
        Self {
            canonical_key: None,
            value_type: SchemaValueType::Float,
            required: false,
            allowed_values: None,
            runtime_visible: true,
            bootstrap_phase: None,
            bootstrap_scope_rule: None,
            bootstrap_value_rule: None,
        }
    }

    pub fn string_list() -> Self {
        Self {
            canonical_key: None,
            value_type: SchemaValueType::StringList,
            required: false,
            allowed_values: None,
            runtime_visible: true,
            bootstrap_phase: None,
            bootstrap_scope_rule: None,
            bootstrap_value_rule: None,
        }
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    pub fn bootstrap_only(mut self, phase: BootstrapPhase, scope_rule: BootstrapScopeRule) -> Self {
        self.runtime_visible = false;
        self.bootstrap_phase = Some(phase);
        self.bootstrap_scope_rule = Some(scope_rule);
        self
    }

    pub fn with_bootstrap_value_rule(mut self, rule: BootstrapValueRule) -> Self {
        self.bootstrap_value_rule = Some(rule);
        self
    }

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

    pub fn value_type(&self) -> SchemaValueType {
        self.value_type
    }

    pub fn allowed_values(&self) -> Option<&[String]> {
        self.allowed_values.as_deref()
    }

    pub fn runtime_visible(&self) -> bool {
        self.runtime_visible
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

#[derive(Debug, Clone)]
pub struct ConfigSchema {
    entries: BTreeMap<String, SchemaEntry>,
    allow_extensions_namespace: bool,
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
        schema.insert("profile.active", SchemaEntry::string().required());
        schema.insert("theme.name", SchemaEntry::string());
        schema.insert("theme.path", SchemaEntry::string_list());
        schema.insert("user.name", SchemaEntry::string());
        schema.insert("user.display_name", SchemaEntry::string());
        schema.insert("user.full_name", SchemaEntry::string());
        schema.insert("domain", SchemaEntry::string());

        schema.insert(
            "ui.format",
            SchemaEntry::string()
                .with_allowed_values(["auto", "json", "table", "md", "mreg", "value"]),
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
            "ui.help.layout",
            SchemaEntry::string().with_allowed_values(["full", "compact", "minimal"]),
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
            "ui.table.overflow",
            SchemaEntry::string().with_allowed_values([
                "clip", "hidden", "crop", "ellipsis", "truncate", "wrap", "none", "visible",
            ]),
        );
        schema.insert(
            "ui.table.border",
            SchemaEntry::string().with_allowed_values(["none", "square", "round"]),
        );
        schema.insert("ui.short_list_max", SchemaEntry::integer());
        schema.insert("ui.medium_list_max", SchemaEntry::integer());
        schema.insert("ui.grid_padding", SchemaEntry::integer());
        schema.insert("ui.grid_columns", SchemaEntry::integer());
        schema.insert("ui.column_weight", SchemaEntry::integer());
        schema.insert("ui.mreg.stack_min_col_width", SchemaEntry::integer());
        schema.insert("ui.mreg.stack_overflow_ratio", SchemaEntry::integer());
        schema.insert(
            "ui.verbosity.level",
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
        schema.insert("repl.history.path", SchemaEntry::string());
        schema.insert("repl.history.max_entries", SchemaEntry::integer());
        schema.insert("repl.history.enabled", SchemaEntry::boolean());
        schema.insert("repl.history.dedupe", SchemaEntry::boolean());
        schema.insert("repl.history.profile_scoped", SchemaEntry::boolean());
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
    pub fn insert(&mut self, key: &'static str, entry: SchemaEntry) {
        self.entries
            .insert(key.to_string(), entry.with_canonical_key(key));
    }

    pub fn set_allow_extensions_namespace(&mut self, value: bool) {
        self.allow_extensions_namespace = value;
    }

    pub fn is_known_key(&self, key: &str) -> bool {
        self.entries.contains_key(key) || self.is_extension_key(key) || self.is_alias_key(key)
    }

    pub fn is_runtime_visible_key(&self, key: &str) -> bool {
        self.entries
            .get(key)
            .is_some_and(SchemaEntry::runtime_visible)
            || self.is_extension_key(key)
    }

    pub fn bootstrap_key_spec(&self, key: &str) -> Option<BootstrapKeySpec> {
        let normalized = key.trim().to_ascii_lowercase();
        self.entries
            .get(&normalized)
            .and_then(SchemaEntry::bootstrap_spec)
    }

    pub fn entries(&self) -> impl Iterator<Item = (&str, &SchemaEntry)> {
        self.entries
            .iter()
            .map(|(key, entry)| (key.as_str(), entry))
    }

    pub fn expected_type(&self, key: &str) -> Option<SchemaValueType> {
        self.entries.get(key).map(|entry| entry.value_type)
    }

    pub fn parse_input_value(&self, key: &str, raw: &str) -> Result<ConfigValue, ConfigError> {
        if !self.is_known_key(key) {
            return Err(ConfigError::UnknownConfigKeys {
                keys: vec![key.to_string()],
            });
        }

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

        if let Some(entry) = self.entries.get(key)
            && let Some(allowed) = &entry.allowed_values
            && let ConfigValue::String(current) = &value
        {
            let normalized = current.to_ascii_lowercase();
            if !allowed.contains(&normalized) {
                return Err(ConfigError::InvalidEnumValue {
                    key: key.to_string(),
                    value: current.clone(),
                    allowed: allowed.clone(),
                });
            }
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    pub profile: Option<String>,
    pub terminal: Option<String>,
}

impl Scope {
    pub fn global() -> Self {
        Self::default()
    }

    pub fn profile(profile: &str) -> Self {
        Self {
            profile: Some(normalize_identifier(profile)),
            terminal: None,
        }
    }

    pub fn terminal(terminal: &str) -> Self {
        Self {
            profile: None,
            terminal: Some(normalize_identifier(terminal)),
        }
    }

    pub fn profile_terminal(profile: &str, terminal: &str) -> Self {
        Self {
            profile: Some(normalize_identifier(profile)),
            terminal: Some(normalize_identifier(terminal)),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LayerEntry {
    pub key: String,
    pub value: ConfigValue,
    pub scope: Scope,
    pub origin: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ConfigLayer {
    pub(crate) entries: Vec<LayerEntry>,
}

impl ConfigLayer {
    pub fn entries(&self) -> &[LayerEntry] {
        &self.entries
    }

    pub fn set<K, V>(&mut self, key: K, value: V)
    where
        K: Into<String>,
        V: Into<ConfigValue>,
    {
        self.insert(key, value, Scope::global());
    }

    pub fn set_for_profile<K, V>(&mut self, profile: &str, key: K, value: V)
    where
        K: Into<String>,
        V: Into<ConfigValue>,
    {
        self.insert(key, value, Scope::profile(profile));
    }

    pub fn set_for_terminal<K, V>(&mut self, terminal: &str, key: K, value: V)
    where
        K: Into<String>,
        V: Into<ConfigValue>,
    {
        self.insert(key, value, Scope::terminal(terminal));
    }

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

    pub fn mark_all_secret(&mut self) {
        for entry in &mut self.entries {
            if !entry.value.is_secret() {
                entry.value = entry.value.clone().into_secret();
            }
        }
    }

    pub fn remove_scoped(&mut self, key: &str, scope: &Scope) -> Option<ConfigValue> {
        let normalized_scope = normalize_scope(scope.clone());
        let index = self
            .entries
            .iter()
            .rposition(|entry| entry.key == key && entry.scope == normalized_scope)?;
        Some(self.entries.remove(index).value)
    }

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
            validate_key_scope(&spec.key, &spec.scope)?;
            let converted = ConfigValue::String(value.as_ref().to_string());
            validate_bootstrap_value(&spec.key, &converted)?;
            layer.insert_with_origin(spec.key, converted, spec.scope, Some(key.to_string()));
        }

        Ok(layer)
    }

    pub(crate) fn validate_entries(&self) -> Result<(), ConfigError> {
        for entry in &self.entries {
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

#[derive(Debug, Clone, Default)]
pub struct ResolveOptions {
    pub profile_override: Option<String>,
    pub terminal: Option<String>,
}

impl ResolveOptions {
    pub fn with_profile(mut self, profile: &str) -> Self {
        self.profile_override = Some(normalize_identifier(profile));
        self
    }

    pub fn with_terminal(mut self, terminal: &str) -> Self {
        self.terminal = Some(normalize_identifier(terminal));
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedValue {
    pub raw_value: ConfigValue,
    pub value: ConfigValue,
    pub source: ConfigSource,
    pub scope: Scope,
    pub origin: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExplainCandidate {
    pub entry_index: usize,
    pub value: ConfigValue,
    pub scope: Scope,
    pub origin: Option<String>,
    pub rank: Option<u8>,
    pub selected_in_layer: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExplainLayer {
    pub source: ConfigSource,
    pub selected_entry_index: Option<usize>,
    pub candidates: Vec<ExplainCandidate>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExplainInterpolationStep {
    pub placeholder: String,
    pub raw_value: ConfigValue,
    pub value: ConfigValue,
    pub source: ConfigSource,
    pub scope: Scope,
    pub origin: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExplainInterpolation {
    pub template: String,
    pub steps: Vec<ExplainInterpolationStep>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveProfileSource {
    Override,
    DefaultProfile,
}

impl ActiveProfileSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Override => "override",
            Self::DefaultProfile => "profile.default",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfigExplain {
    pub key: String,
    pub active_profile: String,
    pub active_profile_source: ActiveProfileSource,
    pub terminal: Option<String>,
    pub known_profiles: BTreeSet<String>,
    pub layers: Vec<ExplainLayer>,
    pub final_entry: Option<ResolvedValue>,
    pub interpolation: Option<ExplainInterpolation>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BootstrapConfigExplain {
    pub key: String,
    pub active_profile: String,
    pub active_profile_source: ActiveProfileSource,
    pub terminal: Option<String>,
    pub known_profiles: BTreeSet<String>,
    pub layers: Vec<ExplainLayer>,
    pub final_entry: Option<ResolvedValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedConfig {
    pub(crate) active_profile: String,
    pub(crate) terminal: Option<String>,
    pub(crate) known_profiles: BTreeSet<String>,
    pub(crate) values: BTreeMap<String, ResolvedValue>,
    pub(crate) aliases: BTreeMap<String, ResolvedValue>,
}

impl ResolvedConfig {
    pub fn active_profile(&self) -> &str {
        &self.active_profile
    }

    pub fn terminal(&self) -> Option<&str> {
        self.terminal.as_deref()
    }

    pub fn known_profiles(&self) -> &BTreeSet<String> {
        &self.known_profiles
    }

    pub fn values(&self) -> &BTreeMap<String, ResolvedValue> {
        &self.values
    }

    pub fn aliases(&self) -> &BTreeMap<String, ResolvedValue> {
        &self.aliases
    }

    pub fn get(&self, key: &str) -> Option<&ConfigValue> {
        self.values.get(key).map(|entry| &entry.value)
    }

    pub fn get_string(&self, key: &str) -> Option<&str> {
        match self.get(key).map(ConfigValue::reveal) {
            Some(ConfigValue::String(value)) => Some(value),
            _ => None,
        }
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        match self.get(key).map(ConfigValue::reveal) {
            Some(ConfigValue::Bool(value)) => Some(*value),
            _ => None,
        }
    }

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

    pub fn get_value_entry(&self, key: &str) -> Option<&ResolvedValue> {
        self.values.get(key)
    }

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
            validate_key_scope(key, scope)?;
            validate_bootstrap_value(key, &converted)?;
            layer.insert(key.to_string(), converted, scope.clone());
            Ok(())
        }
    }
}

pub fn bootstrap_key_spec(key: &str) -> Option<BootstrapKeySpec> {
    builtin_config_schema().bootstrap_key_spec(key)
}

pub fn is_bootstrap_only_key(key: &str) -> bool {
    bootstrap_key_spec(key).is_some_and(|spec| !spec.runtime_visible)
}

pub fn is_alias_key(key: &str) -> bool {
    key.trim().to_ascii_lowercase().starts_with("alias.")
}

pub fn validate_key_scope(key: &str, scope: &Scope) -> Result<(), ConfigError> {
    builtin_config_schema().validate_key_scope(key, scope)
}

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
