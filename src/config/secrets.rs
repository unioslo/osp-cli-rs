//! Persistent secret backends used by runtime config loading and mutation.
//!
//! The config subsystem owns backend selection, scope mapping, and persistence.
//! Product integrations only consume the resulting resolved secret values; they
//! do not discover credential files or keyring entries themselves.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::config::{
    ConfigError, ConfigLayer, ConfigLoader, ConfigSchema, ConfigValue, EnvSecretsLoader,
    LoadedLayers, RuntimeConfigPaths, Scope, SecretsTomlLoader, TomlEditResult,
    TomlStoreEditOptions, normalize_scope, set_scoped_value_in_toml, unset_scoped_value_in_toml,
    validate_key_scope, write_text_atomic,
};

const KEYRING_INDEX_VERSION: u32 = 1;
const KEYRING_SERVICE: &str = "osp-cli:secrets:v1";

/// Supported persistent secrets backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretBackendKind {
    /// Owner-only TOML file.
    Toml,
    /// Native platform credential store with a value-free TOML index.
    Keyring,
}

impl SecretBackendKind {
    /// Parses a resolved config value.
    pub fn parse(value: &str) -> Result<Self, ConfigError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "toml" => Ok(Self::Toml),
            "keyring" => Ok(Self::Keyring),
            other => Err(secret_error(
                "bootstrap",
                format!("unsupported secrets backend `{other}`"),
            )),
        }
    }

    /// Stable config spelling for this backend.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Toml => "toml",
            Self::Keyring => "keyring",
        }
    }
}

/// Backend-neutral result from changing a persisted secret.
#[derive(Debug, Clone, PartialEq)]
pub struct SecretStoreEditResult {
    /// Previously stored value, when present.
    pub previous: Option<ConfigValue>,
    /// Backend that received the operation.
    pub backend: SecretBackendKind,
    /// Human-readable backing-store location.
    pub location: String,
}

/// Runtime-selected persistent secret store.
pub struct RuntimeSecretStore {
    backend: SecretStoreBackend,
}

enum SecretStoreBackend {
    Toml(PathBuf),
    Keyring(KeyringSecretStore),
}

impl RuntimeSecretStore {
    /// Builds the selected store from standard runtime paths.
    pub fn from_paths(
        backend: SecretBackendKind,
        paths: &RuntimeConfigPaths,
    ) -> Result<Self, ConfigError> {
        let backend = match backend {
            SecretBackendKind::Toml => SecretStoreBackend::Toml(
                paths
                    .secrets_file
                    .clone()
                    .ok_or_else(|| secret_error("toml", "secrets file path is unavailable"))?,
            ),
            SecretBackendKind::Keyring => SecretStoreBackend::Keyring(KeyringSecretStore::new(
                paths
                    .secrets_index_file
                    .clone()
                    .ok_or_else(|| secret_error("keyring", "secrets index path is unavailable"))?,
                Arc::new(PlatformCredentialStore),
            )),
        };
        Ok(Self { backend })
    }

    /// Loads all scoped values as one secret-marked config layer.
    pub fn load_layer(&self) -> Result<ConfigLayer, ConfigError> {
        match &self.backend {
            SecretStoreBackend::Toml(path) => {
                SecretsTomlLoader::new(path.clone()).optional().load()
            }
            SecretStoreBackend::Keyring(store) => store.load_layer(),
        }
    }

    /// Sets one validated scoped value.
    pub fn set_scoped(
        &self,
        key: &str,
        value: &ConfigValue,
        scope: &Scope,
        options: TomlStoreEditOptions,
    ) -> Result<SecretStoreEditResult, ConfigError> {
        let value = validate_secret_write(key, value, scope)?;
        match &self.backend {
            SecretStoreBackend::Toml(path) => {
                let TomlEditResult { previous } =
                    set_scoped_value_in_toml(path, key, &value, scope, options.for_secrets())?;
                Ok(SecretStoreEditResult {
                    previous,
                    backend: SecretBackendKind::Toml,
                    location: path.display().to_string(),
                })
            }
            SecretStoreBackend::Keyring(store) => {
                store.set_scoped(key, &value, scope, !options.should_write())
            }
        }
    }

    /// Removes one scoped value.
    pub fn unset_scoped(
        &self,
        key: &str,
        scope: &Scope,
        options: TomlStoreEditOptions,
    ) -> Result<SecretStoreEditResult, ConfigError> {
        validate_secret_key_scope(key, scope)?;
        match &self.backend {
            SecretStoreBackend::Toml(path) => {
                let TomlEditResult { previous } =
                    unset_scoped_value_in_toml(path, key, scope, options.for_secrets())?;
                Ok(SecretStoreEditResult {
                    previous,
                    backend: SecretBackendKind::Toml,
                    location: path.display().to_string(),
                })
            }
            SecretStoreBackend::Keyring(store) => {
                store.unset_scoped(key, scope, !options.should_write())
            }
        }
    }
}

/// Loader that selects one persistent backend from already loaded non-secret
/// bootstrap layers, then overlays `OSP_SECRET__...` values.
pub(crate) struct RuntimeSelectedSecretsLoader {
    paths: RuntimeConfigPaths,
    env: Option<EnvSecretsLoader>,
}

impl RuntimeSelectedSecretsLoader {
    pub(crate) fn new(paths: RuntimeConfigPaths, env: Option<EnvSecretsLoader>) -> Self {
        Self { paths, env }
    }

    pub(crate) fn load(&self, layers: &LoadedLayers) -> Result<ConfigLayer, ConfigError> {
        let backend = selected_backend_from_non_secret_layers(layers)?;
        let mut layer = RuntimeSecretStore::from_paths(backend, &self.paths)?.load_layer()?;
        if let Some(env) = &self.env {
            layer.entries.extend(env.load()?.entries);
        }
        Ok(layer)
    }
}

fn selected_backend_from_non_secret_layers(
    layers: &LoadedLayers,
) -> Result<SecretBackendKind, ConfigError> {
    let mut selected = None;
    for layer in [
        &layers.defaults,
        &layers.presentation,
        &layers.file,
        &layers.env,
        &layers.cli,
        &layers.session,
    ] {
        for entry in &layer.entries {
            if entry.key.eq_ignore_ascii_case("secrets.backend") {
                validate_key_scope(&entry.key, &entry.scope)?;
                let ConfigValue::String(value) = entry.value.reveal() else {
                    return Err(secret_error(
                        "bootstrap",
                        "secrets.backend must be a string",
                    ));
                };
                selected = Some(SecretBackendKind::parse(value)?);
            }
        }
    }
    Ok(selected.unwrap_or(SecretBackendKind::Toml))
}

fn validate_secret_write(
    key: &str,
    value: &ConfigValue,
    scope: &Scope,
) -> Result<ConfigValue, ConfigError> {
    validate_secret_key_scope(key, scope)?;
    ConfigSchema::default().validate_write_value(key, value)
}

fn validate_secret_key_scope(key: &str, scope: &Scope) -> Result<(), ConfigError> {
    ConfigSchema::default().validate_writable_key(key)?;
    validate_key_scope(key, &normalize_scope(scope.clone()))
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyringIndexEntry {
    key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    terminal: Option<String>,
}

impl KeyringIndexEntry {
    fn new(key: &str, scope: &Scope) -> Self {
        let scope = normalize_scope(scope.clone());
        Self {
            key: key.trim().to_ascii_lowercase(),
            profile: scope.profile,
            terminal: scope.terminal,
        }
    }

    fn scope(&self) -> Scope {
        Scope {
            profile: self.profile.clone(),
            terminal: self.terminal.clone(),
        }
    }

    fn username(&self) -> Result<String, ConfigError> {
        serde_json::to_string(&(&self.profile, &self.terminal, &self.key))
            .map_err(|err| secret_error("keyring", err.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyringIndex {
    version: u32,
    #[serde(default)]
    entries: Vec<KeyringIndexEntry>,
}

impl Default for KeyringIndex {
    fn default() -> Self {
        Self {
            version: KEYRING_INDEX_VERSION,
            entries: Vec::new(),
        }
    }
}

impl KeyringIndex {
    fn normalize(&mut self) {
        self.entries.sort();
        self.entries.dedup();
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredSecretEnvelope {
    version: u32,
    value: StoredConfigValue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
enum StoredConfigValue {
    String(String),
    Bool(bool),
    Integer(i64),
    Float(f64),
    List(Vec<StoredConfigValue>),
}

impl StoredConfigValue {
    fn from_config(value: &ConfigValue) -> Self {
        match value.reveal() {
            ConfigValue::String(value) => Self::String(value.clone()),
            ConfigValue::Bool(value) => Self::Bool(*value),
            ConfigValue::Integer(value) => Self::Integer(*value),
            ConfigValue::Float(value) => Self::Float(*value),
            ConfigValue::List(values) => Self::List(values.iter().map(Self::from_config).collect()),
            ConfigValue::Secret(secret) => Self::from_config(secret.expose()),
        }
    }

    fn into_config(self) -> ConfigValue {
        match self {
            Self::String(value) => ConfigValue::String(value),
            Self::Bool(value) => ConfigValue::Bool(value),
            Self::Integer(value) => ConfigValue::Integer(value),
            Self::Float(value) => ConfigValue::Float(value),
            Self::List(values) => {
                ConfigValue::List(values.into_iter().map(Self::into_config).collect())
            }
        }
    }
}

fn encode_secret(value: &ConfigValue) -> Result<String, ConfigError> {
    serde_json::to_string(&StoredSecretEnvelope {
        version: 1,
        value: StoredConfigValue::from_config(value),
    })
    .map_err(|err| secret_error("keyring", format!("failed to encode secret: {err}")))
}

fn decode_secret(raw: &str) -> Result<ConfigValue, ConfigError> {
    let envelope: StoredSecretEnvelope = serde_json::from_str(raw)
        .map_err(|err| secret_error("keyring", format!("invalid secret envelope: {err}")))?;
    if envelope.version != 1 {
        return Err(secret_error(
            "keyring",
            format!("unsupported secret envelope version {}", envelope.version),
        ));
    }
    Ok(envelope.value.into_config())
}

trait CredentialStore: Send + Sync {
    fn get(&self, service: &str, username: &str) -> Result<Option<String>, ConfigError>;
    fn set(&self, service: &str, username: &str, value: &str) -> Result<(), ConfigError>;
    fn delete(&self, service: &str, username: &str) -> Result<(), ConfigError>;
}

struct PlatformCredentialStore;

impl PlatformCredentialStore {
    fn entry(service: &str, username: &str) -> Result<keyring::Entry, ConfigError> {
        keyring::Entry::new(service, username)
            .map_err(|err| secret_error("keyring", err.to_string()))
    }
}

impl CredentialStore for PlatformCredentialStore {
    fn get(&self, service: &str, username: &str) -> Result<Option<String>, ConfigError> {
        match Self::entry(service, username)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(secret_error("keyring", err.to_string())),
        }
    }

    fn set(&self, service: &str, username: &str, value: &str) -> Result<(), ConfigError> {
        Self::entry(service, username)?
            .set_password(value)
            .map_err(|err| secret_error("keyring", err.to_string()))
    }

    fn delete(&self, service: &str, username: &str) -> Result<(), ConfigError> {
        match Self::entry(service, username)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(secret_error("keyring", err.to_string())),
        }
    }
}

struct KeyringSecretStore {
    index_path: PathBuf,
    credentials: Arc<dyn CredentialStore>,
}

impl KeyringSecretStore {
    fn new(index_path: PathBuf, credentials: Arc<dyn CredentialStore>) -> Self {
        Self {
            index_path,
            credentials,
        }
    }

    fn load_layer(&self) -> Result<ConfigLayer, ConfigError> {
        let index = self.read_index()?;
        let mut layer = ConfigLayer::default();
        for entry in index.entries {
            let username = entry.username()?;
            let Some(raw) = self.credentials.get(KEYRING_SERVICE, &username)? else {
                tracing::warn!(key = %entry.key, "keyring index entry has no credential");
                continue;
            };
            let scope = entry.scope();
            layer.insert_with_origin(
                entry.key,
                decode_secret(&raw)?.into_secret(),
                scope,
                Some(format!("keyring:{KEYRING_SERVICE}")),
            );
        }
        Ok(layer)
    }

    fn set_scoped(
        &self,
        key: &str,
        value: &ConfigValue,
        scope: &Scope,
        dry_run: bool,
    ) -> Result<SecretStoreEditResult, ConfigError> {
        let entry = KeyringIndexEntry::new(key, scope);
        let username = entry.username()?;
        let previous_raw = self.credentials.get(KEYRING_SERVICE, &username)?;
        let previous = previous_raw.as_deref().map(decode_secret).transpose()?;
        if !dry_run {
            let mut index = self.read_index()?;
            let encoded = encode_secret(value)?;
            self.credentials.set(KEYRING_SERVICE, &username, &encoded)?;
            if !index.entries.contains(&entry) {
                index.entries.push(entry);
                index.normalize();
            }
            if let Err(index_error) = self.write_index(&index) {
                let rollback = match previous_raw {
                    Some(previous) => self.credentials.set(KEYRING_SERVICE, &username, &previous),
                    None => self.credentials.delete(KEYRING_SERVICE, &username),
                };
                return Err(rollback_error(index_error, rollback));
            }
        }
        Ok(SecretStoreEditResult {
            previous,
            backend: SecretBackendKind::Keyring,
            location: self.index_path.display().to_string(),
        })
    }

    fn unset_scoped(
        &self,
        key: &str,
        scope: &Scope,
        dry_run: bool,
    ) -> Result<SecretStoreEditResult, ConfigError> {
        let entry = KeyringIndexEntry::new(key, scope);
        let username = entry.username()?;
        let previous_raw = self.credentials.get(KEYRING_SERVICE, &username)?;
        let previous = previous_raw.as_deref().map(decode_secret).transpose()?;
        if !dry_run {
            let mut index = self.read_index()?;
            let had_index_entry = index.entries.contains(&entry);
            if previous_raw.is_some() {
                self.credentials.delete(KEYRING_SERVICE, &username)?;
            }
            index.entries.retain(|candidate| candidate != &entry);
            if had_index_entry && let Err(index_error) = self.write_index(&index) {
                let rollback = previous_raw.as_deref().map_or(Ok(()), |previous| {
                    self.credentials.set(KEYRING_SERVICE, &username, previous)
                });
                return Err(rollback_error(index_error, rollback));
            }
        }
        Ok(SecretStoreEditResult {
            previous,
            backend: SecretBackendKind::Keyring,
            location: self.index_path.display().to_string(),
        })
    }

    fn read_index(&self) -> Result<KeyringIndex, ConfigError> {
        if !self.index_path.exists() {
            return Ok(KeyringIndex::default());
        }
        let raw = fs::read_to_string(&self.index_path).map_err(|err| {
            secret_error(
                "keyring",
                format!("failed to read {}: {err}", self.index_path.display()),
            )
        })?;
        let index: KeyringIndex = toml::from_str(&raw).map_err(|err| {
            secret_error(
                "keyring",
                format!("failed to parse {}: {err}", self.index_path.display()),
            )
        })?;
        if index.version != KEYRING_INDEX_VERSION {
            return Err(secret_error(
                "keyring",
                format!("unsupported keyring index version {}", index.version),
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for entry in &index.entries {
            validate_secret_key_scope(&entry.key, &entry.scope())?;
            if entry != &KeyringIndexEntry::new(&entry.key, &entry.scope()) {
                return Err(secret_error(
                    "keyring",
                    "keyring index entries must use normalized keys and scopes",
                ));
            }
            if !seen.insert(entry.clone()) {
                return Err(secret_error(
                    "keyring",
                    "keyring index contains a duplicate scoped key",
                ));
            }
        }
        Ok(index)
    }

    fn write_index(&self, index: &KeyringIndex) -> Result<(), ConfigError> {
        let raw = toml::to_string_pretty(index)
            .map_err(|err| secret_error("keyring", format!("failed to encode index: {err}")))?;
        write_owner_only_atomic(&self.index_path, raw.as_bytes())
    }
}

fn rollback_error(index_error: ConfigError, rollback: Result<(), ConfigError>) -> ConfigError {
    match rollback {
        Ok(()) => index_error,
        Err(rollback) => secret_error(
            "keyring",
            format!("{index_error}; credential rollback also failed: {rollback}"),
        ),
    }
}

fn write_owner_only_atomic(path: &Path, bytes: &[u8]) -> Result<(), ConfigError> {
    let parent = path
        .parent()
        .ok_or_else(|| secret_error("keyring", "index path has no parent"))?;
    fs::create_dir_all(parent).map_err(|err| {
        secret_error(
            "keyring",
            format!("failed to create {}: {err}", parent.display()),
        )
    })?;
    write_text_atomic(path, bytes, true).map_err(|err| {
        secret_error(
            "keyring",
            format!("failed to atomically write {}: {err}", path.display()),
        )
    })
}

fn secret_error(backend: &str, reason: impl Into<String>) -> ConfigError {
    ConfigError::SecretBackend {
        backend: backend.to_string(),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[derive(Default)]
    struct FakeCredentials {
        values: Mutex<BTreeMap<(String, String), String>>,
    }

    impl CredentialStore for FakeCredentials {
        fn get(&self, service: &str, username: &str) -> Result<Option<String>, ConfigError> {
            Ok(self
                .values
                .lock()
                .unwrap()
                .get(&(service.to_string(), username.to_string()))
                .cloned())
        }

        fn set(&self, service: &str, username: &str, value: &str) -> Result<(), ConfigError> {
            self.values.lock().unwrap().insert(
                (service.to_string(), username.to_string()),
                value.to_string(),
            );
            Ok(())
        }

        fn delete(&self, service: &str, username: &str) -> Result<(), ConfigError> {
            self.values
                .lock()
                .unwrap()
                .remove(&(service.to_string(), username.to_string()));
            Ok(())
        }
    }

    fn temp_index() -> PathBuf {
        std::env::temp_dir().join(format!(
            "osp-keyring-index-{}-{}.toml",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn keyring_index_round_trips_all_scope_shapes_without_values() {
        let path = temp_index();
        let credentials = Arc::new(FakeCredentials::default());
        let store = KeyringSecretStore::new(path.clone(), credentials);
        let scopes = [
            Scope::global(),
            Scope::profile("UIO"),
            Scope::terminal("REPL"),
            Scope::profile_terminal("UIO", "REPL"),
        ];

        for (index, scope) in scopes.iter().enumerate() {
            store
                .set_scoped(
                    &format!("extensions.test.token{index}"),
                    &ConfigValue::String(format!("secret-{index}")),
                    scope,
                    false,
                )
                .unwrap();
        }

        let raw = fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("secret-"));
        let layer = store.load_layer().unwrap();
        assert_eq!(layer.entries().len(), 4);
        for (entry, expected_scope) in layer.entries().iter().zip(scopes) {
            assert_eq!(entry.scope, expected_scope);
        }
        assert!(layer.entries().iter().all(|entry| entry.value.is_secret()));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn typed_secret_envelope_round_trips() {
        let values = [
            ConfigValue::String("token".to_string()),
            ConfigValue::Bool(true),
            ConfigValue::Integer(42),
            ConfigValue::Float(1.5),
            ConfigValue::List(vec![ConfigValue::String("a".to_string())]),
        ];

        for value in values {
            assert_eq!(
                decode_secret(&encode_secret(&value).unwrap()).unwrap(),
                value
            );
        }
    }

    #[test]
    fn keyring_index_rejects_unknown_versions_and_duplicate_entries() {
        let path = temp_index();
        fs::write(&path, "version = 2\nentries = []\n").unwrap();
        let store = KeyringSecretStore::new(path.clone(), Arc::new(FakeCredentials::default()));
        assert!(
            store
                .read_index()
                .unwrap_err()
                .to_string()
                .contains("version 2")
        );

        fs::write(
            &path,
            r#"version = 1
[[entries]]
key = "extensions.test.token"
[[entries]]
key = "extensions.test.token"
"#,
        )
        .unwrap();
        assert!(
            store
                .read_index()
                .unwrap_err()
                .to_string()
                .contains("duplicate")
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn keyring_set_and_unset_return_previous_values() {
        let path = temp_index();
        let store = KeyringSecretStore::new(path.clone(), Arc::new(FakeCredentials::default()));
        let scope = Scope::profile("uio");
        let first = ConfigValue::String("first".to_string());
        let second = ConfigValue::String("second".to_string());

        assert_eq!(
            store
                .set_scoped("extensions.test.token", &first, &scope, false)
                .unwrap()
                .previous,
            None
        );
        assert_eq!(
            store
                .set_scoped("extensions.test.token", &second, &scope, false)
                .unwrap()
                .previous,
            Some(first)
        );
        assert_eq!(
            store
                .unset_scoped("extensions.test.token", &scope, false)
                .unwrap()
                .previous,
            Some(second)
        );
        assert!(store.load_layer().unwrap().entries().is_empty());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn keyring_load_skips_index_entries_deleted_outside_the_cli() {
        let path = temp_index();
        let credentials = Arc::new(FakeCredentials::default());
        let store = KeyringSecretStore::new(path.clone(), credentials.clone());
        let scope = Scope::global();
        store
            .set_scoped(
                "extensions.test.token",
                &ConfigValue::String("secret".to_string()),
                &scope,
                false,
            )
            .unwrap();
        let entry = KeyringIndexEntry::new("extensions.test.token", &scope);
        credentials
            .delete(KEYRING_SERVICE, &entry.username().unwrap())
            .unwrap();

        assert!(store.load_layer().unwrap().entries().is_empty());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn keyring_set_rolls_back_credential_when_index_write_fails() {
        let blocker = temp_index();
        fs::write(&blocker, "not a directory").unwrap();
        let credentials = Arc::new(FakeCredentials::default());
        let store = KeyringSecretStore::new(blocker.join("index.toml"), credentials.clone());

        let error = store
            .set_scoped(
                "extensions.test.token",
                &ConfigValue::String("must-not-remain".to_string()),
                &Scope::global(),
                false,
            )
            .expect_err("index write should fail");

        assert!(error.to_string().contains("failed to"));
        assert!(credentials.values.lock().unwrap().is_empty());
        let _ = fs::remove_file(blocker);
    }

    #[cfg(unix)]
    #[test]
    fn keyring_index_is_owner_only() {
        let path = temp_index();
        let store = KeyringSecretStore::new(path.clone(), Arc::new(FakeCredentials::default()));
        store
            .set_scoped(
                "extensions.test.token",
                &ConfigValue::String("secret".to_string()),
                &Scope::global(),
                false,
            )
            .unwrap();

        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let _ = fs::remove_file(path);
    }

    #[test]
    fn backend_selection_uses_only_non_secret_source_precedence() {
        let mut layers = LoadedLayers::default();
        layers.defaults.set("secrets.backend", "toml");
        layers.file.set("secrets.backend", "keyring");
        layers.secrets.set("secrets.backend", "toml");

        assert_eq!(
            selected_backend_from_non_secret_layers(&layers).unwrap(),
            SecretBackendKind::Keyring
        );

        layers.env.set("secrets.backend", "toml");
        assert_eq!(
            selected_backend_from_non_secret_layers(&layers).unwrap(),
            SecretBackendKind::Toml
        );
    }

    #[test]
    fn backend_selection_rejects_scoped_values() {
        let mut layers = LoadedLayers::default();
        layers
            .file
            .insert("secrets.backend", "keyring", Scope::profile("uio"));

        assert!(selected_backend_from_non_secret_layers(&layers).is_err());
    }
}
