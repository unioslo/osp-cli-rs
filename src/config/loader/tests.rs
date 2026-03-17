use super::{
    ChainedLoader, ConfigLoader, EnvSecretsLoader, EnvVarLoader, LoaderPipeline, SecretsTomlLoader,
    StaticLayerLoader, TomlFileLoader,
};
use crate::config::{
    ConfigError, ConfigLayer, ConfigSchema, ConfigValue, ResolveOptions, SchemaEntry, Scope,
};

fn make_temp_dir(prefix: &str) -> crate::tests::TestTempDir {
    crate::tests::make_temp_dir(prefix)
}

#[test]
fn toml_file_loader_covers_existing_missing_optional_and_directory_paths_unit() {
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

    let err = TomlFileLoader::new(root.to_path_buf())
        .required()
        .load()
        .expect_err("reading a directory as TOML should fail");
    let root_display = root.to_string_lossy().to_string();
    assert!(matches!(
        err,
        ConfigError::FileRead { path, .. } if path == root_display
    ));
}

#[test]
fn secrets_and_env_loaders_mark_secret_entries_and_directory_errors_unit() {
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

    let err = SecretsTomlLoader::new(root.to_path_buf())
        .with_strict_permissions(false)
        .required()
        .load()
        .expect_err("reading a directory as secrets TOML should fail");
    let root_display = root.to_string_lossy().to_string();
    assert!(matches!(
        err,
        ConfigError::FileRead { path, .. } if path == root_display
    ));
}

#[test]
fn chained_loader_and_pipeline_resolve_layers_with_optionals_unit() {
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
fn pipeline_builder_covers_schema_collected_env_and_optional_layer_paths_unit() {
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

#[test]
fn pipeline_resolver_preserves_custom_schema_for_explain_unit() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    defaults.set("custom.answer", "42");

    let mut schema = ConfigSchema::default();
    schema.insert(
        "custom.answer",
        SchemaEntry::integer().with_doc("Custom integer used in tests"),
    );

    let resolver = LoaderPipeline::new(StaticLayerLoader::new(defaults))
        .with_schema(schema)
        .resolver()
        .expect("pipeline resolver should preserve schema");

    let explain = resolver
        .explain_key("custom.answer", ResolveOptions::default())
        .expect("custom schema key should explain through the pipeline resolver");

    assert_eq!(
        explain
            .final_entry
            .expect("custom.answer should have a winning value")
            .value
            .reveal(),
        &ConfigValue::Integer(42)
    );
}
