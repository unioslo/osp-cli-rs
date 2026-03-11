#[cfg(unix)]
#[test]
fn timeout_dispatch_covers_timeout_fields_and_process_cleanup_unit() {
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

    let leak_root = make_temp_dir("osp-cli-plugin-manager-timeout-process-group");
    let leak_plugins_dir = leak_root.join("plugins");
    let marker = leak_root.join("leaked-child.txt");
    std::fs::create_dir_all(&leak_plugins_dir).expect("plugin dir should be created");

    write_timeout_leak_test_plugin(&leak_plugins_dir, "hang", &marker);
    let leak_manager = PluginManager::new(vec![leak_plugins_dir.clone()])
        .with_process_timeout(Duration::from_millis(50));

    let err = leak_manager
        .dispatch("hang", &[], &PluginDispatchContext::default())
        .expect_err("hung plugin should time out");
    assert!(matches!(err, PluginDispatchError::TimedOut { .. }));

    std::thread::sleep(Duration::from_millis(350));
    assert!(
        !marker.exists(),
        "timed-out plugin left a background child behind"
    );
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
}

#[test]
fn plugin_dispatch_context_and_error_formats_cover_local_helper_paths_unit() {
    let context = PluginDispatchContext::new(crate::core::runtime::RuntimeHints::default())
        .with_shared_env([("OSP_FORMAT", "json")])
        .with_plugin_env(std::collections::HashMap::from([(
            "alpha".to_string(),
            vec![("OSP_PLUGIN_FLAG".to_string(), "1".to_string())],
        )]));

    let pairs = context.env_pairs_for("alpha").collect::<Vec<_>>();
    assert_eq!(
        pairs,
        vec![("OSP_FORMAT", "json"), ("OSP_PLUGIN_FLAG", "1")]
    );
    assert_eq!(
        context.env_pairs_for("missing").collect::<Vec<_>>(),
        vec![("OSP_FORMAT", "json")]
    );

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

#[cfg(unix)]
#[test]
fn describe_plugin_and_provider_error_paths_cover_missing_nonzero_invalid_and_execute_failed_unit() {
    let _lock = env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = make_temp_dir("osp-cli-plugin-manager-missing-describe");
    let missing = root.join("osp-missing");

    let err = describe_plugin(&missing, Duration::from_millis(50))
        .expect_err("missing executable should fail");
    assert!(err.to_string().contains("failed to execute --describe"));
    use std::os::unix::fs::PermissionsExt;
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

}
