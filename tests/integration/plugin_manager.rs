#[cfg(unix)]
use crate::temp_support::make_temp_dir;
#[cfg(unix)]
use osp_cli::core::command_policy::{CommandPath, VisibilityMode};
#[cfg(unix)]
use osp_cli::plugin::{PluginManager, PluginSource};

#[cfg(unix)]
fn write_executable_script(path: &std::path::Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::write(path, script).expect("plugin script should be written");
    let mut perms = std::fs::metadata(path)
        .expect("plugin metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).expect("plugin script should be executable");
}

#[cfg(unix)]
fn write_provider_plugin(dir: &std::path::Path, plugin_id: &str, command_name: &str) {
    let plugin_path = dir.join(format!("osp-{plugin_id}"));
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
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
        command_name = command_name,
    );
    write_executable_script(&plugin_path, &script);
}

#[cfg(unix)]
fn write_auth_plugin(dir: &std::path::Path, plugin_id: &str) {
    let plugin_path = dir.join(format!("osp-{plugin_id}"));
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{plugin_id}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{plugin_id}","about":"{plugin_id} plugin","auth":{{"visibility":"authenticated"}},"args":[],"flags":{{}},"subcommands":[{{"name":"approval","about":"approval commands","args":[],"flags":{{}},"subcommands":[{{"name":"decide","about":"decide approvals","auth":{{"visibility":"capability_gated","required_capabilities":["{plugin_id}.approval.decide"],"feature_flags":["{plugin_id}"]}},"args":[],"flags":{{}},"subcommands":[]}}]}}]}}]}}
JSON
  exit 0
fi

cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"ok"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
        plugin_id = plugin_id,
    );
    write_executable_script(&plugin_path, &script);
}

#[cfg(unix)]
#[test]
fn plugin_manager_surfaces_provider_selection_across_catalog_help_and_completion() {
    let root = make_temp_dir("osp-cli-plugin-manager-integration-selection");
    let plugins_dir = root.join("plugins");
    std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");

    write_provider_plugin(&plugins_dir, "alpha", "shared");
    write_provider_plugin(&plugins_dir, "beta", "shared");
    let manager = PluginManager::new(vec![plugins_dir]);

    let mut providers = manager
        .command_providers("shared")
        .expect("provider labels should load");
    providers.sort();
    assert_eq!(
        providers,
        vec![
            "alpha (explicit)".to_string(),
            "beta (explicit)".to_string()
        ]
    );

    let ambiguous_catalog = manager.command_catalog().expect("catalog should load");
    let ambiguous_entry = ambiguous_catalog
        .iter()
        .find(|entry| entry.name == "shared")
        .expect("shared command should exist");
    assert_eq!(ambiguous_entry.provider, None);
    assert!(ambiguous_entry.conflicted);
    assert!(ambiguous_entry.requires_selection);
    assert!(!ambiguous_entry.selected_explicitly);
    assert!(
        ambiguous_entry
            .about
            .contains("provider selection required")
    );
    assert_eq!(
        manager
            .selected_provider_label("shared")
            .expect("selected provider label should load"),
        None
    );

    let ambiguous_help = manager.repl_help_text().expect("help text should render");
    assert!(ambiguous_help.contains("Plugin commands:"));
    assert!(ambiguous_help.contains("shared"));
    assert!(ambiguous_help.contains("providers: alpha (explicit), beta (explicit)"));

    let words = manager
        .completion_words()
        .expect("completion words should render");
    assert!(words.contains(&"shared".to_string()));
    assert!(words.contains(&"help".to_string()));
    assert!(words.contains(&"|".to_string()));

    let doctor = manager.doctor().expect("doctor report should load");
    assert!(
        doctor
            .conflicts
            .iter()
            .any(|conflict| conflict.command == "shared"
                && conflict.providers.contains(&"alpha (explicit)".to_string())
                && conflict.providers.contains(&"beta (explicit)".to_string()))
    );

    manager
        .set_preferred_provider("shared", "beta")
        .expect("preferred provider should be saved");

    let selected_catalog = manager.command_catalog().expect("catalog should reload");
    let selected_entry = selected_catalog
        .iter()
        .find(|entry| entry.name == "shared")
        .expect("shared command should exist");
    assert_eq!(selected_entry.provider.as_deref(), Some("beta"));
    assert_eq!(selected_entry.source, Some(PluginSource::Explicit));
    assert!(selected_entry.conflicted);
    assert!(!selected_entry.requires_selection);
    assert!(selected_entry.selected_explicitly);
    assert_eq!(
        manager
            .selected_provider_label("shared")
            .expect("selected provider label should load")
            .as_deref(),
        Some("beta (explicit)")
    );

    let selected_help = manager.repl_help_text().expect("help text should reload");
    assert!(selected_help.contains("shared - beta plugin"));
    assert!(selected_help.contains("(beta/explicit)"));
    assert!(selected_help.contains("conflicts: alpha (explicit), beta (explicit)"));

    assert!(
        manager
            .clear_preferred_provider("shared")
            .expect("preferred provider should clear")
    );
    assert_eq!(
        manager
            .selected_provider_label("shared")
            .expect("selected provider label should load after clear"),
        None
    );
}

#[cfg(unix)]
#[test]
fn plugin_manager_projects_recursive_auth_metadata_into_catalog_and_policy_registry() {
    let root = make_temp_dir("osp-cli-plugin-manager-integration-policy");
    let plugins_dir = root.join("plugins");
    std::fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");

    write_auth_plugin(&plugins_dir, "orch");
    let manager = PluginManager::new(vec![plugins_dir]);

    let catalog = manager.command_catalog().expect("catalog should load");
    let orch = catalog
        .iter()
        .find(|entry| entry.name == "orch")
        .expect("orch command should exist");
    assert_eq!(orch.auth_hint().as_deref(), Some("auth"));
    assert_eq!(orch.subcommands, vec!["approval".to_string()]);

    let help = manager.repl_help_text().expect("help text should render");
    assert!(help.contains("orch [approval] - orch plugin [auth] (orch/explicit)"));

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
}
