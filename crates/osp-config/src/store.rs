use std::path::{Path, PathBuf};

use crate::{ConfigError, ConfigValue, Scope, TomlSetResult, normalize_scope, with_path_context};

pub fn set_scoped_value_in_toml(
    path: &Path,
    key: &str,
    value: &ConfigValue,
    scope: &Scope,
    dry_run: bool,
    strict_secret_permissions: bool,
) -> Result<TomlSetResult, ConfigError> {
    let normalized_scope = normalize_scope(scope.clone());
    let mut root = load_or_create_toml_root(path)?;
    let root_table = root
        .as_table_mut()
        .ok_or(ConfigError::TomlRootMustBeTable)?;

    let scoped_table = scoped_table_mut(root_table, &normalized_scope)?;
    let previous = set_dotted_value(scoped_table, key, value)?;

    if !dry_run {
        write_toml_root(path, &root, strict_secret_permissions)?;
    }

    Ok(TomlSetResult { previous })
}

fn load_or_create_toml_root(path: &Path) -> Result<toml::Value, ConfigError> {
    if !path.exists() {
        return Ok(toml::Value::Table(toml::value::Table::new()));
    }

    let raw = std::fs::read_to_string(path).map_err(|err| ConfigError::FileRead {
        path: path.display().to_string(),
        reason: err.to_string(),
    })?;

    raw.parse::<toml::Value>().map_err(|err| {
        with_path_context(
            path.display().to_string(),
            ConfigError::TomlParse(err.to_string()),
        )
    })
}

fn write_toml_root(
    path: &Path,
    root: &toml::Value,
    strict_secret_permissions: bool,
) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| ConfigError::FileWrite {
            path: parent.display().to_string(),
            reason: err.to_string(),
        })?;
    }

    let payload =
        toml::to_string_pretty(root).map_err(|err| ConfigError::TomlParse(err.to_string()))?;
    std::fs::write(path, payload).map_err(|err| ConfigError::FileWrite {
        path: path.display().to_string(),
        reason: err.to_string(),
    })?;

    if strict_secret_permissions {
        set_permissions_600(path)?;
    }

    Ok(())
}

fn scoped_table_mut<'a>(
    root: &'a mut toml::value::Table,
    scope: &Scope,
) -> Result<&'a mut toml::value::Table, ConfigError> {
    match (scope.profile.as_deref(), scope.terminal.as_deref()) {
        (None, None) => ensure_table(root, "default"),
        (Some(profile), None) => {
            let profiles = ensure_table(root, "profile")?;
            ensure_table(profiles, profile)
        }
        (None, Some(terminal)) => {
            let terminals = ensure_table(root, "terminal")?;
            ensure_table(terminals, terminal)
        }
        (Some(profile), Some(terminal)) => {
            let terminals = ensure_table(root, "terminal")?;
            let terminal_table = ensure_table(terminals, terminal)?;
            let profile_table = ensure_table(terminal_table, "profile")?;
            ensure_table(profile_table, profile)
        }
    }
}

fn ensure_table<'a>(
    table: &'a mut toml::value::Table,
    key: &str,
) -> Result<&'a mut toml::value::Table, ConfigError> {
    let entry = table
        .entry(key.to_string())
        .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
    match entry {
        toml::Value::Table(inner) => Ok(inner),
        _ => Err(ConfigError::InvalidSection {
            section: key.to_string(),
            expected: "table".to_string(),
        }),
    }
}

fn set_dotted_value(
    table: &mut toml::value::Table,
    dotted_key: &str,
    value: &ConfigValue,
) -> Result<Option<ConfigValue>, ConfigError> {
    let parts = dotted_key
        .split('.')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<&str>>();

    if parts.is_empty() {
        return Err(ConfigError::InvalidConfigKey {
            key: dotted_key.to_string(),
            reason: "empty key path".to_string(),
        });
    }

    let mut cursor = table;
    for key in &parts[..parts.len() - 1] {
        cursor = ensure_table(cursor, key)?;
    }

    let leaf = parts[parts.len() - 1];
    let previous = cursor
        .insert(leaf.to_string(), config_value_to_toml(value))
        .and_then(|existing| ConfigValue::from_toml(dotted_key, &existing).ok());

    Ok(previous)
}

fn config_value_to_toml(value: &ConfigValue) -> toml::Value {
    match value {
        ConfigValue::String(v) => toml::Value::String(v.clone()),
        ConfigValue::Bool(v) => toml::Value::Boolean(*v),
        ConfigValue::Integer(v) => toml::Value::Integer(*v),
        ConfigValue::Float(v) => toml::Value::Float(*v),
        ConfigValue::List(values) => {
            toml::Value::Array(values.iter().map(config_value_to_toml).collect())
        }
        ConfigValue::Secret(secret) => config_value_to_toml(secret.expose()),
    }
}

#[cfg(unix)]
pub(crate) fn validate_secrets_permissions(
    path: &PathBuf,
    strict: bool,
) -> Result<(), ConfigError> {
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
pub(crate) fn validate_secrets_permissions(
    _path: &PathBuf,
    _strict: bool,
) -> Result<(), ConfigError> {
    Ok(())
}

#[cfg(unix)]
fn set_permissions_600(path: &Path) -> Result<(), ConfigError> {
    use std::os::unix::fs::PermissionsExt;

    let mut perms = std::fs::metadata(path)
        .map_err(|err| ConfigError::FileWrite {
            path: path.display().to_string(),
            reason: err.to_string(),
        })?
        .permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(path, perms).map_err(|err| ConfigError::FileWrite {
        path: path.display().to_string(),
        reason: err.to_string(),
    })
}

#[cfg(not(unix))]
fn set_permissions_600(_path: &Path) -> Result<(), ConfigError> {
    Ok(())
}
