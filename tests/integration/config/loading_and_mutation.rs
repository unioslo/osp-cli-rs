use crate::temp_support::make_temp_dir;
use osp_cli::config::{
    ConfigLayer, ConfigResolver, ConfigSource, ConfigValue, EnvVarLoader, LoaderPipeline,
    ResolveOptions, Scope, StaticLayerLoader, TomlFileLoader, TomlStoreEditOptions,
    set_scoped_value_in_toml, unset_scoped_value_in_toml,
};

fn defaults_layer() -> ConfigLayer {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    defaults.set("theme.name", "plain");
    defaults.set("ui.presentation", "austere");
    defaults
}

fn file_pipeline(path: &std::path::Path) -> LoaderPipeline {
    LoaderPipeline::new(StaticLayerLoader::new(defaults_layer()))
        .with_file(TomlFileLoader::new(path.to_path_buf()).required())
}

#[test]
fn config_file_mutation_round_trips_through_reload_and_explain() {
    let temp = make_temp_dir("osp-cli-config-integration");
    let path = temp.path().join("config.toml");

    set_scoped_value_in_toml(
        &path,
        "theme.name",
        &ConfigValue::String("dracula".to_string()),
        &Scope::global(),
        TomlStoreEditOptions::new(),
    )
    .expect("theme should be written");
    set_scoped_value_in_toml(
        &path,
        "ui.presentation",
        &ConfigValue::String("compact".to_string()),
        &Scope::profile("tsd"),
        TomlStoreEditOptions::new(),
    )
    .expect("profile-scoped presentation should be written");

    let options = ResolveOptions::default()
        .with_profile("tsd")
        .with_terminal("cli");
    let resolved = file_pipeline(&path)
        .resolve(options.clone())
        .expect("file-backed config should resolve");
    assert_eq!(resolved.active_profile(), "tsd");
    assert_eq!(resolved.get_string("theme.name"), Some("dracula"));
    assert_eq!(resolved.get_string("ui.presentation"), Some("compact"));

    set_scoped_value_in_toml(
        &path,
        "theme.name",
        &ConfigValue::String("gruvbox".to_string()),
        &Scope::global(),
        TomlStoreEditOptions::new(),
    )
    .expect("theme update should be written");

    let reloaded = file_pipeline(&path)
        .resolve(options.clone())
        .expect("mutated config should reload");
    assert_eq!(reloaded.get_string("theme.name"), Some("gruvbox"));

    let env_pipeline =
        file_pipeline(&path).with_env(EnvVarLoader::from_pairs([("OSP__theme__name", "nord")]));
    let env_resolved = env_pipeline
        .resolve(options.clone())
        .expect("env-backed config should resolve");
    assert_eq!(env_resolved.get_string("theme.name"), Some("nord"));

    let explain = ConfigResolver::from_loaded_layers(
        env_pipeline
            .load_layers()
            .expect("layers should load for explain"),
    )
    .explain_key("theme.name", options)
    .expect("theme explain should succeed");

    let final_entry = explain
        .final_entry
        .as_ref()
        .expect("theme explain should have a winner");
    assert_eq!(final_entry.source, ConfigSource::Environment);
    assert_eq!(final_entry.origin.as_deref(), Some("OSP__theme__name"));

    let file_layer = explain
        .layers
        .iter()
        .find(|layer| layer.source == ConfigSource::ConfigFile)
        .expect("file layer should be present in explain");
    assert!(
        file_layer
            .candidates
            .iter()
            .any(|candidate| candidate.selected_in_layer
                && candidate.value == ConfigValue::String("gruvbox".to_string())),
        "expected explain to retain the file-backed winner before env override: {file_layer:?}"
    );
}

#[test]
fn optional_file_and_env_layers_resolve_without_a_persisted_config_file() {
    let temp = make_temp_dir("osp-cli-config-integration-optional");
    let missing = temp.path().join("missing.toml");

    let resolved = LoaderPipeline::new(StaticLayerLoader::new(defaults_layer()))
        .with_file(TomlFileLoader::new(missing).optional())
        .with_env(EnvVarLoader::from_pairs([
            ("OSP__ui__presentation", "compact"),
            ("OSP__theme__name", "nord"),
        ]))
        .resolve(ResolveOptions::default().with_terminal("cli"))
        .expect("optional missing file plus env layers should resolve");

    assert_eq!(resolved.active_profile(), "default");
    assert_eq!(resolved.get_string("ui.presentation"), Some("compact"));
    assert_eq!(resolved.get_string("theme.name"), Some("nord"));
}

#[test]
fn config_store_round_trips_terminal_profile_scope_and_list_values_through_reload() {
    let temp = make_temp_dir("osp-cli-config-integration-store");
    let path = temp.path().join("config.toml");
    std::fs::write(&path, "[profile.tsd]\nui.presentation = \"austere\"\n").unwrap();

    let formats = ConfigValue::List(vec![
        ConfigValue::String("json".to_string()),
        ConfigValue::String("table".to_string()),
    ]);
    set_scoped_value_in_toml(
        &path,
        "theme.path",
        &formats,
        &Scope::global(),
        TomlStoreEditOptions::new(),
    )
    .expect("global list value should be written");
    set_scoped_value_in_toml(
        &path,
        "ui.format",
        &ConfigValue::String("mreg".to_string()),
        &Scope {
            profile: Some("tsd".to_string()),
            terminal: Some("repl".to_string()),
        },
        TomlStoreEditOptions::new(),
    )
    .expect("terminal-profile override should be written");

    let resolved = file_pipeline(&path)
        .resolve(
            ResolveOptions::default()
                .with_profile("tsd")
                .with_terminal("repl"),
        )
        .expect("scoped config should resolve");
    assert_eq!(resolved.get_string("ui.format"), Some("mreg"));
    assert_eq!(
        resolved.get_string_list("theme.path"),
        Some(vec!["json".to_string(), "table".to_string()])
    );

    let unset = unset_scoped_value_in_toml(
        &path,
        "ui.format",
        &Scope {
            profile: Some("tsd".to_string()),
            terminal: Some("repl".to_string()),
        },
        TomlStoreEditOptions::new(),
    )
    .expect("terminal-profile override should unset");
    assert_eq!(
        unset.previous,
        Some(ConfigValue::String("mreg".to_string()))
    );

    let updated = file_pipeline(&path)
        .resolve(
            ResolveOptions::default()
                .with_profile("tsd")
                .with_terminal("repl"),
        )
        .expect("updated config should resolve");
    assert_eq!(updated.get_string("ui.format"), None);
    assert_eq!(
        updated.get_string_list("theme.path"),
        Some(vec!["json".to_string(), "table".to_string()])
    );
}

#[test]
fn embedded_client_loads_typed_product_config_and_rotates_persisted_credentials() {
    use osp_cli::config::{
        ChainedLoader, ConfigLoader, ConfigSchema, EnvSecretsLoader, RuntimeConfig,
        RuntimeConfigPaths, RuntimeSecretStore, SchemaEntry, SecretBackendKind, SecretsTomlLoader,
        TomlSecretPermissions, TomlStoreEditMode,
    };

    let temp = make_temp_dir("osp-cli-config-embedded-client");
    let config_path = temp.path().join("config.toml");
    let secrets_path = temp.path().join("secrets.toml");
    std::fs::write(
        &config_path,
        r#"
[default]
extensions.site.retry_ratio = 1.5
extensions.site.targets = ["alice", "bob"]
extensions.site.enabled = true
extensions.site.endpoint = "https://api.example/${extensions.site.tenant}/${profile.active}"
extensions.site.health_probe = "https://api.example/health?enabled=${extensions.site.enabled}&ratio=${extensions.site.retry_ratio}"

[profile.tsd]
ui.presentation = "compact"
"#,
    )
    .expect("client configuration should be written");

    let paths = RuntimeConfigPaths {
        config_file: Some(config_path.clone()),
        secrets_file: Some(secrets_path.clone()),
        secrets_index_file: None,
    };
    let store = RuntimeSecretStore::from_paths(SecretBackendKind::Toml, &paths)
        .expect("client credential store should open");
    let credential_scope = Scope::profile("tsd");
    store
        .set_scoped(
            "extensions.site.token",
            &ConfigValue::String("persisted-token-v1".to_string()),
            &credential_scope,
            TomlStoreEditOptions::new().with_secret_permissions(TomlSecretPermissions::OwnerOnly),
        )
        .expect("initial credential should persist");

    let mut schema = ConfigSchema::default();
    schema.set_allow_extensions_namespace(false);
    schema.insert(
        "extensions.site.retry_ratio",
        SchemaEntry::float()
            .required()
            .with_doc("Client retry multiplier"),
    );
    schema.insert(
        "extensions.site.targets",
        SchemaEntry::string_list().required(),
    );
    schema.insert("extensions.site.enabled", SchemaEntry::boolean());
    schema.insert("extensions.site.endpoint", SchemaEntry::string());
    schema.insert("extensions.site.health_probe", SchemaEntry::string());
    schema.insert("extensions.site.tenant", SchemaEntry::string());
    schema.insert("extensions.site.token", SchemaEntry::string().required());
    assert_eq!(
        schema.doc_for_key("extensions.site.retry_ratio"),
        Some("Client retry multiplier")
    );

    let tenant_origin = "OSP_SECRET__TERM__cli__PROFILE__tsd__EXTENSIONS__SITE__tenant";
    let secret_loaders = || {
        ChainedLoader::new(SecretsTomlLoader::new(secrets_path.clone()).required())
            .with(EnvSecretsLoader::from_pairs([(tenant_origin, "uio")]))
    };
    let pipeline = file_pipeline(&config_path)
        .with_secrets(secret_loaders())
        .with_env(EnvVarLoader::from_pairs([(
            "OSP__EXTENSIONS__SITE__retry_ratio",
            "0.75",
        )]));
    let mut resolver = pipeline.resolver().expect("client sources should load");
    resolver.set_schema(schema);
    let options = ResolveOptions::new()
        .with_profile("tsd")
        .with_terminal("cli");
    let resolved = resolver
        .resolve(options.clone())
        .expect("client config should resolve");
    assert_eq!(
        RuntimeConfig::from_resolved(&resolved).active_profile,
        "tsd"
    );
    assert_eq!(
        resolved.get("extensions.site.retry_ratio"),
        Some(&ConfigValue::Float(0.75))
    );
    assert_eq!(resolved.get_bool("extensions.site.enabled"), Some(true));
    assert_eq!(
        resolved.get_string_list("extensions.site.targets"),
        Some(vec!["alice".to_string(), "bob".to_string()])
    );
    assert_eq!(
        resolved.get_string("extensions.site.token"),
        Some("persisted-token-v1")
    );
    assert_eq!(
        resolved.get_string("extensions.site.endpoint"),
        Some("https://api.example/uio/tsd")
    );
    assert_eq!(
        resolved.get_string("extensions.site.health_probe"),
        Some("https://api.example/health?enabled=true&ratio=0.75")
    );
    let credential = resolved.get_value_entry("extensions.site.token").unwrap();
    assert_eq!(credential.source, ConfigSource::Secrets);
    assert_eq!(credential.scope, credential_scope);
    assert_eq!(format!("{}", credential.value), "[REDACTED]");
    let ConfigValue::Secret(secret) = &credential.value else {
        panic!("credential should retain its secret wrapper");
    };
    assert_eq!(format!("{secret:?}"), "[REDACTED]");

    let endpoint = resolver
        .explain_key("extensions.site.endpoint", options.clone())
        .expect("client endpoint provenance should explain");
    let endpoint_entry = endpoint.final_entry.unwrap();
    assert_eq!(endpoint_entry.source, ConfigSource::ConfigFile);
    assert_eq!(format!("{}", endpoint_entry.value), "[REDACTED]");
    let interpolation = endpoint.interpolation.unwrap();
    let tenant_step = interpolation
        .steps
        .iter()
        .find(|step| step.placeholder == "extensions.site.tenant")
        .expect("endpoint should explain its tenant credential");
    assert_eq!(
        tenant_step.value.reveal(),
        &ConfigValue::String("uio".to_string())
    );
    assert_eq!(tenant_step.source, ConfigSource::Secrets);
    assert_eq!(tenant_step.scope, Scope::profile_terminal("tsd", "cli"));
    assert_eq!(tenant_step.origin.as_deref(), Some(tenant_origin));

    let rotated = ConfigValue::String("persisted-token-v2".to_string());
    let preview = store
        .set_scoped(
            "extensions.site.token",
            &rotated,
            &credential_scope,
            TomlStoreEditOptions::new().with_mode(TomlStoreEditMode::DryRun),
        )
        .expect("credential rotation should preview");
    assert_eq!(preview.backend, SecretBackendKind::Toml);
    assert_eq!(preview.location, secrets_path.display().to_string());
    assert_eq!(
        preview.previous,
        Some(ConfigValue::String("persisted-token-v1".to_string()))
    );
    resolver.set_secrets(
        secret_loaders()
            .load()
            .expect("preview should reload existing credential"),
    );
    assert_eq!(
        resolver
            .resolve(options.clone())
            .unwrap()
            .get_string("extensions.site.token"),
        Some("persisted-token-v1")
    );

    let persisted = store
        .set_scoped(
            "extensions.site.token",
            &rotated,
            &credential_scope,
            TomlStoreEditOptions::new().with_mode(TomlStoreEditMode::Persist),
        )
        .expect("credential rotation should persist");
    assert_eq!(persisted.previous, preview.previous);
    resolver.set_secrets(
        secret_loaders()
            .load()
            .expect("rotated credential should reload"),
    );
    assert_eq!(
        resolver
            .env_mut()
            .remove_scoped("extensions.site.retry_ratio", &Scope::global()),
        Some(ConfigValue::String("0.75".to_string()))
    );
    resolver.schema_mut().insert(
        "extensions.site.batch_size",
        SchemaEntry::positive_integer(),
    );
    resolver
        .presentation_mut()
        .set("extensions.site.batch_size", 8_i64);
    let reloaded = resolver
        .resolve(options.clone())
        .expect("updated client policy should resolve");
    assert_eq!(
        reloaded.get_string("extensions.site.token"),
        Some("persisted-token-v2")
    );
    assert_eq!(
        reloaded.get("extensions.site.retry_ratio"),
        Some(&ConfigValue::Float(1.5))
    );
    let batch_size = reloaded
        .get_value_entry("extensions.site.batch_size")
        .unwrap();
    assert_eq!(batch_size.value, ConfigValue::Integer(8));
    assert_eq!(batch_size.source, ConfigSource::PresentationDefaults);
    assert_eq!(batch_size.scope, Scope::global());
    assert_eq!(reloaded.get_string("ui.presentation"), Some("compact"));

    assert_eq!(
        resolver
            .schema_mut()
            .expected_type("extensions.site.retry_ratio"),
        Some(osp_cli::config::SchemaValueType::Float)
    );
    let ratio_edit = resolver
        .schema_mut()
        .parse_input_value("extensions.site.retry_ratio", "2.25")
        .expect("the client editor should parse its schema-owned float value");
    assert_eq!(ratio_edit, ConfigValue::Float(2.25));
    let ratio_change = set_scoped_value_in_toml(
        &config_path,
        "extensions.site.retry_ratio",
        &ratio_edit,
        &Scope::global(),
        TomlStoreEditOptions::new(),
    )
    .expect("the typed client edit should persist in its real configuration document");
    assert_eq!(ratio_change.previous, Some(ConfigValue::Float(1.5)));
    resolver.set_file(
        TomlFileLoader::new(config_path.clone())
            .required()
            .load()
            .expect("the persisted client edit should reload"),
    );
    let edited = resolver
        .resolve(options.clone())
        .expect("the edited client policy should resolve");
    assert_eq!(edited.get("extensions.site.retry_ratio"), Some(&ratio_edit));
    let ratio_entry = resolver
        .explain_key("extensions.site.retry_ratio", options)
        .expect("the client edit should retain file provenance")
        .final_entry
        .unwrap();
    assert_eq!(ratio_entry.value, ratio_edit);
    assert_eq!(ratio_entry.source, ConfigSource::ConfigFile);
    assert_eq!(ratio_entry.scope, Scope::global());
    assert_eq!(ratio_entry.origin.as_deref(), config_path.to_str());
    #[cfg(unix)]
    assert_eq!(
        osp_cli::config::secret_file_mode(&secrets_path).unwrap(),
        0o600
    );
}
