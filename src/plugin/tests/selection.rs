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
fn provider_selection_validation_rejects_empty_unknown_and_mismatched_inputs_unit() {
    let empty_manager = PluginManager::new(Vec::new());
    let err = empty_manager
        .clear_provider_selection("   ")
        .expect_err("empty command should fail");
    assert!(err.to_string().contains("command must not be empty"));

    let err = empty_manager
        .select_provider("shared", "   ")
        .expect_err("empty plugin id should fail");
    assert!(err.to_string().contains("plugin id must not be empty"));

    let root = make_temp_dir("osp-cli-plugin-manager-invalid-provider");
    let plugins_dir = root.join("plugins");
    let config_root = root.join("config");
    let cache_root = root.join("cache");
    std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");

    write_provider_test_plugin(&plugins_dir, "alpha", "shared");
    let manager = PluginManager::new(vec![plugins_dir.clone()])
        .with_roots(Some(config_root), Some(cache_root));

    let err = manager
        .select_provider("missing", "alpha")
        .expect_err("unknown command should fail");
    assert!(
        err.to_string()
            .contains("no available plugin provides command")
    );

    let err = manager
        .select_provider("shared", "beta")
        .expect_err("unknown provider should fail");
    assert!(err.to_string().contains("is not currently available"));
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

    let catalog = manager.command_catalog();
    assert!(
        !catalog.iter().any(|entry| entry.name == "ldap"),
        "disabled command should be removed"
    );
    assert!(
        catalog.iter().any(|entry| entry.name == "extra"),
        "other commands from the same plugin should remain available"
    );
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
    let provider = manager.selected_provider_label("shared");
    assert_eq!(provider.as_deref(), Some("beta (explicit)"));
}

#[cfg(unix)]
#[test]
fn provider_selection_validation_respects_current_command_availability_unit() {
    let root = make_temp_dir("osp-cli-plugin-manager-provider-availability");
    let plugins_dir = root.join("plugins");
    std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");
    write_provider_test_plugin(&plugins_dir, "alpha", "shared");

    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    defaults.set("plugins.shared.state", "disabled");
    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    let resolved = resolver
        .resolve(ResolveOptions::default().with_terminal("cli"))
        .expect("config should resolve");

    let manager = PluginManager::new(vec![plugins_dir])
        .with_command_preferences(PluginCommandPreferences::from_resolved(&resolved));

    let err = manager
        .validate_provider_selection("shared", "alpha")
        .expect_err("disabled command should not accept provider selections");
    assert!(
        err.to_string()
            .contains("no available plugin provides command `shared`")
    );
}
