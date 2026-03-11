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
fn resolved_config_helpers_read_scalar_list_secret_and_alias_views_unit() {
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

    let resolved = ResolvedConfig {
        active_profile: "ops".to_string(),
        terminal: None,
        known_profiles: BTreeSet::new(),
        values: BTreeMap::from([
            (
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
            ),
            (
                "user.name".to_string(),
                ResolvedValue {
                    raw_value: ConfigValue::Integer(42).into_secret(),
                    value: ConfigValue::Integer(42).into_secret(),
                    source: ConfigSource::Secrets,
                    scope: Scope::global(),
                    origin: None,
                },
            ),
        ]),
        aliases: BTreeMap::new(),
    };

    assert_eq!(
        resolved.get_string_list("theme.path"),
        Some(vec!["base".to_string(), "secret".to_string()])
    );
    assert_eq!(resolved.get_string_list("user.name"), None);
}
