#[test]
fn min_osp_version_helper_covers_compatible_and_invalid_inputs_unit() {
    let describe = DescribeV1 {
        protocol_version: 1,
        plugin_id: "hello".to_string(),
        plugin_version: "0.1.0".to_string(),
        min_osp_version: Some("0.1.0".to_string()),
        commands: Vec::new(),
    };

    assert_eq!(min_osp_version_issue(&describe), None);

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

}

#[cfg(unix)]
#[test]
fn hung_describe_marks_plugin_unhealthy_unit() {
    let timeout_root = make_temp_dir("osp-cli-plugin-manager-describe-timeout");
    let timeout_plugins_dir = timeout_root.join("plugins");
    std::fs::create_dir_all(&timeout_plugins_dir).expect("plugin dir should be created");
    write_sleepy_test_plugin(&timeout_plugins_dir, "hang-describe", true);
    let timeout_manager = PluginManager::new(vec![timeout_plugins_dir.clone()])
        .with_process_timeout(Duration::from_millis(50));

    let timeout_plugins = timeout_manager.list_plugins().expect("plugins should list");
    assert_eq!(timeout_plugins.len(), 1);
    assert_eq!(timeout_plugins[0].plugin_id, "hang-describe");
    assert!(!timeout_plugins[0].healthy);
    assert!(
        timeout_plugins[0]
            .issue
            .as_deref()
            .expect("issue should be present")
            .contains("timed out after 50 ms")
    );
}

#[test]
fn duplicate_plugin_ids_keep_first_healthy_provider_and_shadow_later_copies_unit() {
    let mut plugins = vec![
        DiscoveredPlugin {
            plugin_id: "shared".to_string(),
            plugin_version: Some("0.1.0".to_string()),
            executable: std::path::PathBuf::from("/tmp/osp-shared-alpha"),
            source: PluginSource::Explicit,
            commands: vec!["alpha".to_string()],
            describe_commands: Vec::new(),
            command_specs: vec![crate::completion::CommandSpec::new("alpha")],
            issue: None,
            default_enabled: true,
        },
        DiscoveredPlugin {
            plugin_id: "shared".to_string(),
            plugin_version: Some("0.1.0".to_string()),
            executable: std::path::PathBuf::from("/tmp/osp-shared-beta"),
            source: PluginSource::Env,
            commands: vec!["beta".to_string()],
            describe_commands: Vec::new(),
            command_specs: vec![crate::completion::CommandSpec::new("beta")],
            issue: None,
            default_enabled: true,
        },
    ];

    mark_duplicate_plugin_ids(&mut plugins);

    assert!(plugins[0].issue.is_none());
    assert!(plugins[1]
        .issue
        .as_deref()
        .is_some_and(|issue| issue.contains("duplicate plugin id `shared` shadowed by /tmp/osp-shared-alpha")));
}

#[test]
fn duplicate_plugin_ids_fall_through_to_later_healthy_provider_when_earlier_copy_is_broken_unit() {
    let mut plugins = vec![
        DiscoveredPlugin {
            plugin_id: "shared".to_string(),
            plugin_version: Some("0.1.0".to_string()),
            executable: std::path::PathBuf::from("/tmp/osp-shared-alpha"),
            source: PluginSource::Explicit,
            commands: vec!["alpha".to_string()],
            describe_commands: Vec::new(),
            command_specs: vec![crate::completion::CommandSpec::new("alpha")],
            issue: Some("describe failed".to_string()),
            default_enabled: true,
        },
        DiscoveredPlugin {
            plugin_id: "shared".to_string(),
            plugin_version: Some("0.1.0".to_string()),
            executable: std::path::PathBuf::from("/tmp/osp-shared-beta"),
            source: PluginSource::Bundled,
            commands: vec!["beta".to_string()],
            describe_commands: Vec::new(),
            command_specs: vec![crate::completion::CommandSpec::new("beta")],
            issue: None,
            default_enabled: true,
        },
    ];

    mark_duplicate_plugin_ids(&mut plugins);

    assert!(plugins[1].issue.is_none());
    assert!(plugins[0]
        .issue
        .as_deref()
        .is_some_and(|issue| issue.contains("shadowed by /tmp/osp-shared-beta")));
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

}

#[cfg(unix)]
#[test]
fn bundled_plugins_skip_describe_until_manifest_requirements_pass_unit() {
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
    let checksum_root = make_temp_dir("osp-cli-plugin-manager-bundled-checksum");
    let checksum_marker = checksum_root.join("describe.log");
    let checksum_executable = write_marker_describe_plugin(&checksum_root, "bundled", &checksum_marker);
    let mut checksum_describe_cache = DescribeCacheFile::default();
    let mut checksum_seen = std::collections::HashSet::new();
    let mut checksum_cache_dirty = false;
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
    let checksum_plugin = assemble_discovered_plugin(
        PluginSource::Bundled,
        checksum_executable,
        &ManifestState::Valid(manifest),
        &mut checksum_describe_cache,
        &mut checksum_seen,
        &mut checksum_cache_dirty,
        Duration::from_millis(100),
    );
    assert!(
        checksum_plugin
            .issue
            .as_deref()
            .is_some_and(|issue| issue.contains("checksum mismatch"))
    );
    assert!(
        !checksum_marker.exists(),
        "bundled plugin should not execute before checksum validation passes"
    );
}

#[cfg(unix)]
#[test]
fn path_discovery_is_opt_in_and_uses_passive_cache_until_dispatch_unit() {
    let root = make_temp_dir("osp-cli-plugin-manager-path-discovery");
    let _lock = env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let original_path = std::env::var("PATH").ok();
    let plugins_dir = root.join("plugins");
    let config_root = root.join("config");
    let cache_root = root.join("cache");
    let probe_path = root.join("describe-probe");
    std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");
    write_path_probe_plugin(&plugins_dir, "pathdemo", &probe_path);

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
        .with_roots(Some(config_root), Some(cache_root))
        .with_path_discovery(true)
        .list_plugins()
        .expect("path-enabled manager should list plugins");
    let pathdemo = visible
        .iter()
        .find(|plugin| plugin.plugin_id == "pathdemo")
        .expect("path-discovered plugin should be visible");
    assert!(
        pathdemo
            .issue
            .as_deref()
            .is_some_and(|issue| issue.contains("passive discovery does not execute PATH plugins"))
    );
    assert!(
        !probe_path.exists(),
        "--describe should not run during passive discovery"
    );

    let manager = PluginManager::new(Vec::new())
        .with_roots(Some(root.join("config-2")), Some(root.join("cache-2")))
        .with_path_discovery(true);
    manager
        .dispatch("pathdemo", &[], &PluginDispatchContext::default())
        .expect("actual dispatch should resolve and run path plugin");
    assert!(
        probe_path.exists(),
        "--describe should run when resolving an invoked path plugin"
    );

    manager.refresh();
    let refreshed = manager.list_plugins().expect("cached plugins should list");
    let pathdemo = refreshed
        .iter()
        .find(|plugin| plugin.plugin_id == "pathdemo")
        .expect("path-discovered plugin should still be visible");
    assert!(
        pathdemo.issue.is_none(),
        "cached metadata should clear passive issue"
    );
    assert_eq!(pathdemo.commands, vec!["pathdemo".to_string()]);

    match original_path {
        Some(value) => unsafe { std::env::set_var("PATH", value) },
        None => unsafe { std::env::remove_var("PATH") },
    }

}
