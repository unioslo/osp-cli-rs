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
fn schema_extension_runtime_and_scope_helpers_cover_aliases_and_bootstrap_specs_unit() {
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
    let env = parse_env_key("OSP__PROFILE__DEFAULT").expect("bootstrap env key should parse");
    assert_eq!(env.key, "profile.default");
    assert_eq!(env.scope, Scope::global());

    let env = parse_env_key("OSP__TERM__repl__PROFILE__DEFAULT")
        .expect("terminal-scoped bootstrap env key should parse");
    assert_eq!(env.key, "profile.default");
    assert_eq!(env.scope, Scope::terminal("repl"));

    assert!(bootstrap_key_spec("ui.format").is_none());
    assert!(!is_bootstrap_only_key("ui.format"));
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
