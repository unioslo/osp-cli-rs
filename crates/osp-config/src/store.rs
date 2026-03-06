use std::path::{Path, PathBuf};

use crate::{ConfigError, ConfigValue, Scope, TomlEditResult, normalize_scope, with_path_context};

pub fn set_scoped_value_in_toml(
    path: &Path,
    key: &str,
    value: &ConfigValue,
    scope: &Scope,
    dry_run: bool,
    strict_secret_permissions: bool,
) -> Result<TomlEditResult, ConfigError> {
    edit_scoped_value_in_toml(
        path,
        key,
        scope,
        TomlEditOperation::Set(value),
        dry_run,
        strict_secret_permissions,
    )
}

pub fn unset_scoped_value_in_toml(
    path: &Path,
    key: &str,
    scope: &Scope,
    dry_run: bool,
    strict_secret_permissions: bool,
) -> Result<TomlEditResult, ConfigError> {
    edit_scoped_value_in_toml(
        path,
        key,
        scope,
        TomlEditOperation::Unset,
        dry_run,
        strict_secret_permissions,
    )
}

enum TomlEditOperation<'a> {
    Set(&'a ConfigValue),
    Unset,
}

fn edit_scoped_value_in_toml(
    path: &Path,
    key: &str,
    scope: &Scope,
    operation: TomlEditOperation<'_>,
    dry_run: bool,
    strict_secret_permissions: bool,
) -> Result<TomlEditResult, ConfigError> {
    let normalized_scope = normalize_scope(scope.clone());
    let mut root = load_or_create_toml_root(path)?;
    let root_table = root
        .as_table_mut()
        .ok_or(ConfigError::TomlRootMustBeTable)?;

    let previous = match operation {
        TomlEditOperation::Set(value) => {
            let scoped_table = scoped_table_mut(root_table, &normalized_scope)?;
            set_dotted_value(scoped_table, key, value)?
        }
        TomlEditOperation::Unset => unset_dotted_value(root_table, &normalized_scope, key)?,
    };

    if !dry_run {
        write_toml_root(path, &root, strict_secret_permissions)?;
    }

    Ok(TomlEditResult { previous })
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

fn scoped_table<'a>(
    root: &'a toml::value::Table,
    scope: &Scope,
) -> Result<Option<&'a toml::value::Table>, ConfigError> {
    match (scope.profile.as_deref(), scope.terminal.as_deref()) {
        (None, None) => get_table(root, "default"),
        (Some(profile), None) => get_table(root, "profile").and_then(|profiles| match profiles {
            Some(profiles) => get_table(profiles, profile),
            None => Ok(None),
        }),
        (None, Some(terminal)) => {
            get_table(root, "terminal").and_then(|terminals| match terminals {
                Some(terminals) => get_table(terminals, terminal),
                None => Ok(None),
            })
        }
        (Some(profile), Some(terminal)) => {
            let Some(terminals) = get_table(root, "terminal")? else {
                return Ok(None);
            };
            let Some(terminal_table) = get_table(terminals, terminal)? else {
                return Ok(None);
            };
            let Some(profile_table) = get_table(terminal_table, "profile")? else {
                return Ok(None);
            };
            get_table(profile_table, profile)
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

fn get_table<'a>(
    table: &'a toml::value::Table,
    key: &str,
) -> Result<Option<&'a toml::value::Table>, ConfigError> {
    let Some(entry) = table.get(key) else {
        return Ok(None);
    };
    match entry {
        toml::Value::Table(inner) => Ok(Some(inner)),
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

fn unset_dotted_value(
    root: &mut toml::value::Table,
    scope: &Scope,
    dotted_key: &str,
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

    let previous = scoped_table(root, scope)?
        .and_then(|table| read_dotted_value(table, &parts))
        .and_then(|value| ConfigValue::from_toml(dotted_key, value).ok());

    let _ = remove_scoped_value(root, scope, &parts)?;
    prune_empty_scope_tables(root, scope)?;

    Ok(previous)
}

fn remove_scoped_value(
    root: &mut toml::value::Table,
    scope: &Scope,
    parts: &[&str],
) -> Result<bool, ConfigError> {
    let table = match (scope.profile.as_deref(), scope.terminal.as_deref()) {
        (None, None) => ensure_table(root, "default")?,
        (Some(profile), None) => {
            let profiles = ensure_table(root, "profile")?;
            ensure_table(profiles, profile)?
        }
        (None, Some(terminal)) => {
            let terminals = ensure_table(root, "terminal")?;
            ensure_table(terminals, terminal)?
        }
        (Some(profile), Some(terminal)) => {
            let terminals = ensure_table(root, "terminal")?;
            let terminal_table = ensure_table(terminals, terminal)?;
            let profile_table = ensure_table(terminal_table, "profile")?;
            ensure_table(profile_table, profile)?
        }
    };

    remove_dotted_value(table, parts)
}

fn remove_dotted_value(
    table: &mut toml::value::Table,
    parts: &[&str],
) -> Result<bool, ConfigError> {
    if parts.is_empty() {
        return Ok(false);
    }

    if parts.len() == 1 {
        return Ok(table.remove(parts[0]).is_some());
    }

    let Some(entry) = table.get_mut(parts[0]) else {
        return Ok(false);
    };
    let child = match entry {
        toml::Value::Table(inner) => inner,
        _ => {
            return Err(ConfigError::InvalidSection {
                section: parts[0].to_string(),
                expected: "table".to_string(),
            });
        }
    };

    let removed = remove_dotted_value(child, &parts[1..])?;
    if removed && child.is_empty() {
        table.remove(parts[0]);
    }
    Ok(removed)
}

fn prune_empty_scope_tables(
    root: &mut toml::value::Table,
    scope: &Scope,
) -> Result<(), ConfigError> {
    match (scope.profile.as_deref(), scope.terminal.as_deref()) {
        (None, None) => {
            remove_empty_table(root, "default");
        }
        (Some(profile), None) => {
            if let Some(profiles) = root.get_mut("profile") {
                let profiles = as_table_mut(profiles, "profile")?;
                remove_empty_table(profiles, profile);
                if profiles.is_empty() {
                    root.remove("profile");
                }
            }
        }
        (None, Some(terminal)) => {
            if let Some(terminals) = root.get_mut("terminal") {
                let terminals = as_table_mut(terminals, "terminal")?;
                remove_empty_table(terminals, terminal);
                if terminals.is_empty() {
                    root.remove("terminal");
                }
            }
        }
        (Some(profile), Some(terminal)) => {
            if let Some(terminals) = root.get_mut("terminal") {
                let terminals = as_table_mut(terminals, "terminal")?;
                if let Some(terminal_value) = terminals.get_mut(terminal) {
                    let terminal_table = as_table_mut(terminal_value, terminal)?;
                    if let Some(profile_value) = terminal_table.get_mut("profile") {
                        let profile_table = as_table_mut(profile_value, "profile")?;
                        remove_empty_table(profile_table, profile);
                        if profile_table.is_empty() {
                            terminal_table.remove("profile");
                        }
                    }
                    if terminal_table.is_empty() {
                        terminals.remove(terminal);
                    }
                }
                if terminals.is_empty() {
                    root.remove("terminal");
                }
            }
        }
    }

    Ok(())
}

fn remove_empty_table(table: &mut toml::value::Table, key: &str) {
    let should_remove = table
        .get(key)
        .and_then(toml::Value::as_table)
        .is_some_and(|inner| inner.is_empty());
    if should_remove {
        table.remove(key);
    }
}

fn as_table_mut<'a>(
    value: &'a mut toml::Value,
    section: &str,
) -> Result<&'a mut toml::value::Table, ConfigError> {
    match value {
        toml::Value::Table(inner) => Ok(inner),
        _ => Err(ConfigError::InvalidSection {
            section: section.to_string(),
            expected: "table".to_string(),
        }),
    }
}

fn read_dotted_value<'a>(table: &'a toml::value::Table, parts: &[&str]) -> Option<&'a toml::Value> {
    let (head, tail) = parts.split_first()?;
    let value = table.get(*head)?;
    if tail.is_empty() {
        return Some(value);
    }
    read_dotted_value(value.as_table()?, tail)
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
