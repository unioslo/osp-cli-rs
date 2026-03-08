use super::{
    BootstrapPhase, BootstrapScopeRule, ConfigLayer, ConfigSchema, ConfigSource, ConfigValue,
    ResolvedConfig, ResolvedValue, SchemaEntry, SchemaValueType, Scope, SecretValue,
    adapt_value_for_schema, bootstrap_key_spec, is_alias_key, is_bootstrap_only_key, parse_env_key,
    parse_string_list, remaining_parts_are_bootstrap_profile_default, validate_bootstrap_value,
    validate_key_scope, value_type_name,
};
use std::collections::{BTreeMap, BTreeSet};

#[test]
fn bootstrap_key_registry_describes_profile_default() {
    let spec = bootstrap_key_spec("profile.default").expect("spec should exist");
    assert_eq!(spec.key, "profile.default");
    assert_eq!(spec.phase, BootstrapPhase::Profile);
    assert!(!spec.runtime_visible);
    assert_eq!(spec.scope_rule, BootstrapScopeRule::GlobalOrTerminal);
    assert!(is_bootstrap_only_key("profile.default"));
}

#[test]
fn bootstrap_key_registry_rejects_profile_scopes() {
    let err = validate_key_scope("profile.default", &Scope::profile("work"))
        .expect_err("profile scope should be rejected");
    match err {
        crate::config::ConfigError::InvalidBootstrapScope {
            key,
            profile,
            terminal,
        } => {
            assert_eq!(key, "profile.default");
            assert_eq!(profile.as_deref(), Some("work"));
            assert_eq!(terminal, None);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn schema_marks_profile_default_bootstrap_only() {
    let schema = ConfigSchema::default();
    assert!(schema.is_known_key("profile.default"));
    assert!(!schema.is_runtime_visible_key("profile.default"));
    assert!(schema.is_runtime_visible_key("profile.active"));
}

#[test]
fn parse_input_value_adapts_scalars_lists_and_enums() {
    let schema = ConfigSchema::default();

    assert_eq!(
        schema
            .parse_input_value("ui.prompt.secrets", "true")
            .expect("bool should parse"),
        ConfigValue::Bool(true)
    );
    assert_eq!(
        schema
            .parse_input_value("session.cache.max_results", "42")
            .expect("integer should parse"),
        ConfigValue::Integer(42)
    );
    assert_eq!(
        schema
            .parse_input_value("theme.path", "custom, ./theme.toml ,")
            .expect("list should parse"),
        ConfigValue::List(vec![
            ConfigValue::String("custom".to_string()),
            ConfigValue::String("./theme.toml".to_string()),
        ])
    );
    assert_eq!(
        schema
            .parse_input_value("ui.format", "json")
            .expect("enum should parse"),
        ConfigValue::String("json".to_string())
    );

    let enum_err = schema
        .parse_input_value("ui.format", "yaml")
        .expect_err("invalid enum should fail");
    assert!(matches!(
        enum_err,
        crate::config::ConfigError::InvalidEnumValue {
            key,
            value,
            allowed
        } if key == "ui.format" && value == "yaml" && allowed.contains(&"json".to_string())
    ));

    let type_err = schema
        .parse_input_value("ui.prompt.secrets", "wat")
        .expect_err("invalid bool should fail");
    assert!(matches!(
        type_err,
        crate::config::ConfigError::InvalidValueType {
            key,
            expected: SchemaValueType::Bool,
            ..
        } if key == "ui.prompt.secrets"
    ));
}

#[test]
fn config_values_handle_secrets_display_and_interpolation() {
    let secret = ConfigValue::String("shh".to_string()).into_secret();
    assert!(secret.is_secret());
    assert_eq!(secret.reveal(), &ConfigValue::String("shh".to_string()));
    assert_eq!(secret.to_string(), "[REDACTED]");
    assert_eq!(
        format!("{:?}", SecretValue::new(ConfigValue::Bool(true))),
        "[REDACTED]"
    );

    let scalar = ConfigValue::Integer(7);
    assert_eq!(
        scalar
            .as_interpolation_string("ldap.port", "profile.ldap.port")
            .expect("scalar interpolation should work"),
        "7"
    );

    let err = ConfigValue::List(vec![ConfigValue::String("x".to_string())])
        .as_interpolation_string("ldap.uri", "profiles")
        .expect_err("list interpolation should fail");
    assert!(matches!(
        err,
        crate::config::ConfigError::NonScalarPlaceholder { key, placeholder }
            if key == "ldap.uri" && placeholder == "profiles"
    ));
}

#[test]
fn bootstrap_value_and_alias_helpers_match_runtime_rules() {
    assert_eq!(ConfigSource::Environment.to_string(), "env");
    assert!(is_alias_key("alias.lookup"));
    assert!(!is_alias_key("ui.format"));

    validate_bootstrap_value("profile.default", &ConfigValue::String("work".to_string()))
        .expect("non-empty bootstrap default should validate");

    let err = validate_bootstrap_value("profile.default", &ConfigValue::String(String::new()))
        .expect_err("empty bootstrap default should fail");
    assert!(matches!(
        err,
        crate::config::ConfigError::InvalidBootstrapValue { key, .. } if key == "profile.default"
    ));
}

#[test]
fn config_value_from_toml_rejects_tables_and_preserves_datetime_strings() {
    let dt = toml::Value::Datetime("2024-01-02T03:04:05Z".parse().expect("datetime"));
    assert_eq!(
        ConfigValue::from_toml("demo.time", &dt).expect("datetime should adapt"),
        ConfigValue::String("2024-01-02T03:04:05Z".to_string())
    );

    let table = toml::Value::Table(toml::map::Map::new());
    let err = ConfigValue::from_toml("demo.obj", &table).expect_err("table should fail");
    assert!(matches!(
        err,
        crate::config::ConfigError::UnsupportedTomlValue { path, kind }
            if path == "demo.obj" && kind == "table"
    ));
}

#[test]
fn normalize_scope_lowercases_profile_and_terminal_names() {
    let normalized = crate::config::normalize_scope(Scope {
        profile: Some("Work".to_string()),
        terminal: Some("RePl".to_string()),
    });

    assert_eq!(normalized.profile.as_deref(), Some("work"));
    assert_eq!(normalized.terminal.as_deref(), Some("repl"));
}

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
fn validate_and_adapt_rejects_unknown_and_missing_required_keys() {
    let schema = ConfigSchema::default();
    let mut unknown = BTreeMap::new();
    unknown.insert(
        "mystery.key".to_string(),
        ResolvedValue {
            raw_value: ConfigValue::String("x".to_string()),
            value: ConfigValue::String("x".to_string()),
            source: ConfigSource::ConfigFile,
            scope: Scope::global(),
            origin: None,
        },
    );
    let err = schema
        .validate_and_adapt(&mut unknown)
        .expect_err("unknown keys should fail");
    assert!(matches!(
        err,
        crate::config::ConfigError::UnknownConfigKeys { keys } if keys == vec!["mystery.key".to_string()]
    ));

    let mut missing_required = BTreeMap::new();
    let err = schema
        .validate_and_adapt(&mut missing_required)
        .expect_err("missing required profile.active should fail");
    assert!(matches!(
        err,
        crate::config::ConfigError::MissingRequiredKey { key } if key == "profile.active"
    ));
}

#[test]
fn validate_and_adapt_converts_runtime_visible_string_values() {
    let schema = ConfigSchema::default();
    let mut values = BTreeMap::new();
    values.insert(
        "profile.active".to_string(),
        ResolvedValue {
            raw_value: ConfigValue::String("ops".to_string()),
            value: ConfigValue::String("ops".to_string()),
            source: ConfigSource::ConfigFile,
            scope: Scope::global(),
            origin: None,
        },
    );
    values.insert(
        "ui.prompt.secrets".to_string(),
        ResolvedValue {
            raw_value: ConfigValue::String("true".to_string()),
            value: ConfigValue::String("true".to_string()),
            source: ConfigSource::Environment,
            scope: Scope::global(),
            origin: None,
        },
    );
    values.insert(
        "session.cache.max_results".to_string(),
        ResolvedValue {
            raw_value: ConfigValue::String("15".to_string()),
            value: ConfigValue::String("15".to_string()),
            source: ConfigSource::Environment,
            scope: Scope::global(),
            origin: None,
        },
    );
    values.insert(
        "theme.path".to_string(),
        ResolvedValue {
            raw_value: ConfigValue::String("base, custom".to_string()),
            value: ConfigValue::String("base, custom".to_string()),
            source: ConfigSource::Environment,
            scope: Scope::global(),
            origin: None,
        },
    );

    schema
        .validate_and_adapt(&mut values)
        .expect("runtime-visible values should adapt");

    assert_eq!(
        values.get("ui.prompt.secrets").map(|entry| &entry.value),
        Some(&ConfigValue::Bool(true))
    );
    assert_eq!(
        values
            .get("session.cache.max_results")
            .map(|entry| &entry.value),
        Some(&ConfigValue::Integer(15))
    );
    assert_eq!(
        values.get("theme.path").map(|entry| &entry.value),
        Some(&ConfigValue::List(vec![
            ConfigValue::String("base".to_string()),
            ConfigValue::String("custom".to_string()),
        ]))
    );
}

#[test]
fn resolved_config_helpers_read_scalar_list_and_alias_views() {
    let resolved = ResolvedConfig {
        active_profile: "ops".to_string(),
        terminal: Some("repl".to_string()),
        known_profiles: BTreeSet::from(["default".to_string(), "ops".to_string()]),
        values: BTreeMap::from([
            (
                "ui.prompt.secrets".to_string(),
                ResolvedValue {
                    raw_value: ConfigValue::Bool(true),
                    value: ConfigValue::Bool(true),
                    source: ConfigSource::ConfigFile,
                    scope: Scope::global(),
                    origin: None,
                },
            ),
            (
                "theme.path".to_string(),
                ResolvedValue {
                    raw_value: ConfigValue::String("base".to_string()).into_secret(),
                    value: ConfigValue::String("base".to_string()).into_secret(),
                    source: ConfigSource::Secrets,
                    scope: Scope::global(),
                    origin: None,
                },
            ),
        ]),
        aliases: BTreeMap::from([(
            "alias.lookup".to_string(),
            ResolvedValue {
                raw_value: ConfigValue::String("ldap user".to_string()),
                value: ConfigValue::String("ldap user".to_string()),
                source: ConfigSource::ConfigFile,
                scope: Scope::global(),
                origin: None,
            },
        )]),
    };

    assert_eq!(resolved.active_profile(), "ops");
    assert_eq!(resolved.terminal(), Some("repl"));
    assert_eq!(resolved.get_bool("ui.prompt.secrets"), Some(true));
    assert_eq!(
        resolved.get_string_list("theme.path"),
        Some(vec!["base".to_string()])
    );
    assert!(resolved.get_alias_entry("lookup").is_some());
    assert!(resolved.get_alias_entry("alias.lookup").is_some());
    assert!(resolved.known_profiles().contains("default"));
    assert!(resolved.values().contains_key("ui.prompt.secrets"));
    assert!(resolved.aliases().contains_key("alias.lookup"));
}

#[test]
fn scalar_conversion_and_display_helpers_cover_remaining_variants_unit() {
    assert_eq!(ConfigSource::Cli.to_string(), "cli");
    assert_eq!(ConfigSource::Session.to_string(), "session");
    assert_eq!(ConfigSource::Derived.to_string(), "derived");
    assert_eq!(SchemaValueType::String.to_string(), "string");
    assert_eq!(SchemaValueType::Float.to_string(), "float");
    assert_eq!(SchemaValueType::StringList.to_string(), "list");

    let already_secret = ConfigValue::String("hidden".to_string()).into_secret();
    assert_eq!(already_secret.clone().into_secret(), already_secret);
    assert_eq!(
        SecretValue::new(ConfigValue::Integer(9)).into_inner(),
        ConfigValue::Integer(9)
    );
    assert_eq!(ConfigValue::from(1.5_f64), ConfigValue::Float(1.5));
    assert_eq!(
        ConfigValue::List(vec![
            ConfigValue::String("a".to_string()),
            ConfigValue::Bool(true)
        ])
        .to_string(),
        "[a,true]"
    );
}

#[test]
fn parse_input_value_covers_float_unknown_and_allowed_scope_paths_unit() {
    let mut schema = ConfigSchema::default();
    schema.insert("demo.float", super::SchemaEntry::float());

    assert_eq!(
        schema
            .parse_input_value("demo.float", "72.5")
            .expect("float should parse"),
        ConfigValue::Float(72.5)
    );
    assert!(matches!(
        schema
            .parse_input_value("demo.float", "wide")
            .expect_err("invalid float should fail"),
        crate::config::ConfigError::InvalidValueType {
            key,
            expected: SchemaValueType::Float,
            ..
        } if key == "demo.float"
    ));
    assert!(matches!(
        schema
            .parse_input_value("not.real", "x")
            .expect_err("unknown keys should fail"),
        crate::config::ConfigError::UnknownConfigKeys { keys } if keys == vec!["not.real".to_string()]
    ));

    validate_key_scope("profile.default", &Scope::terminal("repl"))
        .expect("bootstrap profile.default is allowed on terminal scope");
    validate_bootstrap_value("ui.format", &ConfigValue::String("json".to_string()))
        .expect("non-bootstrap values are ignored");
}

#[test]
fn schema_extension_runtime_and_scope_helpers_cover_aliases_unit() {
    let mut schema = ConfigSchema::default();
    schema.set_allow_extensions_namespace(false);
    assert!(!schema.is_known_key("extensions.demo.token"));
    schema.set_allow_extensions_namespace(true);
    assert!(schema.is_known_key("extensions.demo.token"));
    assert!(schema.is_runtime_visible_key("extensions.demo.token"));
    assert!(schema.is_known_key("alias.lookup"));
    assert_eq!(
        schema
            .bootstrap_key_spec(" PROFILE.DEFAULT ")
            .map(|spec| spec.phase),
        Some(BootstrapPhase::Profile)
    );
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

#[test]
fn resolved_config_and_layer_helpers_cover_secret_lists_aliases_and_remove_scoped_unit() {
    let resolved = ResolvedConfig {
        active_profile: "default".to_string(),
        terminal: Some("repl".to_string()),
        known_profiles: BTreeSet::new(),
        values: BTreeMap::from([
            (
                "list.secret".to_string(),
                ResolvedValue {
                    raw_value: ConfigValue::List(vec![
                        ConfigValue::String("plain".to_string()),
                        ConfigValue::Secret(SecretValue::new(ConfigValue::String(
                            "hidden".to_string(),
                        ))),
                    ]),
                    value: ConfigValue::List(vec![
                        ConfigValue::String("plain".to_string()),
                        ConfigValue::Secret(SecretValue::new(ConfigValue::String(
                            "hidden".to_string(),
                        ))),
                    ]),
                    source: ConfigSource::Secrets,
                    scope: Scope::global(),
                    origin: None,
                },
            ),
            (
                "single.secret".to_string(),
                ResolvedValue {
                    raw_value: ConfigValue::Secret(SecretValue::new(ConfigValue::String(
                        "solo".to_string(),
                    ))),
                    value: ConfigValue::Secret(SecretValue::new(ConfigValue::String(
                        "solo".to_string(),
                    ))),
                    source: ConfigSource::Secrets,
                    scope: Scope::global(),
                    origin: None,
                },
            ),
        ]),
        aliases: BTreeMap::new(),
    };
    assert_eq!(
        resolved.get_string_list("list.secret"),
        Some(vec!["plain".to_string(), "hidden".to_string()])
    );
    assert_eq!(
        resolved.get_string_list("single.secret"),
        Some(vec!["solo".to_string()])
    );

    let mut layer = ConfigLayer::default();
    layer.set("ui.format", "json");
    layer.set_for_terminal("repl", "ui.format", "table");
    assert_eq!(
        layer.remove_scoped("ui.format", &Scope::terminal("repl")),
        Some(ConfigValue::String("table".to_string()))
    );
}

#[test]
fn schema_adaptation_helpers_cover_scalar_secret_list_and_env_paths_unit() {
    let string_schema = super::SchemaEntry::string();
    let bool_schema = super::SchemaEntry::boolean();
    let int_schema = super::SchemaEntry::integer();
    let float_schema = super::SchemaEntry::float();
    let list_schema = super::SchemaEntry::string_list();

    assert_eq!(
        adapt_value_for_schema(
            "demo.string",
            &ConfigValue::Secret(SecretValue::new(ConfigValue::String("x".to_string()))),
            &string_schema
        )
        .expect("secret strings adapt"),
        ConfigValue::Secret(SecretValue::new(ConfigValue::String("x".to_string())))
    );
    assert_eq!(
        adapt_value_for_schema(
            "demo.bool",
            &ConfigValue::String("true".to_string()),
            &bool_schema
        )
        .expect("string bool adapts"),
        ConfigValue::Bool(true)
    );
    assert!(matches!(
        adapt_value_for_schema("demo.bool", &ConfigValue::Integer(1), &bool_schema)
            .expect_err("integer cannot adapt to bool"),
        crate::config::ConfigError::InvalidValueType { key, .. } if key == "demo.bool"
    ));
    assert_eq!(
        adapt_value_for_schema(
            "demo.int",
            &ConfigValue::String("7".to_string()),
            &int_schema
        )
        .expect("string int adapts"),
        ConfigValue::Integer(7)
    );
    assert_eq!(
        adapt_value_for_schema("demo.float", &ConfigValue::Integer(7), &float_schema)
            .expect("integer float adapts"),
        ConfigValue::Float(7.0)
    );
    assert!(matches!(
        adapt_value_for_schema("demo.float", &ConfigValue::Bool(true), &float_schema)
            .expect_err("bool cannot adapt to float"),
        crate::config::ConfigError::InvalidValueType { key, .. } if key == "demo.float"
    ));
    assert_eq!(
        adapt_value_for_schema(
            "demo.list",
            &ConfigValue::List(vec![
                ConfigValue::String("a".to_string()),
                ConfigValue::Secret(SecretValue::new(ConfigValue::String("b".to_string()))),
            ]),
            &list_schema
        )
        .expect("mixed string list adapts"),
        ConfigValue::List(vec![
            ConfigValue::String("a".to_string()),
            ConfigValue::String("b".to_string()),
        ])
    );
    assert!(matches!(
        adapt_value_for_schema(
            "demo.list",
            &ConfigValue::List(vec![ConfigValue::Integer(1)]),
            &list_schema
        )
        .expect_err("non-string list member should fail"),
        crate::config::ConfigError::InvalidValueType { key, .. } if key == "demo.list"
    ));
    assert_eq!(
        adapt_value_for_schema(
            "demo.secret_list",
            &ConfigValue::Secret(SecretValue::new(ConfigValue::String("a, b".to_string()))),
            &list_schema
        )
        .expect("secret string list adapts"),
        ConfigValue::Secret(SecretValue::new(ConfigValue::List(vec![
            ConfigValue::String("a".to_string()),
            ConfigValue::String("b".to_string()),
        ])))
    );

    assert_eq!(parse_string_list("['a', \"b\", c]"), vec!["a", "b", "c"]);
    assert!(parse_string_list("   ").is_empty());
    assert_eq!(
        value_type_name(&ConfigValue::Secret(SecretValue::new(ConfigValue::String(
            "x".into()
        )))),
        "string"
    );
    assert!(remaining_parts_are_bootstrap_profile_default(&[
        "PROFILE", "DEFAULT"
    ]));

    let env =
        parse_env_key("OSP__TERM__RePl__PROFILE__Ops__UI__FORMAT").expect("env key should parse");
    assert_eq!(env.key, "ui.format");
    assert_eq!(env.scope, Scope::profile_terminal("ops", "repl"));

    assert!(matches!(
        parse_env_key("NOT_OSP").err().expect("missing prefix should fail"),
        crate::config::ConfigError::InvalidEnvOverride { reason, .. } if reason.contains("missing OSP__ prefix")
    ));
    assert!(matches!(
        parse_env_key("OSP__TERM").err().expect("term requires name"),
        crate::config::ConfigError::InvalidEnvOverride { reason, .. } if reason.contains("TERM requires a terminal name")
    ));
    assert!(matches!(
        parse_env_key("OSP__PROFILE").err().expect("profile requires name"),
        crate::config::ConfigError::InvalidEnvOverride { reason, .. } if reason.contains("PROFILE requires a profile name")
    ));
    assert!(matches!(
        parse_env_key("OSP__TERM__a__TERM__b__UI__FORMAT")
            .err()
            .expect("duplicate TERM should fail"),
        crate::config::ConfigError::InvalidEnvOverride { reason, .. } if reason.contains("TERM scope specified more than once")
    ));
    assert!(matches!(
        parse_env_key("OSP__PROFILE__a__PROFILE__b__UI__FORMAT")
            .err()
            .expect("duplicate PROFILE should fail"),
        crate::config::ConfigError::InvalidEnvOverride { reason, .. } if reason.contains("PROFILE scope specified more than once")
    ));
    assert!(matches!(
        parse_env_key("OSP____").err().expect("missing key path should fail"),
        crate::config::ConfigError::InvalidEnvOverride { reason, .. } if reason.contains("missing key path")
    ));
    assert!(matches!(
        parse_env_key("OSP__TERM__repl")
            .err()
            .expect("missing final key should fail"),
        crate::config::ConfigError::InvalidEnvOverride { reason, .. } if reason.contains("missing final config key")
    ));
}

#[test]
fn env_bootstrap_and_scope_normalization_cover_remaining_edge_cases_unit() {
    let env = parse_env_key("OSP__PROFILE__DEFAULT").expect("bootstrap env key should parse");
    assert_eq!(env.key, "profile.default");
    assert_eq!(env.scope, Scope::global());

    let normalized = crate::config::normalize_scope(Scope {
        profile: Some("   ".to_string()),
        terminal: Some(" RePl ".to_string()),
    });
    assert_eq!(normalized.profile, None);
    assert_eq!(normalized.terminal.as_deref(), Some("repl"));

    assert!(!remaining_parts_are_bootstrap_profile_default(&[
        "PROFILE", "OPS"
    ]));
}

#[test]
fn bootstrap_env_helpers_cover_terminal_scoped_default_profile_and_unknown_specs_unit() {
    let env = parse_env_key("OSP__TERM__repl__PROFILE__DEFAULT")
        .expect("terminal-scoped bootstrap env key should parse");
    assert_eq!(env.key, "profile.default");
    assert_eq!(env.scope, Scope::terminal("repl"));

    assert!(bootstrap_key_spec("ui.format").is_none());
    assert!(!is_bootstrap_only_key("ui.format"));
}

#[test]
fn config_value_display_and_from_impls_cover_scalar_variants_unit() {
    assert_eq!(ConfigValue::from(true), ConfigValue::Bool(true));
    assert_eq!(ConfigValue::from(7_i64), ConfigValue::Integer(7));
    assert_eq!(ConfigValue::from(2.5_f64), ConfigValue::Float(2.5));

    assert_eq!(ConfigValue::Bool(true).to_string(), "true");
    assert_eq!(ConfigValue::Integer(7).to_string(), "7");
    assert_eq!(ConfigValue::Float(2.5).to_string(), "2.5");
}

#[test]
fn resolved_config_string_list_helpers_cover_secret_and_non_string_variants_unit() {
    let mut values = BTreeMap::new();
    values.insert(
        "theme.path".to_string(),
        ResolvedValue {
            raw_value: ConfigValue::List(vec![
                ConfigValue::String("base".to_string()),
                ConfigValue::Integer(5),
                ConfigValue::String("secret".to_string()).into_secret(),
            ]),
            value: ConfigValue::List(vec![
                ConfigValue::String("base".to_string()),
                ConfigValue::Integer(5),
                ConfigValue::String("secret".to_string()).into_secret(),
            ]),
            source: ConfigSource::ConfigFile,
            scope: Scope::global(),
            origin: None,
        },
    );
    values.insert(
        "user.name".to_string(),
        ResolvedValue {
            raw_value: ConfigValue::Integer(42).into_secret(),
            value: ConfigValue::Integer(42).into_secret(),
            source: ConfigSource::Secrets,
            scope: Scope::global(),
            origin: None,
        },
    );

    let resolved = ResolvedConfig {
        active_profile: "ops".to_string(),
        terminal: None,
        known_profiles: BTreeSet::new(),
        values,
        aliases: BTreeMap::new(),
    };

    assert_eq!(
        resolved.get_string_list("theme.path"),
        Some(vec!["base".to_string(), "secret".to_string()])
    );
    assert_eq!(resolved.get_string_list("user.name"), None);
}

#[test]
fn adapt_value_for_schema_covers_float_and_secret_string_list_paths_unit() {
    assert_eq!(
        adapt_value_for_schema(
            "demo.float",
            &ConfigValue::Integer(4),
            &SchemaEntry::float()
        )
        .expect("integer should adapt to float"),
        ConfigValue::Float(4.0)
    );
    assert_eq!(
        adapt_value_for_schema(
            "demo.paths",
            &ConfigValue::String("base, custom".to_string()).into_secret(),
            &SchemaEntry::string_list(),
        )
        .expect("secret string should adapt into a secret string list"),
        ConfigValue::List(vec![
            ConfigValue::String("base".to_string()),
            ConfigValue::String("custom".to_string()),
        ])
        .into_secret()
    );

    let err = adapt_value_for_schema(
        "demo.paths",
        &ConfigValue::Integer(9).into_secret(),
        &SchemaEntry::string_list(),
    )
    .expect_err("secret non-string should fail");
    assert!(matches!(
        err,
        crate::config::ConfigError::InvalidValueType {
            key,
            expected: SchemaValueType::StringList,
            ..
        } if key == "demo.paths"
    ));
}
