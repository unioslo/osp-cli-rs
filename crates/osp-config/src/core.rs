use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};

use crate::ConfigError;

#[derive(Debug, Clone, PartialEq)]
pub struct TomlSetResult {
    pub previous: Option<ConfigValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ConfigSource {
    BuiltinDefaults,
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
}

impl ConfigValue {
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
        match self {
            ConfigValue::String(value) => Ok(value.clone()),
            ConfigValue::Bool(value) => Ok(value.to_string()),
            ConfigValue::Integer(value) => Ok(value.to_string()),
            ConfigValue::Float(value) => Ok(value.to_string()),
            ConfigValue::List(_) => Err(ConfigError::NonScalarPlaceholder {
                key: key.to_string(),
                placeholder: placeholder.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaValueType {
    String,
    Bool,
    Integer,
    Float,
}

impl Display for SchemaValueType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            SchemaValueType::String => "string",
            SchemaValueType::Bool => "bool",
            SchemaValueType::Integer => "integer",
            SchemaValueType::Float => "float",
        };
        write!(f, "{value}")
    }
}

#[derive(Debug, Clone)]
pub struct SchemaEntry {
    value_type: SchemaValueType,
    required: bool,
    allowed_values: Option<Vec<String>>,
}

impl SchemaEntry {
    pub fn string() -> Self {
        Self {
            value_type: SchemaValueType::String,
            required: false,
            allowed_values: None,
        }
    }

    pub fn boolean() -> Self {
        Self {
            value_type: SchemaValueType::Bool,
            required: false,
            allowed_values: None,
        }
    }

    pub fn integer() -> Self {
        Self {
            value_type: SchemaValueType::Integer,
            required: false,
            allowed_values: None,
        }
    }

    pub fn float() -> Self {
        Self {
            value_type: SchemaValueType::Float,
            required: false,
            allowed_values: None,
        }
    }

    pub fn required(mut self) -> Self {
        self.required = true;
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
}

#[derive(Debug, Clone)]
pub struct ConfigSchema {
    entries: BTreeMap<String, SchemaEntry>,
    allow_extensions_namespace: bool,
}

impl Default for ConfigSchema {
    fn default() -> Self {
        let mut schema = Self {
            entries: BTreeMap::new(),
            allow_extensions_namespace: true,
        };

        schema.insert("profile.default", SchemaEntry::string().required());
        schema.insert("profile.active", SchemaEntry::string().required());
        schema.insert("context", SchemaEntry::string());
        schema.insert("theme.name", SchemaEntry::string());
        schema.insert("user.name", SchemaEntry::string());
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
            "ui.messages.format",
            SchemaEntry::string().with_allowed_values(["rules", "groups", "boxes"]),
        );
        schema.insert("ui.short_list_max", SchemaEntry::integer());
        schema.insert("ui.medium_list_max", SchemaEntry::integer());
        schema.insert("ui.grid_padding", SchemaEntry::integer());
        schema.insert("ui.grid_columns", SchemaEntry::integer());
        schema.insert("ui.column_weight", SchemaEntry::integer());
        schema.insert(
            "ui.verbosity.level",
            SchemaEntry::string()
                .with_allowed_values(["error", "warning", "success", "info", "trace"]),
        );
        schema.insert("ui.prompt", SchemaEntry::string());
        schema.insert("ui.prompt.secrets", SchemaEntry::boolean());
        schema.insert("repl.prompt", SchemaEntry::string());
        schema.insert("repl.simple_prompt", SchemaEntry::boolean());
        schema.insert("repl.shell_indicator", SchemaEntry::string());
        schema.insert("repl.intro", SchemaEntry::boolean());
        schema.insert("repl.history.path", SchemaEntry::string());
        schema.insert("repl.history.max_entries", SchemaEntry::integer());
        schema.insert("session.cache.max_results", SchemaEntry::integer());
        schema.insert("color.prompt.text", SchemaEntry::string());
        schema.insert("color.prompt.command", SchemaEntry::string());
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
    pub fn insert(&mut self, key: &str, entry: SchemaEntry) {
        self.entries.insert(key.to_string(), entry);
    }

    pub fn set_allow_extensions_namespace(&mut self, value: bool) {
        self.allow_extensions_namespace = value;
    }

    pub fn is_known_key(&self, key: &str) -> bool {
        self.entries.contains_key(key) || self.is_extension_key(key)
    }

    pub fn expected_type(&self, key: &str) -> Option<SchemaValueType> {
        self.entries.get(key).map(|entry| entry.value_type)
    }

    pub fn parse_input_value(&self, key: &str, raw: &str) -> Result<ConfigValue, ConfigError> {
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

        if !self.is_known_key(key) {
            return Err(ConfigError::UnknownConfigKeys {
                keys: vec![key.to_string()],
            });
        }

        Ok(value)
    }

    pub(crate) fn validate_and_adapt(
        &self,
        values: &mut BTreeMap<String, ResolvedValue>,
    ) -> Result<(), ConfigError> {
        let mut unknown = Vec::new();
        for key in values.keys() {
            if self.entries.contains_key(key) || self.is_extension_key(key) {
                continue;
            }
            unknown.push(key.clone());
        }
        if !unknown.is_empty() {
            unknown.sort();
            return Err(ConfigError::UnknownConfigKeys { keys: unknown });
        }

        for (key, entry) in &self.entries {
            if entry.required && !values.contains_key(key) {
                return Err(ConfigError::MissingRequiredKey { key: key.clone() });
            }
        }

        for (key, resolved) in values.iter_mut() {
            let Some(schema_entry) = self.entries.get(key) else {
                continue;
            };
            resolved.value = adapt_value_for_schema(key, &resolved.value, schema_entry)?;
        }

        Ok(())
    }

    fn is_extension_key(&self, key: &str) -> bool {
        self.allow_extensions_namespace && key.starts_with("extensions.")
    }
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

                            for (profile, profile_table_value) in profile_tables {
                                let profile_table =
                                    profile_table_value.as_table().ok_or_else(|| {
                                        ConfigError::InvalidSection {
                                            section: format!(
                                                "terminal.{terminal}.profile.{profile}"
                                            ),
                                            expected: "table".to_string(),
                                        }
                                    })?;
                                flatten_table(
                                    &mut layer,
                                    profile_table,
                                    "",
                                    &Scope::profile_terminal(profile, terminal),
                                )?;
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
            layer.insert_with_origin(spec.key, value.as_ref(), spec.scope, Some(key.to_string()));
        }

        Ok(layer)
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

#[derive(Debug, Clone, PartialEq)]
pub struct ConfigExplain {
    pub key: String,
    pub active_profile: String,
    pub terminal: Option<String>,
    pub known_profiles: BTreeSet<String>,
    pub layers: Vec<ExplainLayer>,
    pub final_entry: Option<ResolvedValue>,
    pub interpolation: Option<ExplainInterpolation>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedConfig {
    pub(crate) active_profile: String,
    pub(crate) terminal: Option<String>,
    pub(crate) known_profiles: BTreeSet<String>,
    pub(crate) values: BTreeMap<String, ResolvedValue>,
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

    pub fn get(&self, key: &str) -> Option<&ConfigValue> {
        self.values.get(key).map(|entry| &entry.value)
    }

    pub fn get_string(&self, key: &str) -> Option<&str> {
        match self.get(key) {
            Some(ConfigValue::String(value)) => Some(value),
            _ => None,
        }
    }

    pub fn get_bool(&self, key: &str) -> Option<bool> {
        match self.get(key) {
            Some(ConfigValue::Bool(value)) => Some(*value),
            _ => None,
        }
    }

    pub fn get_value_entry(&self, key: &str) -> Option<&ResolvedValue> {
        self.values.get(key)
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
            layer.insert(key.to_string(), converted, scope.clone());
            Ok(())
        }
    }
}

fn adapt_value_for_schema(
    key: &str,
    value: &ConfigValue,
    schema: &SchemaEntry,
) -> Result<ConfigValue, ConfigError> {
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
    };

    if let Some(allowed_values) = &schema.allowed_values
        && let ConfigValue::String(value) = &adapted
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

fn value_type_name(value: &ConfigValue) -> &'static str {
    match value {
        ConfigValue::String(_) => "string",
        ConfigValue::Bool(_) => "bool",
        ConfigValue::Integer(_) => "integer",
        ConfigValue::Float(_) => "float",
        ConfigValue::List(_) => "list",
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
