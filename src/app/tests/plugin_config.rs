use super::*;

#[test]
fn plugin_format_hint_parser_supports_known_values_unit() {
    assert_eq!(
        parse_output_format_hint(Some("table")),
        Some(OutputFormat::Table)
    );
    assert_eq!(
        parse_output_format_hint(Some("mreg")),
        Some(OutputFormat::Mreg)
    );
    assert_eq!(
        parse_output_format_hint(Some("markdown")),
        Some(OutputFormat::Markdown)
    );
    assert_eq!(parse_output_format_hint(Some("unknown")), None);
}

#[test]
fn plugin_config_env_name_normalizes_extension_keys_unit() {
    assert_eq!(
        plugin_config_env_name("api.token"),
        Some("OSP_PLUGIN_CFG_API_TOKEN".to_string())
    );
    assert_eq!(
        plugin_config_env_name("nested-value/path"),
        Some("OSP_PLUGIN_CFG_NESTED_VALUE_PATH".to_string())
    );
    assert_eq!(plugin_config_env_name("..."), None);
}

#[test]
fn plugin_config_env_serializes_lists_and_secrets_unit() {
    assert_eq!(
        config_value_to_plugin_env(&ConfigValue::List(vec![
            ConfigValue::String("alpha".to_string()),
            ConfigValue::Integer(2),
            ConfigValue::Bool(true),
        ])),
        r#"["alpha",2,true]"#
    );
    assert_eq!(
        config_value_to_plugin_env(&ConfigValue::String("sekrit".to_string()).into_secret()),
        "sekrit"
    );
}

#[test]
fn plugin_process_timeout_reads_config_override_unit() {
    let config = test_config(&[("extensions.plugins.timeout_ms", "250")]);
    assert_eq!(
        plugin_process_timeout(&config),
        std::time::Duration::from_millis(250)
    );

    let fallback = test_config(&[]);
    assert_eq!(
        plugin_process_timeout(&fallback),
        std::time::Duration::from_millis(DEFAULT_PLUGIN_PROCESS_TIMEOUT_MS as u64)
    );
}

#[test]
fn plugin_path_discovery_defaults_off_and_respects_config_unit() {
    assert!(!plugin_path_discovery_enabled(&test_config(&[])));
    assert!(plugin_path_discovery_enabled(&test_config(&[(
        "extensions.plugins.discovery.path",
        "true",
    )])));
}

#[test]
fn plugin_config_env_collects_shared_and_plugin_specific_entries_unit() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    defaults.set(
        "extensions.plugins.env.shared.url",
        "https://common.example",
    );
    defaults.set("extensions.plugins.env.endpoint", "shared");
    defaults.set("extensions.plugins.cfg.env.endpoint", "plugin");
    defaults.set("extensions.plugins.cfg.env.api.token", "token-123");
    defaults.set("extensions.plugins.other.env.endpoint", "other");
    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    let config = resolver
        .resolve(ResolveOptions::default())
        .expect("test config should resolve");

    let env = collect_plugin_config_env(&config);

    assert_eq!(
        env.shared,
        vec![
            PluginConfigEntry {
                env_key: "OSP_PLUGIN_CFG_ENDPOINT".to_string(),
                value: "shared".to_string(),
                config_key: "extensions.plugins.env.endpoint".to_string(),
                scope: PluginConfigScope::Shared,
            },
            PluginConfigEntry {
                env_key: "OSP_PLUGIN_CFG_SHARED_URL".to_string(),
                value: "https://common.example".to_string(),
                config_key: "extensions.plugins.env.shared.url".to_string(),
                scope: PluginConfigScope::Shared,
            },
        ]
    );
    assert_eq!(
        env.by_plugin_id.get("cfg"),
        Some(&vec![
            PluginConfigEntry {
                env_key: "OSP_PLUGIN_CFG_API_TOKEN".to_string(),
                value: "token-123".to_string(),
                config_key: "extensions.plugins.cfg.env.api.token".to_string(),
                scope: PluginConfigScope::Plugin,
            },
            PluginConfigEntry {
                env_key: "OSP_PLUGIN_CFG_ENDPOINT".to_string(),
                value: "plugin".to_string(),
                config_key: "extensions.plugins.cfg.env.endpoint".to_string(),
                scope: PluginConfigScope::Plugin,
            },
        ])
    );
    assert_eq!(
        env.by_plugin_id.get("other"),
        Some(&vec![PluginConfigEntry {
            env_key: "OSP_PLUGIN_CFG_ENDPOINT".to_string(),
            value: "other".to_string(),
            config_key: "extensions.plugins.other.env.endpoint".to_string(),
            scope: PluginConfigScope::Plugin,
        }])
    );
}

#[test]
fn plugin_dispatch_context_refreshes_cached_plugin_env_after_config_change() {
    let mut state =
        make_completion_state_with_entries(None, &[("extensions.plugins.env.endpoint", "before")]);
    let before = super::plugin_dispatch_context_for_runtime(&state.runtime, &state.clients, None);
    assert_eq!(
        before.shared_env,
        vec![("OSP_PLUGIN_CFG_ENDPOINT".to_string(), "before".to_string(),)]
    );

    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    defaults.set("extensions.plugins.env.endpoint", "after");
    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    let updated = resolver
        .resolve(ResolveOptions::default().with_terminal("repl"))
        .expect("test config should resolve");
    assert!(state.runtime.config.replace_resolved(updated));

    let after = super::plugin_dispatch_context_for_runtime(&state.runtime, &state.clients, None);
    assert_eq!(
        after.shared_env,
        vec![("OSP_PLUGIN_CFG_ENDPOINT".to_string(), "after".to_string(),)]
    );
}

#[cfg(unix)]
#[test]
fn app_state_seeds_plugin_command_policy_registry_unit() {
    let root = make_temp_dir("osp-cli-test-plugin-policy-seed");
    let plugins_dir = root.join("plugins");
    std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");
    write_auth_pipeline_test_plugin(&plugins_dir);

    let state = make_test_state(vec![plugins_dir]);

    let root_policy = state
        .runtime
        .auth
        .external_policy()
        .resolved_policy(&CommandPath::new(["orch"]))
        .expect("root plugin policy should exist");
    assert_eq!(root_policy.visibility, VisibilityMode::Authenticated);

    let nested_policy = state
        .runtime
        .auth
        .external_policy()
        .resolved_policy(&CommandPath::new(["orch", "approval", "decide"]))
        .expect("nested plugin policy should exist");
    assert!(
        nested_policy
            .required_capabilities
            .contains("orch.approval.decide")
    );
}
