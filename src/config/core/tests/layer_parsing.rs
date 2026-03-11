#[test]
fn config_layer_from_toml_str_flattens_default_profile_and_terminal_scopes() {
    let layer = ConfigLayer::from_toml_str(
        r#"
[default.ui]
format = "json"

[profile.ops.ui]
format = "table"

[terminal.repl.ui]
format = "md"

[terminal.repl.profile.ops.ui]
format = "mreg"
"#,
    )
    .expect("toml layer should parse");

    assert!(layer.entries().iter().any(|entry| {
        entry.key == "ui.format"
            && entry.scope == Scope::global()
            && entry.value == ConfigValue::String("json".to_string())
    }));
    assert!(layer.entries().iter().any(|entry| {
        entry.key == "ui.format"
            && entry.scope == Scope::profile("ops")
            && entry.value == ConfigValue::String("table".to_string())
    }));
    assert!(layer.entries().iter().any(|entry| {
        entry.key == "ui.format"
            && entry.scope == Scope::terminal("repl")
            && entry.value == ConfigValue::String("md".to_string())
    }));
    assert!(layer.entries().iter().any(|entry| {
        entry.key == "ui.format"
            && entry.scope == Scope::profile_terminal("ops", "repl")
            && entry.value == ConfigValue::String("mreg".to_string())
    }));
}

#[test]
fn config_layer_from_env_iter_parses_scopes_and_bootstrap_profile_default() {
    let layer = ConfigLayer::from_env_iter([
        ("OSP__TERM__REPL__UI__FORMAT", "json"),
        ("OSP__PROFILE__ops__UI__FORMAT", "table"),
        ("OSP__PROFILE__DEFAULT", "ops"),
    ])
    .expect("env layer should parse");

    assert!(layer.entries().iter().any(|entry| {
        entry.origin.as_deref() == Some("OSP__TERM__REPL__UI__FORMAT")
            && entry.scope == Scope::terminal("repl")
            && entry.key == "ui.format"
    }));
    assert!(layer.entries().iter().any(|entry| {
        entry.origin.as_deref() == Some("OSP__PROFILE__ops__UI__FORMAT")
            && entry.scope == Scope::profile("ops")
            && entry.key == "ui.format"
    }));
    assert!(layer.entries().iter().any(|entry| {
        entry.origin.as_deref() == Some("OSP__PROFILE__DEFAULT")
            && entry.scope == Scope::global()
            && entry.key == "profile.default"
    }));
}

#[test]
fn config_layer_from_toml_str_rejects_derived_profile_active() {
    let err = ConfigLayer::from_toml_str(
        r#"
[default.profile]
active = "ops"
"#,
    )
    .expect_err("derived profile.active should not load from toml");
    assert!(matches!(
        err,
        crate::config::ConfigError::ReadOnlyConfigKey { key, .. } if key == "profile.active"
    ));
}

#[test]
fn config_layer_from_toml_str_reports_invalid_sections_and_unknown_roots_unit() {
    assert!(matches!(
        ConfigLayer::from_toml_str("default = 1").expect_err("default must be table"),
        crate::config::ConfigError::InvalidSection { section, expected }
            if section == "default" && expected == "table"
    ));
    assert!(matches!(
        ConfigLayer::from_toml_str("[profile]\nops = 1\n")
            .expect_err("profile entries must be tables"),
        crate::config::ConfigError::InvalidSection { section, expected }
            if section == "profile.ops" && expected == "table"
    ));
    assert!(matches!(
        ConfigLayer::from_toml_str("[terminal]\nrepl = 1\n")
            .expect_err("terminal entries must be tables"),
        crate::config::ConfigError::InvalidSection { section, expected }
            if section == "terminal.repl" && expected == "table"
    ));
    assert!(matches!(
        ConfigLayer::from_toml_str("[terminal.repl]\nprofile = 1\n")
            .expect_err("terminal profile section must be table"),
        crate::config::ConfigError::InvalidSection { section, expected }
            if section == "terminal.repl.profile" && expected == "table"
    ));
    assert!(matches!(
        ConfigLayer::from_toml_str("[mystery]\nvalue = 1\n")
            .expect_err("unknown top-level section should fail"),
        crate::config::ConfigError::UnknownTopLevelSection(section) if section == "mystery"
    ));
}

#[test]
fn config_layer_helpers_cover_insert_remove_and_profile_terminal_scope_unit() {
    let mut layer = ConfigLayer::default();
    layer.set("ui.format", "json");
    layer.set_for_profile("ops", "ui.format", "table");
    layer.set_for_terminal("repl", "ui.format", "md");
    layer.set_for_profile_terminal("ops", "repl", "ui.format", "mreg");
    layer.insert_with_origin(
        "theme.name",
        "dracula",
        Scope::terminal("repl"),
        Some("CLI"),
    );

    assert!(layer.entries().iter().any(|entry| {
        entry.key == "theme.name"
            && entry.scope == Scope::terminal("repl")
            && entry.origin.as_deref() == Some("CLI")
    }));
    assert_eq!(crate::config::normalize_identifier(" RePl "), "repl");
}
