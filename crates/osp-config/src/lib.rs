use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::{Display, Formatter};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub default_profile: String,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            default_profile: "default".to_string(),
        }
    }
}

impl RuntimeConfig {
    pub fn from_resolved(resolved: &ResolvedConfig) -> Self {
        let default_profile = resolved
            .get_string("profile.default")
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| resolved.active_profile().to_string());
        Self { default_profile }
    }
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
    fn from_toml(path: &str, value: &toml::Value) -> Result<Self, ConfigError> {
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

    fn as_interpolation_string(&self, key: &str, placeholder: &str) -> Result<String, ConfigError> {
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
        schema.insert("ui.messages.boxed", SchemaEntry::boolean());
        schema.insert("ui.prompt", SchemaEntry::string());
        schema.insert("ui.prompt.secrets", SchemaEntry::boolean());
        schema.insert("repl.prompt", SchemaEntry::string());
        schema.insert("repl.simple_prompt", SchemaEntry::boolean());
        schema.insert("repl.shell_indicator", SchemaEntry::string());
        schema.insert("repl.intro", SchemaEntry::boolean());
        schema.insert("color.prompt.text", SchemaEntry::string());
        schema.insert("color.prompt.command", SchemaEntry::string());

        schema.insert("base.dir", SchemaEntry::string());
        schema.insert("ldap.url", SchemaEntry::string());
        schema.insert("ldap.bind_dn", SchemaEntry::string());
        schema.insert("ldap.bind_password", SchemaEntry::string());
        schema.insert("osp.url", SchemaEntry::string());

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

    fn validate_and_adapt(
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
    entries: Vec<LayerEntry>,
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

pub trait ConfigLoader: Send + Sync {
    fn load(&self) -> Result<ConfigLayer, ConfigError>;
}

#[derive(Debug, Clone, Default)]
pub struct StaticLayerLoader {
    layer: ConfigLayer,
}

impl StaticLayerLoader {
    pub fn new(layer: ConfigLayer) -> Self {
        Self { layer }
    }
}

impl ConfigLoader for StaticLayerLoader {
    fn load(&self) -> Result<ConfigLayer, ConfigError> {
        Ok(self.layer.clone())
    }
}

#[derive(Debug, Clone)]
pub struct TomlFileLoader {
    path: PathBuf,
    missing_ok: bool,
}

impl TomlFileLoader {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            missing_ok: true,
        }
    }

    pub fn required(mut self) -> Self {
        self.missing_ok = false;
        self
    }

    pub fn optional(mut self) -> Self {
        self.missing_ok = true;
        self
    }
}

impl ConfigLoader for TomlFileLoader {
    fn load(&self) -> Result<ConfigLayer, ConfigError> {
        if !self.path.exists() {
            if self.missing_ok {
                return Ok(ConfigLayer::default());
            }
            return Err(ConfigError::FileRead {
                path: self.path.display().to_string(),
                reason: "file not found".to_string(),
            });
        }

        let raw = std::fs::read_to_string(&self.path).map_err(|err| ConfigError::FileRead {
            path: self.path.display().to_string(),
            reason: err.to_string(),
        })?;

        let mut layer = ConfigLayer::from_toml_str(&raw)
            .map_err(|err| with_path_context(self.path.display().to_string(), err))?;
        let origin = self.path.display().to_string();
        for entry in &mut layer.entries {
            entry.origin = Some(origin.clone());
        }
        Ok(layer)
    }
}

#[derive(Debug, Clone, Default)]
pub struct EnvVarLoader {
    vars: Vec<(String, String)>,
}

impl EnvVarLoader {
    pub fn from_process_env() -> Self {
        Self {
            vars: std::env::vars().collect(),
        }
    }

    pub fn from_iter<I, K, V>(vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        Self {
            vars: vars
                .into_iter()
                .map(|(k, v)| (k.as_ref().to_string(), v.as_ref().to_string()))
                .collect(),
        }
    }
}

impl ConfigLoader for EnvVarLoader {
    fn load(&self) -> Result<ConfigLayer, ConfigError> {
        ConfigLayer::from_env_iter(self.vars.iter().map(|(k, v)| (k.as_str(), v.as_str())))
    }
}

#[derive(Debug, Clone)]
pub struct SecretsTomlLoader {
    path: PathBuf,
    missing_ok: bool,
    strict_permissions: bool,
}

impl SecretsTomlLoader {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            missing_ok: true,
            strict_permissions: true,
        }
    }

    pub fn required(mut self) -> Self {
        self.missing_ok = false;
        self
    }

    pub fn optional(mut self) -> Self {
        self.missing_ok = true;
        self
    }

    pub fn with_strict_permissions(mut self, strict: bool) -> Self {
        self.strict_permissions = strict;
        self
    }
}

impl ConfigLoader for SecretsTomlLoader {
    fn load(&self) -> Result<ConfigLayer, ConfigError> {
        if !self.path.exists() {
            if self.missing_ok {
                return Ok(ConfigLayer::default());
            }
            return Err(ConfigError::FileRead {
                path: self.path.display().to_string(),
                reason: "file not found".to_string(),
            });
        }

        validate_secrets_permissions(&self.path, self.strict_permissions)?;

        let raw = std::fs::read_to_string(&self.path).map_err(|err| ConfigError::FileRead {
            path: self.path.display().to_string(),
            reason: err.to_string(),
        })?;

        let mut layer = ConfigLayer::from_toml_str(&raw)
            .map_err(|err| with_path_context(self.path.display().to_string(), err))?;
        let origin = self.path.display().to_string();
        for entry in &mut layer.entries {
            entry.origin = Some(origin.clone());
        }
        Ok(layer)
    }
}

#[derive(Debug, Clone, Default)]
pub struct EnvSecretsLoader {
    vars: Vec<(String, String)>,
}

impl EnvSecretsLoader {
    pub fn from_process_env() -> Self {
        Self {
            vars: std::env::vars().collect(),
        }
    }

    pub fn from_iter<I, K, V>(vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        Self {
            vars: vars
                .into_iter()
                .map(|(k, v)| (k.as_ref().to_string(), v.as_ref().to_string()))
                .collect(),
        }
    }
}

impl ConfigLoader for EnvSecretsLoader {
    fn load(&self) -> Result<ConfigLayer, ConfigError> {
        let mut layer = ConfigLayer::default();

        for (name, value) in &self.vars {
            let Some(rest) = name.strip_prefix("OSP_SECRET__") else {
                continue;
            };

            let synthetic = format!("OSP__{rest}");
            let spec = parse_env_key(&synthetic)?;
            layer.insert_with_origin(spec.key, value.clone(), spec.scope, Some(name.clone()));
        }

        Ok(layer)
    }
}

#[derive(Default)]
pub struct ChainedLoader {
    loaders: Vec<Box<dyn ConfigLoader>>,
}

impl ChainedLoader {
    pub fn new<L>(loader: L) -> Self
    where
        L: ConfigLoader + 'static,
    {
        Self {
            loaders: vec![Box::new(loader)],
        }
    }

    pub fn with<L>(mut self, loader: L) -> Self
    where
        L: ConfigLoader + 'static,
    {
        self.loaders.push(Box::new(loader));
        self
    }
}

impl ConfigLoader for ChainedLoader {
    fn load(&self) -> Result<ConfigLayer, ConfigError> {
        let mut merged = ConfigLayer::default();
        for loader in &self.loaders {
            let layer = loader.load()?;
            merged.entries.extend(layer.entries);
        }
        Ok(merged)
    }
}

#[derive(Debug, Clone, Default)]
pub struct LoadedLayers {
    pub defaults: ConfigLayer,
    pub file: ConfigLayer,
    pub secrets: ConfigLayer,
    pub env: ConfigLayer,
    pub cli: ConfigLayer,
    pub session: ConfigLayer,
}

pub struct LoaderPipeline {
    defaults: Box<dyn ConfigLoader>,
    file: Option<Box<dyn ConfigLoader>>,
    secrets: Option<Box<dyn ConfigLoader>>,
    env: Option<Box<dyn ConfigLoader>>,
    cli: Option<Box<dyn ConfigLoader>>,
    session: Option<Box<dyn ConfigLoader>>,
    schema: ConfigSchema,
}

impl LoaderPipeline {
    pub fn new<L>(defaults: L) -> Self
    where
        L: ConfigLoader + 'static,
    {
        Self {
            defaults: Box::new(defaults),
            file: None,
            secrets: None,
            env: None,
            cli: None,
            session: None,
            schema: ConfigSchema::default(),
        }
    }

    pub fn with_file<L>(mut self, loader: L) -> Self
    where
        L: ConfigLoader + 'static,
    {
        self.file = Some(Box::new(loader));
        self
    }

    pub fn with_secrets<L>(mut self, loader: L) -> Self
    where
        L: ConfigLoader + 'static,
    {
        self.secrets = Some(Box::new(loader));
        self
    }

    pub fn with_env<L>(mut self, loader: L) -> Self
    where
        L: ConfigLoader + 'static,
    {
        self.env = Some(Box::new(loader));
        self
    }

    pub fn with_cli<L>(mut self, loader: L) -> Self
    where
        L: ConfigLoader + 'static,
    {
        self.cli = Some(Box::new(loader));
        self
    }

    pub fn with_session<L>(mut self, loader: L) -> Self
    where
        L: ConfigLoader + 'static,
    {
        self.session = Some(Box::new(loader));
        self
    }

    pub fn with_schema(mut self, schema: ConfigSchema) -> Self {
        self.schema = schema;
        self
    }

    pub fn load_layers(&self) -> Result<LoadedLayers, ConfigError> {
        Ok(LoadedLayers {
            defaults: self.defaults.load()?,
            file: load_optional_loader(self.file.as_deref())?,
            secrets: load_optional_loader(self.secrets.as_deref())?,
            env: load_optional_loader(self.env.as_deref())?,
            cli: load_optional_loader(self.cli.as_deref())?,
            session: load_optional_loader(self.session.as_deref())?,
        })
    }

    pub fn resolve(&self, options: ResolveOptions) -> Result<ResolvedConfig, ConfigError> {
        let layers = self.load_layers()?;
        let mut resolver = ConfigResolver::from_loaded_layers(layers);
        resolver.set_schema(self.schema.clone());
        resolver.resolve(options)
    }
}

#[derive(Debug)]
struct EnvKeySpec {
    key: String,
    scope: Scope,
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
    active_profile: String,
    terminal: Option<String>,
    known_profiles: BTreeSet<String>,
    values: BTreeMap<String, ResolvedValue>,
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

#[derive(Debug, Clone)]
pub struct ConfigResolver {
    defaults: ConfigLayer,
    file: ConfigLayer,
    secrets: ConfigLayer,
    env: ConfigLayer,
    cli: ConfigLayer,
    session: ConfigLayer,
    schema: ConfigSchema,
}

impl Default for ConfigResolver {
    fn default() -> Self {
        Self {
            defaults: ConfigLayer::default(),
            file: ConfigLayer::default(),
            secrets: ConfigLayer::default(),
            env: ConfigLayer::default(),
            cli: ConfigLayer::default(),
            session: ConfigLayer::default(),
            schema: ConfigSchema::default(),
        }
    }
}

impl ConfigResolver {
    pub fn from_loaded_layers(layers: LoadedLayers) -> Self {
        Self {
            defaults: layers.defaults,
            file: layers.file,
            secrets: layers.secrets,
            env: layers.env,
            cli: layers.cli,
            session: layers.session,
            schema: ConfigSchema::default(),
        }
    }

    pub fn set_schema(&mut self, schema: ConfigSchema) {
        self.schema = schema;
    }

    pub fn schema_mut(&mut self) -> &mut ConfigSchema {
        &mut self.schema
    }

    pub fn defaults_mut(&mut self) -> &mut ConfigLayer {
        &mut self.defaults
    }

    pub fn file_mut(&mut self) -> &mut ConfigLayer {
        &mut self.file
    }

    pub fn secrets_mut(&mut self) -> &mut ConfigLayer {
        &mut self.secrets
    }

    pub fn env_mut(&mut self) -> &mut ConfigLayer {
        &mut self.env
    }

    pub fn cli_mut(&mut self) -> &mut ConfigLayer {
        &mut self.cli
    }

    pub fn session_mut(&mut self) -> &mut ConfigLayer {
        &mut self.session
    }

    pub fn set_defaults(&mut self, layer: ConfigLayer) {
        self.defaults = layer;
    }

    pub fn set_file(&mut self, layer: ConfigLayer) {
        self.file = layer;
    }

    pub fn set_secrets(&mut self, layer: ConfigLayer) {
        self.secrets = layer;
    }

    pub fn set_env(&mut self, layer: ConfigLayer) {
        self.env = layer;
    }

    pub fn set_cli(&mut self, layer: ConfigLayer) {
        self.cli = layer;
    }

    pub fn set_session(&mut self, layer: ConfigLayer) {
        self.session = layer;
    }

    pub fn resolve(&self, options: ResolveOptions) -> Result<ResolvedConfig, ConfigError> {
        let terminal = options.terminal.map(|value| normalize_identifier(&value));
        let profile_override = options
            .profile_override
            .map(|value| normalize_identifier(&value));
        let known_profiles = self.collect_known_profiles();
        let active_profile = self.resolve_active_profile(
            profile_override.as_deref(),
            terminal.as_deref(),
            &known_profiles,
        )?;
        let mut values = self.collect_selected_values(&active_profile, terminal.as_deref());

        interpolate_all(&mut values)?;
        self.schema.validate_and_adapt(&mut values)?;

        Ok(ResolvedConfig {
            active_profile,
            terminal,
            known_profiles,
            values,
        })
    }

    pub fn explain_key(
        &self,
        key: &str,
        options: ResolveOptions,
    ) -> Result<ConfigExplain, ConfigError> {
        let terminal = options.terminal.map(|value| normalize_identifier(&value));
        let profile_override = options
            .profile_override
            .map(|value| normalize_identifier(&value));
        let known_profiles = self.collect_known_profiles();
        let active_profile = self.resolve_active_profile(
            profile_override.as_deref(),
            terminal.as_deref(),
            &known_profiles,
        )?;

        let mut layers = Vec::new();
        for (source, layer) in self.layers() {
            let selected_entry =
                select_scoped_entry(layer, key, &active_profile, terminal.as_deref());
            let selected_entry_index = selected_entry.and_then(|entry| {
                layer
                    .entries
                    .iter()
                    .position(|candidate| std::ptr::eq(candidate, entry))
            });

            let mut candidates = Vec::new();
            for (entry_index, entry) in layer.entries.iter().enumerate() {
                if entry.key != key {
                    continue;
                }

                let rank = scope_rank(&entry.scope, &active_profile, terminal.as_deref());
                candidates.push(ExplainCandidate {
                    entry_index,
                    value: entry.value.clone(),
                    scope: entry.scope.clone(),
                    origin: entry.origin.clone(),
                    rank,
                    selected_in_layer: selected_entry_index == Some(entry_index),
                });
            }

            if !candidates.is_empty() {
                layers.push(ExplainLayer {
                    source,
                    selected_entry_index,
                    candidates,
                });
            }
        }

        let pre_interpolated = self.collect_selected_values(&active_profile, terminal.as_deref());
        let mut final_values = pre_interpolated.clone();
        interpolate_all(&mut final_values)?;
        self.schema.validate_and_adapt(&mut final_values)?;
        let final_entry = final_values.get(key).cloned();
        let interpolation = explain_interpolation(key, &pre_interpolated, &final_values)?;

        Ok(ConfigExplain {
            key: key.to_string(),
            active_profile,
            terminal,
            known_profiles,
            layers,
            final_entry,
            interpolation,
        })
    }

    fn collect_selected_values(
        &self,
        active_profile: &str,
        terminal: Option<&str>,
    ) -> BTreeMap<String, ResolvedValue> {
        let mut keys = self.collect_keys();
        keys.insert("profile.default".to_string());

        let mut values = BTreeMap::new();
        for key in keys {
            if let Some((source, entry)) = self.select_across_layers(&key, active_profile, terminal)
            {
                values.insert(
                    key,
                    ResolvedValue {
                        raw_value: entry.value.clone(),
                        value: entry.value.clone(),
                        source,
                        scope: entry.scope.clone(),
                        origin: entry.origin.clone(),
                    },
                );
            }
        }

        values.insert(
            "profile.active".to_string(),
            ResolvedValue {
                raw_value: ConfigValue::String(active_profile.to_string()),
                value: ConfigValue::String(active_profile.to_string()),
                source: ConfigSource::Derived,
                scope: Scope::global(),
                origin: None,
            },
        );
        values.insert(
            "context".to_string(),
            ResolvedValue {
                raw_value: ConfigValue::String(active_profile.to_string()),
                value: ConfigValue::String(active_profile.to_string()),
                source: ConfigSource::Derived,
                scope: Scope::global(),
                origin: None,
            },
        );

        values
    }

    fn collect_known_profiles(&self) -> BTreeSet<String> {
        let mut known = BTreeSet::new();

        for (_, layer) in self.layers() {
            for entry in &layer.entries {
                if let Some(profile) = entry.scope.profile.as_deref() {
                    known.insert(profile.to_string());
                }
            }
        }

        known
    }

    fn resolve_active_profile(
        &self,
        explicit: Option<&str>,
        terminal: Option<&str>,
        known_profiles: &BTreeSet<String>,
    ) -> Result<String, ConfigError> {
        let chosen = if let Some(profile) = explicit {
            normalize_identifier(profile)
        } else {
            self.resolve_default_profile(terminal)?
        };

        if chosen.trim().is_empty() {
            return Err(ConfigError::MissingDefaultProfile);
        }

        if !known_profiles.is_empty() && !known_profiles.contains(&chosen) {
            return Err(ConfigError::UnknownProfile {
                profile: chosen,
                known: known_profiles.iter().cloned().collect::<Vec<String>>(),
            });
        }

        Ok(chosen)
    }

    fn resolve_default_profile(&self, terminal: Option<&str>) -> Result<String, ConfigError> {
        let mut picked: Option<ConfigValue> = None;

        for (_, layer) in self.layers() {
            if let Some(entry) = select_global_entry(layer, "profile.default", terminal) {
                picked = Some(entry.value.clone());
            }
        }

        match picked {
            None => Ok("default".to_string()),
            Some(ConfigValue::String(profile)) if !profile.trim().is_empty() => {
                Ok(normalize_identifier(&profile))
            }
            Some(other) => Err(ConfigError::InvalidDefaultProfileType(format!("{other:?}"))),
        }
    }

    fn collect_keys(&self) -> BTreeSet<String> {
        let mut keys = BTreeSet::new();

        for (_, layer) in self.layers() {
            for entry in &layer.entries {
                keys.insert(entry.key.clone());
            }
        }

        keys
    }

    fn select_across_layers<'a>(
        &'a self,
        key: &str,
        profile: &str,
        terminal: Option<&str>,
    ) -> Option<(ConfigSource, &'a LayerEntry)> {
        let mut selected: Option<(ConfigSource, &'a LayerEntry)> = None;

        for (source, layer) in self.layers() {
            if let Some(entry) = select_scoped_entry(layer, key, profile, terminal) {
                selected = Some((source, entry));
            }
        }

        selected
    }

    fn layers(&self) -> [(ConfigSource, &ConfigLayer); 6] {
        [
            (ConfigSource::BuiltinDefaults, &self.defaults),
            (ConfigSource::ConfigFile, &self.file),
            (ConfigSource::Secrets, &self.secrets),
            (ConfigSource::Environment, &self.env),
            (ConfigSource::Cli, &self.cli),
            (ConfigSource::Session, &self.session),
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    FileRead {
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
    MissingDefaultProfile,
    InvalidDefaultProfileType(String),
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
            ConfigError::MissingDefaultProfile => {
                write!(f, "missing profile.default and no fallback profile")
            }
            ConfigError::InvalidDefaultProfileType(actual) => {
                write!(f, "profile.default must be string, got {actual}")
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

fn select_scoped_entry<'a>(
    layer: &'a ConfigLayer,
    key: &str,
    profile: &str,
    terminal: Option<&str>,
) -> Option<&'a LayerEntry> {
    select_entry(layer, key, |scope| scope_rank(scope, profile, terminal))
}

fn select_global_entry<'a>(
    layer: &'a ConfigLayer,
    key: &str,
    terminal: Option<&str>,
) -> Option<&'a LayerEntry> {
    select_entry(layer, key, |scope| global_rank(scope, terminal))
}

fn select_entry<'a, F>(layer: &'a ConfigLayer, key: &str, ranker: F) -> Option<&'a LayerEntry>
where
    F: Fn(&Scope) -> Option<u8>,
{
    let mut best: Option<(usize, u8, &'a LayerEntry)> = None;

    for (index, entry) in layer.entries.iter().enumerate() {
        if entry.key != key {
            continue;
        }

        let Some(rank) = ranker(&entry.scope) else {
            continue;
        };

        let replace = match best {
            None => true,
            Some((best_index, best_rank, _)) => {
                rank < best_rank || (rank == best_rank && index > best_index)
            }
        };

        if replace {
            best = Some((index, rank, entry));
        }
    }

    best.map(|(_, _, entry)| entry)
}

fn scope_rank(scope: &Scope, profile: &str, terminal: Option<&str>) -> Option<u8> {
    match (
        scope.profile.as_deref(),
        scope.terminal.as_deref(),
        terminal,
    ) {
        (Some(p), Some(t), Some(active_t)) if p == profile && t == active_t => Some(0),
        (Some(p), None, _) if p == profile => Some(1),
        (None, Some(t), Some(active_t)) if t == active_t => Some(2),
        (None, None, _) => Some(3),
        _ => None,
    }
}

fn global_rank(scope: &Scope, terminal: Option<&str>) -> Option<u8> {
    match (
        scope.profile.as_deref(),
        scope.terminal.as_deref(),
        terminal,
    ) {
        (None, Some(t), Some(active_t)) if t == active_t => Some(0),
        (None, None, _) => Some(1),
        _ => None,
    }
}

fn interpolate_all(values: &mut BTreeMap<String, ResolvedValue>) -> Result<(), ConfigError> {
    let raw = values
        .iter()
        .map(|(key, value)| (key.clone(), value.value.clone()))
        .collect::<HashMap<String, ConfigValue>>();

    let keys = values.keys().cloned().collect::<Vec<String>>();
    let mut cache: HashMap<String, ConfigValue> = HashMap::new();
    let mut stack: Vec<String> = Vec::new();

    for key in keys {
        let value = resolve_interpolated_value(&key, &raw, &mut cache, &mut stack)?;
        if let Some(entry) = values.get_mut(&key) {
            entry.value = value;
        }
    }

    Ok(())
}

fn resolve_interpolated_value(
    key: &str,
    raw: &HashMap<String, ConfigValue>,
    cache: &mut HashMap<String, ConfigValue>,
    stack: &mut Vec<String>,
) -> Result<ConfigValue, ConfigError> {
    if let Some(value) = cache.get(key) {
        return Ok(value.clone());
    }

    if let Some(index) = stack.iter().position(|item| item == key) {
        let mut cycle = stack[index..].to_vec();
        cycle.push(key.to_string());
        return Err(ConfigError::PlaceholderCycle { cycle });
    }

    let value = raw
        .get(key)
        .cloned()
        .ok_or_else(|| ConfigError::UnresolvedPlaceholder {
            key: key.to_string(),
            placeholder: key.to_string(),
        })?;

    stack.push(key.to_string());

    let resolved = match value {
        ConfigValue::String(template) => {
            ConfigValue::String(interpolate_string(key, &template, raw, cache, stack)?)
        }
        other => other,
    };

    stack.pop();
    cache.insert(key.to_string(), resolved.clone());

    Ok(resolved)
}

fn interpolate_string(
    key: &str,
    template: &str,
    raw: &HashMap<String, ConfigValue>,
    cache: &mut HashMap<String, ConfigValue>,
    stack: &mut Vec<String>,
) -> Result<String, ConfigError> {
    let mut out = String::new();
    let mut cursor = 0usize;

    while let Some(rel_start) = template[cursor..].find("${") {
        let start = cursor + rel_start;
        out.push_str(&template[cursor..start]);

        let after_open = start + 2;
        let Some(rel_end) = template[after_open..].find('}') else {
            return Err(ConfigError::InvalidPlaceholderSyntax {
                key: key.to_string(),
                template: template.to_string(),
            });
        };

        let end = after_open + rel_end;
        let placeholder = template[after_open..end].trim();
        if placeholder.is_empty() {
            return Err(ConfigError::InvalidPlaceholderSyntax {
                key: key.to_string(),
                template: template.to_string(),
            });
        }

        if !raw.contains_key(placeholder) {
            return Err(ConfigError::UnresolvedPlaceholder {
                key: key.to_string(),
                placeholder: placeholder.to_string(),
            });
        }

        let interpolated = resolve_interpolated_value(placeholder, raw, cache, stack)?
            .as_interpolation_string(key, placeholder)?;
        out.push_str(&interpolated);

        cursor = end + 1;
    }

    out.push_str(&template[cursor..]);
    Ok(out)
}

fn explain_interpolation(
    key: &str,
    pre_interpolated: &BTreeMap<String, ResolvedValue>,
    final_values: &BTreeMap<String, ResolvedValue>,
) -> Result<Option<ExplainInterpolation>, ConfigError> {
    let Some(entry) = pre_interpolated.get(key) else {
        return Ok(None);
    };
    let ConfigValue::String(template) = &entry.raw_value else {
        return Ok(None);
    };
    if !template.contains("${") {
        return Ok(None);
    }

    let raw = pre_interpolated
        .iter()
        .map(|(entry_key, value)| (entry_key.clone(), value.raw_value.clone()))
        .collect::<HashMap<String, ConfigValue>>();
    let mut steps = Vec::new();
    let mut seen = BTreeSet::new();
    let mut stack = Vec::new();
    collect_interpolation_steps_recursive(
        key,
        &raw,
        final_values,
        &mut steps,
        &mut seen,
        &mut stack,
    )?;

    Ok(Some(ExplainInterpolation {
        template: template.clone(),
        steps,
    }))
}

fn collect_interpolation_steps_recursive(
    key: &str,
    raw: &HashMap<String, ConfigValue>,
    final_values: &BTreeMap<String, ResolvedValue>,
    steps: &mut Vec<ExplainInterpolationStep>,
    seen: &mut BTreeSet<String>,
    stack: &mut Vec<String>,
) -> Result<(), ConfigError> {
    if let Some(index) = stack.iter().position(|item| item == key) {
        let mut cycle = stack[index..].to_vec();
        cycle.push(key.to_string());
        return Err(ConfigError::PlaceholderCycle { cycle });
    }

    let Some(ConfigValue::String(template)) = raw.get(key) else {
        return Ok(());
    };
    if !template.contains("${") {
        return Ok(());
    }

    stack.push(key.to_string());
    let placeholders = extract_placeholders(key, template)?;

    for placeholder in placeholders {
        if !raw.contains_key(&placeholder) {
            return Err(ConfigError::UnresolvedPlaceholder {
                key: key.to_string(),
                placeholder,
            });
        }

        if seen.insert(placeholder.clone())
            && let Some(value_entry) = final_values.get(&placeholder)
        {
            steps.push(ExplainInterpolationStep {
                placeholder: placeholder.clone(),
                value: value_entry.value.clone(),
                source: value_entry.source,
                scope: value_entry.scope.clone(),
                origin: value_entry.origin.clone(),
            });
        }

        collect_interpolation_steps_recursive(&placeholder, raw, final_values, steps, seen, stack)?;
    }

    stack.pop();
    Ok(())
}

fn extract_placeholders(key: &str, template: &str) -> Result<Vec<String>, ConfigError> {
    let mut placeholders = Vec::new();
    let mut cursor = 0usize;

    while let Some(rel_start) = template[cursor..].find("${") {
        let start = cursor + rel_start;
        let after_open = start + 2;
        let Some(rel_end) = template[after_open..].find('}') else {
            return Err(ConfigError::InvalidPlaceholderSyntax {
                key: key.to_string(),
                template: template.to_string(),
            });
        };
        let end = after_open + rel_end;

        let placeholder = template[after_open..end].trim();
        if placeholder.is_empty() {
            return Err(ConfigError::InvalidPlaceholderSyntax {
                key: key.to_string(),
                template: template.to_string(),
            });
        }

        placeholders.push(placeholder.to_string());
        cursor = end + 1;
    }

    Ok(placeholders)
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

#[cfg(unix)]
fn validate_secrets_permissions(path: &PathBuf, strict: bool) -> Result<(), ConfigError> {
    use std::os::unix::fs::PermissionsExt;

    if !strict {
        return Ok(());
    }

    let metadata = std::fs::metadata(path).map_err(|err| ConfigError::FileRead {
        path: path.display().to_string(),
        reason: err.to_string(),
    })?;
    let mode = metadata.permissions().mode() & 0o777;
    if mode & 0o077 != 0 {
        return Err(ConfigError::InsecureSecretsPermissions {
            path: path.display().to_string(),
            mode,
        });
    }

    Ok(())
}

#[cfg(not(unix))]
fn validate_secrets_permissions(_path: &PathBuf, _strict: bool) -> Result<(), ConfigError> {
    Ok(())
}

fn parse_env_key(key: &str) -> Result<EnvKeySpec, ConfigError> {
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

fn load_optional_loader(loader: Option<&dyn ConfigLoader>) -> Result<ConfigLayer, ConfigError> {
    match loader {
        Some(loader) => loader.load(),
        None => Ok(ConfigLayer::default()),
    }
}

fn with_path_context(path: String, error: ConfigError) -> ConfigError {
    ConfigError::LayerLoad {
        path,
        source: Box::new(error),
    }
}

fn normalize_scope(scope: Scope) -> Scope {
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

fn normalize_identifier(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}
