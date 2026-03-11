use crate::core::output::OutputFormat;
use std::sync::{Mutex, OnceLock};

pub(crate) fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[test]
fn stable_top_level_surface_exposes_primary_entrypoints_and_types_unit() {
    let _run_from = |args: Vec<&str>| crate::app::run_from::<Vec<&str>, &str>(args);
    let _run_process = |args: Vec<&str>| crate::app::run_process::<Vec<&str>, &str>(args);
    let mut sink = crate::app::BufferedUiSink::default();
    let _builder = crate::app::AppBuilder::new().build();
    let _runner = crate::app::AppBuilder::new().build_with_sink(&mut sink);
    let _cli_type: Option<crate::cli::Cli> = None;
    let _row: crate::core::row::Row = Default::default();
    let _resolver: Option<crate::config::ConfigResolver> = None;
    let _completion: Option<crate::completion::CompletionEngine> = None;
    let _prompt: Option<crate::repl::ReplPrompt> = None;
    let _plugins: Option<crate::plugin::PluginManager> = None;
    let _ldap: Option<crate::api::MockLdapClient> = None;
    let _app_runtime: Option<crate::app::AppRuntime> = None;
    let _format = OutputFormat::Json;
    let _settings = crate::ui::RenderSettings::test_plain(OutputFormat::Table);
}
