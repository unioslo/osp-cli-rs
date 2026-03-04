use osp_config::{
    ChainedLoader, ConfigError, ConfigLayer, ConfigLoader, ConfigSource, EnvSecretsLoader,
    EnvVarLoader, LoaderPipeline, ResolveOptions, SecretsTomlLoader, StaticLayerLoader,
    TomlFileLoader,
};

#[test]
fn toml_file_loader_optional_missing_returns_empty_layer() {
    let missing = make_temp_dir("osp-config-missing").join("config.toml");
    let loader = TomlFileLoader::new(missing).optional();

    let layer = loader
        .load()
        .expect("missing optional file should be empty");
    assert!(layer.entries().is_empty());
}

#[test]
fn toml_file_loader_required_missing_returns_error() {
    let missing = make_temp_dir("osp-config-missing-required").join("config.toml");
    let loader = TomlFileLoader::new(missing.clone()).required();

    let err = loader
        .load()
        .expect_err("missing required file should fail");
    match err {
        ConfigError::FileRead { path, reason } => {
            assert!(path.contains("config.toml"));
            assert!(reason.contains("not found"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn env_loader_parses_overrides_contract() {
    let loader = EnvVarLoader::from_iter([
        ("UNRELATED", "x"),
        ("OSP__PROFILE__TSD__UI__FORMAT", "json"),
    ]);

    let layer = loader.load().expect("env loader should parse overrides");
    assert_eq!(layer.entries().len(), 1);

    let entry = &layer.entries()[0];
    assert_eq!(entry.key, "ui.format");
    assert_eq!(entry.scope.profile.as_deref(), Some("tsd"));
    assert_eq!(entry.scope.terminal.as_deref(), None);
    assert_eq!(
        entry.origin.as_deref(),
        Some("OSP__PROFILE__TSD__UI__FORMAT")
    );
}

#[test]
fn toml_file_loader_attaches_origin_path_contract() {
    let dir = make_temp_dir("osp-config-file-origin");
    let path = dir.join("config.toml");
    std::fs::write(
        &path,
        r#"
[default]
profile.default = "default"
ui.format = "table"
"#,
    )
    .expect("config file should be written");

    let loader = TomlFileLoader::new(path.clone()).required();
    let layer = loader.load().expect("loader should parse");
    assert!(!layer.entries().is_empty());
    assert!(
        layer
            .entries()
            .iter()
            .all(|entry| entry.origin.as_deref() == Some(path.to_string_lossy().as_ref()))
    );
}

#[test]
fn loader_pipeline_resolves_source_precedence_contract() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    defaults.set("ui.format", "table");

    let mut file = ConfigLayer::default();
    file.set("ui.format", "json");

    let mut cli = ConfigLayer::default();
    cli.set("ui.format", "value");

    let pipeline = LoaderPipeline::new(StaticLayerLoader::new(defaults))
        .with_file(StaticLayerLoader::new(file))
        .with_env(EnvVarLoader::from_iter([("OSP__UI__FORMAT", "mreg")]))
        .with_cli(StaticLayerLoader::new(cli));

    let resolved = pipeline
        .resolve(ResolveOptions::default())
        .expect("pipeline should resolve");

    assert_eq!(resolved.get_string("ui.format"), Some("value"));
    assert_eq!(
        resolved
            .get_value_entry("ui.format")
            .expect("entry should exist")
            .source,
        ConfigSource::Cli
    );
}

#[test]
fn loader_pipeline_supports_profile_override_contract() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "uio");

    let mut file = ConfigLayer::default();
    file.set_for_profile("uio", "ui.format", "table");
    file.set_for_profile("tsd", "ui.format", "json");

    let pipeline = LoaderPipeline::new(StaticLayerLoader::new(defaults))
        .with_file(StaticLayerLoader::new(file));

    let resolved = pipeline
        .resolve(ResolveOptions::default().with_profile("TSD"))
        .expect("pipeline should resolve with profile override");

    assert_eq!(resolved.active_profile(), "tsd");
    assert_eq!(resolved.get_string("ui.format"), Some("json"));
}

#[test]
fn secrets_loader_combines_file_and_env_backends_contract() {
    let dir = make_temp_dir("osp-config-secrets");
    let secrets_path = dir.join("secrets.toml");
    std::fs::write(
        &secrets_path,
        r#"
[default]
extensions.uio.ldap.bind_password = "file-secret"
"#,
    )
    .expect("secrets file should be written");
    set_secrets_mode_600(&secrets_path);

    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    defaults.set("extensions.uio.ldap.bind_password", "default-secret");

    let secrets_loader = ChainedLoader::new(
        SecretsTomlLoader::new(secrets_path.clone()).required(),
    )
    .with(EnvSecretsLoader::from_iter([(
        "OSP_SECRET__EXTENSIONS__UIO__LDAP__BIND_PASSWORD",
        "env-secret",
    )]));

    let pipeline =
        LoaderPipeline::new(StaticLayerLoader::new(defaults)).with_secrets(secrets_loader);

    let resolved = pipeline
        .resolve(ResolveOptions::default())
        .expect("pipeline should resolve");

    assert_eq!(
        resolved.get_string("extensions.uio.ldap.bind_password"),
        Some("env-secret")
    );
    assert_eq!(
        resolved
            .get_value_entry("extensions.uio.ldap.bind_password")
            .expect("entry should exist")
            .source,
        ConfigSource::Secrets
    );
}

#[cfg(unix)]
#[test]
fn insecure_secrets_permissions_are_rejected_contract() {
    use std::os::unix::fs::PermissionsExt;

    let dir = make_temp_dir("osp-config-secrets-perms");
    let secrets_path = dir.join("secrets.toml");
    std::fs::write(
        &secrets_path,
        r#"
[default]
extensions.uio.ldap.bind_password = "file-secret"
"#,
    )
    .expect("secrets file should be written");
    let mut perms = std::fs::metadata(&secrets_path)
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o644);
    std::fs::set_permissions(&secrets_path, perms).expect("permissions should be set");

    let loader = SecretsTomlLoader::new(secrets_path.clone()).required();
    let err = loader
        .load()
        .expect_err("insecure permissions should fail in strict mode");
    match err {
        ConfigError::InsecureSecretsPermissions { path, mode } => {
            assert!(path.contains("secrets.toml"));
            assert_eq!(mode, 0o644);
        }
        other => panic!("unexpected error: {other:?}"),
    }
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

#[cfg(unix)]
fn set_secrets_mode_600(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o600);
    std::fs::set_permissions(path, perms).expect("permissions should be updated");
}

#[cfg(not(unix))]
fn set_secrets_mode_600(_path: &std::path::Path) {}
