use std::path::PathBuf;

use crate::{
    ConfigError, ConfigLayer, ConfigResolver, ConfigSchema, ConfigValue, ResolveOptions,
    ResolvedConfig, core::parse_env_key, store::validate_secrets_permissions, with_path_context,
};

pub trait ConfigLoader: Send + Sync {
    fn load(&self) -> Result<ConfigLayer, ConfigError>;
}

fn collect_string_pairs<I, K, V>(vars: I) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: AsRef<str>,
{
    vars.into_iter()
        .map(|(key, value)| (key.as_ref().to_string(), value.as_ref().to_string()))
        .collect()
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
        tracing::trace!(
            entries = self.layer.entries().len(),
            "loaded static config layer"
        );
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
        tracing::debug!(
            path = %self.path.display(),
            missing_ok = self.missing_ok,
            "loading TOML config layer"
        );
        if !self.path.exists() {
            if self.missing_ok {
                tracing::debug!(path = %self.path.display(), "optional TOML config file missing");
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
        tracing::debug!(
            path = %self.path.display(),
            entries = layer.entries().len(),
            "loaded TOML config layer"
        );
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

    pub fn from_pairs<I, K, V>(vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        Self {
            vars: collect_string_pairs(vars),
        }
    }
}

impl<K, V> std::iter::FromIterator<(K, V)> for EnvVarLoader
where
    K: AsRef<str>,
    V: AsRef<str>,
{
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
        Self {
            vars: collect_string_pairs(iter),
        }
    }
}

impl ConfigLoader for EnvVarLoader {
    fn load(&self) -> Result<ConfigLayer, ConfigError> {
        let layer =
            ConfigLayer::from_env_iter(self.vars.iter().map(|(k, v)| (k.as_str(), v.as_str())))?;
        tracing::debug!(
            input_vars = self.vars.len(),
            entries = layer.entries().len(),
            "loaded environment config layer"
        );
        Ok(layer)
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
        tracing::debug!(
            path = %self.path.display(),
            missing_ok = self.missing_ok,
            strict_permissions = self.strict_permissions,
            "loading TOML secrets layer"
        );
        if !self.path.exists() {
            if self.missing_ok {
                tracing::debug!(path = %self.path.display(), "optional TOML secrets file missing");
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
        layer.mark_all_secret();
        tracing::debug!(
            path = %self.path.display(),
            entries = layer.entries().len(),
            "loaded TOML secrets layer"
        );
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

    pub fn from_pairs<I, K, V>(vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        Self {
            vars: collect_string_pairs(vars),
        }
    }
}

impl<K, V> std::iter::FromIterator<(K, V)> for EnvSecretsLoader
where
    K: AsRef<str>,
    V: AsRef<str>,
{
    fn from_iter<T: IntoIterator<Item = (K, V)>>(iter: T) -> Self {
        Self {
            vars: collect_string_pairs(iter),
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
            layer.insert_with_origin(
                spec.key,
                ConfigValue::String(value.clone()).into_secret(),
                spec.scope,
                Some(name.clone()),
            );
        }

        tracing::debug!(
            input_vars = self.vars.len(),
            entries = layer.entries().len(),
            "loaded environment secrets layer"
        );
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
        tracing::debug!(
            loader_count = self.loaders.len(),
            "loading chained config layer"
        );
        for loader in &self.loaders {
            let layer = loader.load()?;
            merged.entries.extend(layer.entries);
        }
        tracing::debug!(
            entries = merged.entries().len(),
            "loaded chained config layer"
        );
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
        tracing::debug!("loading config layers");
        let layers = LoadedLayers {
            defaults: self.defaults.load()?,
            file: load_optional_loader(self.file.as_deref())?,
            secrets: load_optional_loader(self.secrets.as_deref())?,
            env: load_optional_loader(self.env.as_deref())?,
            cli: load_optional_loader(self.cli.as_deref())?,
            session: load_optional_loader(self.session.as_deref())?,
        };
        tracing::debug!(
            defaults = layers.defaults.entries().len(),
            file = layers.file.entries().len(),
            secrets = layers.secrets.entries().len(),
            env = layers.env.entries().len(),
            cli = layers.cli.entries().len(),
            session = layers.session.entries().len(),
            "loaded config layers"
        );
        Ok(layers)
    }

    pub fn resolve(&self, options: ResolveOptions) -> Result<ResolvedConfig, ConfigError> {
        let layers = self.load_layers()?;
        let mut resolver = ConfigResolver::from_loaded_layers(layers);
        resolver.set_schema(self.schema.clone());
        resolver.resolve(options)
    }
}

fn load_optional_loader(loader: Option<&dyn ConfigLoader>) -> Result<ConfigLayer, ConfigError> {
    match loader {
        Some(loader) => loader.load(),
        None => Ok(ConfigLayer::default()),
    }
}

#[cfg(test)]
mod loader_path_tests {
    use super::{
        ChainedLoader, ConfigLoader, EnvSecretsLoader, EnvVarLoader, LoaderPipeline,
        SecretsTomlLoader, StaticLayerLoader, TomlFileLoader,
    };
    use crate::{ConfigError, ConfigLayer, ConfigSchema, ResolveOptions, Scope};
    use std::path::PathBuf;

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time should be valid")
            .as_nanos();
        dir.push(format!("{prefix}-{nonce}"));
        std::fs::create_dir_all(&dir).expect("temp dir should be created");
        dir
    }

    #[test]
    fn toml_file_loader_covers_existing_optional_and_missing_required_paths() {
        let root = make_temp_dir("osp-config-loader");
        let config_path = root.join("config.toml");
        std::fs::write(&config_path, "[default.ui]\ntheme = \"plain\"\n")
            .expect("config should be written");

        let layer = TomlFileLoader::new(config_path.clone())
            .optional()
            .load()
            .expect("optional existing config should load");
        let config_origin = config_path.to_string_lossy().to_string();
        assert_eq!(layer.entries().len(), 1);
        assert_eq!(layer.entries()[0].key, "ui.theme");
        assert_eq!(
            layer.entries()[0].origin.as_deref(),
            Some(config_origin.as_str())
        );

        let missing_path = root.join("missing.toml");
        let missing = TomlFileLoader::new(missing_path.clone())
            .required()
            .load()
            .expect_err("required missing config should fail");
        let missing_display = missing_path.to_string_lossy().to_string();
        assert!(matches!(
            missing,
            ConfigError::FileRead { path, reason }
            if path == missing_display && reason == "file not found"
        ));

        let optional_missing = TomlFileLoader::new(root.join("optional.toml"))
            .optional()
            .load()
            .expect("optional missing config should be empty");
        assert!(optional_missing.entries().is_empty());
    }

    #[test]
    fn secrets_and_env_loaders_mark_secret_entries_and_origins() {
        let root = make_temp_dir("osp-config-secrets");
        let secrets_path = root.join("secrets.toml");
        std::fs::write(&secrets_path, "[default.auth]\ntoken = \"shh\"\n")
            .expect("secrets file should be written");

        let secrets = SecretsTomlLoader::new(secrets_path.clone())
            .with_strict_permissions(false)
            .required()
            .load()
            .expect("secrets file should load");
        let secrets_origin = secrets_path.to_string_lossy().to_string();
        assert_eq!(secrets.entries().len(), 1);
        assert!(secrets.entries()[0].value.is_secret());
        assert_eq!(
            secrets.entries()[0].origin.as_deref(),
            Some(secrets_origin.as_str())
        );

        let env =
            EnvSecretsLoader::from_iter([("IGNORED", "x"), ("OSP_SECRET__AUTH__TOKEN", "env-shh")])
                .load()
                .expect("env secrets should load");
        assert_eq!(env.entries().len(), 1);
        assert_eq!(env.entries()[0].key, "auth.token");
        assert!(env.entries()[0].value.is_secret());
        assert_eq!(
            env.entries()[0].origin.as_deref(),
            Some("OSP_SECRET__AUTH__TOKEN")
        );
    }

    #[test]
    fn chained_loader_and_pipeline_merge_and_resolve_layers() {
        let chained = ChainedLoader::new(StaticLayerLoader::new({
            let mut layer = ConfigLayer::default();
            layer.insert("theme.name", "plain", Scope::global());
            layer
        }))
        .with(EnvVarLoader::from_pairs([("OSP__THEME__NAME", "dracula")]));
        let merged = chained.load().expect("chained loader should merge");
        assert_eq!(merged.entries().len(), 2);

        let resolved = LoaderPipeline::new(StaticLayerLoader::new({
            let mut layer = ConfigLayer::default();
            layer.insert("theme.name", "plain", Scope::global());
            layer
        }))
        .with_env(EnvVarLoader::from_pairs([("OSP__THEME__NAME", "dracula")]))
        .resolve(ResolveOptions::default())
        .expect("pipeline should resolve");
        assert_eq!(resolved.get_string("theme.name"), Some("dracula"));

        let layers = LoaderPipeline::new(StaticLayerLoader::new(ConfigLayer::default()))
            .load_layers()
            .expect("optional loaders should default to empty");
        assert!(layers.file.entries().is_empty());
        assert!(layers.secrets.entries().is_empty());
        assert!(layers.env.entries().is_empty());
        assert!(layers.cli.entries().is_empty());
        assert!(layers.session.entries().is_empty());
    }

    #[test]
    fn pipeline_builder_covers_schema_and_collected_env_loaders() {
        let env: EnvVarLoader = [("OSP__THEME__NAME", "nord")].into_iter().collect();
        let secrets: EnvSecretsLoader = [("OSP_SECRET__AUTH__TOKEN", "tok")].into_iter().collect();

        let layers = LoaderPipeline::new(StaticLayerLoader::new(ConfigLayer::default()))
            .with_env(env)
            .with_secrets(secrets)
            .with_schema(ConfigSchema::default())
            .load_layers()
            .expect("pipeline should load collected loaders");

        assert_eq!(layers.env.entries().len(), 1);
        assert_eq!(layers.secrets.entries().len(), 1);
    }
}
