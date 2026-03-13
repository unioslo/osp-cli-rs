use std::path::PathBuf;

use crate::config::{
    ConfigError, ConfigLayer, ConfigResolver, ConfigSchema, ConfigValue, ResolveOptions,
    ResolvedConfig, core::parse_env_key, store::validate_secrets_permissions, with_path_context,
};

/// Loads a single config layer from some backing source.
pub trait ConfigLoader: Send + Sync {
    /// Reads the source and returns it as a config layer.
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

/// Loader that returns a prebuilt config layer.
#[derive(Debug, Clone, Default)]
pub struct StaticLayerLoader {
    layer: ConfigLayer,
}

impl StaticLayerLoader {
    /// Wraps an existing layer so it can participate in a loader pipeline.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::config::{ConfigLayer, ConfigLoader, StaticLayerLoader};
    ///
    /// let loader = StaticLayerLoader::new(ConfigLayer::default());
    ///
    /// assert!(loader.load().unwrap().entries().is_empty());
    /// ```
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

/// Loader for ordinary TOML config files.
#[derive(Debug, Clone)]
#[must_use]
pub struct TomlFileLoader {
    path: PathBuf,
    missing_ok: bool,
}

impl TomlFileLoader {
    /// Creates a loader for the given TOML file path.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            missing_ok: true,
        }
    }

    /// Requires the file to exist.
    pub fn required(mut self) -> Self {
        self.missing_ok = false;
        self
    }

    /// Allows the file to be absent.
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

/// Loader for `OSP__...` environment variables.
#[derive(Debug, Clone, Default)]
pub struct EnvVarLoader {
    vars: Vec<(String, String)>,
}

impl EnvVarLoader {
    /// Captures the current process environment.
    pub fn from_process_env() -> Self {
        Self {
            vars: std::env::vars().collect(),
        }
    }

    /// Creates a loader from explicit key-value pairs.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::config::{ConfigLoader, EnvVarLoader};
    ///
    /// let loader = EnvVarLoader::from_pairs([("OSP__output__format", "json")]);
    /// let layer = loader.load().unwrap();
    ///
    /// assert_eq!(layer.entries()[0].key, "output.format");
    /// ```
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

/// Loader for TOML secrets files whose values are marked secret.
#[derive(Debug, Clone)]
#[must_use]
pub struct SecretsTomlLoader {
    path: PathBuf,
    missing_ok: bool,
    strict_permissions: bool,
}

impl SecretsTomlLoader {
    /// Creates a loader for the given secrets file path.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            missing_ok: true,
            strict_permissions: true,
        }
    }

    /// Requires the file to exist.
    pub fn required(mut self) -> Self {
        self.missing_ok = false;
        self
    }

    /// Allows the file to be absent.
    pub fn optional(mut self) -> Self {
        self.missing_ok = true;
        self
    }

    /// Enables or disables permission checks before loading.
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

/// Loader for `OSP_SECRET__...` environment variables.
#[derive(Debug, Clone, Default)]
pub struct EnvSecretsLoader {
    vars: Vec<(String, String)>,
}

impl EnvSecretsLoader {
    /// Captures secret variables from the current process environment.
    pub fn from_process_env() -> Self {
        Self {
            vars: std::env::vars().collect(),
        }
    }

    /// Creates a loader from explicit key-value pairs.
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
            ConfigSchema::default().validate_writable_key(&spec.key)?;
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

/// Loader that merges multiple loaders in the order they are added.
#[derive(Default)]
#[must_use]
pub struct ChainedLoader {
    loaders: Vec<Box<dyn ConfigLoader>>,
}

impl ChainedLoader {
    /// Starts a chain with one loader.
    pub fn new<L>(loader: L) -> Self
    where
        L: ConfigLoader + 'static,
    {
        Self {
            loaders: vec![Box::new(loader)],
        }
    }

    /// Appends another loader to the chain.
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

/// Materialized config layers grouped by source priority.
#[derive(Debug, Clone, Default)]
pub struct LoadedLayers {
    /// Built-in defaults loaded before any user input.
    pub defaults: ConfigLayer,
    /// Presentation-specific defaults layered above built-ins.
    pub presentation: ConfigLayer,
    /// Values loaded from the ordinary config file.
    pub file: ConfigLayer,
    /// Values loaded from the secrets store.
    pub secrets: ConfigLayer,
    /// Values loaded from environment variables.
    pub env: ConfigLayer,
    /// Values synthesized from CLI flags and arguments.
    pub cli: ConfigLayer,
    /// In-memory session overrides.
    pub session: ConfigLayer,
}

/// Builder for the standard multi-source config loading pipeline.
#[must_use]
pub struct LoaderPipeline {
    defaults: Box<dyn ConfigLoader>,
    presentation: Option<Box<dyn ConfigLoader>>,
    file: Option<Box<dyn ConfigLoader>>,
    secrets: Option<Box<dyn ConfigLoader>>,
    env: Option<Box<dyn ConfigLoader>>,
    cli: Option<Box<dyn ConfigLoader>>,
    session: Option<Box<dyn ConfigLoader>>,
    schema: ConfigSchema,
}

impl LoaderPipeline {
    /// Creates a pipeline with the required defaults loader.
    pub fn new<L>(defaults: L) -> Self
    where
        L: ConfigLoader + 'static,
    {
        Self {
            defaults: Box::new(defaults),
            presentation: None,
            file: None,
            secrets: None,
            env: None,
            cli: None,
            session: None,
            schema: ConfigSchema::default(),
        }
    }

    /// Adds the ordinary config file loader.
    pub fn with_file<L>(mut self, loader: L) -> Self
    where
        L: ConfigLoader + 'static,
    {
        self.file = Some(Box::new(loader));
        self
    }

    /// Adds the presentation defaults loader.
    pub fn with_presentation<L>(mut self, loader: L) -> Self
    where
        L: ConfigLoader + 'static,
    {
        self.presentation = Some(Box::new(loader));
        self
    }

    /// Adds the secrets loader.
    pub fn with_secrets<L>(mut self, loader: L) -> Self
    where
        L: ConfigLoader + 'static,
    {
        self.secrets = Some(Box::new(loader));
        self
    }

    /// Adds the environment loader.
    pub fn with_env<L>(mut self, loader: L) -> Self
    where
        L: ConfigLoader + 'static,
    {
        self.env = Some(Box::new(loader));
        self
    }

    /// Adds the CLI override loader.
    pub fn with_cli<L>(mut self, loader: L) -> Self
    where
        L: ConfigLoader + 'static,
    {
        self.cli = Some(Box::new(loader));
        self
    }

    /// Adds the session override loader.
    pub fn with_session<L>(mut self, loader: L) -> Self
    where
        L: ConfigLoader + 'static,
    {
        self.session = Some(Box::new(loader));
        self
    }

    /// Replaces the schema used during resolution.
    pub fn with_schema(mut self, schema: ConfigSchema) -> Self {
        self.schema = schema;
        self
    }

    /// Loads every configured source into concrete layers.
    pub fn load_layers(&self) -> Result<LoadedLayers, ConfigError> {
        tracing::debug!("loading config layers");
        let layers = LoadedLayers {
            defaults: self.defaults.load()?,
            presentation: load_optional_loader(self.presentation.as_deref())?,
            file: load_optional_loader(self.file.as_deref())?,
            secrets: load_optional_loader(self.secrets.as_deref())?,
            env: load_optional_loader(self.env.as_deref())?,
            cli: load_optional_loader(self.cli.as_deref())?,
            session: load_optional_loader(self.session.as_deref())?,
        };
        tracing::debug!(
            defaults = layers.defaults.entries().len(),
            presentation = layers.presentation.entries().len(),
            file = layers.file.entries().len(),
            secrets = layers.secrets.entries().len(),
            env = layers.env.entries().len(),
            cli = layers.cli.entries().len(),
            session = layers.session.entries().len(),
            "loaded config layers"
        );
        Ok(layers)
    }

    /// Loads all layers and resolves them into a runtime config.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::config::{ConfigLayer, LoaderPipeline, ResolveOptions, StaticLayerLoader};
    ///
    /// let mut defaults = ConfigLayer::default();
    /// defaults.set("profile.default", "default");
    /// defaults.set("theme.name", "dracula");
    ///
    /// let resolved = LoaderPipeline::new(StaticLayerLoader::new(defaults))
    ///     .resolve(ResolveOptions::default())
    ///     .unwrap();
    ///
    /// assert_eq!(resolved.get_string("theme.name"), Some("dracula"));
    /// ```
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
mod tests;
