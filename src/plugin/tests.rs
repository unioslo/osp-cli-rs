use super::conversion::{
    collect_completion_words, direct_subcommand_names, to_arg_node, to_command_spec, to_flag_node,
    to_suggestion_entry, to_value_type,
};
use super::discovery::{
    DescribeCacheEntry, DescribeCacheFile, ManifestPlugin, ManifestState, SearchRoot,
    ValidatedBundledManifest, assemble_discovered_plugin, bundled_manifest_path,
    discover_root_executables, existing_unique_search_roots, file_fingerprint, file_sha256_hex,
    find_cached_describe, has_valid_plugin_suffix, load_manifest_state,
    load_manifest_state_from_path, min_osp_version_issue, normalize_checksum,
    prune_stale_describe_cache_entries, upsert_cached_describe,
};
use super::dispatch::{describe_plugin, run_provider};
use super::manager::{
    DiscoveredPlugin, PluginDispatchContext, PluginDispatchError, PluginManager, PluginSource,
};
use super::state::{PluginState, is_enabled, merge_issue};
use crate::core::command_policy::{CommandPath, VisibilityMode};
use crate::core::plugin::{
    DescribeArgV1, DescribeCommandV1, DescribeFlagV1, DescribeSuggestionV1, DescribeV1,
};
use std::collections::{BTreeMap, HashMap};
use std::error::Error as _;
#[cfg(unix)]
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

#[test]
fn explicit_enable_overrides_default_disabled() {
    let state = PluginState {
        enabled: vec!["hello".to_string()],
        disabled: Vec::new(),
        preferred_providers: BTreeMap::new(),
    };

    assert!(is_enabled(&state, "hello", false));
}

#[test]
fn explicit_disable_overrides_default_enabled() {
    let state = PluginState {
        enabled: Vec::new(),
        disabled: vec!["hello".to_string()],
        preferred_providers: BTreeMap::new(),
    };

    assert!(!is_enabled(&state, "hello", true));
}

#[test]
fn enabling_one_plugin_does_not_disable_other_default_enabled_plugins() {
    let state = PluginState {
        enabled: vec!["alpha".to_string()],
        disabled: Vec::new(),
        preferred_providers: BTreeMap::new(),
    };

    assert!(is_enabled(&state, "alpha", true));
    assert!(is_enabled(&state, "beta", true));
}

#[test]
fn explicit_enable_wins_if_state_file_contains_conflicting_entries() {
    let state = PluginState {
        enabled: vec!["hello".to_string()],
        disabled: vec!["hello".to_string()],
        preferred_providers: BTreeMap::new(),
    };

    assert!(is_enabled(&state, "hello", false));
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

#[test]
fn compatible_min_osp_version_has_no_issue() {
    let describe = DescribeV1 {
        protocol_version: 1,
        plugin_id: "hello".to_string(),
        plugin_version: "0.1.0".to_string(),
        min_osp_version: Some("0.1.0".to_string()),
        commands: Vec::new(),
    };

    assert_eq!(min_osp_version_issue(&describe), None);
}

#[test]
fn invalid_min_osp_version_reports_issue() {
    let describe = DescribeV1 {
        protocol_version: 1,
        plugin_id: "hello".to_string(),
        plugin_version: "0.1.0".to_string(),
        min_osp_version: Some("not-a-version".to_string()),
        commands: Vec::new(),
    };

    let issue = min_osp_version_issue(&describe).expect("invalid version should report issue");
    assert!(issue.contains("invalid min_osp_version"));
    assert!(issue.contains("hello"));
}

#[cfg(unix)]
#[test]
fn refresh_picks_up_filesystem_changes_and_prunes_stale_cache() {
    let root = make_temp_dir("osp-cli-plugin-manager-refresh");
    let plugins_dir = root.join("plugins");
    let config_root = root.join("config");
    let cache_root = root.join("cache");
    std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");

    let alpha_path = write_named_test_plugin(&plugins_dir, "alpha");
    let manager = PluginManager::new(vec![plugins_dir.clone()])
        .with_roots(Some(config_root.clone()), Some(cache_root.clone()));

    let first = manager.list_plugins().expect("plugins should list");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].plugin_id, "alpha");

    std::fs::remove_file(&alpha_path).expect("alpha plugin should be removable");
    write_named_test_plugin(&plugins_dir, "beta");

    let cached = manager.list_plugins().expect("cached plugins should list");
    assert_eq!(cached.len(), 1);
    assert_eq!(cached[0].plugin_id, "alpha");

    manager.refresh();
    let refreshed = manager
        .list_plugins()
        .expect("refreshed plugins should list");
    assert_eq!(refreshed.len(), 1);
    assert_eq!(refreshed[0].plugin_id, "beta");

    let cache_path = cache_root.join("describe-v1.json");
    let cache_raw = std::fs::read_to_string(&cache_path).expect("describe cache should be written");
    assert!(cache_raw.contains("osp-beta"));
    assert!(!cache_raw.contains("osp-alpha"));

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
fn incompatible_min_osp_version_marks_plugin_unhealthy() {
    let root = make_temp_dir("osp-cli-plugin-manager-min-version");
    let plugins_dir = root.join("plugins");
    std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");

    write_named_test_plugin_with_min_version(&plugins_dir, "future", "9.9.9");
    let manager = PluginManager::new(vec![plugins_dir.clone()]);

    let plugins = manager.list_plugins().expect("plugins should list");
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].plugin_id, "future");
    assert!(!plugins[0].healthy);
    assert!(
        plugins[0]
            .issue
            .as_deref()
            .expect("issue should be present")
            .contains("requires osp >=")
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn dispatch_times_out_hung_plugin() {
    let root = make_temp_dir("osp-cli-plugin-manager-dispatch-timeout");
    let plugins_dir = root.join("plugins");
    std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");

    write_sleepy_test_plugin(&plugins_dir, "hang", false);
    let manager = PluginManager::new(vec![plugins_dir.clone()])
        .with_process_timeout(Duration::from_millis(50));

    let err = manager
        .dispatch("hang", &[], &PluginDispatchContext::default())
        .expect_err("hung plugin should time out");

    match err {
        PluginDispatchError::TimedOut {
            plugin_id, timeout, ..
        } => {
            assert_eq!(plugin_id, "hang");
            assert_eq!(timeout, Duration::from_millis(50));
        }
        other => panic!("expected timeout error, got {other}"),
    }

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn dispatch_drains_large_plugin_output_without_false_timeout_unit() {
    let root = make_temp_dir("osp-cli-plugin-manager-large-output");
    let plugins_dir = root.join("plugins");
    std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");

    write_large_output_test_plugin(&plugins_dir, "loud");
    let manager = PluginManager::new(vec![plugins_dir.clone()])
        .with_process_timeout(Duration::from_millis(500));

    let response = manager
        .dispatch("loud", &[], &PluginDispatchContext::default())
        .expect("large-output plugin should complete without timing out");

    assert!(response.ok);
    assert!(
        response
            .data
            .as_object()
            .and_then(|data| data.get("blob"))
            .and_then(|value| value.as_str())
            .is_some_and(|blob| blob.len() >= 131_072)
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn timed_out_plugin_terminates_background_process_group_unit() {
    let root = make_temp_dir("osp-cli-plugin-manager-timeout-process-group");
    let plugins_dir = root.join("plugins");
    let marker = root.join("leaked-child.txt");
    std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");

    write_timeout_leak_test_plugin(&plugins_dir, "hang", &marker);
    let manager = PluginManager::new(vec![plugins_dir.clone()])
        .with_process_timeout(Duration::from_millis(50));

    let err = manager
        .dispatch("hang", &[], &PluginDispatchContext::default())
        .expect_err("hung plugin should time out");
    assert!(matches!(err, PluginDispatchError::TimedOut { .. }));

    std::thread::sleep(Duration::from_millis(350));
    assert!(
        !marker.exists(),
        "timed-out plugin left a background child behind"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn duplicate_plugin_ids_are_marked_unhealthy_and_removed_from_catalog_unit() {
    let root = make_temp_dir("osp-cli-plugin-manager-duplicate-ids");
    let plugins_dir = root.join("plugins");
    std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");

    write_mismatched_id_plugin(&plugins_dir, "alpha-bin", "shared-id", "alpha");
    write_mismatched_id_plugin(&plugins_dir, "beta-bin", "shared-id", "beta");
    let manager = PluginManager::new(vec![plugins_dir.clone()]);

    let plugins = manager.list_plugins().expect("plugins should list");
    assert_eq!(plugins.len(), 2);
    assert!(plugins.iter().all(|plugin| !plugin.healthy));
    assert!(plugins.iter().all(|plugin| {
        plugin
            .issue
            .as_deref()
            .is_some_and(|issue| issue.contains("duplicate plugin id `shared-id`"))
    }));

    let catalog = manager.command_catalog().expect("catalog should render");
    assert!(
        catalog.is_empty(),
        "duplicate providers should not stay active"
    );
    assert!(matches!(
        manager.resolve_provider("alpha", None),
        Err(PluginDispatchError::CommandNotFound { .. })
    ));

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn hung_describe_marks_plugin_unhealthy() {
    let root = make_temp_dir("osp-cli-plugin-manager-describe-timeout");
    let plugins_dir = root.join("plugins");
    std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");

    write_sleepy_test_plugin(&plugins_dir, "hang-describe", true);
    let manager = PluginManager::new(vec![plugins_dir.clone()])
        .with_process_timeout(Duration::from_millis(50));

    let plugins = manager.list_plugins().expect("plugins should list");
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].plugin_id, "hang-describe");
    assert!(!plugins[0].healthy);
    assert!(
        plugins[0]
            .issue
            .as_deref()
            .expect("issue should be present")
            .contains("timed out after 50 ms")
    );

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
            .contains("no active plugin provides command")
    );

    let err = manager
        .set_preferred_provider("shared", "beta")
        .expect_err("unknown provider should fail");
    assert!(err.to_string().contains("does not provide active command"));

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
fn set_enabled_and_clear_missing_provider_preference_cover_state_round_trip_unit() {
    let root = make_temp_dir("osp-cli-plugin-manager-state-round-trip");
    let plugins_dir = root.join("plugins");
    let config_root = root.join("config");
    let cache_root = root.join("cache");
    std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");

    write_named_test_plugin(&plugins_dir, "alpha");
    let manager = PluginManager::new(vec![plugins_dir])
        .with_roots(Some(config_root.clone()), Some(cache_root));

    manager
        .set_enabled("alpha", false)
        .expect("disabling plugin should succeed");
    manager
        .set_enabled("alpha", true)
        .expect("enabling plugin should succeed");
    assert!(
        !manager
            .clear_preferred_provider("alpha")
            .expect("clearing missing preference should succeed")
    );

    let state_path = config_root.join("plugins.json");
    let raw = std::fs::read_to_string(&state_path).expect("plugin state should be written");
    assert!(raw.contains("\"enabled\""));
    assert!(raw.contains("alpha"));

    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn corrupt_plugin_state_surfaces_for_result_apis_unit() {
    let root = make_temp_dir("osp-cli-plugin-manager-corrupt-state-result");
    let plugins_dir = root.join("plugins");
    let config_root = root.join("config");
    let cache_root = root.join("cache");
    std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");
    std::fs::create_dir_all(&config_root).expect("config dir should be created");
    write_named_test_plugin(&plugins_dir, "alpha");
    std::fs::write(config_root.join("plugins.json"), "{not-json\n")
        .expect("corrupt state should be written");

    let manager =
        PluginManager::new(vec![plugins_dir]).with_roots(Some(config_root), Some(cache_root));

    let err = manager
        .list_plugins()
        .expect_err("corrupt plugin state should fail");
    let rendered = err.to_string();
    assert!(rendered.contains("failed to parse plugin state"));
    assert!(rendered.contains("plugins.json"));
    assert!(rendered.contains("line 1, column 2"));

    let err = manager
        .set_enabled("alpha", false)
        .expect_err("mutating plugin state should fail");
    let rendered = err.to_string();
    assert!(rendered.contains("failed to parse plugin state"));
    assert!(rendered.contains("plugins.json"));
    assert!(rendered.contains("line 1, column 2"));

    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn corrupt_plugin_state_surfaces_for_dispatch_unit() {
    let root = make_temp_dir("osp-cli-plugin-manager-corrupt-state-dispatch");
    let plugins_dir = root.join("plugins");
    let config_root = root.join("config");
    let cache_root = root.join("cache");
    std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");
    std::fs::create_dir_all(&config_root).expect("config dir should be created");
    write_dispatch_fixture_plugin(
        &plugins_dir,
        "alpha",
        "alpha",
        r#"cat <<'JSON'
{"protocol_version":1,"ok":true,"data":{"message":"ok"},"error":null,"meta":{"format_hint":"table","columns":["message"]}}
JSON"#,
    );
    std::fs::write(config_root.join("plugins.json"), "{not-json\n")
        .expect("corrupt state should be written");

    let manager =
        PluginManager::new(vec![plugins_dir]).with_roots(Some(config_root), Some(cache_root));

    match manager
        .dispatch("alpha", &[], &PluginDispatchContext::default())
        .expect_err("dispatch should fail when plugin state is corrupt")
    {
        PluginDispatchError::StateLoadFailed { source } => {
            let rendered = source.to_string();
            assert!(rendered.contains("failed to parse plugin state"));
            assert!(rendered.contains("plugins.json"));
            assert!(rendered.contains("line 1, column 2"));
        }
        other => panic!("unexpected corrupt state dispatch result: {other:?}"),
    }

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn plugin_dispatch_context_merges_shared_and_plugin_env_pairs_unit() {
    let context = PluginDispatchContext {
        shared_env: vec![("OSP_FORMAT".to_string(), "json".to_string())],
        plugin_env: std::collections::HashMap::from([(
            "alpha".to_string(),
            vec![("OSP_PLUGIN_FLAG".to_string(), "1".to_string())],
        )]),
        ..PluginDispatchContext::default()
    };

    let pairs = context.env_pairs_for("alpha").collect::<Vec<_>>();
    assert_eq!(
        pairs,
        vec![("OSP_FORMAT", "json"), ("OSP_PLUGIN_FLAG", "1")]
    );
    assert_eq!(
        context.env_pairs_for("missing").collect::<Vec<_>>(),
        vec![("OSP_FORMAT", "json")]
    );
}

#[test]
fn plugin_dispatch_error_formats_cover_terminal_variants_unit() {
    let timeout_plain = PluginDispatchError::TimedOut {
        plugin_id: "alpha".to_string(),
        timeout: Duration::from_millis(25),
        stderr: String::new(),
    };
    assert!(
        timeout_plain
            .to_string()
            .contains("plugin alpha timed out after 25 ms")
    );

    let timeout_stderr = PluginDispatchError::TimedOut {
        plugin_id: "alpha".to_string(),
        timeout: Duration::from_millis(25),
        stderr: "stuck".to_string(),
    };
    assert!(timeout_stderr.to_string().contains("stuck"));

    let nonzero_plain = PluginDispatchError::NonZeroExit {
        plugin_id: "beta".to_string(),
        status_code: 9,
        stderr: String::new(),
    };
    assert_eq!(
        nonzero_plain.to_string(),
        "plugin beta exited with status 9"
    );

    let nonzero_stderr = PluginDispatchError::NonZeroExit {
        plugin_id: "beta".to_string(),
        status_code: 9,
        stderr: "boom".to_string(),
    };
    assert!(nonzero_stderr.to_string().contains("boom"));

    let ambiguous = PluginDispatchError::CommandAmbiguous {
        command: "shared".to_string(),
        providers: vec!["alpha".to_string(), "beta".to_string()],
    };
    assert!(ambiguous.to_string().contains("multiple plugins"));

    let provider_missing = PluginDispatchError::ProviderNotFound {
        command: "shared".to_string(),
        requested_provider: "gamma".to_string(),
        providers: vec!["alpha".to_string(), "beta".to_string()],
    };
    assert!(provider_missing.to_string().contains("available providers"));

    let execute_failed = PluginDispatchError::ExecuteFailed {
        plugin_id: "alpha".to_string(),
        source: std::io::Error::other("spawn failed"),
    };
    assert_eq!(
        execute_failed.source().map(|err| err.to_string()),
        Some("spawn failed".to_string())
    );

    let invalid_json = PluginDispatchError::InvalidJsonResponse {
        plugin_id: "alpha".to_string(),
        source: serde_json::from_str::<serde_json::Value>("not-json").expect_err("invalid"),
    };
    assert!(invalid_json.to_string().contains("invalid JSON response"));
    assert!(invalid_json.source().is_some());

    let invalid_payload = PluginDispatchError::InvalidResponsePayload {
        plugin_id: "alpha".to_string(),
        reason: "missing data".to_string(),
    };
    assert!(invalid_payload.to_string().contains("missing data"));
    assert!(invalid_payload.source().is_none());
}

#[test]
fn completion_words_collect_flags_and_backbone_commands_unit() {
    let spec = crate::completion::CommandSpec::new("ldap")
        .flag("--json", crate::completion::FlagNode::new())
        .subcommand(
            crate::completion::CommandSpec::new("user")
                .subcommand(crate::completion::CommandSpec::new("show")),
        );

    let words = collect_completion_words(&spec);
    assert!(words.contains(&"ldap".to_string()));
    assert!(words.contains(&"--json".to_string()));
    assert!(words.contains(&"user".to_string()));
    assert!(words.contains(&"show".to_string()));

    let manager = PluginManager::new(Vec::new());
    assert_eq!(
        manager
            .completion_words()
            .expect("backbone completion words should render"),
        vec![
            "F".to_string(),
            "P".to_string(),
            "V".to_string(),
            "exit".to_string(),
            "help".to_string(),
            "quit".to_string(),
            "|".to_string(),
        ]
    );
    assert!(
        manager
            .repl_help_text()
            .expect("empty help should render")
            .contains("No plugin commands available.")
    );
}

#[test]
fn describe_command_helpers_preserve_nested_completion_metadata_unit() {
    let suggestion = DescribeSuggestionV1 {
        value: "json".to_string(),
        meta: Some("format".to_string()),
        display: Some("JSON".to_string()),
        sort: Some("01".to_string()),
    };
    let command = DescribeCommandV1 {
        name: "ldap".to_string(),
        about: "lookup users".to_string(),
        auth: None,
        args: vec![DescribeArgV1 {
            name: Some("uid".to_string()),
            about: Some("user id".to_string()),
            multi: true,
            value_type: Some(crate::core::plugin::DescribeValueTypeV1::Path),
            suggestions: vec![suggestion.clone()],
        }],
        flags: std::collections::BTreeMap::from([(
            "--format".to_string(),
            DescribeFlagV1 {
                about: Some("output format".to_string()),
                flag_only: false,
                multi: true,
                value_type: Some(crate::core::plugin::DescribeValueTypeV1::Path),
                suggestions: vec![suggestion.clone()],
            },
        )]),
        subcommands: vec![DescribeCommandV1 {
            name: "user".to_string(),
            about: String::new(),
            auth: None,
            args: Vec::new(),
            flags: Default::default(),
            subcommands: Vec::new(),
        }],
    };

    let spec = to_command_spec(&command);
    assert_eq!(spec.name, "ldap");
    assert_eq!(spec.tooltip.as_deref(), Some("lookup users"));
    assert_eq!(direct_subcommand_names(&spec), vec!["user".to_string()]);
    assert!(collect_completion_words(&spec).contains(&"--format".to_string()));

    let arg = to_arg_node(&command.args[0]);
    assert_eq!(arg.name.as_deref(), Some("uid"));
    assert_eq!(arg.tooltip.as_deref(), Some("user id"));
    assert!(arg.multi);
    assert_eq!(arg.value_type, Some(crate::completion::ValueType::Path));

    let flag = to_flag_node(command.flags.get("--format").expect("flag"));
    assert_eq!(flag.tooltip.as_deref(), Some("output format"));
    assert!(flag.multi);
    assert_eq!(flag.value_type, Some(crate::completion::ValueType::Path));

    let entry = to_suggestion_entry(&suggestion);
    assert_eq!(entry.value, "json");
    assert_eq!(entry.display.as_deref(), Some("JSON"));
    assert_eq!(
        to_value_type(crate::core::plugin::DescribeValueTypeV1::Path),
        Some(crate::completion::ValueType::Path)
    );
}

#[test]
fn cache_and_issue_helpers_cover_update_lookup_and_prune_unit() {
    let describe = DescribeV1 {
        protocol_version: 1,
        plugin_id: "demo".to_string(),
        plugin_version: "0.1.0".to_string(),
        min_osp_version: None,
        commands: Vec::new(),
    };
    let mut cache = DescribeCacheFile::default();
    upsert_cached_describe(
        &mut cache,
        "/tmp/demo".to_string(),
        10,
        20,
        30,
        describe.clone(),
    );
    assert_eq!(cache.entries.len(), 1);
    assert!(find_cached_describe(&cache, "/tmp/demo", 10, 20, 30).is_some());

    upsert_cached_describe(&mut cache, "/tmp/demo".to_string(), 11, 21, 31, describe);
    assert_eq!(cache.entries.len(), 1);
    assert!(find_cached_describe(&cache, "/tmp/demo", 11, 21, 31).is_some());

    cache.entries.push(DescribeCacheEntry {
        path: "/tmp/stale".to_string(),
        size: 1,
        mtime_secs: 1,
        mtime_nanos: 1,
        describe: DescribeV1 {
            protocol_version: 1,
            plugin_id: "stale".to_string(),
            plugin_version: "0.1.0".to_string(),
            min_osp_version: None,
            commands: Vec::new(),
        },
    });
    assert!(prune_stale_describe_cache_entries(
        &mut cache,
        &std::collections::HashSet::from(["/tmp/demo".to_string()])
    ));
    assert_eq!(cache.entries.len(), 1);

    let mut issue = None;
    merge_issue(&mut issue, String::new());
    assert_eq!(issue, None);
    merge_issue(&mut issue, "first".to_string());
    merge_issue(&mut issue, "second".to_string());
    assert_eq!(issue.as_deref(), Some("first; second"));
}

#[cfg(unix)]
#[test]
fn search_root_and_checksum_helpers_cover_real_filesystem_paths_unit() {
    use std::os::unix::fs::PermissionsExt;

    let root = make_temp_dir("osp-cli-plugin-manager-fs-helpers");
    let dup = root.join("dup");
    let exec_dir = root.join("execs");
    std::fs::create_dir_all(&dup).expect("dup dir");
    std::fs::create_dir_all(&exec_dir).expect("exec dir");

    let exec_path = exec_dir.join("osp-good");
    std::fs::write(&exec_path, "#!/bin/sh\nexit 0\n").expect("exec written");
    let mut perms = std::fs::metadata(&exec_path)
        .expect("metadata")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&exec_path, perms).expect("chmod");

    std::fs::write(exec_dir.join("osp-bad.sh"), "echo nope\n").expect("ext fixture");
    std::fs::write(exec_dir.join("osp-Bad"), "echo nope\n").expect("suffix fixture");
    std::fs::write(exec_dir.join("not-osp"), "echo nope\n").expect("prefix fixture");

    let roots = existing_unique_search_roots(vec![
        SearchRoot {
            path: exec_dir.clone(),
            source: PluginSource::Explicit,
        },
        SearchRoot {
            path: exec_dir.clone(),
            source: PluginSource::Env,
        },
        SearchRoot {
            path: root.join("missing"),
            source: PluginSource::Path,
        },
    ]);
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].path, exec_dir);

    let executables = discover_root_executables(&roots[0].path);
    assert_eq!(executables, vec![exec_path.clone()]);
    assert!(has_valid_plugin_suffix("osp-good"));
    assert!(!has_valid_plugin_suffix("osp-Bad"));

    let checksum = file_sha256_hex(&exec_path).expect("checksum");
    assert_eq!(checksum.len(), 64);
    assert_eq!(
        normalize_checksum(&checksum.to_uppercase()).expect("normalized"),
        checksum
    );
    assert!(normalize_checksum("xyz").is_err());

    let (size, _, _) = file_fingerprint(&exec_path).expect("fingerprint");
    assert!(size > 0);

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn manifest_helpers_cover_not_bundled_missing_invalid_and_valid_paths_unit() {
    let root = make_temp_dir("osp-cli-plugin-manager-manifest-state");
    let bundled_dir = root.join("bundled");
    std::fs::create_dir_all(&bundled_dir).expect("bundled dir");

    assert!(matches!(
        load_manifest_state(&SearchRoot {
            path: bundled_dir.clone(),
            source: PluginSource::Explicit,
        }),
        ManifestState::NotBundled
    ));
    assert!(matches!(
        load_manifest_state(&SearchRoot {
            path: bundled_dir.clone(),
            source: PluginSource::Bundled,
        }),
        ManifestState::Missing
    ));
    assert_eq!(
        bundled_manifest_path(&SearchRoot {
            path: bundled_dir.clone(),
            source: PluginSource::Bundled,
        }),
        Some(bundled_dir.join("manifest.toml"))
    );

    let invalid = bundled_dir.join("invalid.toml");
    std::fs::write(
        &invalid,
        r#"
protocol_version = 2
[[plugin]]
id = "demo"
exe = "osp-demo"
version = "0.1.0"
commands = ["demo"]
"#,
    )
    .expect("invalid manifest written");
    assert!(matches!(
        load_manifest_state_from_path(&invalid),
        ManifestState::Invalid(message) if message.contains("unsupported manifest protocol_version")
    ));

    let valid = bundled_dir.join("valid.toml");
    std::fs::write(
        &valid,
        r#"
protocol_version = 1
[[plugin]]
id = "demo"
exe = "osp-demo"
version = "0.1.0"
enabled_by_default = false
commands = ["demo", "demo show"]
"#,
    )
    .expect("valid manifest written");

    match load_manifest_state_from_path(&valid) {
        ManifestState::Valid(manifest) => {
            let demo = manifest.by_exe.get("osp-demo").expect("manifest entry");
            assert_eq!(demo.id, "demo");
            assert!(!demo.enabled_by_default);
        }
        ManifestState::NotBundled | ManifestState::Missing | ManifestState::Invalid(_) => {
            panic!("expected valid manifest")
        }
    }

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn bundled_plugins_skip_describe_when_manifest_is_missing_unit() {
    let root = make_temp_dir("osp-cli-plugin-manager-bundled-missing-manifest");
    let marker = root.join("describe.log");
    let executable = write_marker_describe_plugin(&root, "bundled", &marker);
    let mut describe_cache = DescribeCacheFile::default();
    let mut seen = std::collections::HashSet::new();
    let mut cache_dirty = false;

    let plugin = assemble_discovered_plugin(
        PluginSource::Bundled,
        executable,
        &ManifestState::Missing,
        &mut describe_cache,
        &mut seen,
        &mut cache_dirty,
        Duration::from_millis(100),
    );

    assert!(
        plugin
            .issue
            .as_deref()
            .is_some_and(|issue| issue.contains("manifest.toml"))
    );
    assert!(
        !marker.exists(),
        "bundled plugin should not execute without a manifest"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn bundled_plugins_skip_describe_when_checksum_mismatches_unit() {
    let root = make_temp_dir("osp-cli-plugin-manager-bundled-checksum");
    let marker = root.join("describe.log");
    let executable = write_marker_describe_plugin(&root, "bundled", &marker);
    let mut describe_cache = DescribeCacheFile::default();
    let mut seen = std::collections::HashSet::new();
    let mut cache_dirty = false;
    let manifest = ValidatedBundledManifest {
        by_exe: HashMap::from([(
            "osp-bundled".to_string(),
            ManifestPlugin {
                id: "bundled".to_string(),
                exe: "osp-bundled".to_string(),
                version: "0.1.0".to_string(),
                enabled_by_default: true,
                checksum_sha256: Some("0".repeat(64)),
                commands: vec!["bundled".to_string()],
            },
        )]),
    };

    let plugin = assemble_discovered_plugin(
        PluginSource::Bundled,
        executable,
        &ManifestState::Valid(manifest),
        &mut describe_cache,
        &mut seen,
        &mut cache_dirty,
        Duration::from_millis(100),
    );

    assert!(
        plugin
            .issue
            .as_deref()
            .is_some_and(|issue| issue.contains("checksum mismatch"))
    );
    assert!(
        !marker.exists(),
        "bundled plugin should not execute before checksum validation passes"
    );

    let _ = std::fs::remove_dir_all(&root);
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
    let preferred_help = manager
        .repl_help_text()
        .expect("preferred provider help should render");
    assert!(preferred_help.contains("shared - beta plugin (beta/explicit)"));

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn dispatch_surfaces_nonzero_invalid_json_invalid_payload_and_passthrough_unit() {
    let _lock = env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = make_temp_dir("osp-cli-plugin-manager-dispatch-errors");
    let plugins_dir = root.join("plugins");
    std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");

    write_dispatch_fixture_plugin(
        &plugins_dir,
        "fail",
        "fail",
        r#"printf 'nope\n' >&2; exit 9"#,
    );
    write_dispatch_fixture_plugin(
        &plugins_dir,
        "bad-json",
        "bad-json",
        r#"printf 'not-json\n'"#,
    );
    write_dispatch_fixture_plugin(
        &plugins_dir,
        "bad-payload",
        "bad-payload",
        r#"cat <<'JSON'
{"protocol_version":1,"ok":true,"data":{"message":"ok"},"error":{"code":"broken","message":"boom"},"meta":{"format_hint":"table","columns":["message"]}}
JSON"#,
    );
    write_dispatch_fixture_plugin(
        &plugins_dir,
        "raw",
        "raw",
        r#"cat <<'JSON'
{"protocol_version":1,"ok":true,"data":{"message":"ok"},"error":null,"meta":{"format_hint":"table","columns":["message"]}}
JSON"#,
    );

    let manager = PluginManager::new(vec![plugins_dir.clone()]);

    match manager
        .dispatch("fail", &[], &PluginDispatchContext::default())
        .expect_err("non-zero exit should surface")
    {
        PluginDispatchError::NonZeroExit {
            plugin_id,
            status_code,
            stderr,
        } => {
            assert_eq!(plugin_id, "fail");
            assert_eq!(status_code, 9);
            assert!(stderr.contains("nope"));
        }
        other => panic!("unexpected non-zero result: {other:?}"),
    }

    match manager
        .dispatch("bad-json", &[], &PluginDispatchContext::default())
        .expect_err("invalid json should surface")
    {
        PluginDispatchError::InvalidJsonResponse { plugin_id, .. } => {
            assert_eq!(plugin_id, "bad-json");
        }
        other => panic!("unexpected invalid json result: {other:?}"),
    }

    match manager
        .dispatch("bad-payload", &[], &PluginDispatchContext::default())
        .expect_err("invalid payload should surface")
    {
        PluginDispatchError::InvalidResponsePayload { plugin_id, reason } => {
            assert_eq!(plugin_id, "bad-payload");
            assert!(reason.contains("ok=true requires error=null"));
        }
        other => panic!("unexpected invalid payload result: {other:?}"),
    }

    let raw = manager
        .dispatch_passthrough("raw", &[], &PluginDispatchContext::default())
        .expect("passthrough should succeed");
    assert_eq!(raw.status_code, 0);
    assert!(raw.stdout.contains("\"message\":\"ok\""));

    let missing = manager
        .dispatch_passthrough("missing", &[], &PluginDispatchContext::default())
        .expect_err("missing command should fail");
    assert!(matches!(
        missing,
        PluginDispatchError::CommandNotFound { command } if command == "missing"
    ));

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn path_discovery_is_opt_in_unit() {
    let root = make_temp_dir("osp-cli-plugin-manager-path-discovery");
    let _lock = env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let original_path = std::env::var("PATH").ok();
    let plugins_dir = root.join("plugins");
    std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");
    write_named_test_plugin(&plugins_dir, "pathdemo");

    unsafe {
        std::env::set_var(
            "PATH",
            format!(
                "{}:{}",
                plugins_dir.display(),
                original_path.as_deref().unwrap_or("")
            ),
        );
    }

    let hidden = PluginManager::new(Vec::new())
        .list_plugins()
        .expect("path-disabled manager should list plugins");
    assert!(!hidden.iter().any(|plugin| plugin.plugin_id == "pathdemo"));

    let visible = PluginManager::new(Vec::new())
        .with_path_discovery(true)
        .list_plugins()
        .expect("path-enabled manager should list plugins");
    assert!(visible.iter().any(|plugin| plugin.plugin_id == "pathdemo"));

    match original_path {
        Some(value) => unsafe { std::env::set_var("PATH", value) },
        None => unsafe { std::env::remove_var("PATH") },
    }

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn describe_plugin_reports_missing_executable_unit() {
    let root = make_temp_dir("osp-cli-plugin-manager-missing-describe");
    let missing = root.join("osp-missing");

    let err = describe_plugin(&missing, Duration::from_millis(50))
        .expect_err("missing executable should fail");
    assert!(err.to_string().contains("failed to execute --describe"));

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn describe_plugin_and_run_provider_cover_direct_error_paths_unit() {
    use std::os::unix::fs::PermissionsExt;

    let root = make_temp_dir("osp-cli-plugin-manager-direct-dispatch-errors");
    let nonzero = root.join("osp-nonzero");
    let invalid_json = root.join("osp-invalid-json");
    let invalid_payload = root.join("osp-invalid-payload");

    std::fs::write(
        &nonzero,
        "#!/bin/sh\nPATH=/usr/bin:/bin\nif [ \"$1\" = \"--describe\" ]; then echo nope >&2; exit 7; fi\n",
    )
    .expect("fixture should be written");
    std::fs::write(
        &invalid_json,
        "#!/bin/sh\nPATH=/usr/bin:/bin\nif [ \"$1\" = \"--describe\" ]; then printf 'not-json\\n'; exit 0; fi\n",
    )
    .expect("fixture should be written");
    std::fs::write(
        &invalid_payload,
        "#!/bin/sh\nPATH=/usr/bin:/bin\nif [ \"$1\" = \"--describe\" ]; then cat <<'JSON'\n{\"protocol_version\":1,\"plugin_id\":\"\",\"plugin_version\":\"0.1.0\",\"commands\":[]}\nJSON\nexit 0\nfi\n",
    )
    .expect("fixture should be written");

    for path in [&nonzero, &invalid_json, &invalid_payload] {
        let mut perms = std::fs::metadata(path).expect("metadata").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).expect("chmod");
    }

    let err = describe_plugin(&nonzero, Duration::from_millis(50))
        .expect_err("non-zero describe should fail");
    assert!(err.to_string().contains("--describe failed with status"));
    assert!(err.to_string().contains("nope"));

    let err = describe_plugin(&invalid_json, Duration::from_millis(50))
        .expect_err("invalid json should fail");
    assert!(err.to_string().contains("invalid describe JSON"));

    let err = describe_plugin(&invalid_payload, Duration::from_millis(50))
        .expect_err("invalid payload should fail");
    assert!(err.to_string().contains("invalid describe payload"));

    let provider = DiscoveredPlugin {
        plugin_id: "missing".to_string(),
        plugin_version: None,
        executable: root.join("osp-missing-run"),
        source: PluginSource::Explicit,
        commands: vec!["missing".to_string()],
        describe_commands: Vec::new(),
        command_specs: Vec::new(),
        issue: None,
        default_enabled: true,
    };
    let err = run_provider(
        &provider,
        "missing",
        &[],
        &PluginDispatchContext::default(),
        Duration::from_millis(50),
    )
    .expect_err("missing executable should fail");
    assert!(matches!(
        err,
        PluginDispatchError::ExecuteFailed { plugin_id, .. } if plugin_id == "missing"
    ));

    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
fn make_temp_dir(prefix: &str) -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time should be valid")
        .as_nanos();
    dir.push(format!("{prefix}-{nonce}"));
    std::fs::create_dir_all(&dir).expect("temp dir should be created");
    dir
}

#[cfg(unix)]
fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(unix)]
fn write_named_test_plugin(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    write_named_test_plugin_with_min_version(dir, name, "0.1.0")
}

#[cfg(unix)]
fn write_named_test_plugin_with_min_version(
    dir: &std::path::Path,
    name: &str,
    min_osp_version: &str,
) -> std::path::PathBuf {
    let plugin_path = dir.join(format!("osp-{name}"));
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{name}","plugin_version":"0.1.0","min_osp_version":"{min_osp_version}","commands":[{{"name":"{name}","about":"{name} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"ok"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
        name = name,
        min_osp_version = min_osp_version
    );

    write_executable_script_atomically(&plugin_path, &script);
    plugin_path
}

#[cfg(unix)]
fn write_provider_test_plugin(
    dir: &std::path::Path,
    plugin_id: &str,
    command_name: &str,
) -> std::path::PathBuf {
    let plugin_path = dir.join(format!("osp-{plugin_id}"));
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{plugin_id}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{command_name}","about":"{plugin_id} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"ok"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
        plugin_id = plugin_id,
        command_name = command_name
    );

    write_executable_script_atomically(&plugin_path, &script);
    plugin_path
}

#[cfg(unix)]
fn write_auth_test_plugin(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let plugin_path = dir.join(format!("osp-{name}"));
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{name}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{name}","about":"{name} plugin","auth":{{"visibility":"authenticated"}},"args":[],"flags":{{}},"subcommands":[{{"name":"approval","about":"approval commands","args":[],"flags":{{}},"subcommands":[{{"name":"decide","about":"decide approvals","auth":{{"visibility":"capability_gated","required_capabilities":["orch.approval.decide"],"feature_flags":["orch"]}},"args":[],"flags":{{}},"subcommands":[]}}]}}]}}]}}
JSON
  exit 0
fi

cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"ok"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
        name = name,
    );

    write_executable_script_atomically(&plugin_path, &script);
    plugin_path
}

#[cfg(unix)]
fn write_sleepy_test_plugin(
    dir: &std::path::Path,
    name: &str,
    sleep_on_describe: bool,
) -> std::path::PathBuf {
    let plugin_path = dir.join(format!("osp-{name}"));
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin
if [ "$1" = "--describe" ]; then
  if [ "{sleep_on_describe}" = "true" ]; then
    sleep 1
  fi
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{name}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{name}","about":"{name} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

sleep 1
cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"ok"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
        name = name,
        sleep_on_describe = if sleep_on_describe { "true" } else { "false" }
    );

    write_executable_script_atomically(&plugin_path, &script);
    plugin_path
}

#[cfg(unix)]
fn write_timeout_leak_test_plugin(
    dir: &std::path::Path,
    name: &str,
    marker: &std::path::Path,
) -> std::path::PathBuf {
    let plugin_path = dir.join(format!("osp-{name}"));
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{name}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{name}","about":"{name} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

(sleep 0.2; touch "{marker}") &
sleep 1
"#,
        name = name,
        marker = marker.display(),
    );

    write_executable_script_atomically(&plugin_path, &script);
    plugin_path
}

#[cfg(unix)]
fn write_large_output_test_plugin(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let plugin_path = dir.join(format!("osp-{name}"));
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{name}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{name}","about":"{name} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

printf '{{"protocol_version":1,"ok":true,"data":{{"blob":"'
head -c 131072 /dev/zero | tr '\0' 'x'
printf '"}},"error":null,"meta":{{"format_hint":"table","columns":["blob"]}}}}'
"#,
        name = name,
    );

    write_executable_script_atomically(&plugin_path, &script);
    plugin_path
}

#[cfg(unix)]
fn write_mismatched_id_plugin(
    dir: &std::path::Path,
    file_stem: &str,
    describe_id: &str,
    command_name: &str,
) -> std::path::PathBuf {
    let plugin_path = dir.join(format!("osp-{file_stem}"));
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{describe_id}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{command_name}","about":"{file_stem} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"ok"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
        file_stem = file_stem,
        describe_id = describe_id,
        command_name = command_name,
    );

    write_executable_script_atomically(&plugin_path, &script);
    plugin_path
}

#[cfg(unix)]
fn write_marker_describe_plugin(
    dir: &std::path::Path,
    name: &str,
    marker: &std::path::Path,
) -> std::path::PathBuf {
    let plugin_path = dir.join(format!("osp-{name}"));
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin
if [ "$1" = "--describe" ]; then
  touch "{marker}"
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{name}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{name}","about":"{name} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"ok"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
        marker = marker.display(),
        name = name,
    );

    write_executable_script_atomically(&plugin_path, &script);
    plugin_path
}

#[cfg(unix)]
fn write_dispatch_fixture_plugin(
    dir: &std::path::Path,
    plugin_id: &str,
    command_name: &str,
    body: &str,
) -> std::path::PathBuf {
    let plugin_path = dir.join(format!("osp-{plugin_id}"));
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{plugin_id}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{command_name}","about":"{plugin_id} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

{body}
"#,
        plugin_id = plugin_id,
        command_name = command_name,
        body = body
    );

    write_executable_script_atomically(&plugin_path, &script);
    plugin_path
}

#[cfg(unix)]
fn write_executable_script_atomically(path: &std::path::Path, script: &str) {
    use std::fs::File;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    let tmp_path = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp_path);
    let mut file = File::create(&tmp_path).expect("temp plugin should be created");
    file.write_all(script.as_bytes())
        .expect("plugin should be written");
    file.sync_all().expect("temp plugin should be flushed");

    let mut perms = file
        .metadata()
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    file.set_permissions(perms)
        .expect("temp plugin should be executable");
    drop(file);

    // Publish the executable in one rename step so discovery never races a partially
    // written script. This keeps the tests from manufacturing ETXTBSY on CI.
    std::fs::rename(&tmp_path, path).expect("plugin should be installed atomically");
}
