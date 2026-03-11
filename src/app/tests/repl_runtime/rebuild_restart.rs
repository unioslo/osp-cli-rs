#[test]
fn rebuild_repl_state_preserves_session_defaults_and_shell_context_unit() {
    let mut state = make_test_state(Vec::new());
    state.session.prompt_prefix = "osp-dev".to_string();
    state.session.history_enabled = false;
    state.session.max_cached_results = 7;
    state
        .session
        .config_overrides
        .set("user.name", "launch-user");
    state
        .session
        .config_overrides
        .set("ui.message.verbosity", "trace");
    state.session.config_overrides.set("debug.level", 2i64);
    state.session.config_overrides.set("ui.format", "json");
    state.session.config_overrides.set("theme.name", "dracula");
    state.session.scope.enter("orch");

    state.session.history_shell = HistoryShellContext::default();
    state.sync_history_shell_context();

    let next = super::super::rebuild_repl_state(&state).expect("rebuild should succeed");

    assert_eq!(
        next.runtime.config.resolved().get_string("user.name"),
        Some("launch-user")
    );
    assert_eq!(next.runtime.ui.message_verbosity, MessageLevel::Trace);
    assert_eq!(next.runtime.ui.debug_verbosity, 2);
    assert_eq!(next.runtime.ui.render_settings.format, OutputFormat::Json);
    assert_eq!(next.runtime.ui.render_settings.theme_name, "dracula");
    assert_eq!(next.session.prompt_prefix, "osp-dev");
    assert!(!next.session.history_enabled);
    assert_eq!(next.session.max_cached_results, 7);
    assert_eq!(next.session.scope.commands(), vec!["orch".to_string()]);
    assert_eq!(
        next.session.history_shell.prefix(),
        Some("orch ".to_string())
    );
}

#[test]
fn rebuild_repl_state_preserves_rich_terminal_render_runtime_unit() {
    let mut state = make_test_state(Vec::new());
    state.session.config_overrides.set("theme.name", "dracula");
    state.runtime.ui.render_settings.mode = crate::core::output::RenderMode::Auto;
    state.runtime.ui.render_settings.color = crate::core::output::ColorMode::Auto;
    state.runtime.ui.render_settings.unicode = crate::core::output::UnicodeMode::Auto;
    state.runtime.ui.render_settings.runtime.stdout_is_tty = true;
    state.runtime.ui.render_settings.runtime.locale_utf8 = Some(true);
    state.runtime.ui.render_settings.runtime.terminal = Some("xterm-256color".to_string());

    let next = super::super::rebuild_repl_state(&state).expect("rebuild should succeed");
    let resolved = next.runtime.ui.render_settings.resolve_render_settings();

    assert_eq!(next.runtime.ui.render_settings.theme_name, "dracula");
    assert!(resolved.color);
    assert!(resolved.unicode);
    assert_eq!(resolved.backend, crate::ui::RenderBackend::Rich);
    assert_eq!(resolved.theme.id, "dracula");
    assert_eq!(resolved.theme.repl_completion_text_spec(), "#000000");
}

#[test]
fn rebuild_repl_state_preserves_session_render_defaults_unit() {
    let mut state = make_test_state(Vec::new());
    state.session.config_overrides.set("ui.format", "table");

    let next = super::super::rebuild_repl_state(&state).expect("rebuild should succeed");

    assert_eq!(next.runtime.ui.render_settings.format, OutputFormat::Table);
}

#[test]
fn rebuild_repl_state_preserves_path_discovery_enabled_by_config_unit() {
    let _guard = env_lock().lock().expect("env lock should not be poisoned");
    let path_root = make_temp_dir("osp-cli-repl-path-discovery");
    let plugins_dir = path_root.join("plugins");
    std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");
    let _plugin = write_pipeline_test_plugin(&plugins_dir);

    let original_path = std::env::var_os("PATH");
    let joined_path = std::env::join_paths(
        std::iter::once(plugins_dir.clone())
            .chain(original_path.iter().flat_map(std::env::split_paths)),
    )
    .expect("PATH should join");
    unsafe { std::env::set_var("PATH", joined_path) };

    let mut state = make_test_state(Vec::new());
    state
        .session
        .config_overrides
        .set("extensions.plugins.discovery.path", true);

    let next = super::super::rebuild_repl_state(&state).expect("rebuild should succeed");
    let plugins = next
        .clients
        .plugins()
        .list_plugins()
        .expect("path-discovered plugins should list");
    assert!(plugins.iter().any(|plugin| plugin.plugin_id == "hello"));

    match original_path {
        Some(value) => unsafe { std::env::set_var("PATH", value) },
        None => unsafe { std::env::remove_var("PATH") },
    }
    let _ = std::fs::remove_dir_all(path_root);
}

#[test]
fn repl_plugin_enable_restart_refreshes_command_catalog_unit() {
    let dir = make_temp_dir("osp-cli-repl-plugin-enable");
    let _plugin = write_pipeline_test_plugin(&dir);
    let mut state = make_test_state(vec![dir.clone()]);
    state
        .clients
        .plugins()
        .set_command_state("hello", crate::plugin::state::PluginCommandState::Disabled)
        .expect("command should disable");
    assert!(
        state
            .clients
            .plugins()
            .command_catalog()
            .expect("catalog should render")
            .is_empty()
    );

    let history = make_test_history(&mut state);
    let result = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "plugins enable hello",
    )
    .expect("plugin enable should succeed");
    assert!(matches!(
        result,
        crate::repl::ReplLineResult::Restart {
            reload: crate::repl::ReplReloadKind::Default,
            ..
        }
    ));

    let next = super::super::rebuild_repl_state(&state).expect("rebuild should succeed");
    let catalog = next
        .clients
        .plugins()
        .command_catalog()
        .expect("catalog should render");
    assert!(
        catalog.iter().any(|entry| entry.name == "hello"),
        "enabled plugin should appear after rebuild"
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn repl_provider_selection_restart_invalidates_command_cache_unit() {
    let _guard = env_lock().lock().expect("env lock should not be poisoned");
    let dir = make_temp_dir("osp-cli-repl-provider-selection-restart");
    let _alpha = write_provider_test_plugin(&dir, "alpha-provider", "hello", "alpha");
    let _beta = write_provider_test_plugin(&dir, "beta-provider", "hello", "beta");
    let mut state = make_test_state(vec![dir.clone()]);
    let history = make_test_history(&mut state);

    let first = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "--plugin-provider alpha-provider hello --cache",
    )
    .expect("one-shot provider override should succeed");
    match first {
        crate::repl::ReplLineResult::Continue(text) => assert!(text.contains("alpha-from-plugin")),
        other => panic!("unexpected repl result: {other:?}"),
    }
    assert!(!state.session.command_cache.is_empty());

    let selection = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "plugins select-provider hello beta-provider",
    )
    .expect("provider selection should succeed");
    assert!(matches!(
        selection,
        crate::repl::ReplLineResult::Restart {
            reload: crate::repl::ReplReloadKind::Default,
            ..
        }
    ));

    let mut next = super::super::rebuild_repl_state(&state).expect("rebuild should succeed");
    assert!(next.session.command_cache.is_empty());
    assert!(next.session.command_cache_order.is_empty());

    let history = make_test_history(&mut next);
    let second = repl_dispatch::execute_repl_plugin_line(
        &mut next.runtime,
        &mut next.session,
        &next.clients,
        &history,
        "hello --cache",
    )
    .expect("selected provider should execute after rebuild");
    match second {
        crate::repl::ReplLineResult::Continue(text) => {
            assert!(text.contains("beta-from-plugin"));
            assert!(!text.contains("alpha-from-plugin"));
        }
        other => panic!("unexpected repl result: {other:?}"),
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn repl_reload_intent_matches_command_scope_unit() {
    let mut state = make_test_state(Vec::new());
    state.runtime.themes =
        crate::ui::theme_loader::load_theme_catalog(state.runtime.config.resolved());
    let history = make_test_history(&mut state);

    let theme_result = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "theme use dracula",
    )
    .expect("theme use should succeed");
    assert!(matches!(
        theme_result,
        crate::repl::ReplLineResult::Restart {
            reload: crate::repl::ReplReloadKind::WithIntro,
            ..
        }
    ));

    let format_result = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "config set ui.format json",
    )
    .expect("config set should succeed");
    assert!(matches!(
        format_result,
        crate::repl::ReplLineResult::Restart {
            reload: crate::repl::ReplReloadKind::Default,
            ..
        }
    ));

    let color_result = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "config set color.prompt.text '#ffffff'",
    )
    .expect("color config set should succeed");
    assert!(matches!(
        color_result,
        crate::repl::ReplLineResult::Restart {
            reload: crate::repl::ReplReloadKind::WithIntro,
            ..
        }
    ));

    let unset_format_result = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "config unset ui.format",
    )
    .expect("config unset should succeed");
    assert!(matches!(
        unset_format_result,
        crate::repl::ReplLineResult::Restart {
            reload: crate::repl::ReplReloadKind::Default,
            ..
        }
    ));

    let unset_color_result = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "config unset color.prompt.text",
    )
    .expect("color config unset should succeed");
    assert!(matches!(
        unset_color_result,
        crate::repl::ReplLineResult::Restart {
            reload: crate::repl::ReplReloadKind::WithIntro,
            ..
        }
    ));

    let dry_run_unset_result = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "config unset ui.format --dry-run",
    )
    .expect("dry-run config unset should succeed");
    assert!(matches!(
        dry_run_unset_result,
        crate::repl::ReplLineResult::Continue(_)
    ));
}

#[test]
fn repl_config_unset_rebuilds_runtime_state_unit() {
    let mut state = make_test_state(Vec::new());
    state
        .session
        .config_overrides
        .set_for_profile("default", "ui.format", "table");
    state = super::super::rebuild_repl_state(&state).expect("rebuild should succeed");
    assert_eq!(state.runtime.ui.render_settings.format, OutputFormat::Table);

    let history = make_test_history(&mut state);
    let result = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "config unset ui.format",
    )
    .expect("config unset should succeed");
    assert!(matches!(
        result,
        crate::repl::ReplLineResult::Restart {
            reload: crate::repl::ReplReloadKind::Default,
            ..
        }
    ));
    assert_eq!(
        layer_value(&state.session.config_overrides, "ui.format"),
        None
    );

    let next = super::super::rebuild_repl_state(&state).expect("rebuild should succeed");
    assert_eq!(next.runtime.config.resolved().get_string("ui.format"), None);
    assert_eq!(next.runtime.ui.render_settings.format, OutputFormat::Auto);
}

#[test]
fn repl_config_unset_dry_run_preserves_session_state_unit() {
    let mut state = make_test_state(Vec::new());
    state
        .session
        .config_overrides
        .set_for_profile("default", "ui.format", "table");
    state = super::super::rebuild_repl_state(&state).expect("rebuild should succeed");
    assert_eq!(state.runtime.ui.render_settings.format, OutputFormat::Table);

    let history = make_test_history(&mut state);
    let result = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "config unset ui.format --dry-run",
    )
    .expect("dry-run config unset should succeed");
    assert!(matches!(result, crate::repl::ReplLineResult::Continue(_)));
    assert_eq!(
        layer_value(&state.session.config_overrides, "ui.format"),
        Some(&ConfigValue::from("table"))
    );
    assert_eq!(state.runtime.ui.render_settings.format, OutputFormat::Table);
}

#[test]
fn repl_config_prompt_color_change_rebuilds_deterministically_unit() {
    let mut state = make_test_state(Vec::new());
    state
        .session
        .config_overrides
        .set("ui.color.mode", "always");
    state.session.config_overrides.set("ui.mode", "rich");
    state
        .session
        .config_overrides
        .set("repl.simple_prompt", true);
    state = super::super::rebuild_repl_state(&state).expect("rebuild should succeed");
    assert!(
        state
            .runtime
            .ui
            .render_settings
            .resolve_render_settings()
            .color
    );

    let default_prompt =
        crate::repl::presentation::build_repl_prompt(repl_view(&state.runtime, &state.session))
            .left;
    assert!(default_prompt.contains("\x1b["));

    let history = make_test_history(&mut state);
    let result = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "config set color.prompt.text white",
    )
    .expect("prompt color config set should succeed");
    assert!(matches!(
        result,
        crate::repl::ReplLineResult::Restart {
            reload: crate::repl::ReplReloadKind::WithIntro,
            ..
        }
    ));

    state = super::super::rebuild_repl_state(&state).expect("rebuild should succeed");
    let white_prompt =
        crate::repl::presentation::build_repl_prompt(repl_view(&state.runtime, &state.session))
            .left;
    assert!(white_prompt.contains("\x1b[37mdefault"));

    let history = make_test_history(&mut state);
    let result = repl_dispatch::execute_repl_plugin_line(
        &mut state.runtime,
        &mut state.session,
        &state.clients,
        &history,
        "config unset color.prompt.text",
    )
    .expect("prompt color config unset should succeed");
    assert!(matches!(
        result,
        crate::repl::ReplLineResult::Restart {
            reload: crate::repl::ReplReloadKind::WithIntro,
            ..
        }
    ));

    state = super::super::rebuild_repl_state(&state).expect("rebuild should succeed");
    let restored_prompt =
        crate::repl::presentation::build_repl_prompt(repl_view(&state.runtime, &state.session))
            .left;
    assert_eq!(restored_prompt, default_prompt);
}
