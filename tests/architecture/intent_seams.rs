#![allow(missing_docs)]

use std::fs;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_source(relative: &str) -> String {
    let path = workspace_root().join(relative);
    fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
}

#[test]
fn app_runtime_rebuild_flows_share_the_host_assembly_boundary() {
    let host = read_source("src/app/host.rs");
    let rebuild = read_source("src/app/rebuild.rs");
    let lifecycle = read_source("src/app/repl_lifecycle.rs");

    assert!(
        host.contains("ResolvedHostInputs::derive("),
        "host startup should derive runtime inputs through app::assembly"
    );
    assert!(
        !host.contains("PluginManager::new(") && !host.contains("load_theme_catalog(&config)"),
        "host startup should not reassemble plugins/themes ad hoc"
    );
    assert!(
        rebuild.contains("ResolvedHostInputs::derive("),
        "rebuild path should derive runtime inputs through app::assembly"
    );
    assert!(
        !rebuild.contains("PluginManager::new(")
            && !rebuild.contains("load_theme_catalog(&config)")
            && !rebuild.contains("init_developer_logging(")
            && !rebuild.contains("build_logging_config("),
        "rebuild path drifted back into ad hoc assembly or side effects"
    );
    assert!(
        lifecycle.contains("apply_runtime_side_effects("),
        "rebuild lifecycle should own post-rebuild side effects"
    );
}

#[test]
fn ui_document_lowering_consumes_render_plans_instead_of_raw_settings() {
    let resolution = read_source("src/ui/resolution.rs");
    let format = read_source("src/ui/format/mod.rs");
    let ui = read_source("src/ui/mod.rs");

    assert!(
        resolution.contains("struct ResolvedRenderPlan"),
        "ui resolution should define a semantic render plan"
    );
    assert!(
        format.contains("ResolvedRenderPlan") && format.contains("build_document_from_output_plan"),
        "ui::format should lower through the render plan seam"
    );
    assert!(
        !format.contains("settings.resolve_guide_render_settings()")
            && !format.contains("settings.resolve_mreg_build_settings()")
            && !format.contains("build_document_from_output_resolved"),
        "ui::format drifted back to mixing raw settings with resolved rendering state"
    );
    assert!(
        ui.contains("resolve_render_plan(") && ui.contains("build_document_from_output_plan("),
        "ui render entrypoints should route through render plans before lowering"
    );
}

#[test]
fn plugin_manager_routes_read_paths_through_the_active_view() {
    let active = read_source("src/plugin/active.rs");
    let manager = read_source("src/plugin/manager.rs");
    let catalog = read_source("src/plugin/catalog.rs");

    assert!(
        active.contains("struct ActivePluginView"),
        "plugin layer should keep the shared active working set"
    );
    assert!(
        manager.contains("with_passive_view") && manager.contains("with_dispatch_view"),
        "plugin manager should route operations through shared view helpers"
    );
    assert!(
        !manager.contains("let healthy = healthy_plugins")
            && !manager.contains("healthy_plugins(discovered")
            && !manager.contains("resolve_provider_for_command(")
            && !manager.contains("provider_labels_by_command("),
        "plugin manager drifted back into ad hoc health/provider derivation"
    );
    assert!(
        catalog.contains("ActivePluginView"),
        "catalog building should consume the shared active-plugin view"
    );
}

#[test]
fn repl_engine_keeps_host_facing_config_types_out_of_editor_orchestration() {
    let engine = read_source("src/repl/engine.rs");
    let config = read_source("src/repl/engine/config.rs");

    assert!(
        engine.contains("mod config;") && engine.contains("pub use config::{"),
        "engine should re-export host-facing REPL config from a dedicated module"
    );
    assert!(
        !engine.contains("pub struct ReplRunConfig")
            && !engine.contains("pub struct ReplAppearance")
            && !engine.contains("pub enum ReplRunResult"),
        "engine.rs still owns host-facing REPL config/outcome types directly"
    );
    assert!(
        config.contains("pub struct ReplRunConfig")
            && config.contains("pub struct ReplAppearance")
            && config.contains("pub enum ReplRunResult"),
        "repl engine config surface should live in repl/engine/config.rs"
    );
}
