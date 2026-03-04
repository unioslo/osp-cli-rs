use osp_config::{
    ConfigError, ConfigLayer, ConfigResolver, ConfigSource, ConfigValue, ResolveOptions,
};

#[test]
fn profile_scope_beats_unscoped_contract() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "uio");

    let mut file = ConfigLayer::default();
    file.set("ui.format", "table");
    file.set_for_profile("uio", "ui.format", "json");

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    resolver.set_file(file);

    let resolved = resolver
        .resolve(ResolveOptions::default())
        .expect("config should resolve");

    assert_eq!(resolved.active_profile(), "uio");
    assert_eq!(resolved.get_string("ui.format"), Some("json"));
}

#[test]
fn terminal_scope_beats_unscoped_contract() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");

    let mut file = ConfigLayer::default();
    file.set("ui.format", "table");
    file.set_for_terminal("repl", "ui.format", "mreg");

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    resolver.set_file(file);

    let resolved = resolver
        .resolve(ResolveOptions::default().with_terminal("repl"))
        .expect("config should resolve");

    assert_eq!(resolved.get_string("ui.format"), Some("mreg"));
}

#[test]
fn cli_overrides_environment_and_file_contract() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");

    let mut file = ConfigLayer::default();
    file.set("ui.format", "table");

    let mut env = ConfigLayer::default();
    env.set("ui.format", "json");

    let mut cli = ConfigLayer::default();
    cli.set("ui.format", "value");

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    resolver.set_file(file);
    resolver.set_env(env);
    resolver.set_cli(cli);

    let resolved = resolver
        .resolve(ResolveOptions::default())
        .expect("config should resolve");

    let value = resolved
        .get_value_entry("ui.format")
        .expect("ui.format should exist");

    assert_eq!(resolved.get_string("ui.format"), Some("value"));
    assert_eq!(value.source, ConfigSource::Cli);
}

#[test]
fn placeholder_interpolation_happens_after_merge_contract() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "uio");
    defaults.set("base.dir", "/etc/osp");

    let mut file = ConfigLayer::default();
    file.set_for_profile("uio", "extensions.uio.ldap.url", "ldaps://ldap.uio.no");

    let mut env = ConfigLayer::default();
    env.set(
        "ui.prompt",
        "${profile.active}:${extensions.uio.ldap.url}:${base.dir}",
    );

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    resolver.set_file(file);
    resolver.set_env(env);

    let resolved = resolver
        .resolve(ResolveOptions::default())
        .expect("config should resolve");

    assert_eq!(
        resolved.get_string("ui.prompt"),
        Some("uio:ldaps://ldap.uio.no:/etc/osp")
    );
}

#[test]
fn placeholder_cycles_raise_error_contract() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    defaults.set("extensions.a", "${extensions.b}");
    defaults.set("extensions.b", "${extensions.a}");

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);

    let err = resolver
        .resolve(ResolveOptions::default())
        .expect_err("cycle should fail");

    match err {
        ConfigError::PlaceholderCycle { cycle } => {
            assert!(cycle.iter().any(|item| item == "extensions.a"));
            assert!(cycle.iter().any(|item| item == "extensions.b"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn explicit_unknown_profile_errors_contract() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "uio");

    let mut file = ConfigLayer::default();
    file.set_for_profile("uio", "ui.format", "table");
    file.set_for_profile("tsd", "ui.format", "json");

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    resolver.set_file(file);

    let err = resolver
        .resolve(ResolveOptions::default().with_profile("prod"))
        .expect_err("unknown profile should fail");

    match err {
        ConfigError::UnknownProfile { profile, known } => {
            assert_eq!(profile, "prod");
            assert!(known.contains(&"uio".to_string()));
            assert!(known.contains(&"tsd".to_string()));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn toml_layout_parses_scopes_contract() {
    let layer = ConfigLayer::from_toml_str(
        r#"
[default]
profile.default = "uio"
ui.format = "table"

[profile.uio]
extensions.uio.osp.url = "https://osp-orchestrator.uio.no"

[profile.tsd]
ui.format = "json"

[terminal.repl]
ui.prompt.secrets = true

[terminal.repl.profile.tsd]
ui.format = "table"
"#,
    )
    .expect("toml should parse");

    let mut resolver = ConfigResolver::default();
    resolver.set_file(layer);

    let tsd_repl = resolver
        .resolve(
            ResolveOptions::default()
                .with_profile("tsd")
                .with_terminal("repl"),
        )
        .expect("tsd repl should resolve");

    assert_eq!(tsd_repl.get_string("ui.format"), Some("table"));

    let uio_cli = resolver
        .resolve(ResolveOptions::default().with_profile("uio"))
        .expect("uio cli should resolve");

    assert_eq!(
        uio_cli.get_string("extensions.uio.osp.url"),
        Some("https://osp-orchestrator.uio.no")
    );
}

#[test]
fn env_mapping_supports_profile_and_terminal_scopes_contract() {
    let env = ConfigLayer::from_env_iter([
        ("OSP__UI__FORMAT", "json"),
        ("OSP__PROFILE__TSD__UI__FORMAT", "table"),
        ("OSP__TERM__REPL__PROFILE__TSD__UI__FORMAT", "mreg"),
    ])
    .expect("env overrides should parse");

    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "tsd");

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    resolver.set_env(env);

    let resolved = resolver
        .resolve(
            ResolveOptions::default()
                .with_profile("tsd")
                .with_terminal("repl"),
        )
        .expect("config should resolve");

    assert_eq!(resolved.get_string("ui.format"), Some("mreg"));
}

#[test]
fn profile_and_terminal_matching_is_case_insensitive_contract() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "UIO");

    let mut file = ConfigLayer::default();
    file.set_for_profile_terminal("UiO", "RePl", "ui.format", "mreg");

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    resolver.set_file(file);

    let resolved = resolver
        .resolve(ResolveOptions::default().with_terminal("REPL"))
        .expect("config should resolve");

    assert_eq!(resolved.active_profile(), "uio");
    assert_eq!(resolved.terminal(), Some("repl"));
    assert_eq!(resolved.get_string("ui.format"), Some("mreg"));
}

#[test]
fn env_scope_prefix_order_is_flexible_contract() {
    let env = ConfigLayer::from_env_iter([("OSP__PROFILE__TSD__TERM__REPL__UI__FORMAT", "mreg")])
        .expect("env overrides should parse");

    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "tsd");

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    resolver.set_env(env);

    let resolved = resolver
        .resolve(
            ResolveOptions::default()
                .with_profile("TSD")
                .with_terminal("repl"),
        )
        .expect("config should resolve");

    assert_eq!(resolved.get_string("ui.format"), Some("mreg"));
}

#[test]
fn env_key_segments_preserve_single_underscore_contract() {
    let env = ConfigLayer::from_env_iter([("OSP__EXTENSIONS__DATABASE__DB_HOST", "127.0.0.1")])
        .expect("env overrides should parse");

    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    resolver.set_env(env);

    let resolved = resolver
        .resolve(ResolveOptions::default())
        .expect("config should resolve");

    assert_eq!(
        resolved.get_string("extensions.database.db_host"),
        Some("127.0.0.1")
    );
    assert!(resolved.get("database.db.host").is_none());
}

#[test]
fn unknown_keys_are_rejected_unless_extensions_contract() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    defaults.set("some.unknown.key", "value");

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);

    let err = resolver
        .resolve(ResolveOptions::default())
        .expect_err("unknown key should fail");

    match err {
        ConfigError::UnknownConfigKeys { keys } => {
            assert_eq!(keys, vec!["some.unknown.key".to_string()]);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn schema_adapts_bool_from_env_string_contract() {
    let env = ConfigLayer::from_env_iter([("OSP__UI__PROMPT__SECRETS", "true")])
        .expect("env overrides should parse");

    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    resolver.set_env(env);

    let resolved = resolver
        .resolve(ResolveOptions::default())
        .expect("config should resolve");

    assert_eq!(
        resolved.get("ui.prompt.secrets"),
        Some(&osp_config::ConfigValue::Bool(true))
    );
}

#[test]
fn schema_rejects_invalid_enum_value_contract() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    defaults.set("ui.format", "yaml");

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);

    let err = resolver
        .resolve(ResolveOptions::default())
        .expect_err("invalid enum should fail");

    match err {
        ConfigError::InvalidEnumValue { key, value, .. } => {
            assert_eq!(key, "ui.format");
            assert_eq!(value, "yaml");
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn explain_reports_precedence_chain_with_winner_contract() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    defaults.set("ui.format", "table");

    let env = ConfigLayer::from_env_iter([("OSP__UI__FORMAT", "json")])
        .expect("env overrides should parse");

    let mut cli = ConfigLayer::default();
    cli.insert_with_origin(
        "ui.format",
        "value",
        osp_config::Scope::global(),
        Some("--format"),
    );

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    resolver.set_env(env);
    resolver.set_cli(cli);

    let explain = resolver
        .explain_key("ui.format", ResolveOptions::default())
        .expect("explain should succeed");

    let winner = explain.final_entry.expect("winner should exist");
    assert_eq!(winner.source, ConfigSource::Cli);
    assert_eq!(winner.value, ConfigValue::String("value".to_string()));
    assert_eq!(winner.origin.as_deref(), Some("--format"));

    assert_eq!(explain.layers.len(), 3);
    assert_eq!(explain.layers[0].source, ConfigSource::BuiltinDefaults);
    assert_eq!(explain.layers[1].source, ConfigSource::Environment);
    assert_eq!(explain.layers[2].source, ConfigSource::Cli);
    assert!(
        explain.layers[1].candidates[0]
            .origin
            .as_deref()
            .is_some_and(|origin| origin.starts_with("OSP__UI__FORMAT"))
    );
}

#[test]
fn explain_reports_interpolation_trace_contract() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "uio");
    defaults.insert_with_origin(
        "base.dir",
        "/etc/osp",
        osp_config::Scope::global(),
        Some("defaults"),
    );
    defaults.insert_with_origin(
        "ui.prompt",
        "${profile.active}:${extensions.uio.ldap.url}:${base.dir}",
        osp_config::Scope::global(),
        Some("defaults"),
    );

    let mut file = ConfigLayer::default();
    file.insert_with_origin(
        "extensions.uio.ldap.url",
        "ldaps://ldap.uio.no",
        osp_config::Scope::profile("uio"),
        Some("/tmp/config.toml"),
    );

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    resolver.set_file(file);

    let explain = resolver
        .explain_key("ui.prompt", ResolveOptions::default())
        .expect("explain should succeed");

    let trace = explain
        .interpolation
        .expect("interpolation trace should exist");
    assert_eq!(
        trace.template,
        "${profile.active}:${extensions.uio.ldap.url}:${base.dir}"
    );

    let placeholders = trace
        .steps
        .iter()
        .map(|step| step.placeholder.clone())
        .collect::<Vec<String>>();
    assert!(placeholders.contains(&"profile.active".to_string()));
    assert!(placeholders.contains(&"extensions.uio.ldap.url".to_string()));
    assert!(placeholders.contains(&"base.dir".to_string()));
}
