use super::{
    ChainedLoader, ConfigLoader, EnvSecretsLoader, EnvVarLoader, LoaderPipeline, SecretsTomlLoader,
    StaticLayerLoader, TomlFileLoader,
};
use crate::config::{ConfigError, ConfigLayer, ConfigSchema, ResolveOptions, Scope};
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

#[test]
fn file_and_secrets_loaders_report_read_errors_for_directories_unit() {
    let root = make_temp_dir("osp-config-loader-read-error");
    let err = TomlFileLoader::new(root.clone())
        .required()
        .load()
        .expect_err("reading a directory as TOML should fail");
    let root_display = root.to_string_lossy().to_string();
    assert!(matches!(
        err,
        ConfigError::FileRead { path, .. } if path == root_display
    ));

    let err = SecretsTomlLoader::new(root.clone())
        .with_strict_permissions(false)
        .required()
        .load()
        .expect_err("reading a directory as secrets TOML should fail");
    assert!(matches!(
        err,
        ConfigError::FileRead { path, .. } if path == root_display
    ));
}

#[test]
fn process_env_and_pipeline_builder_cover_remaining_loader_paths_unit() {
    let _env_loader = EnvVarLoader::from_process_env();
    let _secret_loader = EnvSecretsLoader::from_process_env();

    let layers = LoaderPipeline::new(StaticLayerLoader::new(ConfigLayer::default()))
        .with_file(StaticLayerLoader::new({
            let mut layer = ConfigLayer::default();
            layer.insert("theme.name", "plain", Scope::global());
            layer
        }))
        .with_presentation(StaticLayerLoader::new({
            let mut layer = ConfigLayer::default();
            layer.insert("repl.intro", "compact", Scope::global());
            layer
        }))
        .with_cli(StaticLayerLoader::new({
            let mut layer = ConfigLayer::default();
            layer.insert("theme.name", "nord", Scope::global());
            layer
        }))
        .with_session(StaticLayerLoader::new({
            let mut layer = ConfigLayer::default();
            layer.insert("session.cache.max_results", 32_i64, Scope::global());
            layer
        }))
        .load_layers()
        .expect("pipeline should load all optional layers");

    assert_eq!(layers.file.entries().len(), 1);
    assert_eq!(layers.presentation.entries().len(), 1);
    assert_eq!(layers.cli.entries().len(), 1);
    assert_eq!(layers.session.entries().len(), 1);
}
