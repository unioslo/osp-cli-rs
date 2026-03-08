use crate::core::output::OutputFormat;

#[test]
fn stable_top_level_surface_exposes_primary_entrypoints_and_types_unit() {
    let _run_from = |args: Vec<&str>| crate::app::run_from::<Vec<&str>, &str>(args);
    let _run_process = |args: Vec<&str>| crate::app::run_process::<Vec<&str>, &str>(args);
    let mut sink = crate::app::BufferedUiSink::default();
    let _builder = crate::app::AppBuilder::new().build();
    let _runner = crate::app::AppBuilder::new().build_with_sink(&mut sink);
    let _cli_type: Option<crate::cli::Cli> = None;
    let _row: crate::core::Row = Default::default();
    let _resolver: Option<crate::config::resolve::ConfigResolver> = None;
    let _completion: Option<crate::completion::CompletionEngine> = None;
    let _prompt: Option<crate::repl::ReplPrompt> = None;
    let _ldap: Option<crate::api::MockLdapClient> = None;
    let _runtime: Option<crate::runtime::AppRuntime> = None;
    let _format = OutputFormat::Json;
    let _settings = crate::ui::RenderSettings::test_plain(OutputFormat::Table);
}

#[test]
fn legacy_osp_namespaces_still_exist_during_transition_unit() {
    let _settings = crate::osp_ui::RenderSettings::test_plain(OutputFormat::Table);
    let _format = crate::osp_core::output::OutputFormat::Json;
    let _cli_type: Option<crate::osp_cli::Cli> = None;
}
