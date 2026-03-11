use super::{
    config_value_to_toml, get_table_path, load_or_create_toml_root, prune_empty_table_path,
    read_dotted_value, secret_file_mode, set_scoped_value_in_toml, unset_scoped_value_in_toml,
    validate_secrets_permissions, write_text_atomic, write_toml_root,
};
use crate::config::{ConfigError, ConfigValue, Scope};

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
fn secret_permission_validation_matrix_covers_strict_and_non_strict_paths_unit() {
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
fn set_and_unset_reject_blank_key_paths_across_dry_run_modes_unit() {
    let dir = make_temp_dir("osp-config-store-empty-key");
    let path = dir.join("config.toml");

    for (key, dry_run) in [(" .. ", true), (" . ", false)] {
        let err = set_scoped_value_in_toml(
            &path,
            key,
            &ConfigValue::String("json".to_string()),
            &Scope::global(),
            dry_run,
            false,
        )
        .expect_err("empty set key should fail");
        assert!(matches!(
            err,
            ConfigError::InvalidConfigKey { key: err_key, .. } if err_key == key
        ));
    }

    for (key, dry_run) in [("   ", true), (" . ", false)] {
        let err = unset_scoped_value_in_toml(&path, key, &Scope::global(), dry_run, false)
            .expect_err("empty unset key should fail");
        assert!(matches!(
            err,
            ConfigError::InvalidConfigKey { key: err_key, .. } if err_key == key
        ));
    }
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

#[test]
fn config_value_to_toml_preserves_scalar_and_secret_variants_unit() {
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
}

#[test]
fn read_dotted_value_returns_nested_scalars_and_none_for_missing_paths_unit() {
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
}

#[test]
fn write_toml_root_persists_regular_payload_without_secret_mode_unit() {
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
}

#[test]
fn load_or_create_toml_root_reports_file_read_errors_unit() {
    let dir = make_temp_dir("osp-config-store-read-dir");

    let err = load_or_create_toml_root(&dir).expect_err("directory path should fail to read");
    match err {
        ConfigError::FileRead { path, reason } => {
            assert!(path.contains("osp-config-store-read-dir"));
            assert!(!reason.is_empty());
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn write_text_atomic_rejects_paths_without_file_name_unit() {
    let err = write_text_atomic(std::path::Path::new("."), b"payload", false)
        .expect_err("path without file name should fail");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert!(err.to_string().contains("path has no file name"));
}

#[cfg(unix)]
#[test]
fn secret_file_mode_reports_missing_files_unit() {
    let dir = make_temp_dir("osp-config-store-secret-file-mode-missing");
    let path = dir.join("missing.toml");

    let err = secret_file_mode(&path).expect_err("missing file should fail");
    match err {
        ConfigError::FileRead { path, reason } => {
            assert!(path.contains("missing.toml"));
            assert!(!reason.is_empty());
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn table_path_helpers_cover_missing_invalid_and_pruned_sections_unit() {
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
