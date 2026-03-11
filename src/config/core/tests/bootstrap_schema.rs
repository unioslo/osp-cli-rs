mod bootstrap_registry_contracts {
    use super::*;

    #[test]
    fn profile_default_is_bootstrap_only_and_rejects_invalid_scopes() {
        let spec = bootstrap_key_spec("profile.default").expect("spec should exist");
        assert_eq!(spec.key, "profile.default");
        assert_eq!(spec.phase, BootstrapPhase::Profile);
        assert!(!spec.runtime_visible);
        assert_eq!(spec.scope_rule, BootstrapScopeRule::GlobalOrTerminal);
        assert!(is_bootstrap_only_key("profile.default"));

        let schema = ConfigSchema::default();
        assert!(schema.is_known_key("profile.default"));
        assert!(!schema.is_runtime_visible_key("profile.default"));
        assert!(schema.is_runtime_visible_key("profile.active"));

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
    fn bootstrap_and_alias_helpers_match_runtime_rules() {
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

        let normalized = crate::config::normalize_scope(Scope {
            profile: Some("Work".to_string()),
            terminal: Some("RePl".to_string()),
        });
        assert_eq!(normalized.profile.as_deref(), Some("work"));
        assert_eq!(normalized.terminal.as_deref(), Some("repl"));
    }
}

mod schema_value_contracts {
    use super::*;

    #[test]
    fn schema_parsing_handles_scalars_lists_enums_and_derived_keys() {
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

        let read_only = schema
            .parse_input_value("profile.active", "ops")
            .expect_err("profile.active should be read-only");
        assert!(matches!(
            read_only,
            crate::config::ConfigError::ReadOnlyConfigKey { key, .. } if key == "profile.active"
        ));
    }

    #[test]
    fn config_values_cover_secret_interpolation_toml_and_display_paths() {
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

        let interpolation_err = ConfigValue::List(vec![ConfigValue::String("x".to_string())])
            .as_interpolation_string("ldap.uri", "profiles")
            .expect_err("list interpolation should fail");
        assert!(matches!(
            interpolation_err,
            crate::config::ConfigError::NonScalarPlaceholder { key, placeholder }
                if key == "ldap.uri" && placeholder == "profiles"
        ));

        let dt = toml::Value::Datetime("2024-01-02T03:04:05Z".parse().expect("datetime"));
        assert_eq!(
            ConfigValue::from_toml("demo.time", &dt).expect("datetime should adapt"),
            ConfigValue::String("2024-01-02T03:04:05Z".to_string())
        );

        let table = toml::Value::Table(toml::map::Map::new());
        let table_err =
            ConfigValue::from_toml("demo.obj", &table).expect_err("table should fail");
        assert!(matches!(
            table_err,
            crate::config::ConfigError::UnsupportedTomlValue { path, kind }
                if path == "demo.obj" && kind == "table"
        ));

        let already_secret = ConfigValue::String("hidden".to_string()).into_secret();
        assert_eq!(already_secret.clone().into_secret(), already_secret);
        assert_eq!(
            SecretValue::new(ConfigValue::Integer(9)).into_inner(),
            ConfigValue::Integer(9)
        );
        assert_eq!(ConfigValue::from(true), ConfigValue::Bool(true));
        assert_eq!(ConfigValue::from(7_i64), ConfigValue::Integer(7));
        assert_eq!(ConfigValue::from(1.5_f64), ConfigValue::Float(1.5));
        assert_eq!(
            ConfigValue::List(vec![
                ConfigValue::String("a".to_string()),
                ConfigValue::Bool(true),
            ])
            .to_string(),
            "[a,true]"
        );
        assert_eq!(ConfigValue::Bool(true).to_string(), "true");
        assert_eq!(ConfigValue::Integer(7).to_string(), "7");
        assert_eq!(ConfigValue::Float(2.5).to_string(), "2.5");

        assert_eq!(ConfigSource::Cli.to_string(), "cli");
        assert_eq!(ConfigSource::Session.to_string(), "session");
        assert_eq!(ConfigSource::Derived.to_string(), "derived");
        assert_eq!(SchemaValueType::String.to_string(), "string");
        assert_eq!(SchemaValueType::Float.to_string(), "float");
        assert_eq!(SchemaValueType::StringList.to_string(), "list");
    }
}
