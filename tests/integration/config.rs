use crate::temp_support::make_temp_dir;
use osp_cli::config::{
    ConfigLayer, ConfigValue, EnvVarLoader, LoaderPipeline, ResolveOptions, Scope,
    StaticLayerLoader, TomlFileLoader, TomlStoreEditOptions, set_scoped_value_in_toml,
    unset_scoped_value_in_toml,
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
        "ui.command_formats.mreg.host.info",
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
    assert_eq!(
        resolved.get_string("ui.command_formats.mreg.host.info"),
        Some("mreg")
    );
    assert_eq!(
        resolved.get_string_list("theme.path"),
        Some(vec!["json".to_string(), "table".to_string()])
    );

    let unset = unset_scoped_value_in_toml(
        &path,
        "ui.command_formats.mreg.host.info",
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
    assert_eq!(
        updated.get_string("ui.command_formats.mreg.host.info"),
        None
    );
    assert_eq!(
        updated.get_string_list("theme.path"),
        Some(vec!["json".to_string(), "table".to_string()])
    );
}
