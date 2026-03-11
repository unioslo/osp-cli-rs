#[test]
fn command_preferences_load_state_and_provider_from_resolved_config_unit() {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    defaults.set("plugins.ldap.state", "disabled");
    defaults.set("plugins.ldap.provider", "uio-ldap");
    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    let resolved = resolver
        .resolve(ResolveOptions::default().with_terminal("cli"))
        .expect("config should resolve");

    let preferences = PluginCommandPreferences::from_resolved(&resolved);
    assert_eq!(
        preferences.command_states.get("ldap"),
        Some(&PluginCommandState::Disabled)
    );
    assert_eq!(
        preferences
            .preferred_providers
            .get("ldap")
            .map(String::as_str),
        Some("uio-ldap")
    );
}

#[cfg(unix)]
#[test]
fn ambiguous_command_requires_explicit_selection() {
    let root = make_temp_dir("osp-cli-plugin-manager-ambiguous-command");
    let plugins_dir = root.join("plugins");
    std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");

    write_provider_test_plugin(&plugins_dir, "alpha", "shared");
    write_provider_test_plugin(&plugins_dir, "beta", "shared");
    let manager = PluginManager::new(vec![plugins_dir.clone()]);

    let catalog = manager.command_catalog().expect("catalog should load");
    let entry = catalog
        .iter()
        .find(|entry| entry.name == "shared")
        .expect("shared command should exist");
    assert_eq!(entry.provider, None);
    assert!(entry.requires_selection);
    assert!(!entry.selected_explicitly);
    assert_eq!(
        manager
            .selected_provider_label("shared")
            .expect("selected provider label should load"),
        None
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn preferred_provider_updates_catalog_and_resolves_command() {
    let root = make_temp_dir("osp-cli-plugin-manager-preferred-provider");
    let plugins_dir = root.join("plugins");
    let config_root = root.join("config");
    let cache_root = root.join("cache");
    std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");

    write_provider_test_plugin(&plugins_dir, "alpha", "shared");
    write_provider_test_plugin(&plugins_dir, "beta", "shared");
    let manager = PluginManager::new(vec![plugins_dir.clone()])
        .with_roots(Some(config_root.clone()), Some(cache_root.clone()));

    manager
        .set_preferred_provider("shared", "beta")
        .expect("preferred provider should be saved");

    let catalog = manager.command_catalog().expect("catalog should load");
    let entry = catalog
        .iter()
        .find(|entry| entry.name == "shared")
        .expect("shared command should exist");
    assert_eq!(entry.provider.as_deref(), Some("beta"));
    assert!(!entry.requires_selection);
    assert!(entry.selected_explicitly);
    assert_eq!(
        manager
            .selected_provider_label("shared")
            .expect("selected provider label should load")
            .as_deref(),
        Some("beta (explicit)")
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn clearing_preferred_provider_requires_selection_again() {
    let root = make_temp_dir("osp-cli-plugin-manager-clear-preference");
    let plugins_dir = root.join("plugins");
    let config_root = root.join("config");
    let cache_root = root.join("cache");
    std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");

    write_provider_test_plugin(&plugins_dir, "alpha", "shared");
    write_provider_test_plugin(&plugins_dir, "beta", "shared");
    let manager = PluginManager::new(vec![plugins_dir.clone()])
        .with_roots(Some(config_root.clone()), Some(cache_root.clone()));

    manager
        .set_preferred_provider("shared", "beta")
        .expect("preferred provider should be saved");
    assert!(
        manager
            .clear_preferred_provider("shared")
            .expect("clearing preferred provider should succeed")
    );

    let catalog = manager.command_catalog().expect("catalog should load");
    let entry = catalog
        .iter()
        .find(|entry| entry.name == "shared")
        .expect("shared command should exist");
    assert_eq!(entry.provider, None);
    assert!(entry.requires_selection);
    assert!(!entry.selected_explicitly);
    assert_eq!(
        manager
            .selected_provider_label("shared")
            .expect("selected provider label should load"),
        None
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn command_policy_registry_collects_recursive_plugin_auth_metadata_unit() {
    let root = make_temp_dir("osp-cli-plugin-manager-policy-registry");
    let plugins_dir = root.join("plugins");
    std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");

    write_auth_test_plugin(&plugins_dir, "orch");
    let manager = PluginManager::new(vec![plugins_dir]);
    let registry = manager
        .command_policy_registry()
        .expect("policy registry should build");

    let root_policy = registry
        .resolved_policy(&CommandPath::new(["orch"]))
        .expect("root command policy should exist");
    assert_eq!(root_policy.visibility, VisibilityMode::Authenticated);

    let nested_policy = registry
        .resolved_policy(&CommandPath::new(["orch", "approval", "decide"]))
        .expect("nested command policy should exist");
    assert_eq!(nested_policy.visibility, VisibilityMode::CapabilityGated);
    assert!(
        nested_policy
            .required_capabilities
            .contains("orch.approval.decide")
    );
    assert!(nested_policy.feature_flags.contains("orch"));

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn preferred_provider_rejects_unknown_command_and_provider_unit() {
    let root = make_temp_dir("osp-cli-plugin-manager-invalid-provider");
    let plugins_dir = root.join("plugins");
    let config_root = root.join("config");
    let cache_root = root.join("cache");
    std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");

    write_provider_test_plugin(&plugins_dir, "alpha", "shared");
    let manager = PluginManager::new(vec![plugins_dir.clone()])
        .with_roots(Some(config_root), Some(cache_root));

    let err = manager
        .set_preferred_provider("missing", "alpha")
        .expect_err("unknown command should fail");
    assert!(
        err.to_string()
            .contains("no healthy plugin provides command")
    );

    let err = manager
        .set_preferred_provider("shared", "beta")
        .expect_err("unknown provider should fail");
    assert!(err.to_string().contains("does not provide healthy command"));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn clear_preferred_provider_rejects_empty_command_unit() {
    let manager = PluginManager::new(Vec::new());
    let err = manager
        .clear_preferred_provider("   ")
        .expect_err("empty command should fail");
    assert!(err.to_string().contains("command must not be empty"));
}

#[test]
fn preferred_provider_rejects_empty_plugin_id_unit() {
    let manager = PluginManager::new(Vec::new());
    let err = manager
        .set_preferred_provider("shared", "   ")
        .expect_err("empty plugin id should fail");
    assert!(err.to_string().contains("plugin id must not be empty"));
}

#[cfg(unix)]
#[test]
fn disabling_a_command_updates_only_that_command_in_memory_unit() {
    let root = make_temp_dir("osp-cli-plugin-manager-in-memory-disable");
    let plugins_dir = root.join("plugins");
    std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");
    write_multi_command_plugin(&plugins_dir, "multi", &["ldap", "extra"]);

    let manager = PluginManager::new(vec![plugins_dir]);
    manager
        .set_command_state("ldap", PluginCommandState::Disabled)
        .expect("disabling command should succeed");

    let catalog = manager.command_catalog().expect("catalog should load");
    assert!(
        !catalog.iter().any(|entry| entry.name == "ldap"),
        "disabled command should be removed"
    );
    assert!(
        catalog.iter().any(|entry| entry.name == "extra"),
        "other commands from the same plugin should remain available"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn config_backed_preferences_can_disable_and_route_commands_unit() {
    let root = make_temp_dir("osp-cli-plugin-manager-config-preferences");
    let plugins_dir = root.join("plugins");
    std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");
    write_provider_test_plugin(&plugins_dir, "alpha", "shared");
    write_provider_test_plugin(&plugins_dir, "beta", "shared");

    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    defaults.set("plugins.shared.state", "disabled");
    defaults.set("plugins.shared.provider", "beta");
    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    let resolved = resolver
        .resolve(ResolveOptions::default().with_terminal("cli"))
        .expect("config should resolve");

    let manager = PluginManager::new(vec![plugins_dir])
        .with_command_preferences(PluginCommandPreferences::from_resolved(&resolved));

    assert!(
        manager
            .dispatch("shared", &[], &PluginDispatchContext::default())
            .is_err(),
        "disabled command should not dispatch"
    );

    manager
        .set_command_state("shared", PluginCommandState::Enabled)
        .expect("re-enabling selected command should work");
    let provider = manager
        .selected_provider_label("shared")
        .expect("selected provider label should load");
    assert_eq!(provider.as_deref(), Some("beta (explicit)"));

    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn repl_help_and_provider_listing_cover_selected_and_conflicted_commands_unit() {
    let root = make_temp_dir("osp-cli-plugin-manager-help");
    let plugins_dir = root.join("plugins");
    let config_root = root.join("config");
    let cache_root = root.join("cache");
    std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");

    write_provider_test_plugin(&plugins_dir, "alpha", "shared");
    write_provider_test_plugin(&plugins_dir, "beta", "shared");
    write_named_test_plugin(&plugins_dir, "solo");
    write_auth_test_plugin(&plugins_dir, "orch");

    let manager = PluginManager::new(vec![plugins_dir.clone()])
        .with_roots(Some(config_root.clone()), Some(cache_root.clone()));

    let ambiguous_catalog = manager.command_catalog().expect("catalog should render");
    let shared_entry = ambiguous_catalog
        .iter()
        .find(|entry| entry.name == "shared")
        .expect("shared command should exist");
    assert!(shared_entry.requires_selection);
    assert_eq!(shared_entry.provider, None);

    let ambiguous_help = manager.repl_help_text().expect("help should render");
    assert!(
        ambiguous_help.contains("shared"),
        "help output:\n{ambiguous_help}"
    );
    assert!(
        ambiguous_help.contains("provider selection required"),
        "help output:\n{ambiguous_help}"
    );
    assert!(
        ambiguous_help.contains("alpha"),
        "help output:\n{ambiguous_help}"
    );
    assert!(
        ambiguous_help.contains("beta"),
        "help output:\n{ambiguous_help}"
    );
    assert!(ambiguous_help.contains("solo - solo plugin"));
    assert!(
        ambiguous_help.contains("orch"),
        "help output:\n{ambiguous_help}"
    );
    assert!(
        ambiguous_help.contains("[auth]"),
        "help output:\n{ambiguous_help}"
    );
    let completion_words = manager
        .completion_words()
        .expect("completion words should render");
    assert!(completion_words.contains(&"help".to_string()));
    assert!(completion_words.contains(&"shared".to_string()));
    assert!(completion_words.contains(&"solo".to_string()));
    assert_eq!(
        manager
            .command_providers("shared")
            .expect("command providers should load"),
        vec![
            format!("alpha ({})", PluginSource::Explicit),
            format!("beta ({})", PluginSource::Explicit)
        ]
    );
    assert_eq!(
        manager
            .selected_provider_label("shared")
            .expect("selected provider label should load"),
        None
    );
    assert_eq!(
        manager
            .selected_provider_label("solo")
            .expect("selected provider label should load")
            .as_deref(),
        Some("solo (explicit)")
    );

    let doctor = manager.doctor().expect("doctor should render");
    assert_eq!(doctor.conflicts.len(), 1);
    assert_eq!(doctor.conflicts[0].command, "shared");
    assert_eq!(doctor.plugins.len(), 4);

    manager
        .set_preferred_provider("shared", "beta")
        .expect("preferred provider should save");
    let preferred_catalog = manager.command_catalog().expect("catalog should refresh");
    let preferred_entry = preferred_catalog
        .iter()
        .find(|entry| entry.name == "shared")
        .expect("shared command should still exist");
    assert_eq!(preferred_entry.provider.as_deref(), Some("beta"));
    assert!(preferred_entry.selected_explicitly);
    let preferred_help = manager
        .repl_help_text()
        .expect("preferred provider help should render");
    assert!(preferred_help.contains("shared - beta plugin (beta/explicit)"));

    let _ = std::fs::remove_dir_all(&root);
}
