use super::{set_scoped_value_in_toml, unset_scoped_value_in_toml, validate_secrets_permissions};
use crate::{ConfigError, ConfigValue, Scope};

#[test]
fn dry_run_set_does_not_create_file() {
    let dir = make_temp_dir("osp-config-store-dry-run");
    let path = dir.join("config.toml");

    let result = set_scoped_value_in_toml(
        &path,
        "ui.format",
        &ConfigValue::String("json".to_string()),
        &Scope::global(),
        true,
        false,
    )
    .expect("dry-run set should succeed");

    assert_eq!(result.previous, None);
    assert!(!path.exists());
}

#[test]
fn invalid_toml_is_reported_with_path_context() {
    let dir = make_temp_dir("osp-config-store-invalid");
    let path = dir.join("config.toml");
    std::fs::write(&path, "not = [valid").expect("fixture should be written");

    let err = set_scoped_value_in_toml(
        &path,
        "ui.format",
        &ConfigValue::String("json".to_string()),
        &Scope::global(),
        false,
        false,
    )
    .expect_err("invalid toml should fail");

    match err {
        ConfigError::LayerLoad {
            path: err_path,
            source,
        } => {
            assert!(err_path.contains("config.toml"));
            assert!(matches!(*source, ConfigError::TomlParse(_)));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn set_rejects_non_table_intermediate_section() {
    let dir = make_temp_dir("osp-config-store-non-table");
    let path = dir.join("config.toml");
    std::fs::write(
        &path,
        r#"
[default]
ui = "json"
"#,
    )
    .expect("fixture should be written");

    let err = set_scoped_value_in_toml(
        &path,
        "ui.format",
        &ConfigValue::String("table".to_string()),
        &Scope::global(),
        false,
        false,
    )
    .expect_err("non-table section should fail");

    match err {
        ConfigError::InvalidSection { section, expected } => {
            assert_eq!(section, "ui");
            assert_eq!(expected, "table");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn unset_missing_value_keeps_existing_scope_content() {
    let dir = make_temp_dir("osp-config-store-unset-missing");
    let path = dir.join("config.toml");
    std::fs::write(
        &path,
        r#"
[default]
profile.default = "default"

[terminal.repl]
ui.format = "json"
"#,
    )
    .expect("fixture should be written");

    let result =
        unset_scoped_value_in_toml(&path, "ui.mode", &Scope::terminal("repl"), false, false)
            .expect("unset should succeed");

    assert_eq!(result.previous, None);
    let payload = std::fs::read_to_string(&path).expect("config should be readable");
    let root: toml::Value = payload.parse().expect("written config should stay valid");
    assert_eq!(
        root.get("terminal")
            .and_then(|value| value.get("repl"))
            .and_then(|value| value.get("ui"))
            .and_then(|value| value.get("format"))
            .and_then(toml::Value::as_str),
        Some("json")
    );
}

#[test]
fn set_and_unset_terminal_profile_scope_round_trip() {
    let dir = make_temp_dir("osp-config-store-terminal-profile");
    let path = dir.join("config.toml");

    let scope = Scope {
        profile: Some("tsd".to_string()),
        terminal: Some("repl".to_string()),
    };
    let set_result = set_scoped_value_in_toml(
        &path,
        "ui.format",
        &ConfigValue::String("mreg".to_string()),
        &scope,
        false,
        false,
    )
    .expect("set should succeed");
    assert_eq!(set_result.previous, None);

    let payload = std::fs::read_to_string(&path).expect("config should exist");
    let root: toml::Value = payload.parse().expect("written config should stay valid");
    assert_eq!(
        root.get("terminal")
            .and_then(|value| value.get("repl"))
            .and_then(|value| value.get("profile"))
            .and_then(|value| value.get("tsd"))
            .and_then(|value| value.get("ui"))
            .and_then(|value| value.get("format"))
            .and_then(toml::Value::as_str),
        Some("mreg")
    );

    let unset_result = unset_scoped_value_in_toml(&path, "ui.format", &scope, false, false)
        .expect("unset should succeed");
    assert_eq!(
        unset_result.previous,
        Some(ConfigValue::String("mreg".to_string()))
    );
    let payload = std::fs::read_to_string(&path).expect("config should still be readable");
    let root: toml::Value = payload.parse().expect("written config should stay valid");
    assert!(
        root.get("terminal")
            .and_then(|value| value.get("repl"))
            .and_then(|value| value.get("profile"))
            .and_then(|value| value.get("tsd"))
            .is_none()
    );
}

#[test]
fn unset_prunes_empty_scope_tables_back_to_root() {
    let dir = make_temp_dir("osp-config-store-prune");
    let path = dir.join("config.toml");
    std::fs::write(
        &path,
        r#"
[terminal.repl.profile.ops.ui]
format = "json"
"#,
    )
    .expect("fixture should be written");

    unset_scoped_value_in_toml(
        &path,
        "ui.format",
        &Scope {
            profile: Some("ops".to_string()),
            terminal: Some("repl".to_string()),
        },
        false,
        false,
    )
    .expect("unset should succeed");

    let payload = std::fs::read_to_string(&path).expect("config should be readable");
    let root: toml::Value = payload.parse().expect("written config should stay valid");
    assert!(root.get("terminal").is_none());
}

#[cfg(unix)]
#[test]
fn strict_secret_write_sets_file_mode_to_600() {
    use std::os::unix::fs::PermissionsExt;

    let dir = make_temp_dir("osp-config-store-secret-mode");
    let path = dir.join("secrets.toml");

    set_scoped_value_in_toml(
        &path,
        "extensions.demo.token",
        &ConfigValue::String("secret".to_string()),
        &Scope::global(),
        false,
        true,
    )
    .expect("secret write should succeed");

    let mode = std::fs::metadata(&path)
        .expect("metadata should exist")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    validate_secrets_permissions(&path, true).expect("strict validation should pass");
}

#[cfg(unix)]
#[test]
fn strict_secret_validation_rejects_group_readable_files() {
    use std::os::unix::fs::PermissionsExt;

    let dir = make_temp_dir("osp-config-store-insecure-mode");
    let path = dir.join("secrets.toml");
    std::fs::write(&path, "token = 'secret'\n").expect("fixture should be written");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640))
        .expect("permissions should be set");

    let err =
        validate_secrets_permissions(&path, true).expect_err("insecure permissions should fail");
    match err {
        ConfigError::InsecureSecretsPermissions { mode, .. } => assert_eq!(mode, 0o640),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn non_strict_secret_validation_allows_group_readable_files() {
    use std::os::unix::fs::PermissionsExt;

    let dir = make_temp_dir("osp-config-store-insecure-mode-nonstrict");
    let path = dir.join("secrets.toml");
    std::fs::write(&path, "token = 'secret'\n").expect("fixture should be written");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640))
        .expect("permissions should be set");

    validate_secrets_permissions(&path, false).expect("non-strict validation should pass");
}

#[test]
fn set_returns_previous_value_when_overwriting_existing_entry() {
    let dir = make_temp_dir("osp-config-store-overwrite");
    let path = dir.join("config.toml");
    std::fs::write(
        &path,
        r#"
[default.ui]
format = "json"
"#,
    )
    .expect("fixture should be written");

    let result = set_scoped_value_in_toml(
        &path,
        "ui.format",
        &ConfigValue::String("table".to_string()),
        &Scope::global(),
        false,
        false,
    )
    .expect("overwrite should succeed");

    assert_eq!(
        result.previous,
        Some(ConfigValue::String("json".to_string()))
    );
    let payload = std::fs::read_to_string(&path).expect("config should be readable");
    assert!(payload.contains("format = \"table\""));
}

#[test]
fn unset_rejects_non_table_intermediate_section() {
    let dir = make_temp_dir("osp-config-store-unset-non-table");
    let path = dir.join("config.toml");
    std::fs::write(
        &path,
        r#"
[default]
ui = "json"
"#,
    )
    .expect("fixture should be written");

    let err = unset_scoped_value_in_toml(&path, "ui.format", &Scope::global(), false, false)
        .expect_err("non-table intermediate section should fail");

    match err {
        ConfigError::InvalidSection { section, expected } => {
            assert_eq!(section, "ui");
            assert_eq!(expected, "table");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn set_and_unset_reject_empty_key_paths() {
    let dir = make_temp_dir("osp-config-store-empty-key");
    let path = dir.join("config.toml");

    let err = set_scoped_value_in_toml(
        &path,
        " . ",
        &ConfigValue::String("json".to_string()),
        &Scope::global(),
        false,
        false,
    )
    .expect_err("empty set key should fail");
    assert!(matches!(
        err,
        ConfigError::InvalidConfigKey { key, .. } if key == " . "
    ));

    let err = unset_scoped_value_in_toml(&path, " . ", &Scope::global(), false, false)
        .expect_err("empty unset key should fail");
    assert!(matches!(
        err,
        ConfigError::InvalidConfigKey { key, .. } if key == " . "
    ));
}

#[test]
fn set_list_values_round_trip_and_unset_returns_previous_list() {
    let dir = make_temp_dir("osp-config-store-list-round-trip");
    let path = dir.join("config.toml");
    let value = ConfigValue::List(vec![
        ConfigValue::String("json".to_string()),
        ConfigValue::String("table".to_string()),
    ]);

    set_scoped_value_in_toml(&path, "ui.formats", &value, &Scope::global(), false, false)
        .expect("list set should succeed");

    let payload = std::fs::read_to_string(&path).expect("config should be readable");
    let root: toml::Value = payload.parse().expect("written config should stay valid");
    assert_eq!(
        root.get("default")
            .and_then(|value| value.get("ui"))
            .and_then(|value| value.get("formats"))
            .and_then(toml::Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(toml::Value::as_str)
                    .collect::<Vec<_>>()
            }),
        Some(vec!["json", "table"])
    );

    let result = unset_scoped_value_in_toml(&path, "ui.formats", &Scope::global(), false, false)
        .expect("list unset should succeed");
    assert_eq!(result.previous, Some(value));
}

#[test]
fn unset_on_missing_file_keeps_empty_root_without_phantom_scopes() {
    let dir = make_temp_dir("osp-config-store-unset-missing-file");
    let path = dir.join("config.toml");

    let result =
        unset_scoped_value_in_toml(&path, "ui.format", &Scope::terminal("repl"), false, false)
            .expect("unset should succeed for missing file");

    assert_eq!(result.previous, None);
    let payload = std::fs::read_to_string(&path).expect("empty config file should be created");
    assert!(payload.trim().is_empty());
}

#[cfg(unix)]
#[test]
fn strict_secret_validation_reports_missing_file_as_read_error() {
    let dir = make_temp_dir("osp-config-store-missing-secret");
    let path = dir.join("missing-secrets.toml");

    let err =
        validate_secrets_permissions(&path, true).expect_err("missing strict secrets file fails");
    assert!(matches!(err, ConfigError::FileRead { .. }));
}

fn make_temp_dir(prefix: &str) -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be valid")
        .as_nanos();
    dir.push(format!("{prefix}-{nonce}"));
    std::fs::create_dir_all(&dir).expect("temp dir should be created");
    dir
}
