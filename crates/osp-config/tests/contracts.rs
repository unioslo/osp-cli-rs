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
fn terminal_scoped_default_profile_bootstraps_contract() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");

    let mut file = ConfigLayer::default();
    file.set_for_terminal("repl", "profile.default", "tsd");
    file.set_for_profile("tsd", "ui.format", "json");

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    resolver.set_file(file);

    let resolved = resolver
        .resolve(ResolveOptions::default().with_terminal("repl"))
        .expect("config should resolve");

    assert_eq!(resolved.active_profile(), "tsd");
    assert_eq!(resolved.get_string("ui.format"), Some("json"));
}

#[test]
fn profile_scoped_default_profile_errors_contract() {
    let mut defaults = ConfigLayer::default();
    defaults.set_for_profile("work", "profile.default", "personal");

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);

    let err = resolver
        .resolve(ResolveOptions::default())
        .expect_err("profile-scoped bootstrap key should fail");

    match err {
        ConfigError::InvalidBootstrapScope {
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
    assert!(resolved.get("profile.default").is_none());
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
fn toml_parser_rejects_profile_scoped_default_profile_contract() {
    let err = ConfigLayer::from_toml_str(
        r#"
[default]
profile.default = "uio"

[profile.work]
profile.default = "personal"
"#,
    )
    .expect_err("profile-scoped bootstrap key should fail at load time");

    match err {
        ConfigError::InvalidBootstrapScope {
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
fn env_mapping_supports_bootstrap_default_profile_contract() {
    let env = ConfigLayer::from_env_iter([("OSP__PROFILE__DEFAULT", "tsd")])
        .expect("env bootstrap override should parse");

    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "uio");

    let mut file = ConfigLayer::default();
    file.set_for_profile("uio", "ui.mode", "plain");
    file.set_for_profile("tsd", "ui.mode", "rich");

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    resolver.set_file(file);
    resolver.set_env(env);

    let resolved = resolver
        .resolve(ResolveOptions::default())
        .expect("config should resolve");

    assert_eq!(resolved.active_profile(), "tsd");
    assert_eq!(resolved.get_string("ui.mode"), Some("rich"));
}

#[test]
fn env_mapping_supports_terminal_bootstrap_default_profile_contract() {
    let env = ConfigLayer::from_env_iter([("OSP__TERM__REPL__PROFILE__DEFAULT", "tsd")])
        .expect("terminal bootstrap override should parse");

    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "uio");

    let mut file = ConfigLayer::default();
    file.set_for_profile("uio", "ui.mode", "plain");
    file.set_for_profile("tsd", "ui.mode", "rich");

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    resolver.set_file(file);
    resolver.set_env(env);

    let resolved = resolver
        .resolve(ResolveOptions::default().with_terminal("repl"))
        .expect("config should resolve");

    assert_eq!(resolved.active_profile(), "tsd");
    assert_eq!(resolved.get_string("ui.mode"), Some("rich"));
}

#[test]
fn env_rejects_empty_bootstrap_default_profile_contract() {
    let err = ConfigLayer::from_env_iter([("OSP__PROFILE__DEFAULT", "")])
        .expect_err("empty bootstrap env override should fail");
    assert!(matches!(
        err,
        osp_config::ConfigError::InvalidBootstrapValue { .. }
    ));
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
    assert!(explain.layers[1].candidates[0]
        .origin
        .as_deref()
        .is_some_and(|origin| origin.starts_with("OSP__UI__FORMAT")));
}

#[test]
fn bootstrap_explain_has_explicit_type_contract() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "uio");

    let mut file = ConfigLayer::default();
    file.set_for_terminal("repl", "profile.default", "tsd");

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    resolver.set_file(file);

    let explain = resolver
        .explain_bootstrap_key(
            "profile.default",
            ResolveOptions::default().with_terminal("repl"),
        )
        .expect("bootstrap explain should succeed");

    assert_eq!(explain.key, "profile.default");
    assert_eq!(explain.active_profile, "tsd");
    assert_eq!(
        explain.active_profile_source,
        osp_config::ActiveProfileSource::DefaultProfile
    );
    assert!(explain.final_entry.is_some());
}

#[test]
fn runtime_explain_reports_active_profile_source_contract() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "uio");

    let mut file = ConfigLayer::default();
    file.set_for_profile("uio", "ui.mode", "plain");
    file.set_for_profile("tsd", "ui.mode", "rich");

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    resolver.set_file(file);

    let explain = resolver
        .explain_key("ui.mode", ResolveOptions::default().with_profile("tsd"))
        .expect("runtime explain should succeed");

    assert_eq!(explain.active_profile, "tsd");
    assert_eq!(
        explain.active_profile_source,
        osp_config::ActiveProfileSource::Override
    );
}

#[test]
fn bootstrap_explain_reports_profile_override_source_contract() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "uio");

    let mut file = ConfigLayer::default();
    file.set_for_profile("tsd", "ui.mode", "plain");

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    resolver.set_file(file);

    let explain = resolver
        .explain_bootstrap_key(
            "profile.default",
            ResolveOptions::default().with_profile("tsd"),
        )
        .expect("bootstrap explain should succeed");

    assert_eq!(explain.active_profile, "tsd");
    assert_eq!(
        explain.active_profile_source,
        osp_config::ActiveProfileSource::Override
    );
}

#[test]
fn bootstrap_rejects_empty_default_profile_contract() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "   ");

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);

    let err = resolver
        .resolve(ResolveOptions::default())
        .expect_err("empty default profile should fail");
    assert!(matches!(
        err,
        osp_config::ConfigError::InvalidBootstrapValue { .. }
    ));
}

#[test]
fn bootstrap_rejects_non_string_default_profile_contract() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", 42_i64);

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);

    let err = resolver
        .resolve(ResolveOptions::default())
        .expect_err("non-string default profile should fail");
    assert!(matches!(
        err,
        osp_config::ConfigError::InvalidBootstrapValue { .. }
    ));
}

#[test]
fn programmatic_layer_rejects_invalid_bootstrap_value_during_prepare_contract() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "   ");

    let resolver = ConfigResolver::from_loaded_layers(osp_config::LoadedLayers {
        defaults,
        ..osp_config::LoadedLayers::default()
    });

    let err = resolver
        .resolve(ResolveOptions::default())
        .expect_err("invalid bootstrap value should fail during layer validation");
    assert!(matches!(
        err,
        osp_config::ConfigError::InvalidBootstrapValue { .. }
    ));
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

#[test]
fn explain_interpolation_records_raw_and_final_placeholder_values_contract() {
    let env = ConfigLayer::from_env_iter([
        ("OSP__DEBUG__LEVEL", "2"),
        ("OSP__UI__PROMPT", "debug=${debug.level}"),
    ])
    .expect("env overrides should parse");

    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    resolver.set_env(env);

    let explain = resolver
        .explain_key("ui.prompt", ResolveOptions::default())
        .expect("explain should succeed");

    let trace = explain
        .interpolation
        .expect("interpolation trace should exist");
    let step = trace
        .steps
        .iter()
        .find(|step| step.placeholder == "debug.level")
        .expect("debug.level placeholder should be traced");

    assert_eq!(step.raw_value, ConfigValue::String("2".to_string()));
    assert_eq!(step.value, ConfigValue::Integer(2));
}

#[test]
fn explain_marks_same_layer_winner_consistently_contract() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "uio");

    let mut file = ConfigLayer::default();
    file.set("ui.format", "table");
    file.set_for_profile("uio", "ui.format", "json");
    file.set_for_terminal("repl", "ui.format", "mreg");
    file.insert_with_origin(
        "ui.format",
        "value",
        osp_config::Scope::profile_terminal("uio", "repl"),
        Some("/tmp/config.toml"),
    );

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    resolver.set_file(file);

    let resolved = resolver
        .resolve(ResolveOptions::default().with_terminal("repl"))
        .expect("config should resolve");
    assert_eq!(resolved.get_string("ui.format"), Some("value"));

    let explain = resolver
        .explain_key("ui.format", ResolveOptions::default().with_terminal("repl"))
        .expect("explain should succeed");
    let file_layer = explain
        .layers
        .iter()
        .find(|layer| layer.source == ConfigSource::ConfigFile)
        .expect("file layer should be present");

    assert_eq!(file_layer.selected_entry_index, Some(3));
    assert!(file_layer
        .candidates
        .iter()
        .any(|candidate| candidate.selected_in_layer
            && candidate.value == ConfigValue::String("value".to_string())));
}

#[test]
fn alias_values_remain_raw_and_skip_generic_interpolation_contract() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    defaults.set("user.name", "tester");
    defaults.set("alias.me", "ldap user ${user.name}");
    defaults.set("alias.arg", "ldap user ${1}");

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);

    let resolved = resolver
        .resolve(ResolveOptions::default())
        .expect("config should resolve");
    assert_eq!(
        resolved.get_string("alias.me"),
        Some("ldap user ${user.name}")
    );
    assert_eq!(resolved.get_string("alias.arg"), Some("ldap user ${1}"));

    let explain = resolver
        .explain_key("alias.me", ResolveOptions::default())
        .expect("explain should succeed");
    assert!(explain.interpolation.is_none());
}
