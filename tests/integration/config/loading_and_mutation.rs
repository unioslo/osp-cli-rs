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
