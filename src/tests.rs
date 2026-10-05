use crate::core::output::OutputFormat;
use std::ffi::OsStr;
use std::ops::Deref;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

pub(crate) fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Owns a test temp directory and removes it when dropped.
///
/// Tests should keep this alive for as long as any child paths are in use
/// instead of discarding ownership and leaking a raw `PathBuf`.
#[must_use = "keep the temp dir alive for as long as child paths are used"]
pub(crate) struct TestTempDir(tempfile::TempDir);

impl TestTempDir {
    pub(crate) fn new(prefix: &str) -> Self {
        let dir = tempfile::Builder::new()
            .prefix(prefix)
            .tempdir()
            .expect("temp dir should be created");
        Self(dir)
    }

    pub(crate) fn path(&self) -> &Path {
        self.0.path()
    }
}

impl AsRef<Path> for TestTempDir {
    fn as_ref(&self) -> &Path {
        self.path()
    }
}

impl AsRef<OsStr> for TestTempDir {
    fn as_ref(&self) -> &OsStr {
        self.path().as_os_str()
    }
}

impl Deref for TestTempDir {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        self.path()
    }
}

pub(crate) fn make_temp_dir(prefix: &str) -> TestTempDir {
    TestTempDir::new(prefix)
}

#[test]
fn stable_top_level_surface_exposes_primary_entrypoints_and_types_unit() {
    let mut sink = crate::app::BufferedUiSink::default();
    let app = crate::App::builder().build();
    assert_eq!(app.run_process_with_sink(["osp", "--help"], &mut sink), 0);
    assert!(sink.stdout.contains("Usage"));
    assert!(sink.stderr.is_empty());
    sink.stdout.clear();
    let mut runner = crate::App::builder().build_with_sink(&mut sink);
    assert_eq!(
        runner
            .run_from(["osp", "--version"])
            .expect("version should run"),
        0
    );
    assert_eq!(sink.stdout, format!("osp {}\n", env!("CARGO_PKG_VERSION")));
    assert!(sink.stderr.is_empty());
    let _cli_type: Option<crate::cli::Cli> = None;
    let _row: crate::core::row::Row = Default::default();
    let _resolver: Option<crate::config::ConfigResolver> = None;
    let _completion: Option<crate::completion::CompletionEngine> = None;
    let _prompt: Option<crate::repl::ReplPrompt> = None;
    let _plugins: Option<crate::plugin::PluginManager> = None;
    let _app_runtime: Option<crate::app::AppRuntime> = None;
    let _format = OutputFormat::Json;
    let _settings = crate::ui::RenderSettings::test_plain(OutputFormat::Table);
}
