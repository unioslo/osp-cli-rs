use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::{
    ConfigError, ConfigValue, Scope, TomlEditResult, normalize_scope, validate_bootstrap_value,
    validate_key_scope, with_path_context,
};

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
    crate::config::ConfigSchema::default().validate_writable_key(key)?;
    validate_key_scope(key, &normalized_scope)?;
    if let TomlEditOperation::Set(value) = operation {
        validate_bootstrap_value(key, value)?;
    }
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
    write_text_atomic(path, payload.as_bytes(), strict_secret_permissions).map_err(|err| {
        ConfigError::FileWrite {
            path: path.display().to_string(),
            reason: err.to_string(),
        }
    })?;

    Ok(())
}

pub(crate) fn write_text_atomic(
    path: &Path,
    payload: &[u8],
    strict_secret_permissions: bool,
) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("path has no file name: {}", path.display()),
        )
    })?;
    let pid = std::process::id();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    for attempt in 0..16u8 {
        let temp_name = format!(
            ".{}.tmp-{pid}-{nonce}-{attempt}",
            file_name.to_string_lossy()
        );
        let temp_path = parent.join(temp_name);
        match create_temp_file(&temp_path, strict_secret_permissions) {
            Ok(mut file) => {
                file.write_all(payload)?;
                file.sync_all()?;
                drop(file);
                replace_file_atomic(&temp_path, path)?;
                sync_parent_dir(parent)?;
                return Ok(());
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        format!("failed to allocate temp file for {}", path.display()),
    ))
}

#[cfg(not(windows))]
fn replace_file_atomic(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(source, destination)
}

#[cfg(windows)]
fn replace_file_atomic(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;

    unsafe extern "system" {
        fn MoveFileExW(
            lp_existing_file_name: *const u16,
            lp_new_file_name: *const u16,
            dw_flags: u32,
        ) -> i32;
    }

    let source_wide = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination_wide = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();

    let replaced = unsafe {
        MoveFileExW(
            source_wide.as_ptr(),
            destination_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if replaced != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(unix)]
fn sync_parent_dir(path: &Path) -> std::io::Result<()> {
    std::fs::File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_dir(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn create_temp_file(
    path: &Path,
    strict_secret_permissions: bool,
) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;

    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    if strict_secret_permissions {
        options.mode(0o600);
    }
    options.open(path)
}

#[cfg(not(unix))]
fn create_temp_file(
    path: &Path,
    _strict_secret_permissions: bool,
) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    options.open(path)
}

#[cfg(unix)]
pub fn secret_file_mode(path: &Path) -> Result<u32, ConfigError> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = std::fs::metadata(path).map_err(|err| ConfigError::FileRead {
        path: path.display().to_string(),
        reason: err.to_string(),
    })?;
    Ok(metadata.permissions().mode() & 0o777)
}

fn scoped_table_mut<'a>(
    root: &'a mut toml::value::Table,
    scope: &Scope,
) -> Result<&'a mut toml::value::Table, ConfigError> {
    ensure_table_path(root, &scope_path(scope))
}

fn scoped_table<'a>(
    root: &'a toml::value::Table,
    scope: &Scope,
) -> Result<Option<&'a toml::value::Table>, ConfigError> {
    get_table_path(root, &scope_path(scope))
}

fn scope_path(scope: &Scope) -> Vec<&str> {
    match (scope.profile.as_deref(), scope.terminal.as_deref()) {
        (None, None) => vec!["default"],
        (Some(profile), None) => vec!["profile", profile],
        (None, Some(terminal)) => vec!["terminal", terminal],
        (Some(profile), Some(terminal)) => vec!["terminal", terminal, "profile", profile],
    }
}

fn ensure_table_path<'a>(
    table: &'a mut toml::value::Table,
    path: &[&str],
) -> Result<&'a mut toml::value::Table, ConfigError> {
    let mut cursor = table;
    for section in path {
        cursor = ensure_table(cursor, section)?;
    }
    Ok(cursor)
}

fn get_table_path<'a>(
    table: &'a toml::value::Table,
    path: &[&str],
) -> Result<Option<&'a toml::value::Table>, ConfigError> {
    let mut cursor = table;
    for section in path {
        let Some(next) = get_table(cursor, section)? else {
            return Ok(None);
        };
        cursor = next;
    }
    Ok(Some(cursor))
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
    let table = ensure_table_path(root, &scope_path(scope))?;

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
    prune_empty_table_path(root, &scope_path(scope))?;
    Ok(())
}

fn prune_empty_table_path(
    table: &mut toml::value::Table,
    path: &[&str],
) -> Result<(), ConfigError> {
    let Some((head, tail)) = path.split_first() else {
        return Ok(());
    };
    if tail.is_empty() {
        remove_empty_table(table, head);
        return Ok(());
    }

    let should_remove = if let Some(value) = table.get_mut(*head) {
        let child = as_table_mut(value, head)?;
        prune_empty_table_path(child, tail)?;
        child.is_empty()
    } else {
        false
    };
    if should_remove {
        table.remove(*head);
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

#[cfg(test)]
mod tests;
