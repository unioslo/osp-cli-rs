use super::{
    config_value_to_toml, get_table_path, load_or_create_toml_root, prune_empty_table_path,
    read_dotted_value, secret_file_mode, set_scoped_value_in_toml, unset_scoped_value_in_toml,
    validate_secrets_permissions, write_text_atomic, write_toml_root,
};
use crate::config::{ConfigError, ConfigValue, Scope, TomlStoreEditOptions};

fn default_edit_options() -> TomlStoreEditOptions {
    TomlStoreEditOptions::new()
}

fn dry_run_edit_options() -> TomlStoreEditOptions {
    TomlStoreEditOptions::dry_run()
}

fn secret_edit_options() -> TomlStoreEditOptions {
    TomlStoreEditOptions::new().for_secrets()
}

#[test]
fn dry_run_set_does_not_create_file() {
    let dir = make_temp_dir("osp-config-store-dry-run");
    let path = dir.join("config.toml");

    let result = set_scoped_value_in_toml(
        &path,
        "ui.format",
        &ConfigValue::String("json".to_string()),
        &Scope::global(),
        dry_run_edit_options(),
    )
    .expect("dry-run set should succeed");

    assert_eq!(result.previous, None);
    assert!(!path.exists());
}

#[test]
fn set_round_trips_cover_overwrite_list_and_terminal_profile_scopes_unit() {
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
        default_edit_options(),
    )
    .expect("overwrite should succeed");
    assert_eq!(
        result.previous,
        Some(ConfigValue::String("json".to_string()))
    );
    let payload = std::fs::read_to_string(&path).expect("config should be readable");
    assert!(payload.contains("format = \"table\""));

    let list_path = dir.join("list.toml");
    let value = ConfigValue::List(vec![
        ConfigValue::String("json".to_string()),
        ConfigValue::String("table".to_string()),
    ]);
    set_scoped_value_in_toml(
        &list_path,
        "ui.formats",
        &value,
        &Scope::global(),
        default_edit_options(),
    )
    .expect("list set should succeed");
    let payload = std::fs::read_to_string(&list_path).expect("config should be readable");
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
    let result = unset_scoped_value_in_toml(
        &list_path,
        "ui.formats",
        &Scope::global(),
        default_edit_options(),
    )
    .expect("list unset should succeed");
    assert_eq!(result.previous, Some(value));

    let terminal_profile_path = dir.join("terminal-profile.toml");
    let scope = Scope {
        profile: Some("tsd".to_string()),
        terminal: Some("repl".to_string()),
    };
    let set_result = set_scoped_value_in_toml(
        &terminal_profile_path,
        "ui.format",
        &ConfigValue::String("mreg".to_string()),
        &scope,
        default_edit_options(),
    )
    .expect("set should succeed");
    assert_eq!(set_result.previous, None);

    let payload = std::fs::read_to_string(&terminal_profile_path).expect("config should exist");
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

    let unset_result = unset_scoped_value_in_toml(
        &terminal_profile_path,
        "ui.format",
        &scope,
        default_edit_options(),
    )
    .expect("unset should succeed");
    assert_eq!(
        unset_result.previous,
        Some(ConfigValue::String("mreg".to_string()))
    );
    let payload =
        std::fs::read_to_string(&terminal_profile_path).expect("config should still be readable");
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
fn unset_behaviors_cover_missing_values_missing_files_and_pruning_unit() {
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

    let result = unset_scoped_value_in_toml(
        &path,
        "ui.mode",
        &Scope::terminal("repl"),
        default_edit_options(),
    )
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

    let missing_path = dir.join("missing.toml");
    let result = unset_scoped_value_in_toml(
        &missing_path,
        "ui.format",
        &Scope::terminal("repl"),
        default_edit_options(),
    )
    .expect("unset should succeed for missing file");
    assert_eq!(result.previous, None);
    let payload =
        std::fs::read_to_string(&missing_path).expect("empty config file should be created");
    assert!(payload.trim().is_empty());

    let prune_path = dir.join("prune.toml");
    std::fs::write(
        &prune_path,
        r#"
[terminal.repl.profile.ops.ui]
format = "json"
"#,
    )
    .expect("fixture should be written");
    unset_scoped_value_in_toml(
        &prune_path,
        "ui.format",
        &Scope {
            profile: Some("ops".to_string()),
            terminal: Some("repl".to_string()),
        },
        default_edit_options(),
    )
    .expect("unset should succeed");
    let payload = std::fs::read_to_string(&prune_path).expect("config should be readable");
    let root: toml::Value = payload.parse().expect("written config should stay valid");
    assert!(root.get("terminal").is_none());
}

#[test]
fn store_reports_invalid_toml_sections_and_blank_keys_for_set_and_unset_unit() {
    let dir = make_temp_dir("osp-config-store-invalid");
    let path = dir.join("config.toml");
    std::fs::write(&path, "not = [valid").expect("fixture should be written");

    let err = set_scoped_value_in_toml(
        &path,
        "ui.format",
        &ConfigValue::String("json".to_string()),
        &Scope::global(),
        default_edit_options(),
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

    let set_non_table_path = dir.join("set-non-table.toml");
    std::fs::write(
        &set_non_table_path,
        r#"
[default]
ui = "json"
"#,
    )
    .expect("fixture should be written");
    let err = set_scoped_value_in_toml(
        &set_non_table_path,
        "ui.format",
        &ConfigValue::String("table".to_string()),
        &Scope::global(),
        default_edit_options(),
    )
    .expect_err("non-table section should fail");
    match err {
        ConfigError::InvalidSection { section, expected } => {
            assert_eq!(section, "ui");
            assert_eq!(expected, "table");
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let unset_non_table_path = dir.join("unset-non-table.toml");
    std::fs::write(
        &unset_non_table_path,
        r#"
[default]
ui = "json"
"#,
    )
    .expect("fixture should be written");
    let err = unset_scoped_value_in_toml(
        &unset_non_table_path,
        "ui.format",
        &Scope::global(),
        default_edit_options(),
    )
    .expect_err("non-table intermediate section should fail");
    match err {
        ConfigError::InvalidSection { section, expected } => {
            assert_eq!(section, "ui");
            assert_eq!(expected, "table");
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let blank_key_path = dir.join("blank-key.toml");
    std::fs::write(&blank_key_path, "[default]\n").expect("fixture should be written");
    for (key, dry_run) in [(" .. ", true), (" . ", false)] {
        let err = set_scoped_value_in_toml(
            &blank_key_path,
            key,
            &ConfigValue::String("json".to_string()),
            &Scope::global(),
            if dry_run {
                dry_run_edit_options()
            } else {
                default_edit_options()
            },
        )
        .expect_err("empty set key should fail");
        assert!(matches!(err, ConfigError::InvalidConfigKey { .. }));
    }

    for (key, dry_run) in [("   ", true), (" . ", false)] {
        let err = unset_scoped_value_in_toml(
            &blank_key_path,
            key,
            &Scope::global(),
            if dry_run {
                dry_run_edit_options()
            } else {
                default_edit_options()
            },
        )
        .expect_err("empty unset key should fail");
        assert!(matches!(err, ConfigError::InvalidConfigKey { .. }));
    }
}

#[cfg(unix)]
#[test]
fn secret_write_and_permission_validation_cover_strict_non_strict_and_missing_paths_unit() {
    use std::os::unix::fs::PermissionsExt;

    let dir = make_temp_dir("osp-config-store-secret-mode");
    let path = dir.join("secrets.toml");
    set_scoped_value_in_toml(
        &path,
        "extensions.demo.token",
        &ConfigValue::String("secret".to_string()),
        &Scope::global(),
        secret_edit_options(),
    )
    .expect("secret write should succeed");

    let mode = std::fs::metadata(&path)
        .expect("metadata should exist")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    validate_secrets_permissions(&path, true).expect("strict validation should pass");

    enum Fixture {
        GroupReadable,
        Missing,
    }

    enum Expected {
        Ok,
        Insecure(u32),
        FileRead,
    }

    for (label, strict, fixture, expected) in [
        (
            "strict group-readable",
            true,
            Fixture::GroupReadable,
            Expected::Insecure(0o640),
        ),
        (
            "non-strict group-readable",
            false,
            Fixture::GroupReadable,
            Expected::Ok,
        ),
        ("non-strict missing", false, Fixture::Missing, Expected::Ok),
        ("strict missing", true, Fixture::Missing, Expected::FileRead),
    ] {
        let mut owner = None;
        let path = match fixture {
            Fixture::GroupReadable => {
                let fixture = write_secret_fixture(label, Some(0o640));
                let path = fixture.path.clone();
                owner = Some(fixture);
                path
            }
            Fixture::Missing => make_temp_dir(label).join("secrets.toml"),
        };
        let _owner = owner;

        match (expected, validate_secrets_permissions(&path, strict)) {
            (Expected::Ok, Ok(())) => {}
            (
                Expected::Insecure(expected_mode),
                Err(ConfigError::InsecureSecretsPermissions { mode, .. }),
            ) => {
                assert_eq!(mode, expected_mode, "{label}");
            }
            (Expected::FileRead, Err(ConfigError::FileRead { .. })) => {}
            (_, other) => panic!("unexpected result for {label}: {other:?}"),
        }
    }

    let missing_path = dir.join("missing.toml");
    let err = secret_file_mode(&missing_path).expect_err("missing file should fail");
    match err {
        ConfigError::FileRead { path, reason } => {
            assert!(path.contains("missing.toml"));
            assert!(!reason.is_empty());
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn helper_value_and_path_functions_cover_scalar_secret_nested_and_pruned_cases_unit() {
    let value = ConfigValue::List(vec![
        ConfigValue::Integer(7),
        ConfigValue::Float(2.5),
        ConfigValue::String("plain".to_string()).into_secret(),
    ]);
    let toml = config_value_to_toml(&value);
    assert_eq!(
        toml,
        toml::Value::Array(vec![
            toml::Value::Integer(7),
            toml::Value::Float(2.5),
            toml::Value::String("plain".to_string()),
        ])
    );

    let root: toml::Value = r#"
[default.ui]
format = "json"
"#
    .parse()
    .expect("fixture should parse");
    let table = root.as_table().expect("root should be a table");
    assert_eq!(
        read_dotted_value(table, &["default", "ui", "format"]).and_then(toml::Value::as_str),
        Some("json")
    );
    assert!(read_dotted_value(table, &["default", "ui", "missing"]).is_none());
    assert!(read_dotted_value(table, &[]).is_none());

    let mut root = toml::value::Table::new();
    root.insert(
        "default".to_string(),
        toml::Value::String("json".to_string()),
    );
    let err = get_table_path(&root, &["default", "ui"]).expect_err("scalar section should fail");
    match err {
        ConfigError::InvalidSection { section, expected } => {
            assert_eq!(section, "default");
            assert_eq!(expected, "table");
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let empty = toml::value::Table::new();
    let missing =
        get_table_path(&empty, &["default", "ui"]).expect("missing branch should return none");
    assert!(missing.is_none());

    let mut nested = toml::value::Table::new();
    nested.insert(
        "default".to_string(),
        toml::Value::String("json".to_string()),
    );
    let err = prune_empty_table_path(&mut nested, &["default", "ui"])
        .expect_err("invalid child section should fail");
    match err {
        ConfigError::InvalidSection { section, expected } => {
            assert_eq!(section, "default");
            assert_eq!(expected, "table");
        }
        other => panic!("unexpected error: {other:?}"),
    }

    prune_empty_table_path(&mut nested, &[]).expect("empty path should short-circuit");

    let mut empty_root = toml::value::Table::new();
    empty_root.insert(
        "default".to_string(),
        toml::Value::Table(toml::value::Table::new()),
    );
    prune_empty_table_path(&mut empty_root, &["default"]).expect("empty leaf table should prune");
    assert!(empty_root.is_empty());
}

#[test]
fn write_and_load_helpers_cover_regular_io_and_error_paths_unit() {
    let dir = make_temp_dir("osp-config-store-write-root");
    let path = dir.join("config.toml");
    let root: toml::Value = r#"
[default.ui]
format = "json"
"#
    .parse()
    .expect("fixture should parse");

    write_toml_root(&path, &root, false).expect("toml root should be written");
    let payload = std::fs::read_to_string(&path).expect("written config should be readable");
    assert!(payload.contains("format = \"json\""));

    let err = load_or_create_toml_root(&dir).expect_err("directory path should fail to read");
    match err {
        ConfigError::FileRead { path, reason } => {
            assert!(path.contains("osp-config-store-write-root"));
            assert!(!reason.is_empty());
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let err = write_text_atomic(std::path::Path::new("."), b"payload", false)
        .expect_err("path without file name should fail");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert!(err.to_string().contains("path has no file name"));
}

fn make_temp_dir(prefix: &str) -> crate::tests::TestTempDir {
    crate::tests::make_temp_dir(prefix)
}

#[cfg(unix)]
struct SecretFixture {
    _dir: crate::tests::TestTempDir,
    path: std::path::PathBuf,
}

#[cfg(unix)]
fn write_secret_fixture(prefix: &str, mode: Option<u32>) -> SecretFixture {
    use std::os::unix::fs::PermissionsExt;

    let dir = make_temp_dir(prefix);
    let path = dir.join("secrets.toml");
    std::fs::write(&path, "token = 'secret'\n").expect("fixture should be written");
    if let Some(mode) = mode {
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
            .expect("permissions should be set");
    }
    SecretFixture { _dir: dir, path }
}
