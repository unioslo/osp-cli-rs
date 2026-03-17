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
        host.contains("prepare_startup_host("),
        "host startup should route startup assembly through the bootstrap/assembly seam"
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
fn ui_runtime_rendering_flows_through_plan_lower_emit_without_document_fallbacks() {
    let settings = read_source("src/ui/settings/mod.rs");
    let plan = read_source("src/ui/plan/mod.rs");
    let lower = read_source("src/ui/lower.rs");
    let emit = read_source("src/ui/emit/mod.rs");
    let ui = read_source("src/ui/mod.rs");

    assert!(
        settings.contains("pub struct ResolvedRenderSettings")
            && settings.contains("pub fn resolve_render_settings(&self) -> ResolvedRenderSettings")
            && settings.contains(
                "pub(crate) fn resolve_output_format(&self, output: &OutputResult) -> OutputFormat"
            ),
        "ui settings should own resolved render facts and output-format resolution"
    );
    assert!(
        plan.contains("pub fn plan_output(")
            && plan.contains("let format = settings.resolve_output_format(output);"),
        "ui planning should own the semantic render plan entrypoint"
    );
    assert!(
        lower.contains("fn lower_output(") && lower.contains("RenderPlan"),
        "ui lowering should consume the semantic render plan"
    );
    assert!(
        emit.contains("pub fn emit_doc(") && emit.contains("OutputFormat::Markdown"),
        "ui emitter should stay behind a single document emission seam"
    );
    assert!(
        ui.contains("render_output_with_profile(")
            && ui.contains("plan_output(output, settings, profile)")
            && ui.contains("emit::emit_doc(")
            && ui.contains("&lower::lower_output(output, &plan)")
            && ui.contains("plan.format")
            && ui.contains("&plan.settings")
            && !ui.contains("document_render::render_document(")
            && !ui.contains("build_document_from_output_plan"),
        "ui runtime should flow through the canonical plan/lower/emit path without document fallback"
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
